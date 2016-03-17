// Copyright 2016 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

use rand::{self, Rng};
use std::cmp;
use std::collections::HashSet;
use std::sync::mpsc;
use xor_name::XorName;

use action::Action;
use authority::Authority;
use core::{Core, RoutingTable};
use data::{Data, ImmutableData, ImmutableDataType};
use error::InterfaceError;
use event::Event;
use id::FullId;
use kademlia_routing_table::{ContactInfo, GROUP_SIZE};
use messages::{RequestContent, RequestMessage};
use mock_crust::{self, Config, Endpoint, Network, ServiceHandle};
use types::{MessageId, RoutingActionSender};

struct TestNode {
    handle: ServiceHandle,
    core: Core,
    event_rx: mpsc::Receiver<Event>,
}

impl TestNode {
    fn new(network: &Network,
           client_restriction: bool,
           config: Option<Config>,
           endpoint: Option<Endpoint>)
           -> Self {
        let handle = network.new_service_handle(config, endpoint);
        let (event_tx, event_rx) = mpsc::channel();

        let (_, core) = mock_crust::make_current(&handle,
                                                 || Core::new(event_tx, client_restriction, None));

        TestNode {
            handle: handle,
            core: core,
            event_rx: event_rx,
        }
    }

    fn poll(&mut self) -> bool {
        let mut result = false;

        while self.core.poll() {
            result = true;
        }

        result
    }

    fn name(&self) -> &XorName {
        self.core.name()
    }

    fn close_group(&self) -> Vec<XorName> {
        self.core.close_group()
    }

    fn routing_table(&self) -> &RoutingTable {
        self.core.routing_table()
    }
}

struct TestClient {
    _handle: ServiceHandle,
    core: Core,
    event_rx: mpsc::Receiver<Event>,
    action_tx: RoutingActionSender,
}

impl TestClient {
    fn new(network: &Network,
           config: Option<Config>,
           endpoint: Option<Endpoint>)
           -> Self {
        let handle = network.new_service_handle(config, endpoint);
        let (event_tx, event_rx) = mpsc::channel();
        let full_id = FullId::new();

        let (action_tx, core) = mock_crust::make_current(&handle, || Core::new(event_tx, true, Some(full_id)));

        TestClient {
            _handle: handle,
            core: core,
            event_rx: event_rx,
            action_tx: action_tx,
        }
    }

    fn _poll(&mut self) -> bool {
        let mut result = false;

        while self.core.poll() {
            result = true;
        }

        result
    }

    fn name(&self) -> &XorName {
        self.core.name()
    }

    fn send_put_request(&self,
                        dst: Authority,
                        data: Data,
                        message_id: MessageId)
                        -> Result<(), InterfaceError> {
        self.send_action(RequestContent::Put(data, message_id), dst)
    }

    fn send_action(&self, content: RequestContent, dst: Authority) -> Result<(), InterfaceError> {
        let (interface_tx, interface_rx) = mpsc::channel();
        let action = Action::ClientSendRequest {
            content: content,
            dst: dst,
            result_tx: interface_tx.clone(),
        };

        try!(self.action_tx.send(action));
        try!(interface_rx.recv())
    }
}

/// Expect that the node raised an event matching the given pattern, panics if
/// not.
macro_rules! expect_event {
    ($node:expr, $pattern:pat) => {
        match $node.event_rx.try_recv() {
            Ok($pattern) => (),
            other => panic!("Expected Ok({}), got {:?}", stringify!($pattern), other),
        }
    }
}

/// Process all events
fn poll_all(nodes: &mut [TestNode]) {
    while nodes.iter_mut().any(TestNode::poll) {}
}

fn create_connected_nodes(network: &Network, size: usize) -> Vec<TestNode> {
    let mut nodes = Vec::new();

    // Create the seed node.
    nodes.push(TestNode::new(network, false, None, Some(Endpoint(0))));
    nodes[0].poll();

    let config = Config::with_contacts(&[nodes[0].handle.endpoint()]);

    // Create other nodes using the seed node endpoint as bootstrap contact.
    for i in 1..size {
        nodes.push(TestNode::new(network, false, Some(config.clone()), Some(Endpoint(i))));
        poll_all(&mut nodes);
    }

    let n = cmp::min(nodes.len(), GROUP_SIZE) - 1;

    for node in nodes.iter() {
        for _ in 0..n {
            expect_event!(node, Event::NodeAdded(..))
        }
    }

    nodes
}

// Drop node at index and verify its close group receives NodeLost.
fn drop_node(nodes: &mut Vec<TestNode>, index: usize) {
    let node = nodes.remove(index);
    let name = node.name().clone();
    let close_names = node.close_group();

    drop(node);

    poll_all(nodes);

    for node in nodes.iter().filter(|n| close_names.contains(n.name())) {
        loop {
            match node.event_rx.try_recv() {
                Ok(Event::NodeLost(lost_name)) if lost_name == name => break,
                Ok(_) => (),
                _ => panic!("Event::NodeLost({:?}) not received", name),
            }
        }
    }
}

