//! The live [`Map`]: start points, and building from [`MapData`] — the grid
//! registers the content's layer vocabulary and takes each cell's terrain.

mod utils;

use ferrets_content::{
    domain::{DomainDef, DomainId},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    pool::{Pool, PoolInitial},
    registry::ContentRegistry,
    terrains::Terrains,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::{FixedU64, facing::Facing, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    layer_mask::LayerMask,
    mover_shape::MoverShape,
    nav_grid::{LayerId, NavGrid},
    terrain_id::TerrainId,
    terrain_mask::TerrainMask,
};
use ferrets_simulation::{
    components::location::LocationComponent,
    map::Map,
    map_data::{MapData, Placement},
    movement_model::MovementModel,
    occupancy::{GridMark, OccupancyClass},
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
        grid(8, 8),
        vec![Some(CellPos::new(1, 2)), None, Some(CellPos::new(5, 6))],
    );

    assert_eq!(map.start_point(0), Some(CellPos::new(1, 2)));
    assert_eq!(map.start_point(1), None, "a seat without a start position");
    assert_eq!(map.start_point(2), Some(CellPos::new(5, 6)));
    assert_eq!(map.start_point(3), None, "a seat the map does not declare");
}

//
// ─── Bounds ───────────────────────────────────────────────────────────────────
//

#[test]
fn contains_cells_inside_and_not_past_edges() {
    let map = Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        grid(8, 6),
        Vec::new(),
    );

    assert!(map.contains(CellPos::new(0, 0)));
    assert!(map.contains(CellPos::new(7, 5)));
    assert!(!map.contains(CellPos::new(8, 5)), "past the right edge");
    assert!(!map.contains(CellPos::new(7, 6)), "past the bottom edge");
}

//
// ─── Hierarchy upkeep ─────────────────────────────────────────────────────────
//

#[test]
fn new_map_serves_no_mover() {
    let map = Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        grid(8, 2),
        vec![],
    );
    assert_eq!(map.hierarchy().shapes().count(), 0);
}

#[test]
fn map_serves_each_mover_shape_once() {
    let mut map = wall_test_map();
    // Beside the 1×1 walker: a second 1×1 mover, which shares its shape, a
    // 2×2 mover, and a 3×3 building, which plans nothing.
    let mut registry = walker_registry();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            ),
    );
    registry.register(
        EntityTypeDef::new("cart")
            .with_location(
                WALKER_DOMAIN,
                CellSize::new(2, 2),
                Solidity::Solid,
                GROUND_LAYER,
            )
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            ),
    );
    registry.register(EntityTypeDef::new("hut").with_location(
        WALKER_DOMAIN,
        CellSize::new(3, 3),
        Solidity::Solid,
        GROUND_LAYER,
    ));

    map.serve_movers(&registry);

    // In name order — cart, runner, walker — each shape kept the first time.
    assert_eq!(
        map.hierarchy().shapes().collect::<Vec<_>>(),
        vec![shape(CellSize::new(2, 2)), ground_shape()]
    );
}

#[test]
fn map_serves_mover_shape_per_terrains_and_occupied_layers() {
    // Three 1×1 movers on the ground layer: the walker over land, a boat over
    // deep water, and a tall walker over land that occupies the air too.
    // Each plans through different cells, so each gets its own abstraction.
    let mut registry = walker_registry();
    let air = registry.register_layer("air");
    let mut nav_grid = grid(8, 2);
    nav_grid.add_layer(air);
    let mut map = Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        nav_grid,
        vec![],
    );
    let deep_water = registry.register_terrain("deep_water");
    let naval = registry.register_domain(
        "naval",
        DomainDef::new(GROUND_LAYER, Terrains::Only(deep_water.into())),
    );
    for (name, domain, occupies) in [
        ("boat", naval, LayerMask::from(GROUND_LAYER)),
        ("tall_walker", WALKER_DOMAIN, GROUND_LAYER | air),
    ] {
        registry.register(
            EntityTypeDef::new(name)
                .with_location(domain, CellSize::ONE, Solidity::Solid, occupies)
                .with_movement(
                    utils::fixed("0.5"),
                    utils::fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                ),
        );
    }

    map.serve_movers(&registry);

    // In name order — boat, tall_walker, walker.
    assert_eq!(
        map.hierarchy().shapes().collect::<Vec<_>>(),
        vec![
            MoverShape::point(deep_water.into(), GROUND_LAYER),
            MoverShape::point(TerrainMask::from(LAND), GROUND_LAYER | air),
            ground_shape(),
        ]
    );
}

