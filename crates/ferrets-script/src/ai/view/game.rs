//! The per-think game view: what one AI player observes each think step.
//!
//! Entity lists are ordered by ascending simulation id so positional
//! iteration stays deterministic.

/// Everything one AI player observes for one think step.
pub struct GameView {
    pub tick: u32,
    pub player: u32,
    pub race: String,
    pub map_width: u32,
    pub map_height: u32,
    /// Stockpile per resource kind, in ascending kind order.
    pub resources: Vec<(String, u32)>,
    pub my_entities: Vec<EntityView>,
    /// Entities owned by allied players (teammates), excluding the viewer's own.
    pub ally_entities: Vec<EntityView>,
    pub enemy_entities: Vec<EntityView>,
    pub neutral_entities: Vec<EntityView>,
}

/// One entity, snapshotted to integers.
pub struct EntityView {
    pub id: u32,
    pub type_name: String,
    /// Cell coordinates of the entity's position.
    pub x: u32,
    pub y: u32,
    /// `None` when the type has no health.
    pub health: Option<u32>,
    /// Effective attack damage, `None` when the entity cannot attack.
    pub damage: Option<u32>,
    /// Effective flat armor.
    pub armor: Option<u32>,
    /// `true` when the order queue is empty.
    pub idle: bool,
    /// `true` when the entity is temporarily off the map (e.g. harvesting
    /// inside a source).
    pub hidden: bool,
    /// The carried resource load, when any.
    pub carrying: Option<(String, u32)>,
    /// In-flight production, front first. Empty when nothing is queued.
    pub train_queue: Vec<String>,
    pub under_construction: bool,
    /// The stance name, when the entity has one.
    pub stance: Option<String>,
    /// Remaining amount in a resource source. `None` when not a source.
    pub resource_amount: Option<u32>,
}
