//! Per-tick fog of war recompute: re-stamps each player's visible cells from
//! the sight of their owned entities.

use bevy_ecs::{prelude::*, world::World};

use crate::{
    components::{
        entity_info::EntityInfoComponent, hidden::HiddenComponent, location::LocationComponent,
        owner::OwnerComponent,
    },
    content::registry::ContentRegistry,
    session::player_slot::PlayerId,
    visibility::VisibilityGrid,
};

/// Recomputes the visibility grid for the current tick.
///
/// Last tick's visible cells demote to explored (sticky), then every owned,
/// on-map entity reveals a circle of its `sight_range` cells for its player.
/// The result is a pure function of entity positions, the (static) sight stat,
/// and team membership, so it is identical on every node.
pub fn recompute_visibility(world: &mut World) {
    // Owned, on-map sight sources: (player, cell x, cell y, type name).
    let sources: Vec<(PlayerId, u32, u32, String)> = world
        .query_filtered::<(&EntityInfoComponent, &LocationComponent, &OwnerComponent), Without<HiddenComponent>>()
        .iter(world)
        .map(|(info, location, owner)| {
            (
                owner.player(),
                location.position.x.to_num::<u32>(),
                location.position.y.to_num::<u32>(),
                info.type_name().to_string(),
            )
        })
        .collect();

    // Resolve each source's sight range from content; an unregistered type or an
    // unset stat sees only its own cell.
    let sources: Vec<(PlayerId, u32, u32, u32)> = {
        let registry = world.resource::<ContentRegistry>();
        sources
            .into_iter()
            .map(|(player, x, y, type_name)| {
                let sight = registry.entity(&type_name).map_or(0, |def| def.sight_range);
                (player, x, y, sight)
            })
            .collect()
    };

    let mut grid = world.resource_mut::<VisibilityGrid>();
    grid.age();
    let (width, height) = (grid.width(), grid.height());
    for (player, cx, cy, sight) in sources {
        stamp_sight(&mut grid, player, cx, cy, sight, width, height);
    }
}

/// Reveals every cell within `sight` cells of `(cx, cy)` for `player` — a circle
/// (Euclidean radius), independent of the map's movement metric — clamped to the
/// map bounds.
fn stamp_sight(
    grid: &mut VisibilityGrid,
    player: PlayerId,
    cx: u32,
    cy: u32,
    sight: u32,
    width: u32,
    height: u32,
) {
    let radius = sight as i64;
    let radius_sq = radius * radius;
    let (cxi, cyi) = (cx as i64, cy as i64);
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy > radius_sq {
                continue;
            }
            let (x, y) = (cxi + dx, cyi + dy);
            if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                grid.reveal(player, x as u32, y as u32);
            }
        }
    }
}