#[test]
#[should_panic(
    expected = "map 'test' serves no hierarchy for mover 'walker' (terrains 0b1, layers 0b1 at 1x1)"
)]
fn map_serving_no_mover_refuses_content_that_moves() {
    let map = Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        grid(8, 2),
        vec![],
    );
    map.assert_serves_movers(&walker_registry());
}

#[test]
fn placed_footprint_reaches_hierarchy_after_refresh() {
    // A 3×2 wall splits an 8×2 map once placed, refreshed, and only then.
    let mut map = wall_test_map();
    let wall = shape(CellSize::new(3, 2));
    let location = LocationComponent::new(world_pos(3, 0), Facing::SOUTH);

    map.place(mark(&location, wall, OccupancyClass::Static));
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

    map.displace(mark(&location, wall, OccupancyClass::Static));
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
    let location = LocationComponent::new(world_pos(2, 0), Facing::SOUTH);
    map.place(mark(
        &location,
        shape(CellSize::new(2, 2)),
        OccupancyClass::Claim,
    ));

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
    let unit = shape(CellSize::new(2, 2));
    let location = LocationComponent::new(world_pos(2, 0), Facing::SOUTH);
    map.place(mark(&location, unit, OccupancyClass::Claim));

    // Its own claim must not read as a blockage, or a wide mover could never
    // take a step that keeps most of its footprint where it was.
    assert!(map.can_step_footprint(unit, CellPos::new(2, 0), CellPos::new(3, 0)));

    // A wall on the leading column does block it.
    map.place(mark(
        &LocationComponent::new(world_pos(4, 0), Facing::SOUTH),
        shape(CellSize::new(1, 2)),
        OccupancyClass::Static,
    ));
    assert!(!map.can_step_footprint(unit, CellPos::new(2, 0), CellPos::new(3, 0)));
}

#[test]
fn footprint_step_is_blocked_by_terrain_off_its_shape() {
    let registry = lake_registry();
    let map = Map::from_data(&lake_map(), &registry);
    let unit = MoverShape::point(TerrainMask::from(LAND), GROUND_LAYER);

    assert!(map.can_step_footprint(unit, CellPos::new(0, 0), CellPos::new(1, 0)));
    assert!(
        !map.can_step_footprint(unit, CellPos::new(0, 0), CellPos::new(1, 1)),
        "(1,1) is deep water, which a land shape may not enter"
    );
}

#[test]
fn claimed_footprint_never_reaches_hierarchy() {
    let mut map = wall_test_map();
    let unit = shape(CellSize::new(3, 2));
    let location = LocationComponent::new(world_pos(3, 0), Facing::SOUTH);

    map.place(mark(&location, unit, OccupancyClass::Claim));
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
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_stat(EntityStatId::SPEED, FixedU64::from_num(0.5))
            .with_stat(EntityStatId::TURN_RATE, FixedU64::from_num(360))
            .with_stat(EntityStatId::PIVOT_RATE, FixedU64::from_num(360)),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4, "land");
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
#[should_panic(
    expected = "entity type 'runner' authors a radius beyond half its footprint's narrow side"
)]
fn continuous_map_rejects_radius_beyond_footprint() {
    let mut registry = lake_registry();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.75),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            ),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4, "land");
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
fn continuous_map_accepts_radius_at_half_footprint() {
    let mut registry = lake_registry();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.5),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            ),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4, "land");
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
#[should_panic(expected = "entity type 'runner' moves but defines no weight")]
fn continuous_map_rejects_mover_without_weight() {
    let mut registry = lake_registry();
    // Stated stat by stat, as scripted content states them — the movement
    // builder cannot express a mover missing one of the three.
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_stat(EntityStatId::SPEED, FixedU64::from_num(0.5))
            .with_stat(EntityStatId::TURN_RATE, FixedU64::from_num(360))
            .with_stat(EntityStatId::PIVOT_RATE, FixedU64::from_num(360))
            .with_stat(EntityStatId::RADIUS, FixedU64::from_num(0.5)),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4, "land");
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

