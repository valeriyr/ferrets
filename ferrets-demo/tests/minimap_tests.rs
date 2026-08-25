//! The minimap's pixel arithmetic, its widget hit-test, and what a refresh
//! composes: the parts a wrong camera, a non-square map, or a mis-ordered
//! transform would break. The arithmetic runs on its own; the composition and
//! the look run against the real demo map.

mod utils;

use std::f32::consts::{FRAC_1_SQRT_2, SQRT_2};

use bevy::prelude::*;
use ferrets_demo::{
    minimap::{self, Canvas},
    render::FogReveal,
};

//
// ─── Widget geometry ──────────────────────────────────────────────────────────
//

#[test]
fn widget_size_spans_frame_on_both_axes_of_square_map() {
    assert_eq!(
        minimap::widget_size(192.0, (96, 96)),
        Vec2::new(192.0, 192.0)
    );
}

#[test]
fn widget_size_letterboxes_map_wider_than_tall() {
    // The longer axis takes the whole frame; the shorter one keeps proportion
    // rather than stretching to fill it.
    assert_eq!(
        minimap::widget_size(200.0, (100, 50)),
        Vec2::new(200.0, 100.0)
    );
}

#[test]
fn widget_size_letterboxes_map_taller_than_wide() {
    assert_eq!(
        minimap::widget_size(200.0, (50, 100)),
        Vec2::new(100.0, 200.0)
    );
}

#[test]
fn widget_size_refuses_to_divide_by_empty_map() {
    assert_eq!(minimap::widget_size(192.0, (0, 0)), Vec2::ZERO);
}

#[test]
fn cell_at_maps_widget_corners_to_map_corners() {
    assert_eq!(minimap::cell_at((96, 96), Vec2::ZERO), Some((0, 0)));
    assert_eq!(
        minimap::cell_at((96, 96), Vec2::new(0.999, 0.999)),
        Some((95, 95))
    );
}

#[test]
fn cell_at_maps_middle_to_middle_cell() {
    assert_eq!(minimap::cell_at((96, 96), Vec2::splat(0.5)), Some((48, 48)));
}

#[test]
fn cell_at_keeps_far_edge_inside_map() {
    // Rounding at the very edge must not name a cell one past the last.
    let cells = (10, 10);
    let (x, y) =
        minimap::cell_at(cells, Vec2::new(0.99999, 0.99999)).expect("edge lands on a cell");
    assert!(x < cells.0 && y < cells.1);
}

#[test]
fn cell_at_refuses_position_outside_widget() {
    // A click can land in the frame around the map without landing on the map.
    assert_eq!(minimap::cell_at((96, 96), Vec2::new(-0.01, 0.5)), None);
    assert_eq!(minimap::cell_at((96, 96), Vec2::new(0.5, 1.01)), None);
}

#[test]
fn cell_at_refuses_empty_map() {
    assert_eq!(minimap::cell_at((0, 0), Vec2::splat(0.5)), None);
}

#[test]
fn overhang_reaches_across_square_frame_but_not_below_it() {
    // A square map's diamond is wider than its frame and, once flattened,
    // shorter — so it needs room to the side and none above or below.
    let past = minimap::overhang(192.0, (96, 96));
    assert!(
        (past.x - 192.0 * (SQRT_2 - 1.0) / 2.0).abs() < 1e-3,
        "{past}"
    );
    assert_eq!(past.y, 0.0);
}

#[test]
fn overhang_follows_widget_rather_than_frame_on_map_wider_than_tall() {
    // The widget letterboxes to 192 × 96, which the frame constant alone cannot
    // describe: the diamond spreads only a little sideways but now outgrows the
    // frame vertically too.
    let past = minimap::overhang(192.0, (96, 48));
    assert!((past.x - 5.8).abs() < 0.1, "{past}");
    assert!((past.y - 2.9).abs() < 0.1, "{past}");
}

#[test]
fn overhang_follows_widget_rather_than_frame_on_map_taller_than_wide() {
    // Letterboxed to 96 × 192, the diamond reaches far past a frame half as
    // wide — nine times the sideways room the square map asked for.
    let past = minimap::overhang(192.0, (48, 96));
    assert!((past.x - 53.8).abs() < 0.1, "{past}");
    assert_eq!(past.y, 0.0);
}

//
// ─── World-to-cell conversion ─────────────────────────────────────────────────
//

