//! Content-defined dying-phase property struct.

/// Content-defined configuration of an entity's dying phase: how long it lasts
/// and what, if anything, it leaves behind.
///
/// A destroyed entity spends a fixed number of ticks in this phase before it is
/// removed. Destruction is independent of health — depleted resource sources,
/// cancelled constructions, and scripted removals all destroy entities that may
/// never take damage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DyingDef {
    /// Ticks the dying phase lasts.
    dying_time: u32,
    /// The entity type left behind when the dying phase completes. `None`
    /// means the entity disappears without a trace.
    corpse_type: Option<String>,
}

impl DyingDef {
    /// Creates a new `DyingDef` with the given data. `corpse_type` is
    /// the entity left behind when the dying phase completes; it decays through
    /// its own dying phase, so chained corpse types form decay stages
    /// (corpse → bones → gone).
    ///
    /// Panics if `dying_time` is `0` (an entity without a dying phase simply
    /// omits this component) or `corpse_type` is empty.
    pub fn new(dying_time: u32, corpse_type: Option<&str>) -> Self {
        assert!(dying_time > 0, "dying_time must be greater than 0");
        assert!(
            corpse_type.is_none_or(|corpse| !corpse.is_empty()),
            "corpse_type must not be empty"
        );

        Self {
            dying_time,
            corpse_type: corpse_type.map(String::from),
        }
    }

    /// Returns the duration of the dying phase in ticks.
    pub fn dying_time(&self) -> u32 {
        self.dying_time
    }

    /// Returns the entity type left behind when the dying phase completes.
    pub fn corpse_type(&self) -> Option<&str> {
        self.corpse_type.as_deref()
    }
}