/// Nothing weighs against zero, so a weightless mover is a body that yields to
/// everything and shoves nothing — an authored choice, not a missing one.
#[test]
fn continuous_map_accepts_weightless_mover() {
    let mut registry = lake_registry();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_movement(
                FixedU64::from_num(0.5),
                FixedU64::from_num(0.5),
                FixedU64::ZERO,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            ),
    );

    let mut data = MapData::new("pond", Projection::Isometric, 4, 4, "land");
    data.set_movement_model(MovementModel::Continuous);

    Map::from_data(&data, &registry);
}

#[test]
fn cell_map_accepts_mover_without_weight() {
    let mut registry = lake_registry();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_stat(EntityStatId::SPEED, FixedU64::from_num(0.5))
            .with_stat(EntityStatId::TURN_RATE, FixedU64::from_num(360))
            .with_stat(EntityStatId::PIVOT_RATE, FixedU64::from_num(360)),
    );

    let data = MapData::new("pond", Projection::Isometric, 4, 4, "land");

    Map::from_data(&data, &registry);
}

#[test]
fn cell_map_accepts_mover_without_radius() {
    let mut registry = lake_registry();
    registry.register(
        EntityTypeDef::new("runner")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_stat(EntityStatId::SPEED, FixedU64::from_num(0.5))
            .with_stat(EntityStatId::TURN_RATE, FixedU64::from_num(360))
            .with_stat(EntityStatId::PIVOT_RATE, FixedU64::from_num(360)),
    );

    let data = MapData::new("pond", Projection::Isometric, 4, 4, "land");

    Map::from_data(&data, &registry);
}

#[test]
fn from_data_carries_movement_model() {
    let mut data = MapData::new("pond", Projection::Isometric, 4, 4, "land");
    data.set_movement_model(MovementModel::Continuous);

    let map = Map::from_data(&data, &lake_registry());

    assert_eq!(map.movement_model(), MovementModel::Continuous);
}

//
// ─── Terrain seeding ──────────────────────────────────────────────────────────
//

#[test]
fn from_data_gives_each_cell_its_terrain() {
    let registry = lake_registry();
    let map = Map::from_data(&lake_map(), &registry);

    assert_eq!(map.nav_grid().terrain(CellPos::new(1, 1)), Some(DEEP_WATER));
    assert_eq!(map.nav_grid().terrain(CellPos::new(0, 0)), Some(LAND));
    assert_eq!(map.nav_grid().terrain(CellPos::new(3, 3)), Some(LAND));
}

#[test]
fn shape_fits_only_cells_on_its_terrains() {
    let registry = lake_registry();
    let map = Map::from_data(&lake_map(), &registry);
    let walker = MoverShape::point(TerrainMask::from(LAND), GROUND_LAYER);
    let boat = MoverShape::point(DEEP_WATER.into(), GROUND_LAYER);

    // The lake cell floats boats and refuses walkers; land is the inverse.
    let lake = CellPos::new(1, 1);
    assert!(!map.nav_grid().fits(lake, walker));
    assert!(map.nav_grid().fits(lake, boat));

    let land = CellPos::new(0, 0);
    assert!(map.nav_grid().fits(land, walker));
    assert!(!map.nav_grid().fits(land, boat));
}

#[test]
fn standing_footprint_leaves_cell_terrain_alone() {
    let registry = lake_registry();
    let mut map = Map::from_data(&lake_map(), &registry);
    let land = CellPos::new(0, 0);

    map.set_static_occupied(GROUND_LAYER, land, true);

    assert!(
        !map.nav_grid().fits(land, ground_shape()),
        "the footprint blocks walkers"
    );
    assert!(
        map.nav_grid()
            .fits_terrain(land, TerrainMask::from(LAND), CellSize::ONE),
        "the terrain under it still takes them"
    );
}

#[test]
#[should_panic(expected = "placement 'corpse' names remains, which only a death may leave")]
fn placement_of_remains_panics() {
    let mut registry = walker_registry();
    registry.register_tag("remains");
    registry.register(
        EntityTypeDef::new("corpse")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_tags(["remains"])
            .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
    );

    let mut data = MapData::new("field", Projection::Isometric, 8, 8, "land");
    data.add_placement(Placement {
        type_name: "corpse".to_string(),
        cell: (2, 2),
        owner: None,
        amount: None,
        pools: Vec::new(),
    });

    Map::from_data(&data, &registry);
}