#[test]
fn cell_of_reads_world_point_as_cell() {
    // Bevy's y points up and the map's down, so a world point below the origin
    // sits at a positive cell row.
    assert_eq!(minimap::cell_of(Vec2::new(0.0, 0.0)), IVec2::new(0, 0));
    assert_eq!(minimap::cell_of(Vec2::new(48.0, -48.0)), IVec2::new(1, 1));
}

#[test]
fn cell_of_floors_toward_lower_cell() {
    assert_eq!(minimap::cell_of(Vec2::new(31.9, -31.9)), IVec2::new(0, 0));
    assert_eq!(minimap::cell_of(Vec2::new(32.1, -32.1)), IVec2::new(1, 1));
}

#[test]
fn cell_of_keeps_points_past_map_edge_negative() {
    // A zoomed-out viewport reaches past the field; the canvas clips it, so the
    // conversion must not clamp and fake a corner.
    assert_eq!(minimap::cell_of(Vec2::new(-40.0, 40.0)), IVec2::new(-2, -2));
}

//
// ─── Canvas painting ──────────────────────────────────────────────────────────
//

#[test]
fn canvas_starts_filled() {
    let canvas = Canvas::new(3, 2, RED);
    assert_eq!(canvas.get(0, 0), Some(RED));
    assert_eq!(canvas.get(2, 1), Some(RED));
    assert_eq!(canvas.bytes().len(), 3 * 2 * 4);
}

#[test]
fn get_refuses_cell_past_edge() {
    let canvas = Canvas::new(3, 2, RED);
    assert_eq!(canvas.get(3, 0), None);
    assert_eq!(canvas.get(0, 2), None);
}

#[test]
fn put_paints_only_named_cell() {
    let mut canvas = Canvas::new(3, 3, RED);
    canvas.put(1, 2, BLUE);
    assert_eq!(canvas.get(1, 2), Some(BLUE));
    assert_eq!(canvas.get(2, 1), Some(RED));
}

#[test]
fn put_past_edge_paints_nothing() {
    let mut canvas = Canvas::new(2, 2, RED);
    canvas.put(9, 9, BLUE);
    for y in 0..2 {
        for x in 0..2 {
            assert_eq!(canvas.get(x, y), Some(RED));
        }
    }
}

#[test]
fn fill_covers_whole_footprint() {
    // A two-by-two unit must read as two by two, not as a point.
    let mut canvas = Canvas::new(4, 4, RED);
    canvas.fill(1, 1, 2, 2, BLUE);
    assert_eq!(canvas.get(1, 1), Some(BLUE));
    assert_eq!(canvas.get(2, 2), Some(BLUE));
    assert_eq!(canvas.get(0, 1), Some(RED));
    assert_eq!(canvas.get(3, 3), Some(RED));
}

#[test]
fn fill_clips_footprint_at_edge() {
    let mut canvas = Canvas::new(3, 3, RED);
    canvas.fill(2, 2, 3, 3, BLUE);
    assert_eq!(canvas.get(2, 2), Some(BLUE));
}

#[test]
fn line_paints_both_ends() {
    let mut canvas = Canvas::new(5, 5, RED);
    canvas.line(IVec2::new(0, 0), IVec2::new(4, 0), BLUE);
    assert_eq!(canvas.get(0, 0), Some(BLUE));
    assert_eq!(canvas.get(4, 0), Some(BLUE));
    assert_eq!(canvas.get(2, 0), Some(BLUE));
    assert_eq!(canvas.get(2, 1), Some(RED));
}

#[test]
fn line_paints_diagonal_run() {
    let mut canvas = Canvas::new(4, 4, RED);
    canvas.line(IVec2::new(0, 0), IVec2::new(3, 3), BLUE);
    for along in 0..4 {
        assert_eq!(canvas.get(along, along), Some(BLUE));
    }
}

#[test]
fn line_paints_single_cell_when_ends_meet() {
    let mut canvas = Canvas::new(3, 3, RED);
    canvas.line(IVec2::new(1, 1), IVec2::new(1, 1), BLUE);
    assert_eq!(canvas.get(1, 1), Some(BLUE));
    assert_eq!(canvas.get(0, 0), Some(RED));
}

