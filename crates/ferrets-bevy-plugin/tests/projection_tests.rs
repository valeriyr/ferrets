//! Projection-dependent geometry in the running game: diagonal travel time
//! under the cell model, and body shapes under the continuous model.

mod utils;

use ferrets_geometry::{cell_pos::CellPos, projection::Projection};
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand, components::location::LocationComponent, movement_model::MovementModel,
};

#[test]
fn orthogonal_diagonal_walk_takes_its_priced_time() {
    // Six diagonal cells at speed 0.5: the isometric walk takes 12 ticks,
    // the orthogonal one √2 as long — the time its own path costs charge.
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Orthogonal, MovementModel::Cell);
    let (soldier, id) = utils::spawn_owned(&mut app, "soldier", 2, 2, 0);

    utils::select(&mut app, id);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(8, 8),
            flush: true,
        },
    );

    // Where the isometric walk would already have arrived, the orthogonal
    // one is still on its way…
    utils::run_ticks(&mut app, utils::APPLY + 13);
    assert_eq!(utils::cell_of(app.world_mut(), soldier), CellPos::new(6, 6));

    // …and lands within the √2 budget.
    utils::run_ticks(&mut app, 10);
    assert_eq!(utils::cell_of(app.world_mut(), soldier), CellPos::new(8, 8));
}

#[test]
fn orthogonal_bodies_separate_as_circles() {
    let mut app = utils::orders_app();
    utils::install_map(&mut app, Projection::Orthogonal, MovementModel::Continuous);
    let first = utils::spawn_owned(&mut app, "soldier", 3, 5, 0);
    let second = utils::spawn_owned(&mut app, "soldier", 7, 5, 0);
    for (_, id) in [first, second] {
        utils::select(&mut app, id);
        utils::push_command(
            &mut app,
            PlayerCommand::Move {
                target: utils::pos(5, 5),
                flush: true,
            },
        );
    }

    utils::run_ticks(&mut app, 300);

    // Resting circles keep a full Euclidean body apart — not merely a
    // Chebyshev one, which would allow diagonal centers √2 closer.
    let world = app.world_mut();
    let position_of = |entity: bevy::prelude::Entity| {
        world
            .entity(entity)
            .get::<LocationComponent>()
            .unwrap()
            .position
    };
    let a = position_of(first.0);
    let b = position_of(second.0);
    let dx = (a.x.abs_diff(b.x).to_bits() as u128).pow(2);
    let dy = (a.y.abs_diff(b.y).to_bits() as u128).pow(2);
    let minimum = (FixedU64::from_num(0.99).to_bits() as u128).pow(2);
    assert!(
        dx + dy >= minimum,
        "circle bodies must rest a Euclidean body apart: {a:?} vs {b:?}"
    );
    // The exact equilibrium — identical to the isometric one, because
    // bodies are circles under every projection: both walks touch the
    // contested spot the same tick and finish there, and the pushing pass
    // parts the coincident pair half a body each way.
    assert_eq!(a, utils::position_bits(0x4_8000_0000, 0x5_0000_0000));
    assert_eq!(b, utils::position_bits(0x5_8000_0000, 0x5_0000_0000));
}