#[test]
#[should_panic(expected = "placement 'post' names the unregistered pool 'mana'")]
fn placement_naming_unregistered_pool_panics() {
    place_post_with(vec![("mana".to_string(), PoolInitial::Full)]);
}

#[test]
#[should_panic(expected = "placement 'ghost' names the unregistered pool 'mana'")]
fn placement_of_unregistered_type_naming_unregistered_pool_panics() {
    let registry = walker_registry();
    let mut data = MapData::new("field", Projection::Isometric, 8, 8, "land");
    data.add_placement(Placement {
        type_name: "ghost".to_string(),
        cell: (2, 2),
        owner: None,
        amount: None,
        pools: vec![("mana".to_string(), PoolInitial::Full)],
    });
    Map::from_data(&data, &registry);
}

#[test]
#[should_panic(
    expected = "placement 'post' names the energy pool, which its type does not declare"
)]
fn placement_naming_undeclared_pool_panics() {
    place_post_with(vec![("energy".to_string(), PoolInitial::Full)]);
}

#[test]
#[should_panic(expected = "placement 'post' names the health pool twice")]
fn placement_naming_pool_twice_panics() {
    place_post_with(vec![
        ("health".to_string(), PoolInitial::Full),
        ("health".to_string(), PoolInitial::Full),
    ]);
}

#[test]
#[should_panic(expected = "placement 'post' starts the health pool at a share of 1.5, above 1")]
fn placement_starting_share_above_whole_panics() {
    place_post_with(vec![(
        "health".to_string(),
        PoolInitial::Share(utils::fixed("1.5")),
    )]);
}

#[test]
#[should_panic(
    expected = "placement 'post' starts the health pool at 150, above its maximum of 100"
)]
fn placement_starting_amount_above_maximum_panics() {
    place_post_with(vec![(
        "health".to_string(),
        PoolInitial::Amount(FixedU64::from_num(150)),
    )]);
}

#[test]
#[should_panic(expected = "placement 'post' starts the health pool at 0")]
fn placement_starting_health_empty_panics() {
    place_post_with(vec![(
        "health".to_string(),
        PoolInitial::Share(FixedU64::ZERO),
    )]);
}

#[test]
#[should_panic(expected = "map 'pond' uses unregistered terrain 'deep_water'")]
fn unregistered_terrain_panics() {
    Map::from_data(&lake_map(), &walker_registry());
}

//
// ─── Lifting a claim out of a search's way ───────────────────────────────────
//

#[test]
fn taken_claim_frees_its_cells_and_restores_exactly() {
    let mut map = wall_test_map();
    let location = LocationComponent::new(world_pos(2, 0), Facing::SOUTH);
    map.place(mark(
        &location,
        shape(CellSize::new(2, 2)),
        OccupancyClass::Claim,
    ));

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
    let unit = shape(CellSize::new(1, 2));
    let location = LocationComponent::new(world_pos(2, 0), Facing::SOUTH);
    map.place(mark(&location, unit, OccupancyClass::Claim));

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
    let unit = shape(CellSize::new(1, 2));
    let location = LocationComponent::new(world_pos(2, 0), Facing::SOUTH);
    map.place(mark(&location, unit, OccupancyClass::Claim));

    map.take_claim(GROUND_LAYER.into(), CellPos::new(2, 0), CellSize::new(2, 2));
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "guards a debug assertion")]
#[should_panic(expected = "restoring a taken claim must find its cells free")]
fn restoring_over_standing_claim_panics() {
    let mut map = wall_test_map();
    let unit = shape(CellSize::ONE);
    let location = LocationComponent::new(world_pos(2, 0), Facing::SOUTH);
    map.place(mark(&location, unit, OccupancyClass::Claim));

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
    let unit = shape(CellSize::ONE);
    let location = LocationComponent::new(world_pos(0, 0), Facing::SOUTH);
    map.place(mark(&location, unit, OccupancyClass::Claim));

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
    assert!(map.nav_grid().is_occupied_by(GROUND_LAYER, cell));

    map.set_static_occupied(GROUND_LAYER, cell, false);
    assert!(!map.nav_grid().is_occupied_by(GROUND_LAYER, cell));
}

