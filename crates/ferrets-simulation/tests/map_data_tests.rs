//! Authoring a [`MapData`]: the mutators keep the described map consistent —
//! everything stays in bounds and the terrain always covers the whole grid.

use ferrets_geometry::projection::Projection;
use ferrets_simulation::{
    map_data::{MapData, Placement},
    movement_model::MovementModel,
};

//
// ─── Construction ─────────────────────────────────────────────────────────────
//

#[test]
fn new_map_is_one_terrain_everywhere() {
    let data = MapData::new("field", Projection::Isometric, 4, 3, "land");

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
    MapData::new("field", Projection::Isometric, 4, 0, "land");
}

//
// ─── Movement model ───────────────────────────────────────────────────────────
//

#[test]
fn movement_model_defaults_to_cell() {
    let data = MapData::new("field", Projection::Isometric, 4, 3, "land");

    assert_eq!(data.movement_model(), MovementModel::Cell);
}

#[test]
fn set_movement_model_replaces_default() {
    let mut data = MapData::new("field", Projection::Isometric, 4, 3, "land");
    data.set_movement_model(MovementModel::Continuous);

    assert_eq!(data.movement_model(), MovementModel::Continuous);
}

//
// ─── Terrain ──────────────────────────────────────────────────────────────────
//

#[test]
fn set_terrain_covers_every_cell_through_palette() {
    let mut data = MapData::new("field", Projection::Isometric, 2, 2, "land");
    data.set_terrain((1, 0), "deep_water");
    data.set_terrain((1, 1), "deep_water");

    assert_eq!(data.terrains(), ["land", "deep_water"]);
    assert_eq!(data.terrain_cells(), [0, 1, 0, 1]);
}

#[test]
fn setting_cell_to_known_terrain_reuses_palette_entry() {
    let mut data = MapData::new("field", Projection::Isometric, 2, 1, "land");
    data.set_terrain((0, 0), "land");

    assert_eq!(data.terrains(), ["land"]);
}

#[test]
#[should_panic(expected = "cell (2, 0) is out of bounds")]
fn setting_terrain_out_of_bounds_panics() {
    let mut data = MapData::new("field", Projection::Isometric, 2, 2, "land");
    data.set_terrain((2, 0), "deep_water");
}

//
// ─── Seats and placements ─────────────────────────────────────────────────────
//

#[test]
fn seats_take_slot_ids_in_declaration_order() {
    let mut data = MapData::new("field", Projection::Isometric, 8, 8, "land");

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
    let mut data = MapData::new("field", Projection::Isometric, 8, 8, "land");
    data.add_player_slot((8, 1));
}

#[test]
#[should_panic(expected = "placement 'tree' cell (3, 9) is out of bounds")]
fn placement_out_of_bounds_panics() {
    let mut data = MapData::new("field", Projection::Isometric, 8, 8, "land");
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
    let mut data = MapData::new("field", Projection::Isometric, 8, 8, "land");
    data.add_player_slot((1, 1));
    data.add_placement(Placement {
        type_name: "barracks".to_string(),
        cell: (2, 2),
        owner: Some(1),
        amount: None,
        pools: Vec::new(),
    });
}
