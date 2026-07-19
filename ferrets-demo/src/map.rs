//! Demo map: a 64×64 grass field with a start point in each of the four corners
//! and a boss lake in the center.
//!
//! Cells are `(x, y)` with `x` right and `y` down. Start points are indexed
//! `[top-left, bottom-right, bottom-left, top-right]`, so the first two slots sit
//! on opposite corners (a natural 1v1):
//!
//! ```text
//! y=0 +-----------------------------------------+
//! y=8 |  P0#(8,8)                    (52,8)#P3  |
//!     |     $(14,8)                 $(46,8)     |
//!     |                                         |
//! y=32|               ~~~≈F≈~~~                 |
//!     |                                         |
//!     |     $(14,52)               $(46,52)     |
//! y=52|  P2#(8,52)                  (52,52)#P1  |
//! y=63+-----------------------------------------+
//! ```
//!
//! `#` start/base, `$` gold mine, `~` the lake, `F` the boss sea fortress. Each
//! corner also has a small tree grove. The mines and groves are neutral map
//! placements; the fortress and its ships belong to the boss slot; each lobby
//! slot's base is spawned by the game for its occupant. The camera opens framed
//! on the local player's start point.

use ferrets_pathfinder::astar::Projection;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::map::Map;
use ferrets_simulation::map_data::{MapData, Placement};
use ferrets_simulation::session::player_slot::PlayerId;

use crate::content;

/// The demo map's name — the session and replays reference it by this.
pub const NAME: &str = "demo";

/// The demo's navigation layers, by content name.
pub const GROUND: &str = "ground";
pub const WATER: &str = "water";

/// The slot id of the boss — the environment AI holding the lake. The map's
/// owner-tagged placements and the lobby's appended slot must agree on it.
pub const BOSS: PlayerId = 4;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;

/// The lake: a circle of water terrain in the map center.
const LAKE_CENTER: (u32, u32) = (32, 32);
const LAKE_RADIUS_SQ: i64 = 36;

/// The boss fortress footprint origin (3×3, roughly centered in the lake).
const FORTRESS: (u32, u32) = (30, 30);
/// Boss ship cells, spread around the fortress on open water.
const SHIPS: [(u32, u32); 3] = [(28, 34), (36, 30), (34, 36)];

/// Player start cells, one per corner, indexed by slot id.
pub const START_POINTS: [(u32, u32); 4] = [(8, 8), (52, 52), (8, 52), (52, 8)];
/// Gold mine cells: one near each start point.
const GOLD_MINES: [(u32, u32); 4] = [(14, 8), (46, 52), (14, 52), (46, 8)];
/// Tree cells (1×1 wood sources): a grove near each start.
const TREES: &[(u32, u32)] = &[
    // Grove near player 0.
    (4, 12),
    (5, 12),
    (6, 12),
    (4, 13),
    (5, 13),
    (6, 13),
    // Grove near player 1.
    (57, 50),
    (58, 50),
    (59, 50),
    (57, 51),
    (58, 51),
    (59, 51),
    // Grove near player 2.
    (4, 47),
    (5, 47),
    (6, 47),
    (4, 48),
    (5, 48),
    (6, 48),
    // Grove near player 3.
    (57, 12),
    (58, 12),
    (59, 12),
    (57, 13),
    (58, 13),
    (59, 13),
];

/// Returns `true` if the cell lies within the lake.
pub fn in_lake(x: u32, y: u32) -> bool {
    let dx = x as i64 - LAKE_CENTER.0 as i64;
    let dy = y as i64 - LAKE_CENTER.1 as i64;
    dx * dx + dy * dy <= LAKE_RADIUS_SQ
}

/// Looks up a map this game knows by name. The session names its map — like
/// scenarios, maps are content the game must already have — and the scene
/// spawner and replay playback resolve the name here.
pub fn by_name(name: &str) -> Option<MapData> {
    (name == NAME).then(data)
}

/// The demo map as data: the grass field with its central lake, the corner
/// start points, the neutral mines and groves, and the boss fleet.
pub fn data() -> MapData {
    let mut data = MapData::new(NAME, Projection::Isometric, WIDTH, HEIGHT);

    data.fill_terrain("grass");
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if in_lake(x, y) {
                data.set_terrain((x, y), "water");
            }
        }
    }

    for &cell in &START_POINTS {
        data.add_player_slot(cell);
    }
    let boss = data.add_environment_slot();
    debug_assert_eq!(boss, BOSS, "the boss placements name this seat");

    for &cell in &GOLD_MINES {
        data.add_placement(Placement {
            type_name: "gold_mine".to_string(),
            cell,
            owner: None,
            amount: Some(5000),
        });
    }
    for &cell in TREES {
        data.add_placement(Placement {
            type_name: "tree".to_string(),
            cell,
            owner: None,
            amount: Some(400),
        });
    }
    data.add_placement(Placement {
        type_name: "sea_fortress".to_string(),
        cell: FORTRESS,
        owner: Some(BOSS),
        amount: None,
    });
    for &cell in &SHIPS {
        data.add_placement(Placement {
            type_name: "ship".to_string(),
            cell,
            owner: Some(BOSS),
            amount: None,
        });
    }

    data
}

/// Builds the live demo map, empty of placements — the placeholder standing in
/// before a game instantiates its own map from loaded content. Its throwaway
/// registry is loaded from the demo content, so the vocabulary the map is
/// seeded against cannot drift from the one the game plays with.
pub fn build() -> Map {
    let registry = ferrets_script::content::load(&LuaEngine, content::CONTENT)
        .expect("demo content must load");
    Map::from_data(&data(), &registry)
}