#[test]
fn static_write_lands_under_mover_claim() {
    // The flip is judged on the static plane alone: a mover's claim over the
    // cell says nothing about its static bit, so blocking claimed ground is
    // a legal first write, not a double-block.
    let mut map = wall_test_map();
    let unit = shape(CellSize::ONE);
    let location = LocationComponent::new(world_pos(3, 1), Facing::SOUTH);
    map.place(mark(&location, unit, OccupancyClass::Claim));

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
/// The terrain the test maps are made of.
const LAND: TerrainId = TerrainId::new(1);
/// The lake's terrain, registered second.
const DEEP_WATER: TerrainId = TerrainId::new(2);
/// The domain [`land_registry`] declares: the ground layer over land.
const WALKER_DOMAIN: DomainId = DomainId::new(0);

/// What a solid body of `shape` standing at `location` marks on the plane
/// `class` selects.
fn mark(location: &LocationComponent, shape: MoverShape, class: OccupancyClass) -> GridMark {
    GridMark {
        footprint: CellRect::new(CellPos::from(location.position), shape.size),
        layers: shape.layers,
        class,
    }
}

/// A `size` shape over land on the ground layer.
fn shape(size: CellSize) -> MoverShape {
    MoverShape::new(TerrainMask::from(LAND), GROUND_LAYER, size)
}

/// The single-cell ground mover whose abstraction the test maps carry.
fn ground_shape() -> MoverShape {
    shape(CellSize::ONE)
}

/// A `width × height` land grid with the ground layer registered.
fn grid(width: u32, height: u32) -> NavGrid {
    let mut nav_grid = NavGrid::new(width, height, |_| LAND);
    nav_grid.add_layer(GROUND_LAYER);
    nav_grid
}

/// An 8×2 map whose only cut is whatever the test places, with a hierarchy
/// for the single-cell ground mover.
fn wall_test_map() -> Map {
    let mut map = Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Cell,
        grid(8, 2),
        vec![],
    );
    map.serve_movers(&walker_registry());
    map
}

/// A [`land_registry`] whose one type is a single-cell ground mover.
fn walker_registry() -> ContentRegistry {
    let mut registry = land_registry();
    registry.register(
        EntityTypeDef::new("walker")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_movement(
                utils::fixed("0.5"),
                utils::fixed("0.5"),
                FixedU64::ONE,
                FixedU64::from_num(360),
                FixedU64::from_num(360),
            ),
    );
    registry
}

/// A registry declaring the ground layer, the land terrain and the
/// [`WALKER_DOMAIN`] over it.
fn land_registry() -> ContentRegistry {
    let mut registry = ContentRegistry::default();
    assert_eq!(registry.register_layer("ground"), GROUND_LAYER);
    assert_eq!(registry.register_terrain("land"), LAND);
    assert_eq!(
        registry.register_domain(
            "ground",
            DomainDef::new(GROUND_LAYER, Terrains::Only(LAND.into()))
        ),
        WALKER_DOMAIN
    );
    registry
}

/// An 8×2 map under the continuous model, whose claim plane is a rebuilt
/// summary rather than law.
fn continuous_test_map() -> Map {
    Map::new(
        "test",
        Projection::Isometric,
        MovementModel::Continuous,
        grid(8, 2),
        vec![],
    )
}

fn world_pos(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// A [`land_registry`] that also knows the deep-water terrain.
fn lake_registry() -> ContentRegistry {
    let mut registry = land_registry();
    assert_eq!(registry.register_terrain("deep_water"), DEEP_WATER);
    registry
}

/// A 4×4 land map with one deep-water cell at (1, 1).
fn lake_map() -> MapData {
    let mut data = MapData::new("pond", Projection::Isometric, 4, 4, "land");
    data.set_terrain((1, 1), "deep_water");
    data
}

/// Builds a map placing a post of 100 health with `pools` started as named.
fn place_post_with(pools: Vec<(String, PoolInitial)>) -> Map {
    let mut registry = land_registry();
    registry.register(
        EntityTypeDef::new("post")
            .with_location(WALKER_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND_LAYER)
            .with_pool(Pool::health(100)),
    );
    let mut data = MapData::new("field", Projection::Isometric, 8, 8, "land");
    data.add_placement(Placement {
        type_name: "post".to_string(),
        cell: (2, 2),
        owner: None,
        amount: None,
        pools,
    });
    Map::from_data(&data, &registry)
}
