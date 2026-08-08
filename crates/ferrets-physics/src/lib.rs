//! Deterministic contact resolution for continuous-model RTS bodies:
//! circles over a cell grid, pushing apart on contact and settling one to a
//! cell. Pure fixed-point math over body data — who the bodies are, and what
//! their displacement means, is the caller's business.

pub mod body;
pub mod contact;
pub mod terrain;
