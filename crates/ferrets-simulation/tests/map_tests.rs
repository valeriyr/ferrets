//! Map start points.

use ferrets_pathfinder::{astar::Projection, nav_grid::NavGrid, nav_pos::NavPos};
use ferrets_simulation::map::Map;

#[test]
fn start_point_indexes_by_player() {
    let map = Map::new(
        "test",
        Projection::Isometric,
        NavGrid::new(8, 8),
        vec![NavPos::new(1, 2), NavPos::new(5, 6)],
    );

    assert_eq!(map.start_points().len(), 2);
    assert_eq!(map.start_point(0), Some(NavPos::new(1, 2)));
    assert_eq!(map.start_point(1), Some(NavPos::new(5, 6)));
    assert_eq!(map.start_point(2), None);
}
