//! The host-coordinated lobby: dynamic slot assignment and live state sync.
//!
//! The host owns the authoritative slot list, assigns a slot to each client as it
//! connects, and re-broadcasts the whole state on every change so every client
//! mirrors it. Clients send change requests the host validates. Because the state
//! is always current on every node, starting the game is a minimal signal.

pub mod client;
pub mod host;
