//! Core of mqtt-viewer: everything that does not draw.
//!
//! The crate owns the broker connection ([`connection`]), the topic tree the UI renders ([`tree`]), payload
//! decoding ([`decode`]), recording and replay ([`record`]) and the optional history database ([`store`]).
//! Nothing in here depends on a UI toolkit.

pub mod config;
pub mod connection;
pub mod decode;
pub mod diff;
pub mod message;
pub mod numeric;
pub mod record;
pub mod stats;
pub mod store;
pub mod topic;
pub mod tree;

#[cfg(test)]
mod tests;
