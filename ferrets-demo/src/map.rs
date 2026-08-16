//! Demo map: a 96×96 grass field split into four quadrants by rivers, with a
//! start point in each corner and a boss lake in the center.
//!
//! Cells are `(x, y)` with `x` right and `y` down. Start points are indexed
//! `[top-left, bottom-right, bottom-left, top-right]`, so the first two slots sit
//! on opposite corners (a natural 1v1):
//!
//! ```text
//! y=0 +---------------------|---------------------+
//! y=12|  P0#(12,12)         |          (78,12)#P3 |
//!     |     $(21,12)        =           $(69,12)  |
//!     |                     |                     |
//! y=48|---=------------~~~≈≈F≈≈~~~------------=---|
//!     |                     |                     |
//!     |     $(21,78)        =           $(69,78)  |
//! y=78|  P2#(12,78)         |          (78,78)#P1 |
//! y=95+---------------------|---------------------+
//! ```
//!
//! `#` start/base, `$` gold mine, `~` the lake, `F` the boss sea fortress, `|`/`-`
//! the rivers, `=` their fords — the three-cell gaps that make each quadrant
//! border a chokepoint. Each corner also has a small tree grove. The mines and
//! groves are neutral map placements; the fortress and its ships belong to the
//! boss slot; each lobby slot's base is spawned by the game for its occupant.
//! The camera opens framed on the local player's start point.

use ferrets_geometry::projection::Projection;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::{
    map::Map,
    map_data::{MapData, Placement},
    session::player_slot::PlayerId,
};

use crate::content;

/// The demo map's name — the session and replays reference it by this.
pub const NAME: &str = "demo";

/// The demo's navigation layers, by content name.
pub const GROUND: &str = "ground";
pub const WATER: &str = "water";
pub const AIR: &str = "air";

/// The slot id of the boss — the environment AI holding the lake. The map's
/// owner-tagged placements and the lobby's appended slot must agree on it.
pub const BOSS: PlayerId = 4;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 96;

/// The lake: a circle of water terrain in the map center.
const LAKE_CENTER: (u32, u32) = (48, 48);
const LAKE_RADIUS_SQ: i64 = 81;

/// The rivers: one-cell water lines along the map's middle axes, from each
/// edge to the lake, each broken by a three-cell ford.
const RIVER_AXIS: u32 = 48;
/// The fords: the spans left open in the near-edge and far-edge river arms.
const FORD_NEAR: (u32, u32) = (18, 20);
const FORD_FAR: (u32, u32) = (75, 77);

/// The boss fortress footprint anchor (3×3, roughly centered in the lake).
const FORTRESS: (u32, u32) = (46, 46);
/// Boss ship cells, spread around the fortress on open water.
const SHIPS: [(u32, u32); 3] = [(44, 50), (52, 46), (50, 52)];

/// Player start cells, one per corner, indexed by slot id.
pub const START_POINTS: [(u32, u32); 4] = [(12, 12), (78, 78), (12, 78), (78, 12)];
/// Gold mine cells: one near each start point.
const GOLD_MINES: [(u32, u32); 4] = [(21, 12), (69, 78), (21, 78), (69, 12)];
/// Tree cells (1×1 wood sources): a grove near each start.
const TREES: &[(u32, u32)] = &[
    // Grove near player 0.
    (8, 16),
    (9, 16),
    (10, 16),
    (8, 17),
    (9, 17),
    (10, 17),
    // Grove near player 1.
    (83, 76),
    (84, 76),
    (85, 76),
    (83, 77),
    (84, 77),
    (85, 77),
    // Grove near player 2.
    (8, 73),
    (9, 73),
    (10, 73),
    (8, 74),
    (9, 74),
    (10, 74),
    // Grove near player 3.
    (83, 16),
    (84, 16),
    (85, 16),
    (83, 17),
    (84, 17),
    (85, 17),
];

/// Returns `true` if the cell lies within the lake.
pub fn in_lake(x: u32, y: u32) -> bool {
    let dx = x as i64 - LAKE_CENTER.0 as i64;
    let dy = y as i64 - LAKE_CENTER.1 as i64;
    dx * dx + dy * dy <= LAKE_RADIUS_SQ
}

/// Returns `true` if the cell lies on a river: the middle axes outside the
/// lake, minus the fords.
fn in_river(x: u32, y: u32) -> bool {
    if in_lake(x, y) {
        return false;
    }
    let ford = |along: u32| {
        (FORD_NEAR.0..=FORD_NEAR.1).contains(&along) || (FORD_FAR.0..=FORD_FAR.1).contains(&along)
    };
    (x == RIVER_AXIS && !ford(y)) || (y == RIVER_AXIS && !ford(x))
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
            if in_lake(x, y) || in_river(x, y) {
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
