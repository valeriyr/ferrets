//! Replay recording and deterministic playback.
//!
//! A replay is the initial session configuration (a
//! [`ReplayHeader`](header::ReplayHeader)) followed by the realized input for
//! every simulated tick (a stream of [`TickRecord`](record::TickRecord)s). Feeding
//! those frames back through the simulation in tick order reproduces the game
//! exactly, since the simulation is deterministic. A [`Recorder`](recorder::Recorder)
//! streams and flushes per tick, so a replay survives a crash mid-game;
//! [`Replay`](replay::Replay) loads one back, keeping every complete record even if
//! the stream was cut short.
//!
//! The crate reads and writes over any [`Read`](std::io::Read) /
//! [`Write`](std::io::Write); opening files is left to the caller.

pub mod error;
mod format;
pub mod header;
pub mod record;
pub mod recorder;
pub mod replay;

use crate::error::ReplayError;

/// A result whose error is a [`ReplayError`].
pub type Result<T> = std::result::Result<T, ReplayError>;
