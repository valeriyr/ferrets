//! Demo map: a 96×96 field split into four quadrants by rivers meeting in a
//! central lake that holds the boss's fleet, with a start point in each
//! corner, a sea along the top-left joined to the vertical river, two
//! plateaus and a mesa.
//!
//! Cells are `(x, y)` with `x` right and `y` down. Start points are indexed
//! `[top-left, bottom-right, bottom-left, top-right]`, so the first two slots sit
//! on opposite corners (a natural 1v1):
//!
//! ```text
//! y=0 +≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈≈|---------------------+
//!     |~~~~~~~~~~~~~~~~~~~~~|                     |
//! y=12|                     |  $(69,12) (78,12)#P3|
//! y=16| P0#(12,16)          =                     |
//!     |     $(21,16)        |                     |
//! y=28|                     |  >::MMM:::          |
//! y=37|     MM              |     ^               |
//! y=48|--------=---------( F )-------------=------|
//! y=57|             v       |                     |
//! y=64|          ::::::<    |                     |
//! y=78| P2#(12,78)          =         (78,78)#P1  |
//!     |     $(21,78)        |        $(69,78)     |
//! y=95+---------------------|---------------------+
//! ```
//!
//! `#` start/base, `$` gold mine, `≈` the sea's deep water, `~` its shallow
//! coast, `( F )` the central lake with the boss sea fortress and its ships,
//! `|`/`-` the rivers — deep water between shallow banks, so only what swims
//! crosses them — and `=` their fords, five cells of shallow water that
//! soldiers wade and ships sail, each quadrant border's choke point. `:` is a
//! plateau one level up, `M` ground two levels up — a mountain on the east
//! plateau with no ramp, and a mesa between P0 and P2 standing straight from
//! low ground, its cliff too tall even for the stilt walker — and `>`/`^`/`v`/`<`
//! the ramps, each rising the way it points. Each corner also has a small tree
//! grove. The mines and groves are neutral map placements; the fortress and
//! its ships belong to the boss slot; each lobby slot's base is spawned by
//! the game for its occupant. The camera opens framed on the local player's
//! start point.

use std::cmp::Ordering;

use ferrets_content::pool::PoolInitial;
use ferrets_geometry::projection::Projection;
use ferrets_math::FixedU64;
use ferrets_nav_grid::slope::{Direction, Slope};
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

/// The sea: the top-left strip reaching east into the vertical river, deep
/// water down to `SEA_DEEP_TO` and a shallow coast below it down to
/// `SEA_COAST_TO`.
const SEA_EAST: u32 = RIVER_AXIS;
const SEA_DEEP_TO: u32 = 9;
const SEA_COAST_TO: u32 = 11;

/// The lake: a circle in the map center, deep water within a shallow rim.
const LAKE_CENTER: (u32, u32) = (RIVER_AXIS, RIVER_AXIS);
const LAKE_RADIUS_SQ: i64 = 81;
/// The squared radius within which the lake is deep water rather than shallow.
const DEEP_RADIUS_SQ: i64 = 49;

/// The rivers: deep water along the map's middle axes, from edge to edge
/// through the lake, between shallow banks, each arm broken by a five-cell
/// ford.
const RIVER_AXIS: u32 = 48;
/// How many cells of shallow bank flank a river's deep water on each side.
const RIVER_BANK: u32 = 1;
/// The fords: the spans of shallow water across the near-edge and far-edge
/// river arms.
const FORD_NEAR: (u32, u32) = (17, 21);
const FORD_FAR: (u32, u32) = (74, 78);

/// The level of low ground, of a plateau, and of a mountain.
const LOW: u32 = 1;
const PLATEAU: u32 = 2;
const MOUNTAIN: u32 = 3;

/// The plateau by player 3, `(origin, far corner)`, with a mountain on it.
const EAST_PLATEAU: ((u32, u32), (u32, u32)) = ((56, 22), (71, 37));
const MOUNTAIN_PEAK: ((u32, u32), (u32, u32)) = ((62, 27), (66, 31));
/// The plateau by player 2.
const WEST_PLATEAU: ((u32, u32), (u32, u32)) = ((24, 58), (39, 73));
/// A mesa two levels up straight from low ground between players 0 and 2,
/// its cliff too tall for any walker.
const MESA: ((u32, u32), (u32, u32)) = ((12, 36), (15, 39));
/// The ramps, each a band of cells rising onto a plateau, its lower cells on
/// low ground and its upper cells on the plateau's edge:
/// `(origin, far corner, the way it rises)`.
const RAMPS: [((u32, u32), (u32, u32), Direction); 4] = [
    // East plateau, west face: 3 wide, 2 long.
    ((55, 28), (56, 30), Direction::East),
    // East plateau, south face: 2 wide, 4 long.
    ((62, 36), (63, 39), Direction::North),
    // West plateau, east face: 3 wide, 2 long.
    ((39, 64), (40, 66), Direction::West),
    // West plateau, north face: 3 wide, 2 long.
    ((30, 57), (32, 58), Direction::South),
];

/// The boss fortress footprint anchor (5×5, centered on the lake's middle).
const FORTRESS: (u32, u32) = (46, 46);
/// Boss ship cells, spread around the fortress on the lake's open water.
const SHIPS: [(u32, u32); 3] = [(44, 50), (52, 46), (50, 52)];