// Get names of all entries in the `bucket_index`-th bucket in the routing table.
fn entry_names_in_bucket(table: &RoutingTable, bucket_index: usize) -> HashSet<XorName> {
    let our_name = table.our_name();
    let far_name = our_name.with_flipped_bit(bucket_index).unwrap();

    table.closest_nodes_to(&far_name, GROUP_SIZE, false)
         .into_iter()
         .map(|info| info.name().clone())
         .filter(|name| our_name.bucket_index(name) == bucket_index)
         .collect()
}

// Get names of all nodes that belong to the `index`-th bucket in the `name`s
// routing table.
fn node_names_in_bucket(nodes: &[TestNode],
                        name: &XorName,
                        bucket_index: usize)
                        -> HashSet<XorName> {
    nodes.iter()
         .filter(|node| name.bucket_index(node.name()) == bucket_index)
         .map(|node| node.name().clone())
         .collect()
}

// Verify that the kademlia invariant is upheld for the node at `index`.
fn verify_kademlia_invariant_for_node(nodes: &[TestNode], index: usize) {
    let node = &nodes[index];
    let mut count = nodes.len() - 1;
    let mut bucket_index = 0;

    while count > 0 {
        let entries = entry_names_in_bucket(node.routing_table(), bucket_index);
        let actual_bucket = node_names_in_bucket(nodes, node.name(), bucket_index);
        if entries.len() < GROUP_SIZE {
            assert_eq!(actual_bucket, entries);
        }
        count -= actual_bucket.len();
        bucket_index += 1;
    }
}

// Verify that the kademlia invariant is upheld for all nodes.
fn verify_kademlia_invariant_for_all_nodes(nodes: &[TestNode]) {
    for node_index in 0..nodes.len() {
        verify_kademlia_invariant_for_node(nodes, node_index);
    }
}

fn test_nodes(size: usize) {
    let network = Network::new();
    let nodes = create_connected_nodes(&network, size);
    verify_kademlia_invariant_for_all_nodes(&nodes);
}

#[test]
fn less_than_group_size_nodes() {
    test_nodes(3)
}

#[test]
fn group_size_nodes() {
    test_nodes(GROUP_SIZE);
}

#[test]
fn more_than_group_size_nodes() {
    test_nodes(GROUP_SIZE * 2);
}

#[test]
fn failing_connections_group_of_three() {
    let network = Network::new();
    network.block_connection(Endpoint(1), Endpoint(2));
    network.block_connection(Endpoint(1), Endpoint(3));
    network.block_connection(Endpoint(2), Endpoint(3));
    let mut nodes = create_connected_nodes(&network, 5);
    verify_kademlia_invariant_for_all_nodes(&nodes);
    drop_node(&mut nodes, 0); // Drop the tunnel node. Node 4 should replace it.
    verify_kademlia_invariant_for_all_nodes(&nodes);
    drop_node(&mut nodes, 1); // Drop a tunnel client. The others should be notified.
    verify_kademlia_invariant_for_all_nodes(&nodes);
}

#[test]
fn failing_connections_ring() {
    let network = Network::new();
    let len = GROUP_SIZE * 2;
    for i in 0..(len - 1) {
        network.block_connection(Endpoint(1 + i), Endpoint(1 + (i % len)));
    }
    let nodes = create_connected_nodes(&network, len);
    verify_kademlia_invariant_for_all_nodes(&nodes);
}

#[test]
fn client_connects_to_nodes() {
    let network = Network::new();
    let mut nodes = create_connected_nodes(&network, GROUP_SIZE + 1);

    // Create one client that tries to connect to the network.
    let client = TestNode::new(&network,
                               true,
                               Some(Config::with_contacts(&[nodes[0].handle.endpoint()])),
                               None);

    nodes.push(client);

    poll_all(&mut nodes);

    expect_event!(nodes.iter().last().unwrap(), Event::Connected);
}

#[test]
fn node_drops() {
    let network = Network::new();
    let mut nodes = create_connected_nodes(&network, GROUP_SIZE + 2);
    drop_node(&mut nodes, 0);

    verify_kademlia_invariant_for_all_nodes(&nodes);
}

#[test]
fn send_put_request() {
    let network = Network::new();
    let mut nodes = create_connected_nodes(&network, GROUP_SIZE + 2);
    let client = TestClient::new(&network,
                                 Some(Config::with_contacts(&[nodes[0].handle.endpoint()])),
                                 None);
    poll_all(&mut nodes);
    expect_event!(client, Event::Connected);

    let dst = Authority::ClientManager(client.name().clone());
    let data = rand::thread_rng().gen_iter().take(1024).collect();
    let immutable_data = ImmutableData::new(ImmutableDataType::Normal, data);
    let message_id = MessageId::new();

    assert!(client.send_put_request(dst, Data::Immutable(immutable_data), message_id).is_ok());
    poll_all(&mut nodes);

    for node in nodes.iter().filter(|n| n.routing_table().is_close(client.name())) {
        loop {
            match node.event_rx.try_recv() {
                Ok(Event::Request(RequestMessage { content: RequestContent::Put(ref data, ref id), .. })) =>
                    if let Data::Immutable(_) = *data { if *id == message_id { break; } },
                Ok(_) => (),
                _ => panic!("Event::Request(..) not received"),
            }
        }
    }
}
