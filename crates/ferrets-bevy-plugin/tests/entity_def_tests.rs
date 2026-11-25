//! An entity's body as its readers say: its shape and the cells it stands on.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    entity_type_def::EntityTypeDef, location::Solidity, pool::Pool, registry::ContentRegistry,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
    shape::Shape,
};
use ferrets_math::{FixedU64, fixed_urect::FixedURect};
use ferrets_simulation::{
    command::PlayerCommand,
    components::location::LocationComponent,
    entity_def,
    movement_model::{self, MovementModel},
    simulation_id::SimulationId,
};

//
// ─── Shape ────────────────────────────────────────────────────────────────────
//

#[test]
fn cell_walker_is_box_of_its_bounds() {
    let mut app = body_app(MovementModel::Cell);
    let (walker, _) = utils::create_owned(&mut app, "walker", 5, 5, 0);

    assert_eq!(
        entity_def::shape(app.world(), walker),
        Shape::cell(utils::cell(5, 5))
    );
}

#[test]
fn continuous_walker_is_circle_of_its_radius() {
    let mut app = body_app(MovementModel::Continuous);
    let (walker, _) = utils::create_owned(&mut app, "walker", 5, 5, 0);

    // Anchored at (5, 5): its middle at 5 + 1/2, its radius 0.5.
    assert_eq!(
        entity_def::shape(app.world(), walker),
        Shape::Circle {
            middle: utils::point("5.5", "5.5"),
            radius: utils::fixed("0.5"),
        }
    );
}

#[test]
fn standing_body_is_box_under_either_model() {
    for model in [MovementModel::Cell, MovementModel::Continuous] {
        let mut app = body_app(model);
        let (tower, _) = utils::create_owned(&mut app, "tower", 5, 5, 0);

        // A 2×2 on (5, 5): bounds 5–7 by 5–7.
        assert_eq!(
            entity_def::shape(app.world(), tower),
            Shape::Box(FixedURect::new(
                utils::point("5", "5"),
                utils::point("7", "7")
            )),
            "{model:?}"
        );
    }
}

#[test]
fn shape_at_candidate_is_shape_it_would_have_there() {
    let mut app = body_app(MovementModel::Continuous);
    let (walker, _) = utils::create_owned(&mut app, "walker", 5, 5, 0);

    // Anchored at (8, 3): its middle at (8.5, 3.5).
    assert_eq!(
        entity_def::shape_at(app.world(), walker, utils::cell(8, 3)),
        Shape::Circle {
            middle: utils::point("8.5", "3.5"),
            radius: utils::fixed("0.5"),
        }
    );
}

//
// ─── Body cells ───────────────────────────────────────────────────────────────
//

#[test]
fn cell_walker_stands_on_its_claim() {
    let mut app = body_app(MovementModel::Cell);
    let (walker, _) = utils::create_owned(&mut app, "walker", 5, 5, 0);

    assert_eq!(
        entity_def::body_cells(app.world(), walker),
        Some(vec![utils::cell(5, 5)])
    );
}

#[test]
fn crossing_cell_walker_stands_on_cell_it_steps_to() {
    for walker_type in ["walker", "slider"] {
        let mut app = body_app(MovementModel::Cell);
        let (walker, walker_id) = utils::create_owned(&mut app, walker_type, 5, 5, 0);
        walk_to(&mut app, (walker, walker_id), "9.5", "5.5");

        // A quarter of the way from (5, 5) east, its middle at 5.75, it is
        // still anchored at (5, 5) yet stands on (6, 5), whether or not it
        // claims cells.
        assert_eq!(
            utils::position_of(app.world(), walker),
            utils::point("5.75", "5.5"),
            "{walker_type}"
        );
        assert_eq!(
            utils::anchor_of(app.world(), walker),
            utils::cell(5, 5),
            "{walker_type}"
        );
        assert_eq!(
            entity_def::body_cells(app.world(), walker),
            Some(vec![utils::cell(6, 5)]),
            "{walker_type}"
        );
    }
}

#[test]
fn continuous_walker_stands_on_cells_its_circle_overlaps() {
    let mut app = body_app(MovementModel::Continuous);
    let (walker, walker_id) = utils::create_owned(&mut app, "walker", 5, 5, 0);
    utils::select(&mut app, walker_id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::point("6", "5.5"),
            flush: true,
        },
    );
    // Half a cell at a quarter a tick is 2 ticks; the run goes on past its
    // arrival, where it stays.
    utils::run_ticks(&mut app, utils::APPLY + 6);

    // Its middle on x 6, its half-cell circle from 5.5 to 6.5: cells 5 and 6,
    // though it claims (6, 5) alone.
    assert_eq!(
        utils::position_of(app.world(), walker),
        utils::point("6", "5.5")
    );
    assert_eq!(
        entity_def::body_cells(app.world(), walker),
        Some(vec![utils::cell(5, 5), utils::cell(6, 5)])
    );
    assert_eq!(
        entity_def::grid_mark(app.world(), walker).map(|mark| mark.footprint),
        Some(CellRect::cell(utils::cell(6, 5)))
    );
}

