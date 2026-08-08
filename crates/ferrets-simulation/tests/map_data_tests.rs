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
fn new_map_declares_no_terrain() {
    let data = MapData::new("field", Projection::Isometric, 4, 3);

    assert_eq!(data.name(), "field");
    assert_eq!(data.width(), 4);
    assert_eq!(data.height(), 3);
    assert!(data.terrains().is_empty());
    assert!(data.terrain_cells().is_empty());
    assert!(data.slots().is_empty());
    assert!(data.placements().is_empty());
}

#[test]
#[should_panic(expected = "map dimensions must be greater than 0")]
fn zero_dimension_panics() {
    MapData::new("field", Projection::Isometric, 4, 0);
}

//
// ─── Movement model ───────────────────────────────────────────────────────────
//

#[test]
fn movement_model_defaults_to_cell() {
    let data = MapData::new("field", Projection::Isometric, 4, 3);

    assert_eq!(data.movement_model(), MovementModel::Cell);
}

#[test]
fn set_movement_model_replaces_default() {
    let mut data = MapData::new("field", Projection::Isometric, 4, 3);
    data.set_movement_model(MovementModel::Continuous);

    assert_eq!(data.movement_model(), MovementModel::Continuous);
}

//
// ─── Terrain ──────────────────────────────────────────────────────────────────
//

#[test]
fn fill_and_set_cover_every_cell_through_palette() {
    let mut data = MapData::new("field", Projection::Isometric, 2, 2);
    data.fill_terrain("grass");
    data.set_terrain((1, 0), "water");
    data.set_terrain((1, 1), "water");

    assert_eq!(data.terrains(), ["grass", "water"]);
    assert_eq!(data.terrain_cells(), [0, 1, 0, 1]);
}

#[test]
fn setting_cell_to_known_terrain_reuses_palette_entry() {
    let mut data = MapData::new("field", Projection::Isometric, 2, 1);
    data.fill_terrain("grass");
    data.set_terrain((0, 0), "grass");

    assert_eq!(data.terrains(), ["grass"]);
}

#[test]
#[should_panic(expected = "declare the map's terrain with fill_terrain before setting cells")]
fn setting_terrain_before_fill_panics() {
    let mut data = MapData::new("field", Projection::Isometric, 2, 2);
    data.set_terrain((0, 0), "water");
}

#[test]
#[should_panic(expected = "cell (2, 0) is out of bounds")]
fn setting_terrain_out_of_bounds_panics() {
    let mut data = MapData::new("field", Projection::Isometric, 2, 2);
    data.fill_terrain("grass");
    data.set_terrain((2, 0), "water");
}

//
// ─── Seats and placements ─────────────────────────────────────────────────────
//

#[test]
fn seats_take_slot_ids_in_declaration_order() {
    let mut data = MapData::new("field", Projection::Isometric, 8, 8);

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
    let mut data = MapData::new("field", Projection::Isometric, 8, 8);
    data.add_player_slot((8, 1));
}

#[test]
#[should_panic(expected = "placement 'tree' cell (3, 9) is out of bounds")]
fn placement_out_of_bounds_panics() {
    let mut data = MapData::new("field", Projection::Isometric, 8, 8);
    data.add_placement(Placement {
        type_name: "tree".to_string(),
        cell: (3, 9),
        owner: None,
        amount: None,
    });
}

#[test]
#[should_panic(expected = "placement 'barracks' owner 1 is not a declared seat")]
fn placement_with_undeclared_owner_panics() {
    let mut data = MapData::new("field", Projection::Isometric, 8, 8);
    data.add_player_slot((1, 1));
    data.add_placement(Placement {
        type_name: "barracks".to_string(),
        cell: (2, 2),
        owner: Some(1),
        amount: None,
    });
}
