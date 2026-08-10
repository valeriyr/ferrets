//! The scripting-engine seam.
//!
//! Everything scripts can declare or do is specified above this seam as
//! engine-agnostic contracts and plain data — the content registry the
//! `define_*` declarations fill, the AI view snapshots, and the
//! [`AiRuntime`] brain interface — while everything
//! VM-specific lives below it, in one binding module per engine. The Lua
//! binding is [`lua`]; another runtime slots in behind the same trait.

pub mod lua;

use ferrets_content::registry::ContentRegistry;

use crate::{
    ai::{AiRuntime, view::content::ContentView},
    scenario::ScenarioRuntime,
};

/// A scripting runtime: one loader per script-facing layer of the game.
pub trait ScriptEngine {
    /// Evaluates a content script, returning the registry its declarations
    /// filled.
    ///
    /// Content-consistency errors (an unregistered race, layer, or tag)
    /// panic, matching the registry's Rust API; script and field errors are
    /// returned.
    fn load_content(&self, source: &str) -> crate::Result<ContentRegistry>;

    /// Evaluates an AI script against the static `content` catalogue,
    /// returning the live brain it declared.
    ///
    /// Errors if the script fails, does not declare exactly one brain, or
    /// declares an invalid one.
    fn load_ai(&self, source: &str, content: &ContentView) -> crate::Result<Box<dyn AiRuntime>>;

    /// Evaluates a scenario script against the static `content` catalogue,
    /// returning the live evaluator it declared.
    ///
    /// Errors if the script fails, does not declare exactly one scenario, or
    /// declares an invalid one.
    fn load_scenario(
        &self,
        source: &str,
        content: &ContentView,
    ) -> crate::Result<Box<dyn ScenarioRuntime>>;
}
