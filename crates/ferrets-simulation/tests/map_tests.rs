//! The live [`Map`]: start points, and building from [`MapData`] — the grid
//! registers the content's layer vocabulary, and per-cell terrain seeds which
//! layers each cell blocks.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{
    layer_mask::LayerMask,
    nav_grid::{LayerId, NavGrid},
};
use ferrets_simulation::{
    components::location::LocationComponent,
    content::{
        entity_stats::EntityStatId,
        entity_type_def::EntityTypeDef,
        location::{LocationDef, Solidity},
        registry::ContentRegistry,
    },
    map::{Map, OccupancyClass},
    map_data::MapData,
    movement_model::MovementModel,
};

//
// ─── Start points ─────────────────────────────────────────────────────────────
//

#[test]
fn start_point_indexes_by_player() {
    let map = Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        NavGrid::new(8, 8),
        vec![Some(CellPos::new(1, 2)), None, Some(CellPos::new(5, 6))],
    );

    assert_eq!(map.start_point(0), Some(CellPos::new(1, 2)));
    assert_eq!(map.start_point(1), None, "a seat without a start position");
    assert_eq!(map.start_point(2), Some(CellPos::new(5, 6)));
    assert_eq!(map.start_point(3), None, "a seat the map does not declare");
}

//
// ─── Hierarchy upkeep ─────────────────────────────────────────────────────────
//

#[test]
fn placed_footprint_reaches_hierarchy_after_refresh() {
    // A 3×2 wall splits an 8×2 map once placed, refreshed, and only then.
    let mut map = wall_test_map();
    let wall = LocationDef::new(GROUND_LAYER, CellSize::new(3, 2), Solidity::Solid);
    let location = LocationComponent::new(world_pos(3, 0), FixedVec2::default());

    map.place_entity(&location, &wall, OccupancyClass::Static);
    assert!(
        map.hierarchy()
            .same_region(GROUND_LAYER, CellPos::new(0, 0), CellPos::new(7, 0)),
        "the hierarchy must stay stale until the refresh point"
    );

    map.refresh_hierarchy();
    assert!(
        !map.hierarchy()
            .same_region(GROUND_LAYER, CellPos::new(0, 0), CellPos::new(7, 0))
    );

    map.displace_entity(&location, &wall, OccupancyClass::Static);
    map.refresh_hierarchy();
    assert!(
        map.hierarchy()
            .same_region(GROUND_LAYER, CellPos::new(0, 0), CellPos::new(7, 0))
    );
}

#[test]
fn claimed_footprint_never_reaches_hierarchy() {
    let mut map = wall_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::new(3, 2), Solidity::Solid);
    let location = LocationComponent::new(world_pos(3, 0), FixedVec2::default());

    map.place_entity(&location, &unit, OccupancyClass::Claim);
    map.refresh_hierarchy();

    assert!(
        map.hierarchy()
            .same_region(GROUND_LAYER, CellPos::new(0, 0), CellPos::new(7, 0))
    );
    assert!(
        map.nav_grid()
            .is_occupied_by(GROUND_LAYER, CellPos::new(3, 0))
    );
}

//
// ─── Movement model ───────────────────────────────────────────────────────────
//

#[test]
#[should_panic(expected = "entity type 'runner' moves but defines no positive radius")]
fn continuous_map_rejects_mover_without_radius() {
    let mut registry = lake_registry();
    let ground = registry.layer("ground").unwrap();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::SPEED, FixedU64::from_num(0.5)),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
#[should_panic(
    expected = "entity type 'runner' authors a radius beyond half its footprint's narrow side"
)]
fn continuous_map_rejects_radius_beyond_footprint() {
    let mut registry = lake_registry();
    let ground = registry.layer("ground").unwrap();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.75)),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
fn continuous_map_accepts_radius_at_half_footprint() {
    let mut registry = lake_registry();
    let ground = registry.layer("ground").unwrap();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5)),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
fn cell_map_accepts_mover_without_radius() {
    let mut registry = lake_registry();
    let ground = registry.layer("ground").unwrap();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::SPEED, FixedU64::from_num(0.5)),
    );

    let data = MapData::new("pond", Projection::Isometric, 4, 4);

    Map::from_data(&data, &registry);
}

#[test]
fn from_data_carries_movement_model() {
    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.set_movement_model(MovementModel::Continuous);

    let map = Map::from_data(&data, &lake_registry());

    assert_eq!(map.movement_model(), MovementModel::Continuous);
}

//
// ─── Terrain seeding ──────────────────────────────────────────────────────────
//

#[test]
fn terrain_blocks_layers_it_does_not_pass() {
    let registry = lake_registry();
    let map = Map::from_data(&lake_map(), &registry);

    let ground = registry.layer("ground").unwrap();
    let water = registry.layer("water").unwrap();

    // The lake cell floats ships and blocks walkers; grass is the inverse.
    let lake = CellPos::new(1, 1);
    assert!(!map.nav_grid().is_passable(ground, lake));
    assert!(map.nav_grid().is_passable(water, lake));

    let grass = CellPos::new(0, 0);
    assert!(map.nav_grid().is_passable(ground, grass));
    assert!(!map.nav_grid().is_passable(water, grass));
}

#[test]
fn map_without_terrain_opens_every_layer_everywhere() {
    let data = MapData::new("pond", Projection::Isometric, 4, 4);
    let registry = lake_registry();
    let map = Map::from_data(&data, &registry);

    for y in 0..data.height() {
        for x in 0..data.width() {
            let cell = CellPos::new(x, y);
            for (name, layer) in registry.layers() {
                assert!(
                    map.nav_grid().is_passable(layer, cell),
                    "cell ({x}, {y}) must be open on layer '{name}'"
                );
            }
        }
    }
}

#[test]
#[should_panic(expected = "map 'pond' uses unregistered terrain 'water'")]
fn unregistered_terrain_panics() {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    registry.register_layer("water");
    registry.register_terrain("grass", ground);

    Map::from_data(&lake_map(), &registry);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The ground layer every hierarchy test uses.
const GROUND_LAYER: LayerId = LayerId::new(1);

/// An 8×2 map whose only cut is whatever the test places, with a hierarchy
/// for the ground mover mask.
fn wall_test_map() -> Map {
    let mut nav_grid = NavGrid::new(8, 2);
    nav_grid.add_layer(GROUND_LAYER);
    Map::with_hierarchy_masks(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        nav_grid,
        vec![],
        &[LayerMask::from(GROUND_LAYER)],
    )
}

fn world_pos(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// A registry declaring ground/water layers and grass/water terrains.
fn lake_registry() -> ContentRegistry {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    let water = registry.register_layer("water");
    registry.register_terrain("grass", ground);
    registry.register_terrain("water", water);
    registry
}

/// A 4×4 grass map with one water cell at (1, 1).
fn lake_map() -> MapData {
    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.fill_terrain("grass");
    data.set_terrain((1, 1), "water");
    data
}
