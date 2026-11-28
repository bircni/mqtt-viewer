//! What a broker reports about itself under `$SYS`.
//!
//! The topics follow Mosquitto's layout, which several other brokers copy. Brokers that publish nothing there
//! (or use another layout) simply yield no entries.

use crate::decode;
use crate::tree::TopicTree;

/// `$SYS` topics shown as broker information, with their labels, in display order.
pub const TOPICS: [(&str, &str); 11] = [
    ("Version", "$SYS/broker/version"),
    ("Uptime", "$SYS/broker/uptime"),
    ("Clients connected", "$SYS/broker/clients/connected"),
    ("Subscriptions", "$SYS/broker/subscriptions/count"),
    ("Retained messages", "$SYS/broker/retained messages/count"),
    ("Messages received", "$SYS/broker/messages/received"),
    ("Messages sent", "$SYS/broker/messages/sent"),
    ("Received per minute", "$SYS/broker/load/messages/received/1min"),
    ("Sent per minute", "$SYS/broker/load/messages/sent/1min"),
    ("Bytes received", "$SYS/broker/bytes/received"),
    ("Bytes sent", "$SYS/broker/bytes/sent"),
];

/// The broker information found in `tree`, as `(label, value)` pairs in display order.
#[must_use]
pub fn info(tree: &TopicTree) -> Vec<(&'static str, String)> {
    TOPICS
        .iter()
        .filter_map(|(label, topic)| {
            let entry = tree.node(tree.find(topic)?).last()?;
            Some((*label, decode::preview(&entry.payload, 80)))
        })
        .collect()
}
