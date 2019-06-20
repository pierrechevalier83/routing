use crate::id::PublicId;
use crate::messages::RoutingMessage;
use lru_time_cache::LruCache;
use std::collections::{BTreeSet, HashMap};

type Digest = [u8; 32];

const MAX_RESENDS: u8 = 1;

pub enum TargetState {
    Unknown,
    Resending(u8),
    Failed,
}

pub struct SendingTargetsCache {
    cache: LruCache<Digest, HashMap<PublicId, TargetState>>,
}

impl SendingTargetsCache {
    pub fn new() -> Self {
        Self {
            cache: LruCache::with_capacity(0),
        }
    }

    pub fn insert_message(&mut self, msg: &RoutingMessage, initial_targets: BTreeSet<PublicId>) {
        let hash = match msg.hash() {
            Ok(hash) => hash,
            Err(_) => {
                return;
            }
        };

        let targets = initial_targets
            .into_iter()
            .map(|tgt| (tgt, TargetState::Unknown));

        // the message shouldn't really exist in the cache when we're inserting it - but if it
        // does, we will just add the targets that don't yet have a status assigned
        let entry = self.cache.entry(hash).or_insert_with(HashMap::new);
        for (target, status) in targets {
            if !entry.contains_key(&target) {
                let _ = entry.insert(target, status);
            }
        }
    }
}
