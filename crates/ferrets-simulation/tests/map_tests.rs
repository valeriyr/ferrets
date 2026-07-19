//! The live [`Map`]: start points, and building from [`MapData`] — the grid
//! registers the content's layer vocabulary, and per-cell terrain seeds which
//! layers each cell blocks.

use ferrets_pathfinder::{astar::Projection, nav_grid::NavGrid, nav_pos::NavPos};
use ferrets_simulation::{content::registry::ContentRegistry, map::Map, map_data::MapData};

//
// ─── Start points ─────────────────────────────────────────────────────────────
//

#[test]
fn start_point_indexes_by_player() {
    let map = Map::new(
        "test",
        Projection::Isometric,
        NavGrid::new(8, 8),
        vec![Some(NavPos::new(1, 2)), None, Some(NavPos::new(5, 6))],
    );

    assert_eq!(map.start_point(0), Some(NavPos::new(1, 2)));
    assert_eq!(map.start_point(1), None, "a seat without a start position");
    assert_eq!(map.start_point(2), Some(NavPos::new(5, 6)));
    assert_eq!(map.start_point(3), None, "a seat the map does not declare");
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
    let lake = NavPos::new(1, 1);
    assert!(!map.nav_grid().is_passable(ground, lake));
    assert!(map.nav_grid().is_passable(water, lake));

    let grass = NavPos::new(0, 0);
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
            let cell = NavPos::new(x, y);
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
