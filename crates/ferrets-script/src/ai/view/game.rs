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
    /// Supply the player's standing entities provide, after any ceiling.
    pub supply_provided: u32,
    /// Supply the player occupies — standing entities plus queued training.
    pub supply_used: u32,
    /// Researches the player has completed, in ascending name order.
    pub researched: Vec<String>,
    /// Researches the player's entities are working on or have queued, in
    /// ascending name order.
    pub researching: Vec<String>,
    pub my_entities: Vec<EntityView>,
    /// Entities owned by allied players (teammates), excluding the viewer's own.
    pub ally_entities: Vec<EntityView>,
    pub enemy_entities: Vec<EntityView>,
    pub neutral_entities: Vec<EntityView>,
    /// Remains lying on the map, in ascending id order — oldest first. Only
    /// those the brain's team can see, unless it is omniscient.
    pub remains: Vec<RemainsView>,
    /// The ground a concealed entity the brain's side cannot make out stands
    /// on: its sight reaches the entity, its detection does not. In ascending
    /// id order of the entity glimpsed; empty for a brain whose detection
    /// reaches everywhere.
    pub glimpses: Vec<Glimpse>,
}

/// The footprint of one glimpsed entity, and nothing else about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glimpse {
    /// The x of its footprint's lowest corner.
    pub x: u32,
    /// The y of that corner.
    pub y: u32,
    /// How many cells across it stands.
    pub width: u32,
    /// How many cells deep it stands.
    pub height: u32,
}

/// One body lying on the map, for the casts that spend them.
pub struct RemainsView {
    pub id: u32,
    /// What fell here.
    pub type_name: String,
    /// Cell coordinates of the remains.
    pub x: u32,
    pub y: u32,
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
    /// The current energy pool, floored to whole points. `None` when the type
    /// has none.
    pub energy: Option<u32>,
    /// Effective attack damage, `None` when the entity cannot attack.
    pub damage: Option<u32>,
    /// Effective flat armor.
    pub armor: Option<u32>,
    /// `true` when the order queue is empty.
    pub idle: bool,
    /// `true` when the entity is temporarily off the map (e.g. harvesting
    /// inside a source).
    pub hidden: bool,
    /// `true` while the entity is concealed: not seen by a side that is not
    /// its own unless that side's detection covers a cell it stands on.
    pub concealed: bool,
    /// The carried resource load, when any.
    pub carrying: Option<(String, u32)>,
    /// In-flight production, front first. Empty when nothing is queued.
    pub train_queue: Vec<String>,
    pub under_construction: bool,
    /// `true` while a field effect switches the entity off: it stands but
    /// starts no order but Train and Research, which wait, and neither fights,
    /// hunts, casts nor moves.
    pub disabled: bool,
    /// The stance name, when the entity has one.
    pub stance: Option<String>,
    /// Remaining amount in a resource source. `None` when not a source.
    pub resource_amount: Option<u32>,
    /// The holder this entity rides in. `None` when not aboard anything.
    pub boarded: Option<u32>,
    /// The ids riding inside this entity, in ascending order. Empty when it
    /// carries nobody.
    pub passengers: Vec<u32>,
    /// The ids of the broodlings this entity counts, in the order they joined
    /// its brood. Empty when it counts none.
    pub broodlings: Vec<u32>,
    /// The breeder that bore this entity and still ties it. `None` when none
    /// does.
    pub bred_by: Option<u32>,
    /// Ticks left of a timed life. `None` when the entity has none.
    pub lifetime_left: Option<u32>,
}
