//! The scripting-engine seam.
//!
//! Everything scripts can declare or do is specified above this seam as
//! engine-agnostic contracts and plain data — content
//! [`Definition`]s, the AI view snapshots, and the
//! [`AiRuntime`] brain interface — while everything
//! VM-specific lives below it, in one binding module per engine. The Lua
//! binding is [`lua`]; another runtime slots in behind the same trait.

pub mod lua;

use crate::ai::AiRuntime;
use crate::ai::view::content::ContentView;
use crate::content::Definition;

/// A scripting runtime: one loader per script-facing layer of the game.
pub trait ScriptEngine {
    /// Evaluates a content script, returning the declarations it produced, in
    /// the order the script made them.
    fn load_content(&self, source: &str) -> crate::Result<Vec<Definition>>;

    /// Evaluates an AI script against the static `content` catalogue,
    /// returning the live brain it declared.
    ///
    /// Errors if the script fails, does not declare exactly one brain, or
    /// declares an invalid one.
    fn load_ai(&self, source: &str, content: &ContentView) -> crate::Result<Box<dyn AiRuntime>>;
}
