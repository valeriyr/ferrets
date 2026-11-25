//! Deterministic contact for continuous-model RTS bodies: circles over a cell
//! grid, the rules of a pair in contact, the ground a body may not enter, the
//! one way a body moves over it, and the pass that settles every body's
//! contacts. Pure fixed-point math over body data — who the bodies are, and
//! what their displacement means, is the caller's business.

pub mod body;
pub mod contact;
pub mod ground;
pub mod motion;
pub mod resolve;