#[test]
fn line_skips_cells_outside_canvas() {
    // The run keeps the slope it would have had off-canvas, so the part that
    // lands back inside is still in the right place.
    let mut canvas = Canvas::new(4, 4, RED);
    canvas.line(IVec2::new(-2, -2), IVec2::new(2, 2), BLUE);
    assert_eq!(canvas.get(0, 0), Some(BLUE));
    assert_eq!(canvas.get(2, 2), Some(BLUE));
    assert_eq!(canvas.get(3, 3), Some(RED));
}

#[test]
fn restore_repaints_every_cell_from_base() {
    let mut base = Canvas::new(2, 2, RED);
    base.put(0, 0, BLACK);
    let mut canvas = Canvas::new(2, 2, BLUE);
    canvas.restore(&base);
    assert_eq!(canvas.get(0, 0), Some(BLACK));
    assert_eq!(canvas.get(1, 1), Some(RED));
}

//
// ─── Viewport outline ─────────────────────────────────────────────────────────
//

#[test]
fn outline_closes_around_axis_aligned_viewport() {
    let mut canvas = Canvas::new(6, 6, RED);
    canvas.outline(
        [
            IVec2::new(1, 1),
            IVec2::new(4, 1),
            IVec2::new(4, 4),
            IVec2::new(1, 4),
        ],
        BLUE,
    );
    // Every edge drawn, including the one closing back to the first corner.
    assert_eq!(canvas.get(2, 1), Some(BLUE));
    assert_eq!(canvas.get(4, 2), Some(BLUE));
    assert_eq!(canvas.get(2, 4), Some(BLUE));
    assert_eq!(canvas.get(1, 2), Some(BLUE));
    // And the enclosed ground left alone.
    assert_eq!(canvas.get(2, 2), Some(RED));
}

#[test]
fn outline_of_turned_viewport_keeps_quad_not_bounding_box() {
    // Under the diamond look the visible region is a turned square. Its
    // bounding box would claim about twice the ground, so the corners must stay
    // four separate points: cells inside the box but outside the quad — the
    // triangles by its corners — go unpainted.
    let mut canvas = Canvas::new(9, 9, RED);
    canvas.outline(
        [
            IVec2::new(4, 0),
            IVec2::new(8, 4),
            IVec2::new(4, 8),
            IVec2::new(0, 4),
        ],
        BLUE,
    );
    // The quad's own corners are painted.
    assert_eq!(canvas.get(4, 0), Some(BLUE));
    assert_eq!(canvas.get(0, 4), Some(BLUE));
    // The bounding box's corners are not — they lie outside the turned quad.
    assert_eq!(canvas.get(0, 0), Some(RED));
    assert_eq!(canvas.get(8, 0), Some(RED));
    assert_eq!(canvas.get(0, 8), Some(RED));
    assert_eq!(canvas.get(8, 8), Some(RED));
}

//
// ─── Layer shading ────────────────────────────────────────────────────────────
//

#[test]
fn dim_keeps_fraction_of_each_channel() {
    assert_eq!(minimap::dim([100, 200, 40, 255], 0.5), [50, 100, 20, 255]);
}

#[test]
fn dim_leaves_color_alone_at_full_factor() {
    assert_eq!(minimap::dim([100, 200, 40, 255], 1.0), [100, 200, 40, 255]);
}

#[test]
fn dim_keeps_alpha() {
    // Explored ground is darkened, never made see-through: the void behind it
    // must not show.
    assert_eq!(minimap::dim([200, 200, 200, 255], 0.45)[3], 255);
}

#[test]
fn bytes_of_reads_color_channels() {
    assert_eq!(
        minimap::bytes_of(Color::srgb(1.0, 0.0, 0.0)),
        [255, 0, 0, 255]
    );
    assert_eq!(
        minimap::bytes_of(Color::srgba(0.0, 0.0, 0.0, 0.0)),
        [0, 0, 0, 0]
    );
}

#[test]
fn bytes_of_clamps_channels_past_full() {
    assert_eq!(
        minimap::bytes_of(Color::srgb(2.0, -1.0, 0.5)),
        [255, 0, 128, 255]
    );
}

//
// ─── Composition against the real demo map ────────────────────────────────────
//

#[test]
fn composition_reads_terrain_per_cell() {
    // Revealed, the picture is the map: the central lake and the grass around
    // it must not come out the same color, or the terrain layer is not being
    // read per cell.
    let mut app = utils::view_app();
    app.world_mut().resource_mut::<FogReveal>().0 = true;
    utils::compose_minimap(&mut app);

    assert_ne!(painted(&app, 48, 48), painted(&app, 5, 5));
}

