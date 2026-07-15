//! A live scripting runtime that evaluates one scenario's objectives and
//! win/loss conditions.
//!
//! Like an [`AiRuntime`](crate::ai::AiRuntime), a [`ScenarioRuntime`] stays
//! alive for a whole game session and observes the same integer-only
//! [`GameView`] snapshot; unlike a brain, it issues no commands — it only
//! reports how far the objectives have progressed and whether the scenario has
//! been won or lost. A script declares its scenario once:
//!
//! ```lua
//! define_scenario("build_army", {
//!     period = 10,                        -- ticks between evaluations
//!     objectives = {
//!         { id = "barracks", label = "Build a barracks" },
//!         { id = "archers",  label = "Train 3 archers" },
//!     },
//!     check = function(state, view)
//!         -- returns which objectives are met and the overall outcome
//!         return {
//!             objectives = { barracks = true, archers = false },
//!             outcome = "ongoing",        -- or "victory" / "defeat"
//!         }
//!     end,
//! })
//! ```
//!
//! Each evaluate call receives the script's persistent `state` and a fresh
//! `view` snapshot. The declared `objectives` list (id and label) lives on the
//! engine side and fixes the display order; `check` returns only a boolean per
//! objective id (a missing id counts as not met) plus the outcome. An omitted
//! `outcome` is treated as `"ongoing"`. Mapping the outcome to a concrete game
//! result is the caller's job — the script only names it.
//!
//! Runtimes are loaded through the
//! [`ScriptEngine`](crate::engine::ScriptEngine) seam, so the scripting VM can
//! be swapped without touching callers.

use crate::ai::view::game::GameView;

/// A live evaluator for one scenario, kept alive for a whole session.
///
/// A runtime is single-threaded (`!Send`); callers own keeping it on one
/// thread.
pub trait ScenarioRuntime {
    /// The name the script declared.
    fn name(&self) -> &str;

    /// Ticks between evaluate calls, as declared by the script.
    fn period(&self) -> u32;

    /// Evaluates the scenario against `view`, returning per-objective progress
    /// and the overall outcome.
    ///
    /// Errors leave the runtime usable; for a deterministic script they are
    /// themselves deterministic (the same script and view fail identically
    /// everywhere).
    fn evaluate(&mut self, view: &GameView) -> crate::Result<ScenarioStatus>;
}

/// The result of one [`ScenarioRuntime::evaluate`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioStatus {
    /// The declared objectives with their current completion, in declared order.
    pub objectives: Vec<ObjectiveStatus>,
    pub outcome: Outcome,
}

/// One objective's identity and whether it is currently met.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectiveStatus {
    pub id: String,
    pub label: String,
    pub done: bool,
}

/// Whether the scenario is still in progress, or has been won or lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ongoing,
    Victory,
    Defeat,
}