/// Player start cells, one per corner, indexed by slot id.
pub const START_POINTS: [(u32, u32); 4] = [(12, 16), (78, 78), (12, 78), (78, 12)];
/// Gold mine cells: one near each start point.
const GOLD_MINES: [(u32, u32); 4] = [(21, 16), (69, 78), (21, 78), (69, 12)];
/// Tree cells (1×1 wood sources): a grove near each start.
const TREES: &[(u32, u32)] = &[
    // Grove near player 0.
    (8, 20),
    (9, 20),
    (10, 20),
    (8, 21),
    (9, 21),
    (10, 21),
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

/// The demo map as data: the field of land with its sea, lake, rivers and
/// plateaus, the corner start points, the neutral mines and groves, and the
/// boss fleet.
pub fn data() -> MapData {
    let mut data = MapData::new(NAME, Projection::Isometric, WIDTH, HEIGHT, "land");
    data.add_relief(GROUND, LOW);

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            match sea_water(x, y).max(lake_water(x, y)).max(river_water(x, y)) {
                Water::Deep => data.set_terrain((x, y), "deep_water"),
                Water::Shallow => data.set_terrain((x, y), "shallow_water"),
                Water::Dry => {}
            }
            data.set_cell_level(GROUND, (x, y), ground_level(x, y));
        }
    }
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if let Some(slope) = slope_of(x, y) {
                data.set_slope(GROUND, (x, y), slope);
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
    // The boss's ships come limping into the lake at half their health.
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

/// The water the sea gives the cell: deep along the top, shallow on its
/// coast.
fn sea_water(x: u32, y: u32) -> Water {
    if x > SEA_EAST {
        Water::Dry
    } else if y <= SEA_DEEP_TO {
        Water::Deep
    } else if y <= SEA_COAST_TO {
        Water::Shallow
    } else {
        Water::Dry
    }
}

/// The water the lake gives the cell: deep in its middle, shallow on its rim.
fn lake_water(x: u32, y: u32) -> Water {
    let dx = x as i64 - LAKE_CENTER.0 as i64;
    let dy = y as i64 - LAKE_CENTER.1 as i64;
    match dx * dx + dy * dy {
        distance if distance <= DEEP_RADIUS_SQ => Water::Deep,
        distance if distance <= LAKE_RADIUS_SQ => Water::Shallow,
        _ => Water::Dry,
    }
}

/// The ground layer's level at a cell: a mountain on the east plateau and the
/// mesa, the two plateaus, low ground everywhere else.
fn ground_level(x: u32, y: u32) -> u32 {
    let within = |(origin, corner): ((u32, u32), (u32, u32))| {
        (origin.0..=corner.0).contains(&x) && (origin.1..=corner.1).contains(&y)
    };
    if within(MOUNTAIN_PEAK) || within(MESA) {
        MOUNTAIN
    } else if within(EAST_PLATEAU) || within(WEST_PLATEAU) {
        PLATEAU
    } else {
        LOW
    }
}

/// The water a river gives the cell: deep on a middle axis, shallow on its
/// banks and across a ford, and none beyond the banks.
fn river_water(x: u32, y: u32) -> Water {
    let ford = |along: u32| {
        (FORD_NEAR.0..=FORD_NEAR.1).contains(&along) || (FORD_FAR.0..=FORD_FAR.1).contains(&along)
    };
    // Each arm by how far the cell lies across it and whether the cell is on
    // one of its fords: the vertical arm across columns along rows, the
    // horizontal one across rows along columns.
    [
        (x.abs_diff(RIVER_AXIS), ford(y)),
        (y.abs_diff(RIVER_AXIS), ford(x)),
    ]
    .into_iter()
    .map(|(across, on_ford)| match (across, on_ford) {
        (across, _) if across > RIVER_BANK => Water::Dry,
        (_, true) => Water::Shallow,
        (0, false) => Water::Deep,
        (_, false) => Water::Shallow,
    })
    .max()
    .expect("a river has two arms")
}

/// The slope a cell is part of: a ramp where one lies, else a cliff where
/// the cell borders ground at another level, rising the way the level does.
fn slope_of(x: u32, y: u32) -> Option<Slope> {
    let ramp = RAMPS.into_iter().find_map(|(origin, corner, rises)| {
        ((origin.0..=corner.0).contains(&x) && (origin.1..=corner.1).contains(&y)).then_some(rises)
    });
    match ramp {
        Some(rises) => Some(Slope::Ramp(rises)),
        None => cliff_rises(x, y).map(Slope::Cliff),
    }
}

/// The way a cliff through the cell rises, or `None` where the cell borders
/// no ground at another level: from the cardinal neighbors that are, else from
/// the diagonal ones, which make the cell an outer corner.
fn cliff_rises(x: u32, y: u32) -> Option<Direction> {
    let level = ground_level(x, y);
    let diagonal = [
        Direction::NorthEast,
        Direction::SouthEast,
        Direction::SouthWest,
        Direction::NorthWest,
    ];
    [Direction::CARDINAL, diagonal]
        .into_iter()
        .find_map(|ways| {
            let (mut sum_x, mut sum_y, mut found) = (0, 0, false);
            for way in ways {
                let (dx, dy) = way.step();
                let Some((nx, ny)) = x
                    .checked_add_signed(dx)
                    .zip(y.checked_add_signed(dy))
                    .filter(|&(nx, ny)| nx < WIDTH && ny < HEIGHT)
                else {
                    continue;
                };
                // A higher neighbor lies the way the cliff rises, a lower one
                // the way it falls.
                let sign = match ground_level(nx, ny).cmp(&level) {
                    Ordering::Greater => 1,
                    Ordering::Less => -1,
                    Ordering::Equal => continue,
                };
                (sum_x, sum_y, found) = (sum_x + sign * dx, sum_y + sign * dy, true);
            }
            found.then(|| Direction::of_step(sum_x.signum(), sum_y.signum()))
        })
}
