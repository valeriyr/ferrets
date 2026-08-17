//! The live [`Map`]: start points, and building from [`MapData`] — the grid
//! registers the content's layer vocabulary, and per-cell terrain seeds which
//! layers each cell blocks.

use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::{LocationDef, Solidity},
    registry::ContentRegistry,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize, projection::Projection};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};
use ferrets_pathfinder::{
    mover_shape::MoverShape,
    nav_grid::{LayerId, NavGrid},
};
use ferrets_simulation::{
    components::location::LocationComponent,
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
            .same_region(CellPos::new(0, 0), ground_shape(), CellPos::new(7, 0)),
        "the hierarchy must stay stale until the refresh point"
    );

    map.refresh_hierarchy();
    assert!(
        !map.hierarchy()
            .same_region(CellPos::new(0, 0), ground_shape(), CellPos::new(7, 0))
    );

    map.displace_entity(&location, &wall, OccupancyClass::Static);
    map.refresh_hierarchy();
    assert!(
        map.hierarchy()
            .same_region(CellPos::new(0, 0), ground_shape(), CellPos::new(7, 0))
    );
}

//
// ─── Stepping a footprint ─────────────────────────────────────────────────────
//

#[test]
fn footprint_step_only_writes_cells_that_change_hands() {
    // A 2×2 claim stepping one cell east keeps its shared column: the trailing
    // column is released and the leading one taken, and the overlap is never
    // touched — which is what lets one-claimant-per-cell hold through a crossing.
    let mut map = wall_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::new(2, 2), Solidity::Solid);
    let location = LocationComponent::new(world_pos(2, 0), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    map.step_claim(
        GROUND_LAYER.into(),
        CellSize::new(2, 2),
        CellPos::new(2, 0),
        CellPos::new(3, 0),
    );

    // Left column released, middle column still held, right column taken.
    assert!(
        !map.nav_grid()
            .is_claimed_by(GROUND_LAYER, CellPos::new(2, 0))
    );
    assert!(
        !map.nav_grid()
            .is_claimed_by(GROUND_LAYER, CellPos::new(2, 1))
    );
    for cell in [
        CellPos::new(3, 0),
        CellPos::new(3, 1),
        CellPos::new(4, 0),
        CellPos::new(4, 1),
    ] {
        assert!(
            map.nav_grid().is_claimed_by(GROUND_LAYER, cell),
            "the stepped footprint does not hold {cell:?}"
        );
    }
}

#[test]
fn footprint_step_is_blocked_by_what_lies_ahead_only() {
    let mut map = wall_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::new(2, 2), Solidity::Solid);
    let size = CellSize::new(2, 2);
    let location = LocationComponent::new(world_pos(2, 0), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    // Its own claim must not read as a blockage, or a wide mover could never
    // take a step that keeps most of its footprint where it was.
    assert!(map.can_step_footprint(
        GROUND_LAYER.into(),
        size,
        CellPos::new(2, 0),
        CellPos::new(3, 0)
    ));

    // A wall on the leading column does block it.
    let wall = LocationDef::new(GROUND_LAYER, CellSize::new(1, 2), Solidity::Solid);
    map.place_entity(
        &LocationComponent::new(world_pos(4, 0), FixedVec2::default()),
        &wall,
        OccupancyClass::Static,
    );
    assert!(!map.can_step_footprint(
        GROUND_LAYER.into(),
        size,
        CellPos::new(2, 0),
        CellPos::new(3, 0)
    ));
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
            .same_region(CellPos::new(0, 0), ground_shape(), CellPos::new(7, 0))
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
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.75),
                FixedU64::ONE,
            ),
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
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.5),
                FixedU64::ONE,
            ),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
#[should_panic(expected = "entity type 'runner' moves but defines no weight")]
fn continuous_map_rejects_mover_without_weight() {
    let mut registry = lake_registry();
    let ground = registry.layer("ground").unwrap();
    // Stated stat by stat, as scripted content states them — the movement
    // builder cannot express a mover missing one of the three.
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_stat(EntityStatId::SPEED, FixedU64::from_num(0.5))
            .with_stat(EntityStatId::RADIUS, FixedU64::from_num(0.5)),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

/// Nothing weighs against zero, so a weightless mover is a body that yields to
/// everything and shoves nothing — an authored choice, not a missing one.
#[test]
fn continuous_map_accepts_weightless_mover() {
    let mut registry = lake_registry();
    let ground = registry.layer("ground").unwrap();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(ground, CellSize::ONE, Solidity::Solid)
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.5),
                FixedU64::ZERO,
            ),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4);
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
fn cell_map_accepts_mover_without_weight() {
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
// ─── Lifting a claim out of a search's way ───────────────────────────────────
//

#[test]
fn taken_claim_frees_its_cells_and_restores_exactly() {
    let mut map = wall_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::new(2, 2), Solidity::Solid);
    let location = LocationComponent::new(world_pos(2, 0), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    let held = map.take_claim(GROUND_LAYER.into(), CellPos::new(2, 0), CellSize::new(2, 2));

    assert_eq!(held.len(), 4, "the whole held footprint is released");
    for &cell in &held {
        assert!(!map.nav_grid().is_claimed_by(GROUND_LAYER, cell));
    }

    map.restore_claim(GROUND_LAYER.into(), &held);
    for &cell in &held {
        assert!(map.nav_grid().is_claimed_by(GROUND_LAYER, cell));
    }
}

#[test]
fn taken_claim_reports_only_cells_that_were_held() {
    // Half the footprint queried carries no claim — routine under the
    // continuous model, whose plane is a once-per-tick summary trailing the
    // bodies: taking must release and report only what was held, so
    // restoring cannot mint claims out of thin air.
    let mut map = continuous_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::new(1, 2), Solidity::Solid);
    let location = LocationComponent::new(world_pos(2, 0), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    let held = map.take_claim(GROUND_LAYER.into(), CellPos::new(2, 0), CellSize::new(2, 2));

    assert_eq!(held, vec![CellPos::new(2, 0), CellPos::new(2, 1)]);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(
    expected = "a settled claimant's footprint must be fully claimed under the cell model"
)]
fn taking_partially_claimed_rect_panics_under_cell_model() {
    let mut map = wall_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::new(1, 2), Solidity::Solid);
    let location = LocationComponent::new(world_pos(2, 0), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    map.take_claim(GROUND_LAYER.into(), CellPos::new(2, 0), CellSize::new(2, 2));
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(expected = "restoring a taken claim must find its cells free")]
fn restoring_over_standing_claim_panics() {
    let mut map = wall_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::ONE, Solidity::Solid);
    let location = LocationComponent::new(world_pos(2, 0), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    map.restore_claim(GROUND_LAYER.into(), &[CellPos::new(2, 0)]);
}

//
// ─── Reserving ground ahead of standing on it ─────────────────────────────────
//

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(expected = "releasing a reservation must find its cells claimed")]
fn releasing_unclaimed_cells_panics() {
    let mut map = wall_test_map();
    map.release_claim(GROUND_LAYER.into(), &[CellPos::new(2, 0)]);
}

