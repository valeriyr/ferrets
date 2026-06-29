//! The tagged envelope every transport carries.
//!
//! All traffic is one of these, so a single canonical stream carries every kind
//! and the format never has to break to add a new one. When the control and
//! gameplay channels share one socket, this variant tag is what tells them apart.

pub mod control;
pub mod error;
pub mod gameplay;

use serde::{Deserialize, Serialize};

use crate::message::control::ControlMessage;
use crate::message::error::MessageError;
use crate::message::gameplay::GameplayMessage;

/// The result type related to messages.
pub type Result<T> = std::result::Result<T, MessageError>;

/// One message exchanged between peers, tagged by the channel it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Gameplay-channel traffic.
    Gameplay(GameplayMessage),
    /// Control-channel traffic.
    Control(ControlMessage),
}

/// Encodes a [`Message`] to its canonical bytes.
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    Ok(bcs::to_bytes(message)?)
}

/// Decodes a [`Message`] from canonical bytes.
pub fn decode(bytes: &[u8]) -> Result<Message> {
    Ok(bcs::from_bytes(bytes)?)
}
