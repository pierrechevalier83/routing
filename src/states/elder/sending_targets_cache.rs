use std::collections::HashMap;

type Digest = [u8; 32];

// TODO: figure out what we want
const MAX_RESENDS: u8 = 5;

pub enum MessageStatus {
    // We've called `send` but haven't heard back from the network
    // It was the n-th attempt at sending this message
    Sending(u8),
    // We received n failures.
    Failed(u8),
    // Won't fail anymore
    Sent,
}

type SendingTargetsCache = HashMap<Digest, MessageStatus>;
