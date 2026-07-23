//! Bevy wiring for building a described map into a world.

use bevy::prelude::*;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::components::resource::ResourceSourceComponent;
use ferrets_simulation::content::registry::ContentRegistry;
use ferrets_simulation::map::Map;
use ferrets_simulation::map_data::MapData;
use ferrets_simulation::session::GameSession;
use ferrets_simulation::spawn;
use ferrets_simulation::visibility::VisibilityGrid;

/// Builds the described map in the world: installs the live grid and spawns
/// the declared placements.
///
/// The map is installed before any placement is spawned, so spawning consults
/// the map's own occupation grid. A placement is skipped (with a log) when its
/// owner slot is unoccupied or unknown — there is no player to own it — or
/// when its cell cannot host its entity. Every node builds from the same data
/// and slots, so the skips are identical everywhere.
pub fn instantiate_map(world: &mut World, data: &MapData) {
    let map = Map::from_data(data, world.resource::<ContentRegistry>());
    let (width, height) = (map.width(), map.height());
    world.insert_resource(map);
    let player_count = world.resource::<GameSession>().slots().len();
    world.insert_resource(VisibilityGrid::new(player_count, width, height));

    for placement in data.placements() {
        if let Some(owner) = placement.owner {
            let occupied = world
                .resource::<GameSession>()
                .slot(owner)
                .is_some_and(|slot| slot.player_type().is_some());
            if !occupied {
                eprintln!(
                    "placement '{}' belongs to unoccupied slot {owner}; skipped",
                    placement.type_name
                );
                continue;
            }
        }
        let (x, y) = placement.cell;
        let position = FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y));
        let Some((entity, _)) =
            spawn::spawn_entity(world, &placement.type_name, position, placement.owner)
        else {
            eprintln!(
                "map cell ({x},{y}) cannot host '{}'; placement skipped",
                placement.type_name
            );
            continue;
        };
        if let Some(amount) = placement.amount
            && let Some(mut source) = world
                .entity_mut(entity)
                .get_mut::<ResourceSourceComponent>()
        {
            source.amount = amount;
        }
    }
}
