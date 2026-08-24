//! The game-facing session requests: what the frontend asks of the running
//! session, decoupled from how the change is applied.
//!
//! A game only ever states intent, and the engine picks the mechanism the
//! invariants require: with no network session installed an intent applies
//! immediately, through its own `apply_local_*` system; with one, the control
//! plane turns the same intents into tick-aligned changes every node applies
//! together ([`net_control`](crate::network::net_control)).

pub mod pause;
pub mod speed;
