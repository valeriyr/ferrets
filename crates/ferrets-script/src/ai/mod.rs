//! A live scripting runtime that drives one AI player.
//!
//! Unlike content loading, which evaluates a script once and drops it, an
//! [`AiRuntime`] stays alive for a whole game session. A script declares its
//! brain once:
//!
//! ```lua
//! define_ai("default", {
//!     period = 20,                        -- ticks between think calls
//!     think = function(state, view)
//!         return { { kind = "stop" } }    -- an array of command tables
//!     end,
//! })
//! ```
//!
//! Each think call receives the script's persistent `state` and a fresh `view`
//! snapshot (see [`view`]), and returns commands that the caller feeds into
//! the ordinary input path — the script observes and orders, it never mutates
//! the simulation. The command schema (`flush` defaults to `true`; returning
//! `nil` means no commands; one malformed element fails the whole batch, since
//! commands compose sequentially and skipping one would silently misdirect the
//! rest):
//!
//! | `kind` | fields |
//! |---|---|
//! | `"select"` | `id` |
//! | `"select_area"` | `x1`, `y1`, `x2`, `y2` (inclusive cells) |
//! | `"move"` | `x`, `y`, `flush?` |
//! | `"attack"` | `target`, `flush?` |
//! | `"send"` | `target`, `flush?` |
//! | `"train"` | `trainer`, `type_name` |
//! | `"build"` | `builder`, `type_name`, `x`, `y`, `flush?` |
//! | `"stop"` | — |
//!
//! Determinism contract: view entity lists are ordered by ascending simulation
//! id and results are read positionally, so scripts should iterate with
//! `ipairs`; `pairs()` order on the script's own tables must never influence
//! the returned commands; only integers cross the boundary (integral floats
//! are accepted — integer division yields floats); `os`, `io`, and
//! `math.random` are unavailable.
//!
//! Runtimes are loaded through the
//! [`ScriptEngine`](crate::engine::ScriptEngine) seam, so the scripting VM can
//! be swapped without touching callers.

pub mod view;

use ferrets_simulation::command::PlayerCommand;

use crate::ai::view::game::GameView;

/// A live brain for one AI player, kept alive for a whole session.
///
/// A runtime is single-threaded (`!Send`); callers own keeping it on one
/// thread.
pub trait AiRuntime {
    /// The name the script declared.
    fn name(&self) -> &str;

    /// Ticks between think calls, as declared by the script.
    fn period(&self) -> u32;

    /// Runs one think step against `view`, returning the commands the script
    /// produced.
    ///
    /// Errors leave the runtime usable; for a deterministic script they are
    /// themselves deterministic (the same script and view fail identically
    /// everywhere). The caller guarantees at most one think per tick — the
    /// script's persistent state advances on every call.
    fn think(&mut self, view: &GameView) -> crate::Result<Vec<PlayerCommand>>;
}
