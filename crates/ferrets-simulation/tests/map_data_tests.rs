//! Authoring a [`MapData`]: the mutators keep the described map consistent —
//! everything stays in bounds and the terrain always covers the whole grid.

use ferrets_geometry::projection::Projection;
use ferrets_nav_grid::{
    relief::Relief,
    slope::{Direction, Slope},
};
use ferrets_simulation::{
    map_data::{MapData, Placement},
    movement_model::MovementModel,
};

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_map_is_one_terrain_everywhere() {
    let data = field_map(4, 3);

    assert_eq!(data.name(), "field");
    assert_eq!(data.width(), 4);
    assert_eq!(data.height(), 3);
    assert_eq!(data.terrains(), ["land"]);
    // 4 × 3 = 12 cells, each the palette's first entry.
    assert_eq!(data.terrain_cells(), [0; 12]);
    assert!(data.slots().is_empty());
    assert!(data.placements().is_empty());
}

#[test]
#[should_panic(expected = "map dimensions must be greater than 0")]
fn zero_dimension_panics() {
    field_map(4, 0);
}

//
// ─── Movement model ───────────────────────────────────────────────────────────
//

#[test]
fn movement_model_defaults_to_cell() {
    let data = field_map(4, 3);

    assert_eq!(data.movement_model(), MovementModel::Cell);
}

#[test]
fn set_movement_model_replaces_default() {
    let mut data = field_map(4, 3);
    data.set_movement_model(MovementModel::Continuous);

    assert_eq!(data.movement_model(), MovementModel::Continuous);
}

//
// ─── Terrain ──────────────────────────────────────────────────────────────────
//

#[test]
fn set_terrain_covers_every_cell_through_palette() {
    let mut data = field_map(2, 2);
    data.set_terrain((1, 0), "deep_water");
    data.set_terrain((1, 1), "deep_water");

    assert_eq!(data.terrains(), ["land", "deep_water"]);
    assert_eq!(data.terrain_cells(), [0, 1, 0, 1]);
}

#[test]
fn setting_cell_to_known_terrain_reuses_palette_entry() {
    let mut data = field_map(2, 1);
    data.set_terrain((0, 0), "land");

    assert_eq!(data.terrains(), ["land"]);
}

#[test]
#[should_panic(expected = "cell (2, 0) is out of bounds")]
fn setting_terrain_out_of_bounds_panics() {
    let mut data = field_map(2, 2);
    data.set_terrain((2, 0), "deep_water");
}

//
// ─── Seats and placements ─────────────────────────────────────────────────────
//

#[test]
fn seats_take_slot_ids_in_declaration_order() {
    let mut data = field_map(8, 8);

    assert_eq!(data.add_player_slot((1, 1)), 0);
    assert_eq!(data.add_environment_slot(), 1);
    assert_eq!(data.add_player_slot((6, 6)), 2);

    assert_eq!(
        data.player_seats().collect::<Vec<_>>(),
        [(0, (1, 1)), (2, (6, 6))]
    );
    assert_eq!(data.environment_seats().collect::<Vec<_>>(), [1]);
}

#[test]
#[should_panic(expected = "player seat start (8, 1) is out of bounds")]
fn player_seat_out_of_bounds_panics() {
    let mut data = field_map(8, 8);
    data.add_player_slot((8, 1));
}

#[test]
#[should_panic(expected = "placement 'tree' cell (3, 9) is out of bounds")]
fn placement_out_of_bounds_panics() {
    let mut data = field_map(8, 8);
    data.add_placement(Placement {
        type_name: "tree".to_string(),
        cell: (3, 9),
        owner: None,
        amount: None,
        pools: Vec::new(),
    });
}

#[test]
#[should_panic(expected = "placement 'barracks' owner 1 is not a declared seat")]
fn placement_with_undeclared_owner_panics() {
    let mut data = field_map(8, 8);
    data.add_player_slot((1, 1));
    data.add_placement(Placement {
        type_name: "barracks".to_string(),
        cell: (2, 2),
        owner: Some(1),
        amount: None,
        pools: Vec::new(),
    });
}

//
// ─── Reliefs ──────────────────────────────────────────────────────────────────
//

#[test]
fn relief_reads_back_levels_and_slopes() {
    let mut data = field_map(2, 1);
    data.add_relief("ground", 1);
    data.set_cell_level("ground", (1, 0), 2);
    data.set_slope("ground", (0, 0), Slope::Ramp(Direction::East));

    let relief = [
        Relief {
            level: 1,
            slope: Some(Slope::Ramp(Direction::East)),
        },
        Relief {
            level: 2,
            slope: None,
        },
    ];
    assert_eq!(data.relief("ground"), Some(&relief[..]));
    assert_eq!(data.relief("air"), None);
    assert_eq!(
        data.reliefs().collect::<Vec<_>>(),
        vec![("ground", &relief[..])]
    );
}

#[test]
#[should_panic(expected = "map 'field' gives layer 'ground' a relief twice")]
fn relief_given_twice_panics() {
    let mut data = field_map(2, 1);
    data.add_relief("ground", 1);
    data.add_relief("ground", 2);
}

#[test]
#[should_panic(expected = "map 'field' gives layer 'ground' no relief")]
fn cell_level_on_layer_without_relief_panics() {
    field_map(2, 1).set_cell_level("ground", (0, 0), 2);
}

#[test]
#[should_panic(expected = "map 'field' gives layer 'ground' no relief")]
fn slope_on_layer_without_relief_panics() {
    field_map(2, 1).set_slope("ground", (0, 0), Slope::Cliff(Direction::East));
}

#[test]
#[should_panic(expected = "cell (2, 0) is out of bounds")]
fn cell_level_out_of_bounds_panics() {
    let mut data = field_map(2, 1);
    data.add_relief("ground", 1);
    data.set_cell_level("ground", (2, 0), 2);
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A `width × height` map named `field`, every cell land.
fn field_map(width: u32, height: u32) -> MapData {
    MapData::new("field", Projection::Isometric, width, height, "land")
}
