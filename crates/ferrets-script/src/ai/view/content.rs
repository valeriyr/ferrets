//! The static content catalogue a script can consult, snapshotted once per
//! session.

use ferrets_simulation::components::stats::StatId;
use ferrets_simulation::content::entity_type_def::EntityTypeDef;
use ferrets_simulation::content::registry::ContentRegistry;

/// The static content catalogue a script can consult.
pub struct ContentView {
    /// Registered resource kinds, in ascending order.
    pub resources: Vec<String>,
    pub entities: Vec<EntityContentView>,
}

impl ContentView {
    /// Snapshots the registered content, in ascending name order.
    pub fn from_registry(registry: &ContentRegistry) -> ContentView {
        ContentView {
            resources: registry.resources().map(str::to_string).collect(),
            entities: registry
                .entities()
                .map(EntityContentView::from_def)
                .collect(),
        }
    }
}

/// The fields of one entity type definition a script can consult.
pub struct EntityContentView {
    pub name: String,
    /// Price per resource kind, in ascending kind order. Empty means free.
    pub cost: Vec<(String, u32)>,
    pub train_time: Option<u32>,
    pub build_time: Option<u32>,
    /// Trainable types. `None` when instances cannot train.
    pub trains: Option<Vec<String>>,
    /// Constructible types. `None` when instances cannot build.
    pub builds: Option<Vec<String>>,
    /// Footprint width and height in cells.
    pub size: (u32, u32),
    /// Maximum health (the `max_health` stat). `None` when the type has none.
    pub max_health: Option<u32>,
    /// The weapon. `None` when the type cannot attack.
    pub attack: Option<AttackView>,
    /// Harvestable resource kinds. `None` when the type cannot harvest.
    pub harvests: Option<Vec<String>>,
    /// Resource kinds accepted for delivery. `None` when not a storage.
    pub stores: Option<Vec<String>>,
    pub can_move: bool,
}

/// A type's weapon — the combat stats a script reads together (a type carries
/// them all, or has no weapon at all).
pub struct AttackView {
    pub damage: u32,
    pub attack_range: u32,
}

impl EntityContentView {
    /// Snapshots the fields a script can consult from one type definition.
    pub fn from_def(def: &EntityTypeDef) -> EntityContentView {
        EntityContentView {
            name: def.name.clone(),
            cost: def
                .cost
                .iter()
                .map(|(kind, amount)| (kind.clone(), *amount))
                .collect(),
            train_time: def.train_time,
            build_time: def.build_time,
            trains: def
                .trainer
                .as_ref()
                .map(|t| t.trains().map(str::to_string).collect()),
            builds: def
                .builder
                .as_ref()
                .map(|b| b.builds().map(str::to_string).collect()),
            size: def
                .location
                .as_ref()
                .map_or((1, 1), |l| (l.size().width, l.size().height)),
            max_health: def.base_stat(StatId::MAX_HEALTH).map(|v| v.to_num::<u32>()),
            attack: def
                .base_stat(StatId::DAMAGE)
                .zip(def.base_stat(StatId::ATTACK_RANGE))
                .map(|(damage, range)| AttackView {
                    damage: damage.to_num::<u32>(),
                    attack_range: range.to_num::<u32>(),
                }),
            harvests: def
                .resource_carrier
                .as_ref()
                .map(|c| c.kinds().map(str::to_string).collect()),
            stores: def
                .resource_storage
                .as_ref()
                .map(|s| s.kinds().map(str::to_string).collect()),
            can_move: def.can_move(),
        }
    }
}