//
// ─── Rebuilding the claim plane ───────────────────────────────────────────────
//

#[test]
fn rebuilt_claims_stamp_footprints_and_reassert_reservations() {
    let mut map = continuous_test_map();
    // A claim from the previous tick vanishes with the wipe.
    let unit = LocationDef::new(GROUND_LAYER, CellSize::ONE, Solidity::Solid);
    let location = LocationComponent::new(world_pos(0, 0), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    map.rebuild_claims(
        &[(GROUND_LAYER.into(), CellSize::new(2, 1), CellPos::new(3, 0))],
        &[(GROUND_LAYER.into(), vec![CellPos::new(6, 1)])],
    );

    assert!(
        !map.nav_grid()
            .is_claimed_by(GROUND_LAYER, CellPos::new(0, 0))
    );
    assert!(
        map.nav_grid()
            .is_claimed_by(GROUND_LAYER, CellPos::new(3, 0))
    );
    assert!(
        map.nav_grid()
            .is_claimed_by(GROUND_LAYER, CellPos::new(4, 0))
    );
    assert!(
        map.nav_grid()
            .is_claimed_by(GROUND_LAYER, CellPos::new(6, 1))
    );
}

#[test]
#[should_panic(expected = "the claim plane is never rebuilt under the cell model")]
fn rebuilding_claims_panics_under_cell_model() {
    let mut map = wall_test_map();
    map.rebuild_claims(&[], &[]);
}

//
// ─── Writing the static plane ─────────────────────────────────────────────────
//

#[test]
fn static_write_blocks_and_frees_cell() {
    let mut map = wall_test_map();
    let cell = CellPos::new(3, 1);

    map.set_static_occupied(GROUND_LAYER, cell, true);
    assert!(!map.nav_grid().is_passable(GROUND_LAYER, cell));

    map.set_static_occupied(GROUND_LAYER, cell, false);
    assert!(map.nav_grid().is_passable(GROUND_LAYER, cell));
}

#[test]
fn static_write_lands_under_mover_claim() {
    // The flip is judged on the static plane alone: a mover's claim over the
    // cell says nothing about its static bit, so blocking claimed ground is
    // a legal first write, not a double-block.
    let mut map = wall_test_map();
    let unit = LocationDef::new(GROUND_LAYER, CellSize::ONE, Solidity::Solid);
    let location = LocationComponent::new(world_pos(3, 1), FixedVec2::default());
    map.place_entity(&location, &unit, OccupancyClass::Claim);

    let cell = CellPos::new(3, 1);
    map.set_static_occupied(GROUND_LAYER, cell, true);
    assert!(map.nav_grid().is_statically_occupied_by(GROUND_LAYER, cell));
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(
    expected = "a static write must flip the cell: blocking needs it free, freeing needs it blocked"
)]
fn static_write_of_state_cell_already_holds_panics() {
    // The plane's bits carry no owner, so a second writer blocking the same
    // cell would merge with the first — the flip is where that surfaces.
    let mut map = wall_test_map();
    let cell = CellPos::new(3, 1);
    map.set_static_occupied(GROUND_LAYER, cell, true);
    map.set_static_occupied(GROUND_LAYER, cell, true);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// The ground layer every hierarchy test uses.
const GROUND_LAYER: LayerId = LayerId::new(1);

/// The single-cell ground mover whose abstraction the test maps carry.
fn ground_shape() -> MoverShape {
    MoverShape::point(GROUND_LAYER)
}

/// An 8×2 map whose only cut is whatever the test places, with a hierarchy
/// for the single-cell ground mover.
fn wall_test_map() -> Map {
    let mut nav_grid = NavGrid::new(8, 2);
    nav_grid.add_layer(GROUND_LAYER);
    Map::with_hierarchy_shapes(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        nav_grid,
        vec![],
        &[ground_shape()],
    )
}

/// An 8×2 map under the continuous model, whose claim plane is a rebuilt
/// summary rather than law.
fn continuous_test_map() -> Map {
    let mut nav_grid = NavGrid::new(8, 2);
    nav_grid.add_layer(GROUND_LAYER);
    Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Continuous,
        nav_grid,
        vec![],
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