#[test]
fn standing_body_stands_on_its_footprint() {
    for model in [MovementModel::Cell, MovementModel::Continuous] {
        let mut app = body_app(model);
        let (tower, _) = utils::create_owned(&mut app, "tower", 5, 5, 0);

        // The 2×2 anchored at (5, 5): the four cells its grid mark stamps.
        assert_eq!(
            entity_def::body_cells(app.world(), tower),
            Some(
                CellRect::new(utils::cell(5, 5), CellSize::new(2, 2))
                    .cells()
                    .collect::<Vec<CellPos>>()
            ),
            "{model:?}"
        );
    }
}

#[test]
#[should_panic(
    expected = "a cell-model body off its cells is crossing toward the anchor it steps to"
)]
fn cell_body_off_its_cells_with_no_walk_panics() {
    let mut app = body_app(MovementModel::Cell);
    let (tower, _) = utils::create_owned(&mut app, "tower", 5, 5, 0);
    // Its middle (6, 6) written to (6.6, 6) by hand: off the lattice, with no
    // walk stepping it on.
    app.world_mut()
        .get_mut::<LocationComponent>(tower)
        .expect("the tower stands somewhere")
        .position = utils::point("6.6", "6");

    entity_def::body_cells(app.world(), tower);
}

#[test]
#[should_panic(expected = "a body read for its grid mark stands on the grid")]
fn seated_body_has_no_grid_mark() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);
    // The first grub is bred on the tenth tick, seated on its berth at
    // (10.5, 13.5): lined up on (10, 13), yet off the grid.
    utils::run_ticks(&mut app, 10);
    let grub = utils::single_owned_of_type(app.world_mut(), "grub", 0);

    entity_def::grid_mark(app.world(), grub);
}

#[test]
fn boarded_body_stands_off_grid() {
    let mut app = utils::transport_app();
    let rifleman = boarded_rifleman(&mut app);

    assert_eq!(entity_def::body_cells(app.world(), rifleman), None);
}

#[test]
#[should_panic(expected = "an entity on the map has a location")]
fn boarded_body_has_no_shape() {
    let mut app = utils::transport_app();
    let rifleman = boarded_rifleman(&mut app);

    entity_def::shape(app.world(), rifleman);
}

#[test]
fn seated_body_stands_off_grid() {
    let mut app = utils::brood_app(MovementModel::Cell);
    utils::place(&mut app, "hatch", 10, 10, 0);
    // The first grub is bred on the tenth tick, seated on its berth.
    utils::run_ticks(&mut app, 10);
    let grub = utils::single_owned_of_type(app.world_mut(), "grub", 0);

    assert!(entity_def::attached(app.world(), grub));
    assert_eq!(entity_def::body_cells(app.world(), grub), None);
    // Its shape is still its body at its seat: the hatch anchored at
    // (10, 10), the first berth 0.5 and 3.5 in, the 1×1 box around (10.5,
    // 13.5).
    assert_eq!(
        entity_def::shape(app.world(), grub),
        Shape::cell(utils::cell(10, 13))
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// An app under `model` with a `walker` (a solid 1×1 mover of half-cell
/// radius walking a quarter cell a tick), a `slider` (the same lying
/// underfoot, claiming no cells) and a `tower` (a solid 2×2 that never
/// walks).
fn body_app(model: MovementModel) -> App {
    let mut app = utils::make_app(utils::human_slots(2));
    utils::install_map(&mut app, Projection::Isometric, model);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        for (name, solidity) in [("walker", Solidity::Solid), ("slider", Solidity::Underfoot)] {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(
                        utils::GROUND_DOMAIN,
                        CellSize::ONE,
                        Some(utils::fixed("0.5")),
                        solidity,
                        utils::GROUND,
                    )
                    .with_movement(
                        utils::fixed("0.25"),
                        FixedU64::ONE,
                        FixedU64::from_num(360),
                        FixedU64::from_num(360),
                    ),
            );
        }
        registry.register(
            EntityTypeDef::new("tower")
                .with_location(
                    utils::GROUND_DOMAIN,
                    CellSize::new(2, 2),
                    None,
                    Solidity::Solid,
                    utils::GROUND,
                )
                .with_pool(Pool::health(100)),
        );
    }
    utils::start_game(&mut app);
    app
}

/// A rifleman of player 0 boarded by order onto a ferry beside it.
fn boarded_rifleman(app: &mut App) -> Entity {
    let (rifleman, rifleman_id) = utils::create_owned(app, "rifleman", 5, 5, 0);
    let (ferry, ferry_id) = utils::create_owned(app, "ferry", 6, 5, 0);
    utils::run_ticks(app, 1);
    utils::board(app, rifleman_id, ferry_id);
    utils::run_until_aboard(app, ferry, 1, 40);
    rifleman
}

/// Orders `walker` to walk to the point `(x, y)` and runs until it is part
/// way across its first step.
fn walk_to(app: &mut App, walker: (Entity, SimulationId), x: &str, y: &str) {
    let (entity, id) = walker;
    utils::select(app, id);
    utils::push_command(
        app,
        PlayerCommand::Move {
            target: utils::point(x, y),
            flush: true,
        },
    );
    utils::run_ticks(app, utils::APPLY);
    for _ in 0..10 {
        if movement_model::is_mid_crossing(app.world(), entity) {
            return;
        }
        utils::run_ticks(app, 1);
    }
    panic!("the walker sets off within ten ticks");
}
