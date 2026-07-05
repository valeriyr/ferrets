//! Lua scripting runtime: game content loading and AI brains.
//!
//! Game content — races, resource kinds, and entity types — is authored as a Lua
//! script (`define_race`, `define_resource`, `define_entity`) and loaded once at
//! startup into a [`ContentRegistry`](ferrets_simulation::content::registry::ContentRegistry).
//! The content script runs only at load time and produces plain data (numbers
//! converted to fixed-point at the boundary), so it never executes inside the
//! deterministic tick loop.
//!
//! AI scripts additionally keep a live per-player Lua state (see
//! [`AiRuntime`](ai::AiRuntime)) that is invoked on a deterministic cadence
//! during the game. The boundary holds in both directions: the script observes
//! an integer-only snapshot and only ever produces player commands for the
//! ordinary input path — it never touches simulation state or fixed-point math.
//!
//! All scripting sits behind the [`ScriptEngine`](engine::ScriptEngine) seam:
//! the DSLs and their data types are engine-agnostic contracts, and each
//! scripting VM binds them in its own module under [`engine`] — Lua is the
//! current binding. Callers pick an engine and supply every script as a
//! `&str` — opening files is left to them.

pub mod ai;
pub mod content;
pub mod engine;
pub mod error;

/// A result whose error is a [`ScriptError`](error::ScriptError).
pub type Result<T> = std::result::Result<T, error::ScriptError>;
