//! The demo's embedded content script and map: they load, validate, and agree
//! with each other.

use ferrets_demo::content::CONTENT;
use ferrets_demo::map;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::map::Map;

#[test]
fn content_loads_and_validates() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    for name in [
        "gold_mine",
        "tree",
        "peasant",
        "town_hall",
        "barracks",
        "archer",
        "peon",
        "great_hall",
        "orc_barracks",
        "grunt",
        "ship",
        "sea_fortress",
    ] {
        assert!(registry.entity(name).is_some(), "missing entity '{name}'");
    }
    assert!(registry.has_race("human") && registry.has_race("orc"));
    assert!(registry.has_layer(map::GROUND) && registry.has_layer(map::WATER));
    assert!(registry.has_terrain("grass") && registry.has_terrain("water"));
}

#[test]
fn map_builds_against_content_with_lake_blocking_ground() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let data = map::data();
    let live = Map::from_data(&data, &registry);

    let ground = registry.layer(map::GROUND).unwrap();
    let water = registry.layer(map::WATER).unwrap();

    // The lake center floats ships and blocks walkers; a corner is the inverse.
    let lake = NavPos::new(32, 32);
    assert!(!live.nav_grid().is_passable(ground, lake));
    assert!(live.nav_grid().is_passable(water, lake));

    let corner = NavPos::new(1, 1);
    assert!(live.nav_grid().is_passable(ground, corner));
    assert!(!live.nav_grid().is_passable(water, corner));
}

#[test]
fn boss_placements_sit_on_water() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");
    let data = map::data();
    let live = Map::from_data(&data, &registry);
    let water = registry.layer(map::WATER).unwrap();

    for placement in data
        .placements()
        .iter()
        .filter(|p| p.owner == Some(map::BOSS))
    {
        let size = registry
            .entity(&placement.type_name)
            .and_then(|def| def.location.as_ref())
            .map(|location| location.size())
            .expect("boss placement type has a location");
        for dy in 0..size.height {
            for dx in 0..size.width {
                let cell = NavPos::new(placement.cell.0 + dx, placement.cell.1 + dy);
                assert!(
                    live.nav_grid().is_passable(water, cell),
                    "boss '{}' cell ({}, {}) is not open water",
                    placement.type_name,
                    cell.x,
                    cell.y
                );
            }
        }
    }
}
