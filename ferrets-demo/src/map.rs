//! Demo map: a 96×96 field of land split into four quadrants by rivers, with a
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
//! `#` start/base, `$` gold mine, `~` the lake's shallow rim, `≈` its deep
//! water, `F` the boss sea fortress, `|`/`-` the rivers — deep water between
//! shallow banks, so only what swims crosses them — and `=` their fords, the
//! five-cell gaps that make each quadrant border a choke point. Each corner
//! also has a small tree grove. The mines and groves are neutral map
//! placements; the fortress and its ships belong to the boss slot; each lobby
//! slot's base is spawned by the game for its occupant. The camera opens
//! framed on the local player's start point.

use ferrets_content::pool::PoolInitial;
use ferrets_geometry::projection::Projection;
use ferrets_math::FixedU64;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::{
    map::Map,
    map_data::{MapData, Placement},
    session::{GameSession, player_id::PlayerId},
};

use crate::{content, scenario::CurrentScenario};

/// The demo map's name — the session and replays reference it by this.
pub const NAME: &str = "demo";

/// The demo's navigation layers, by content name.
pub const GROUND: &str = "ground";
pub const AIR: &str = "air";

/// The slot id of the boss — the environment AI holding the lake. The map's
/// owner-tagged placements and the lobby's appended slot must agree on it.
pub const BOSS: PlayerId = 4;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 96;

/// The lake: a circle in the map center, deep water within a shallow rim.
const LAKE_CENTER: (u32, u32) = (48, 48);
const LAKE_RADIUS_SQ: i64 = 81;
/// The squared radius within which the lake is deep water rather than shallow.
const DEEP_RADIUS_SQ: i64 = 49;

/// The rivers: deep water along the map's middle axes, from each edge into the
/// lake's deep water, between shallow banks, each arm broken by a five-cell
/// ford.
const RIVER_AXIS: u32 = 48;
/// How many cells of shallow bank flank a river's deep water on each side.
const RIVER_BANK: u32 = 1;
/// The fords: the spans left open in the near-edge and far-edge river arms.
const FORD_NEAR: (u32, u32) = (17, 21);
const FORD_FAR: (u32, u32) = (74, 78);

/// The boss fortress footprint anchor (5×5, centered on the lake's middle).
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
    from_lake_center(x, y) <= LAKE_RADIUS_SQ
}

/// Looks up a map this game knows by name. The session names its map — like
/// scenarios, maps are content the game must already have — and the scene
/// spawner and replay playback resolve the name here.
pub fn by_name(name: &str) -> Option<MapData> {
    (name == NAME).then(data)
}

/// The map the running game opened on: a loaded scenario carries its own, and
/// any other game names one this game knows.
pub fn opened(session: &GameSession, scenario: Option<&CurrentScenario>) -> Option<MapData> {
    match scenario {
        Some(scenario) => Some(scenario.0.map.clone()),
        None => by_name(session.map()),
    }
}

/// The demo map as data: the field of land with its central lake, the corner
/// start points, the neutral mines and groves, and the boss fleet.
pub fn data() -> MapData {
    let mut data = MapData::new(NAME, Projection::Isometric, WIDTH, HEIGHT, "land");

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            // The lake's deep water and the rivers' meet: a river's deep
            // channel runs on through the lake's shallow rim.
            match lake_water(x, y).max(river_water(x, y)) {
                Water::Deep => data.set_terrain((x, y), "deep_water"),
                Water::Shallow => data.set_terrain((x, y), "shallow_water"),
                Water::Dry => {}
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
            pools: Vec::new(),
        });
    }
    for &cell in TREES {
        data.add_placement(Placement {
            type_name: "tree".to_string(),
            cell,
            owner: None,
            amount: Some(400),
            pools: Vec::new(),
        });
    }
    data.add_placement(Placement {
        type_name: "sea_fortress".to_string(),
        cell: FORTRESS,
        owner: Some(BOSS),
        amount: None,
        pools: Vec::new(),
    });
    // The boss's ships come limping into the bay at half their health.
    for &cell in &SHIPS {
        data.add_placement(Placement {
            type_name: "ship".to_string(),
            cell,
            owner: Some(BOSS),
            amount: None,
            pools: vec![(
                "health".to_string(),
                PoolInitial::Share(FixedU64::lit("0.5")),
            )],
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

/// How deep the water over a cell is, shallowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Water {
    /// No water: the cell is land.
    Dry,
    /// Shallow water.
    Shallow,
    /// Deep water.
    Deep,
}

/// The squared distance of a cell from the lake's center.
fn from_lake_center(x: u32, y: u32) -> i64 {
    let dx = x as i64 - LAKE_CENTER.0 as i64;
    let dy = y as i64 - LAKE_CENTER.1 as i64;
    dx * dx + dy * dy
}

/// The water the lake gives the cell: deep in its middle, shallow on its rim.
fn lake_water(x: u32, y: u32) -> Water {
    if from_lake_center(x, y) <= DEEP_RADIUS_SQ {
        Water::Deep
    } else if in_lake(x, y) {
        Water::Shallow
    } else {
        Water::Dry
    }
}

/// The water a river gives the cell: deep on a middle axis, shallow on its
/// banks, and none on a ford or beyond the banks.
fn river_water(x: u32, y: u32) -> Water {
    let ford = |along: u32| {
        (FORD_NEAR.0..=FORD_NEAR.1).contains(&along) || (FORD_FAR.0..=FORD_FAR.1).contains(&along)
    };
    // How far the cell lies across each arm: from the vertical arm's axis
    // where the column is not a ford row, and from the horizontal arm's axis
    // where the row is not a ford column.
    let across = [
        (!ford(y)).then(|| x.abs_diff(RIVER_AXIS)),
        (!ford(x)).then(|| y.abs_diff(RIVER_AXIS)),
    ];
    match across.into_iter().flatten().min() {
        Some(0) => Water::Deep,
        Some(distance) if distance <= RIVER_BANK => Water::Shallow,
        Some(_) | None => Water::Dry,
    }
}