#[test]
fn composition_hides_ground_never_seen() {
    // Nothing is placed, so nothing has ever been seen: every cell reads as the
    // same void, lake and grass alike.
    let mut app = utils::view_app();
    utils::compose_minimap(&mut app);

    assert_eq!(painted(&app, 48, 48), painted(&app, 5, 5));
}

#[test]
fn composition_covers_whole_map() {
    let mut app = utils::view_app();
    utils::compose_minimap(&mut app);

    let canvas = app.world().resource::<minimap::Minimap>().canvas();
    assert_eq!((canvas.width(), canvas.height()), (96, 96));
    assert_eq!(canvas.bytes().len(), 96 * 96 * 4);
}

//
// ─── Following the world's look ───────────────────────────────────────────────
//

#[test]
fn flat_look_leaves_widget_square() {
    let mut app = utils::view_app();
    utils::compose_minimap(&mut app);
    utils::point_minimap(&mut app, false);

    assert_eq!(look_matrix(&mut app), Mat2::IDENTITY);
    assert_eq!(footprint(Mat2::IDENTITY), (1.0, 1.0));
}

#[test]
fn diamond_look_turns_widget_before_flattening_it() {
    // One step east on the map must read as two right and one down — the
    // isometric signature the world itself draws. Flattening before turning
    // would give a different shape entirely, which is why the turn and the
    // flattening sit on separate nodes.
    let mut app = utils::view_app();
    utils::compose_minimap(&mut app);
    utils::point_minimap(&mut app, true);

    let look = look_matrix(&mut app);
    let east = look * Vec2::X;
    let south = look * Vec2::Y;

    assert!((east.x / east.y - 2.0).abs() < 1e-5, "east reads {east}");
    assert!(
        (south.x / south.y + 2.0).abs() < 1e-5,
        "south reads {south}"
    );
    // Both head down the screen, and away from each other across it.
    assert!(east.x > 0.0 && east.y > 0.0);
    assert!(south.x < 0.0 && south.y > 0.0);
}

#[test]
fn diamond_look_widens_and_flattens_widget() {
    // The turn is not paid for by shrinking the picture — the diamond spans √2
    // of the flat width and half its height, keeping the map's area rather than
    // trading it away. The frame shifts inward by the overhang so the wider
    // shape still clears the window edge.
    let mut app = utils::view_app();
    utils::compose_minimap(&mut app);
    utils::point_minimap(&mut app, true);

    let (across, down) = footprint(look_matrix(&mut app));

    assert!((across - SQRT_2).abs() < 1e-5, "spans {across} across");
    assert!((down - FRAC_1_SQRT_2).abs() < 1e-5, "spans {down} down");
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Stand-in colors, distinct enough that a mis-painted cell is unmistakable.
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// What the composed minimap holds for one cell.
fn painted(app: &App, x: u32, y: u32) -> [u8; 4] {
    app.world()
        .resource::<minimap::Minimap>()
        .canvas()
        .get(x, y)
        .expect("cell inside the demo map")
}

/// The turn and flattening the widget is currently pointed by, composed the way
/// Bevy composes a parent's transform with its child's.
fn look_matrix(app: &mut App) -> Mat2 {
    let of = |transform: &UiTransform| {
        Mat2::from(transform.rotation) * Mat2::from_diagonal(transform.scale)
    };
    let frame = *app
        .world_mut()
        .query_filtered::<&UiTransform, With<minimap::MinimapFrame>>()
        .single(app.world())
        .expect("one frame");
    let picture = *app
        .world_mut()
        .query_filtered::<&UiTransform, With<minimap::MinimapNode>>()
        .single(app.world())
        .expect("one picture");
    of(&frame) * of(&picture)
}

/// How wide and tall the widget sits on screen under `look`, as multiples of an
/// untransformed one: the bounding box of the unit square's four corners.
fn footprint(look: Mat2) -> (f32, f32) {
    let corners = [
        look * Vec2::new(0.5, 0.5),
        look * Vec2::new(0.5, -0.5),
        look * Vec2::new(-0.5, -0.5),
        look * Vec2::new(-0.5, 0.5),
    ];
    let span = |axis: fn(Vec2) -> f32| {
        let values = corners.map(axis);
        values.iter().copied().fold(f32::MIN, f32::max)
            - values.iter().copied().fold(f32::MAX, f32::min)
    };
    (span(|corner| corner.x), span(|corner| corner.y))
}
