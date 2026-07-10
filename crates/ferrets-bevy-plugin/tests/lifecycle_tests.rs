//! Order-independent lifecycle tests: dying phase, removal, and corpse decay.

mod utils;

use bevy::prelude::*;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_simulation::{
    components::dying::{CorpseComponent, DyingComponent},
    entity_index::EntityIndex,
    map::Map,
    selection::Selection,
    spawn,
};

#[test]
fn destroy_entity_starts_dying_and_removes_after_timer() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (entity, id) = spawn::spawn_entity(world, "dummy", utils::pos(4, 4), None).unwrap();
    world.resource_mut::<Selection>().set(0, vec![id]);

    spawn::destroy_entity(world, entity);

    // The entity immediately leaves the alive set and all selections, but holds
    // its cell through the dying phase.
    assert!(world.get::<DyingComponent>(entity).is_some());
    assert_eq!(world.resource::<EntityIndex>().alive(id), None);
    assert!(world.resource::<Selection>().get(0).is_empty());
    assert!(
        world
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, NavPos::new(4, 4))
    );

    // The dying phase runs its 3-tick timer, then the entity is despawned.
    utils::run_ticks(&mut app, 4);
    utils::assert_despawned(app.world_mut(), entity);
}

#[test]
fn dying_leaves_corpse_that_decays() {
    let mut app = utils::combat_app();
    let world = app.world_mut();
    let (entity, _) = spawn::spawn_entity(world, "dummy", utils::pos(4, 4), None).unwrap();

    spawn::destroy_entity(world, entity);

    // The destroyed dummy is dying, but it is not a corpse — it is playing out
    // its death transition.
    assert!(world.get::<DyingComponent>(entity).is_some());
    assert!(world.get::<CorpseComponent>(entity).is_none());

    // The dummy finishes dying and leaves bones behind. The bones declare
    // ground occupation and the cell is free, so they block it while decaying.
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::count_of_type(app.world_mut(), "bones"), 1);
    {
        let world = app.world_mut();
        utils::assert_despawned(world, entity);
        assert!(
            world
                .resource::<Map>()
                .nav_grid()
                .is_occupied_by(utils::GROUND, NavPos::new(4, 4))
        );
        let corpses = world.query::<&CorpseComponent>().iter(world).count();
        assert_eq!(corpses, 1);
    }

    // The bones decay through their own dying phase, disappear, and free the cell.
    utils::run_ticks(&mut app, 3);
    assert_eq!(utils::count_of_type(app.world_mut(), "bones"), 0);
    assert!(
        !app.world_mut()
            .resource::<Map>()
            .nav_grid()
            .is_occupied_by(utils::GROUND, NavPos::new(4, 4))
    );
}
