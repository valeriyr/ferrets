//! Renders simulation entities as placeholder colored shapes: triangles for
//! combat units, circles for workers, and squares for buildings and resources.
//!
//! Render components are attached directly to the simulation entities, so they
//! despawn automatically with them. Positions are interpolated between the
//! previous and current tick against the fixed-step overstep, so motion is
//! smooth and stays locked to the simulation cadence (it can never outrun it).
//! Unit shapes rotate to point in their facing direction.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use std::{
    collections::{HashMap, HashSet},
    f32::consts::FRAC_PI_2,
};

use bevy::prelude::*;

use crate::{map, scenario::CurrentScenario, states::InGameUi};
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2};

use ferrets_content::{
    entity_stats::EntityStatId, entity_type_def::EntityTypeDef, morph::MorphTime,
    registry::ContentRegistry, resource::ResourceSourceDef, tags,
};
use ferrets_simulation::{
    components::{
        build::{BuildComponent, UnderConstructionComponent},
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        entity_skills::SkillsComponent,
        entity_stats::StatsComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        morph::MorphComponent,
        order_queue::OrderQueueComponent,
        owner::OwnerComponent,
        rally::{RallyPointComponent, RallyTarget},
        repair::{RepairComponent, UnderRepairComponent},
        research::ResearchComponent,
        resource::{HarvestComponent, UnderHarvestComponent},
        tags::TagsComponent,
        train::{TrainComponent, TrainQueueComponent},
        transport::TransporterComponent,
    },
    impacts::PendingImpacts,
    order::Order,
    selection::Selection,
    session::GameSession,
    simulation_id::SimulationId,
    visibility::{CellVisibility, VisibilityGrid},
};

/// The one colour every projectile in flight is drawn in, so shape alone tells the
/// kinds apart.
const SHOT_COLOR: Color = Color::srgb(1.0, 0.82, 0.4);

/// Screen pixels per grid cell.
pub const CELL_PX: f32 = 32.0;

/// The interpolated render position from the previous tick.
#[derive(Component)]
pub struct PrevPos(Vec3);

/// Marks an entity that already has its render components attached.
#[derive(Component)]
pub struct Renderable;

/// Marks a renderable whose shape rotates to point in its facing direction
/// (mobile units; buildings and resources stay axis-aligned).
#[derive(Component)]
pub struct Directional;

/// A per-cell fog overlay sprite, darkened by the local team's visibility.
#[derive(Component)]
pub struct FogTile {
    x: u32,
    y: u32,
}

/// The outline a ghost redraws, matching the shape its building renders as (see
/// [`shape_for`]).
enum GhostShape {
    /// A rectangle spanning the footprint — square buildings.
    Rect { extent: Vec2 },
    /// A regular polygon of `sides` — the hexagon barracks and octagon fortress.
    Polygon { sides: u32, circumradius: f32 },
}

/// The last-seen appearance of a scouted enemy building, kept so it can be drawn
/// as a dimmed ghost while its cell is remembered but out of sight. Render-only
/// and local to this client; persists (stale) even after the real building dies
/// in the fog, until the cell is seen again.
struct GhostSprite {
    origin: (u32, u32),
    center: Vec2,
    shape: GhostShape,
}

/// Last-seen enemy buildings, keyed by [`SimulationId`] (see [`GhostSprite`]).
#[derive(Resource, Default)]
pub struct Ghosts(HashMap<SimulationId, GhostSprite>);

/// When set, the local view reveals the whole map — the fog overlay clears and
/// fogged entities draw — for inspecting the game. A presentation-only toggle;
/// the simulation and AI still respect fog, so it cannot cause a desync.
#[derive(Resource, Default)]
pub struct FogReveal(pub bool);

/// Toggles the map-reveal view (see [`FogReveal`]) on the `V` key.
pub fn toggle_fog_reveal(keys: Res<ButtonInput<KeyCode>>, mut reveal: ResMut<FogReveal>) {
    if keys.just_pressed(KeyCode::KeyV) {
        reveal.0 = !reveal.0;
    }
}

/// World-space center of a footprint, in pixels (Bevy y points up, sim y down).
pub(crate) fn world_center(position: FixedUVec2, size: CellSize) -> Vec3 {
    let cx = position.x.to_num::<f32>() + size.width as f32 / 2.0;
    let cy = position.y.to_num::<f32>() + size.height as f32 / 2.0;
    Vec3::new(cx * CELL_PX, -cy * CELL_PX, 1.0)
}

/// How far up the screen an airborne sprite is drawn from the cell it is over,
/// and how far its shadow sits below it.
const AIR_LIFT_PX: f32 = CELL_PX * 0.9;

/// The draw offset for a type: airborne things are lifted up the screen and
/// drawn over everything on the ground.
///
/// Altitude is presentation only — the simulation knows nothing about it, and a
/// flier's position is the cell it is over. The lift is what makes an air unit
/// crossing a lake or a keep read as passing above it rather than through it.
///
/// Airborne means occupying the air *alone*: something that holds the air on
/// top of a surface — a fortress tall enough to wall the sky — stands on that
/// surface and casts no flight shadow.
pub(crate) fn air_lift(registry: &ContentRegistry, def: &EntityTypeDef) -> Vec3 {
    let airborne = match (registry.layer(map::AIR), def.location) {
        (Some(air), Some(location)) => location.occupation() == *air,
        _ => false,
    };
    if airborne {
        Vec3::new(0.0, AIR_LIFT_PX, 1.0)
    } else {
        Vec3::ZERO
    }
}

fn color_for(
    owner: Option<&OwnerComponent>,
    source: Option<&ResourceSourceDef>,
    session: &GameSession,
) -> Color {
    // Resource sources are colored by what they yield, regardless of ownership.
    if let Some(source) = source {
        return match source.kind() {
            "wood" => Color::srgb(0.45, 0.30, 0.15), // tree — brown
            "gold" => Color::srgb(0.85, 0.7, 0.2),   // gold mine — yellow
            _ => Color::srgb(0.75, 0.7, 0.4),        // other source — tan
        };
    }
    // Environment combatants share one color, whichever slot seats them.
    if owner.is_some_and(|owner| session.is_environment_slot(owner.player())) {
        return Color::srgb(0.2, 0.22, 0.26); // dark slate
    }
    match owner.map(|o| o.player()) {
        Some(0) => Color::srgb(0.35, 0.55, 1.0), // player 0 — blue
        Some(1) => Color::srgb(1.0, 0.35, 0.35), // player 1 — red
        Some(2) => Color::srgb(0.4, 0.8, 0.4),   // player 2 — green
        Some(3) => Color::srgb(0.7, 0.4, 0.9),   // player 3 — purple
        _ => Color::srgb(0.75, 0.7, 0.4),        // neutral — tan
    }
}

/// The placeholder shape used for an entity type.
enum Shape {
    /// The archer — a triangle that points where it faces.
    Triangle,
    /// The grunt — a diamond.
    Diamond,
    /// Siege units (those whose hits burst) — a pentagon.
    Pentagon,
    /// Workers — a circle.
    Circle,
    /// Ships — a hull with a triangular bow pointing where it faces.
    Ship,
    /// Barracks — a hexagon, distinct from the main hall.
    Hexagon,
    /// The sea fortress — an octagon with an inner keep.
    Fortress,
    /// Support units — a disc bearing a lighter cross.
    Cross,
    /// The gryphon pair — a beast with a saddle disc and a wing bar whose
    /// span tells the two forms apart.
    Gryphon {
        /// Whether this is the airborne form, wearing the full wingspan.
        aloft: bool,
    },
    /// The zeppelin — an envelope longer than it is wide, with a gondola
    /// slung amidships and a tail fin across the stern.
    Zeppelin,
    /// The watch tower — a square base bearing a lighter lookout platform.
    WatchTower,
    /// The guard tower — the watch tower's armed upgrade: the lookout gains
    /// a darker four-point turret.
    GuardTower,
    /// Main buildings and resource sources — a square.
    Square,
}

/// Picks a shape from the entity type name. Add new types here.
fn shape_for(type_name: &str) -> Shape {
    match type_name {
        "peasant" | "peon" => Shape::Circle,
        "grunt" => Shape::Diamond,
        "archer" => Shape::Triangle,
        "mortar" => Shape::Pentagon,
        "medic" | "shaman" => Shape::Cross,
        "ship" => Shape::Ship,
        "barracks" | "war_camp" => Shape::Hexagon,
        "sea_fortress" => Shape::Fortress,
        "gryphon" => Shape::Gryphon { aloft: false },
        "gryphon_aloft" => Shape::Gryphon { aloft: true },
        "zeppelin" => Shape::Zeppelin,
        "watch_tower" => Shape::WatchTower,
        "guard_tower" => Shape::GuardTower,
        _ => Shape::Square,
    }
}

/// Attaches a placeholder shape to any simulation entity that lacks one.
///
/// The shape comes from [`shape_for`]. Units (the non-square shapes) also get a
/// [`Directional`] marker so they rotate to face their look direction.
pub fn attach_sprites(
    mut commands: Commands,
    session: Res<GameSession>,
    registry: Res<ContentRegistry>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    query: Query<
        (
            Entity,
            &EntityInfoComponent,
            &LocationComponent,
            Option<&OwnerComponent>,
        ),
        Without<Renderable>,
    >,
) {
    for (entity, info, location, owner) in &query {
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        let center = world_center(location.position, size) + air_lift(&registry, def);
        let color = color_for(owner, def.resource_source.as_ref(), &session);
        let radius = size.width.min(size.height) as f32 * CELL_PX * 0.45;

        let mut entity = commands.entity(entity);
        match shape_for(info.type_name()) {
            Shape::Triangle => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 3))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
            }
            Shape::Diamond => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 4))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
            }
            Shape::Pentagon => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 5))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
            }
            Shape::Circle => {
                entity.insert((
                    Mesh2d(meshes.add(Circle::new(radius))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
            }
            Shape::Ship => {
                // A hull with a triangular bow whose base matches the hull's
                // width, sitting flush on the forward (+Y at rest) edge; the
                // whole silhouette rotates with the facing.
                let hull = Vec2::new(0.55, 0.65) * CELL_PX;
                // An equilateral triangle with side `s` has circumradius
                // `s/√3` and inradius `s/(2√3)` — the base offset below its
                // center.
                let circumradius = hull.x / 3f32.sqrt();
                let bow = meshes.add(RegularPolygon::new(circumradius, 3));
                let material = materials.add(color);
                entity.insert((Sprite::from_color(color, hull), Directional));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(bow),
                        MeshMaterial2d(material),
                        Transform::from_translation(Vec3::new(
                            0.0,
                            hull.y / 2.0 + circumradius / 2.0,
                            0.1,
                        )),
                    ));
                });
            }
            Shape::Hexagon => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 6))),
                    MeshMaterial2d(materials.add(color)),
                ));
            }
            Shape::Fortress => {
                // An octagonal wall with a lighter inner keep.
                let keep = meshes.add(RegularPolygon::new(radius * 0.45, 8));
                let keep_color = materials.add(color.lighter(0.12));
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 8))),
                    MeshMaterial2d(materials.add(color)),
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(keep),
                        MeshMaterial2d(keep_color),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::Cross => {
                // A disc with a lighter cross laid over it, so a medic reads as
                // support at a glance rather than as another combat silhouette.
                let arm = Vec2::new(radius * 1.1, radius * 0.36);
                let mark = color.lighter(0.35);
                entity.insert((
                    Mesh2d(meshes.add(Circle::new(radius))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Sprite::from_color(mark, arm),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                    parent.spawn((
                        Sprite::from_color(mark, Vec2::new(arm.y, arm.x)),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::Gryphon { aloft } => {
                // The beast's body points where it goes and a lighter disc
                // marks the saddle its archer fights from. The wing bar
                // across the shoulders tells the forms apart at a glance:
                // tucked short on the ground, spread to a full span aloft.
                let wing_span = if aloft { radius * 3.2 } else { radius * 1.4 };
                let wing = Vec2::new(wing_span, radius * 0.4);
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius * 0.95, 3))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Sprite::from_color(color.lighter(0.12), wing),
                        Transform::from_translation(Vec3::new(0.0, -radius * 0.2, 0.1)),
                    ));
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.3))),
                        MeshMaterial2d(materials.add(color.lighter(0.3))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.2)),
                    ));
                });
            }
            Shape::Zeppelin => {
                // The envelope's nose leads the facing; the darker gondola
                // hangs amidships and the fin crosses the stern.
                let gondola = Vec2::new(radius * 0.35, radius * 0.8);
                let fin = Vec2::new(radius * 0.9, radius * 0.18);
                entity.insert((
                    Mesh2d(meshes.add(Ellipse::new(radius * 0.6, radius * 1.05))),
                    MeshMaterial2d(materials.add(color)),
                    Directional,
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Sprite::from_color(color.darker(0.12), gondola),
                        Transform::from_translation(Vec3::new(0.0, -radius * 0.1, 0.1)),
                    ));
                    parent.spawn((
                        Sprite::from_color(color.darker(0.12), fin),
                        Transform::from_translation(Vec3::new(0.0, -radius * 0.95, 0.1)),
                    ));
                });
            }
            Shape::WatchTower => {
                // A square base bearing a lighter round lookout — the height
                // that makes it answerable to anti-air, unarmed until upgraded.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.55))),
                        MeshMaterial2d(materials.add(color.lighter(0.18))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::GuardTower => {
                // The watch tower's silhouette with a darker four-point turret
                // mounted on the lookout — the visible difference the upgrade
                // buys.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.55))),
                        MeshMaterial2d(materials.add(color.lighter(0.18))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                    parent.spawn((
                        Mesh2d(meshes.add(RegularPolygon::new(radius * 0.42, 4))),
                        MeshMaterial2d(materials.add(color.darker(0.1))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.2)),
                    ));
                });
            }
            Shape::Square => {
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
            }
        }
        entity.insert((
            Transform::from_translation(center),
            PrevPos(center),
            Renderable,
        ));
    }
}

/// Strips the render components of any entity whose type was rewritten, so
/// [`attach_sprites`] rebuilds the new form's shape — a gryphon that lands
/// must stop wearing its flight silhouette.
///
/// `Changed` also fires the tick a component is added, but a freshly spawned
/// entity is not yet [`Renderable`], so only real type rewrites pass the
/// filter.
pub fn refresh_changed_sprites(
    mut commands: Commands,
    query: Query<Entity, (Changed<EntityInfoComponent>, With<Renderable>)>,
) {
    for entity in &query {
        commands
            .entity(entity)
            .remove::<(
                Renderable,
                Directional,
                PrevPos,
                Mesh2d,
                MeshMaterial2d<ColorMaterial>,
                Sprite,
            )>()
            .despawn_related::<Children>();
    }
}

/// The facing direction as a normalized world-space vector, or `None` when there
/// is no facing yet. Sim y points down; Bevy y points up.
fn facing_dir(facing: FixedVec2) -> Option<Vec2> {
    if facing == FixedVec2::ZERO {
        return None;
    }
    Vec2::new(facing.x.to_num::<f32>(), -facing.y.to_num::<f32>()).try_normalize()
}

/// Converts a sim facing direction into a Z rotation for a shape that points
/// `+Y` at rest, or `None` when there is no facing yet (so rotation is left as is).
fn facing_rotation(facing: FixedVec2) -> Option<Quat> {
    let dir = facing_dir(facing)?;
    Some(Quat::from_rotation_z(dir.y.atan2(dir.x) - FRAC_PI_2))
}

/// Snapshots each sprite's current sim position as the interpolation start, run
/// before the simulation advances (`FixedPreUpdate`).
pub fn record_prev(
    registry: Res<ContentRegistry>,
    mut query: Query<(&EntityInfoComponent, &LocationComponent, &mut PrevPos)>,
) {
    for (info, location, mut prev) in &mut query {
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        prev.0 = world_center(location.position, size) + air_lift(&registry, def);
    }
}

/// Interpolates each sprite between its previous and current sim position by the
/// fixed-step overstep, and hides off-map entities (run in `Update`).
pub fn interpolate_sprites(
    fixed: Res<Time<Fixed>>,
    session: Res<GameSession>,
    registry: Res<ContentRegistry>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    mut query: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &PrevPos,
        &mut Transform,
        &mut Visibility,
        Option<&HiddenComponent>,
        Option<&OwnerComponent>,
        Option<&Directional>,
    )>,
) {
    let alpha = fixed.overstep_fraction().clamp(0.0, 1.0);
    let local = session.local_player();
    for (info, location, prev, mut transform, mut visibility, hidden, owner, directional) in
        &mut query
    {
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        let curr = world_center(location.position, size) + air_lift(&registry, def);
        // Snap rather than slide across teleports/reveals.
        transform.translation = if prev.0.distance(curr) > 1.5 * CELL_PX {
            curr
        } else {
            prev.0.lerp(curr, alpha)
        };
        if directional.is_some()
            && let Some(rotation) = facing_rotation(location.facing)
        {
            transform.rotation = rotation;
        }
        // Own and allied entities always draw; an enemy or neutral one is hidden
        // while its cell is not in the local team's vision (fog of war). Building
        // ghosts (draw_ghosts) stand in for last-seen enemy structures.
        let fogged = !reveal.0
            && match owner {
                Some(owner)
                    if owner.player() == local || session.are_allied(local, owner.player()) =>
                {
                    false
                }
                _ => !fog.is_visible_to(
                    &session,
                    local,
                    location.position.x.to_num::<u32>(),
                    location.position.y.to_num::<u32>(),
                ),
            };
        *visibility = if hidden.is_some() || fogged {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}

/// Draws a shadow on the ground under every airborne entity (run in `Update`).
///
/// The lift alone could read as a unit standing further up the map, so the pair
/// is what says "above": the shadow marks the cell the flier actually occupies,
/// and the gap between them is the altitude.
pub fn draw_air_shadows(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    query: Query<
        (&EntityInfoComponent, &Transform, &Visibility),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    const SHADOW: Color = Color::srgba(0.0, 0.0, 0.0, 0.35);

    for (info, transform, visibility) in &query {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let def = registry.def(info.type_id());
        let lift = air_lift(&registry, def);
        if lift == Vec3::ZERO {
            continue;
        }
        let size = def.location.unwrap().size();
        let ground = transform.translation.truncate() - Vec2::new(0.0, lift.y);
        gizmos.circle_2d(
            ground,
            size.width.min(size.height) as f32 * CELL_PX * 0.3,
            SHADOW,
        );
    }
}

/// Draws a ring around the local player's selected entities (run in `Update`).
pub fn draw_selection(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    selection: Res<Selection>,
    registry: Res<ContentRegistry>,
    query: Query<
        (&EntityInfoComponent, &Transform, &Visibility),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    let selected = selection.get(session.local_player());
    if selected.is_empty() {
        return;
    }
    for (info, transform, visibility) in &query {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        if selected.contains(&info.id()) {
            let size = registry.def(info.type_id()).location.unwrap().size();
            // Larger than the sprite so the ring isn't hidden behind it.
            let radius = size.width.max(size.height) as f32 * CELL_PX * 0.7;
            gizmos.circle_2d(
                transform.translation.truncate(),
                radius,
                Color::srgb(0.2, 1.0, 0.4),
            );
        }
    }
}

/// How long a cast pulse stays on screen, in ticks — 0.45 s at the nominal
/// cadence. Counted in ticks because it marks something the simulation did: at
/// any game speed, and under any throttle, the ring covers the same stretch of
/// the game rather than the same stretch of wall time.
const PULSE_TICKS: u32 = 9;

/// Ring pulses marking skills that have just been cast.
#[derive(Resource, Default)]
pub struct SkillPulses {
    /// Which of each caster's skills were off cooldown last frame, in declaration
    /// order. A skill leaving that set started its cooldown, which only a
    /// successful cast does — so rejected casts never draw a pulse.
    was_ready: HashMap<SimulationId, Vec<bool>>,
    /// Live pulses: the caster and the tick its cast was spotted on.
    active: Vec<(SimulationId, u32)>,
}

/// Draws an expanding ring on a unit for a moment after one of its skills is cast
/// (run in `Update`).
///
/// The cast is spotted from the caster's own cooldowns rather than from the issued
/// command, so a cast the simulation refused — too little energy, or still on
/// cooldown — draws nothing. The ring marks the caster, which is also the target
/// for every self-targeted skill; a skill aimed at another entity would still
/// mark the caster.
pub fn draw_skill_pulses(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    fixed: Res<Time<Fixed>>,
    mut pulses: ResMut<SkillPulses>,
    casters: Query<(&EntityInfoComponent, &SkillsComponent)>,
    rendered: Query<
        (&EntityInfoComponent, &Transform, &Visibility),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    let mut seen = HashSet::new();
    for (info, skills) in &casters {
        let ready: Vec<bool> = skills.skills().map(|id| skills.ready(id)).collect();
        seen.insert(info.id());
        if let Some(previous) = pulses.was_ready.get(&info.id())
            && previous.len() == ready.len()
            && previous
                .iter()
                .zip(&ready)
                .any(|(before, now)| *before && !*now)
        {
            pulses.active.push((info.id(), session.tick()));
        }
        pulses.was_ready.insert(info.id(), ready);
    }
    pulses.was_ready.retain(|id, _| seen.contains(id));

    // Ticks elapsed, with the fixed step's overstep folded in so the ring grows
    // smoothly between ticks rather than in steps.
    let now = session.tick() as f32 + fixed.overstep_fraction();
    pulses
        .active
        .retain(|&(_, started)| now - started as f32 <= PULSE_TICKS as f32);

    for &(caster, started) in &pulses.active {
        let Some((_, transform, _)) = rendered.iter().find(|(info, _, visibility)| {
            info.id() == caster && !matches!(visibility, Visibility::Hidden)
        }) else {
            continue;
        };
        // Expand and fade over the pulse's life.
        let progress = ((now - started as f32) / PULSE_TICKS as f32).clamp(0.0, 1.0);
        let radius = CELL_PX * (0.35 + 0.55 * progress);
        gizmos.circle_2d(
            transform.translation.truncate(),
            radius,
            Color::srgba(1.0, 0.85, 0.35, 1.0 - progress),
        );
    }
}

/// Draws each shot in flight as a dot sliding from its origin toward its impact
/// (run in `Update`).
///
/// The simulation stores no position for a shot — only the ticks it was released on
/// and lands on — so the dot is interpolated here. A shot over cells the local
/// player cannot see is not drawn, so a shell fired out of unexplored terrain
/// reveals nothing.
pub fn draw_shots(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    fixed: Res<Time<Fixed>>,
    impacts: Res<PendingImpacts>,
    registry: Res<ContentRegistry>,
    grid: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    targets: Query<(&EntityInfoComponent, &Transform, Option<&HiddenComponent>), With<Renderable>>,
    holders: Query<(&TransporterComponent, &Transform)>,
) {
    let tick = session.tick();
    let overstep = fixed.overstep_fraction();
    for shot in impacts.in_flight() {
        let flight = shot.lands_on_tick.saturating_sub(shot.emitted_on_tick);
        if flight == 0 {
            continue;
        }
        let elapsed = tick.saturating_sub(shot.emitted_on_tick) as f32 + overstep;
        let progress = (elapsed / flight as f32).clamp(0.0, 1.0);

        // Drawn from the shooter itself, so a shot from a large one leaves its middle
        // rather than the corner cell its position names. A hidden shooter stands
        // nowhere — a garrisoned archer's arrow leaves its holder, exactly as the
        // simulation releases it — and the recorded release point is the last
        // fallback for a shooter that has since died or gone dark.
        let from = targets
            .iter()
            .find(|(info, _, hidden)| info.id() == shot.attacker && hidden.is_none())
            .map(|(_, transform, _)| transform.translation)
            .or_else(|| {
                holders
                    .iter()
                    .find(|(transporter, _)| transporter.passengers.contains(&shot.attacker))
                    .map(|(_, transform)| transform.translation)
            })
            .unwrap_or_else(|| world_center(shot.origin, CellSize::ONE));
        // Every shot is aimed at an entity, and the damage follows that entity
        // wherever it moves, so the shot is drawn heading there. The committed point
        // is the fallback for a target that is gone or out of sight — and, once a
        // weapon can be aimed at bare ground, for a shot that never had one.
        let to = targets
            .iter()
            .find(|(info, _, hidden)| Some(info.id()) == shot.target && hidden.is_none())
            .map(|(_, transform, _)| transform.translation)
            .unwrap_or_else(|| world_center(shot.impact, CellSize::ONE));
        let at = from.lerp(to, progress);

        let cell = CellPos::new(
            (at.x / CELL_PX).max(0.0) as u32,
            (-at.y / CELL_PX).max(0.0) as u32,
        );
        if !reveal.0
            && grid.visibility_to(&session, session.local_player(), cell.x, cell.y)
                != CellVisibility::Visible
        {
            continue;
        }
        // Shapes tell the kinds apart; the colour is shared so every shot in the air
        // reads as the same class of thing at a glance.
        let at = at.truncate();
        match shot_shape(registry.projectile_name(shot.projectile)) {
            ShotShape::Arrow => {
                let along = (to - from).truncate().normalize_or_zero() * CELL_PX * 0.22;
                gizmos.line_2d(at - along, at + along, SHOT_COLOR);
            }
            ShotShape::Ball { radius } => {
                gizmos.circle_2d(at, radius, SHOT_COLOR);
            }
            ShotShape::Shell { circumradius } => {
                gizmos.primitive_2d(
                    &RegularPolygon::new(circumradius, 3),
                    Isometry2d::from_translation(at),
                    SHOT_COLOR,
                );
            }
        }
    }
}

/// How one projectile kind is drawn.
enum ShotShape {
    /// A short line lying along the flight direction.
    Arrow,
    /// A circle.
    Ball {
        /// Radius in pixels.
        radius: f32,
    },
    /// A triangle, for a shot that bursts on arrival.
    Shell {
        /// Distance from the centre to a corner, in pixels.
        circumradius: f32,
    },
}

/// The shape a projectile kind is drawn as, by its registered name.
///
/// An unregistered or unnamed kind falls back to a plain ball rather than
/// vanishing, so a content addition is visible before it is styled.
fn shot_shape(name: Option<&str>) -> ShotShape {
    match name {
        Some("arrow") => ShotShape::Arrow,
        Some("cannonball") => ShotShape::Ball {
            radius: CELL_PX * 0.16,
        },
        Some("shell") => ShotShape::Shell {
            circumradius: CELL_PX * 0.18,
        },
        _ => ShotShape::Ball {
            radius: CELL_PX * 0.1,
        },
    }
}

/// Draws the rally point of each selected own producer: a line from the
/// building to the target and a circle marking it (run in `Update`).
pub fn draw_rally(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    selection: Res<Selection>,
    registry: Res<ContentRegistry>,
    holders: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &OwnerComponent,
        &RallyPointComponent,
    )>,
    targets: Query<(&EntityInfoComponent, &LocationComponent), Without<HiddenComponent>>,
) {
    const COLOR: Color = Color::srgb(1.0, 0.65, 0.2);

    let local = session.local_player();
    for (info, location, owner, rally) in &holders {
        if owner.player() != local || !selection.get(local).contains(&info.id()) {
            continue;
        }
        let Some(target) = rally.0 else {
            continue;
        };
        let end = match target {
            // The move lands in the cell containing the position, so mark that
            // cell's center rather than the raw sub-cell position.
            RallyTarget::Position(position) => {
                world_center(FixedUVec2::from(CellPos::from(position)), CellSize::ONE)
            }
            RallyTarget::Entity(id) => {
                // A vanished target leaves nothing to point at.
                let Some((target_info, target_location)) = targets
                    .iter()
                    .find(|(target_info, ..)| target_info.id() == id)
                else {
                    continue;
                };
                let target_size = registry.def(target_info.type_id()).location.unwrap().size();
                world_center(target_location.position, target_size)
            }
        };
        let start_size = registry.def(info.type_id()).location.unwrap().size();
        let start = world_center(location.position, start_size);
        gizmos.line_2d(start.truncate(), end.truncate(), COLOR);
        gizmos.circle_2d(end.truncate(), CELL_PX * 0.3, COLOR);
    }
}

/// Draws a short line from each unit's center in its facing direction (Update).
pub fn draw_facing(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    query: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &Transform,
            &Visibility,
        ),
        (With<Directional>, Without<HiddenComponent>),
    >,
) {
    for (info, location, transform, visibility) in &query {
        // Don't trace a unit hidden by fog (interpolate_sprites set its visibility).
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let Some(dir) = facing_dir(location.facing) else {
            continue;
        };
        let size = registry.def(info.type_id()).location.unwrap().size();
        let length = size.width.min(size.height) as f32 * CELL_PX * 0.6;
        let center = transform.translation.truncate();
        gizmos.line_2d(center, center + dir * length, Color::srgb(1.0, 1.0, 0.4));
    }
}

/// The tile color for a terrain, by content name.
fn terrain_color(terrain: &str) -> Color {
    match terrain {
        "grass" => Color::srgb(0.20, 0.34, 0.17),
        "water" => Color::srgb(0.15, 0.35, 0.6),
        _ => Color::srgb(0.35, 0.33, 0.30), // unknown terrain — bare dirt
    }
}

/// Spawns a colored tile per cell from the map's terrain, so the whole
/// playable field is drawn and the void outside it stays the clear color.
/// Terrain is static, so the tiles spawn once on entering the game; they
/// despawn with the in-game overlay on teardown.
pub fn spawn_terrain_tiles(
    mut commands: Commands,
    session: Res<GameSession>,
    scenario: Option<Res<CurrentScenario>>,
) {
    let map = match &scenario {
        Some(scenario) => scenario.0.map.clone(),
        None => match map::by_name(session.map()) {
            Some(map) => map,
            None => return,
        },
    };

    for (i, &terrain) in map.terrain_cells().iter().enumerate() {
        let Some(name) = map.terrains().get(terrain as usize) else {
            continue;
        };
        let (x, y) = (i as u32 % map.width(), i as u32 / map.width());
        let center = Vec3::new((x as f32 + 0.5) * CELL_PX, -(y as f32 + 0.5) * CELL_PX, 0.0);
        commands.spawn((
            Sprite::from_color(terrain_color(name), Vec2::splat(CELL_PX)),
            Transform::from_translation(center),
            InGameUi,
        ));
    }

    // Fog overlay: one darkening tile per cell above the terrain but below
    // entities (z 0.5), updated each frame from the local team's visibility.
    for y in 0..map.height() {
        for x in 0..map.width() {
            let center = Vec3::new((x as f32 + 0.5) * CELL_PX, -(y as f32 + 0.5) * CELL_PX, 0.5);
            commands.spawn((
                FogTile { x, y },
                Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 1.0), Vec2::splat(CELL_PX)),
                Transform::from_translation(center),
                InGameUi,
            ));
        }
    }
}

/// Darkens each fog tile by the local team's knowledge of its cell: black when
/// unexplored, dimmed when explored-but-unseen, clear when visible.
pub fn update_fog_overlay(
    session: Res<GameSession>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    mut tiles: Query<(&FogTile, &mut Sprite)>,
) {
    let local = session.local_player();
    for (tile, mut sprite) in &mut tiles {
        let alpha = if reveal.0 {
            0.0
        } else {
            match fog.visibility_to(&session, local, tile.x, tile.y) {
                CellVisibility::Unexplored => 1.0,
                CellVisibility::Explored => 0.55,
                CellVisibility::Visible => 0.0,
            }
        };
        let color = Color::srgba(0.0, 0.0, 0.0, alpha);
        // Write only on change: an unconditional write re-dirties every
        // fog sprite every frame, and the renderer re-extracts them all.
        if sprite.color != color {
            sprite.color = color;
        }
    }
}

/// Snapshots scouted enemy buildings while visible and draws their last-seen
/// ghost while their cell is remembered but out of sight; purges a ghost once
/// its cell is seen again and the real building is gone.
pub fn draw_ghosts(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    registry: Res<ContentRegistry>,
    fog: Res<VisibilityGrid>,
    reveal: Res<FogReveal>,
    mut ghosts: ResMut<Ghosts>,
    buildings: Query<
        (
            &EntityInfoComponent,
            &LocationComponent,
            &TagsComponent,
            Option<&OwnerComponent>,
        ),
        Without<HiddenComponent>,
    >,
) {
    let local = session.local_player();
    let mut alive = HashSet::new();
    for (info, location, tags, owner) in &buildings {
        let own_team =
            owner.is_some_and(|o| o.player() == local || session.are_allied(local, o.player()));
        if own_team || !tags.contains(tags::BUILDING) {
            continue;
        }
        alive.insert(info.id());
        let (x, y) = (
            location.position.x.to_num::<u32>(),
            location.position.y.to_num::<u32>(),
        );
        if fog.is_visible_to(&session, local, x, y) {
            let size = registry.def(info.type_id()).location.unwrap().size();
            ghosts.0.insert(
                info.id(),
                GhostSprite {
                    origin: (x, y),
                    center: world_center(location.position, size).truncate(),
                    shape: ghost_shape(info.type_name(), size),
                },
            );
        }
    }

    ghosts.0.retain(|id, ghost| {
        match fog.visibility_to(&session, local, ghost.origin.0, ghost.origin.1) {
            // Seen again: keep only if the building is still there (else it was
            // destroyed while we were away, so drop the stale ghost).
            CellVisibility::Visible => alive.contains(id),
            // Remembered but unseen: draw the ghost in its last-known place
            // (unless the whole map is revealed, when the real entities show).
            CellVisibility::Explored => {
                if !reveal.0 {
                    let iso = Isometry2d::from_translation(ghost.center);
                    let color = Color::srgba(0.55, 0.55, 0.62, 0.6);
                    match &ghost.shape {
                        GhostShape::Rect { extent } => gizmos.rect_2d(iso, *extent, color),
                        GhostShape::Polygon {
                            sides,
                            circumradius,
                        } => {
                            gizmos.primitive_2d(
                                &RegularPolygon::new(*circumradius, *sides),
                                iso,
                                color,
                            );
                        }
                    }
                }
                true
            }
            CellVisibility::Unexplored => false,
        }
    });
}

/// The ghost outline for a building of `type_name` occupying `size` cells —
/// matched to what [`attach_sprites`] draws for the live building.
fn ghost_shape(type_name: &str, size: CellSize) -> GhostShape {
    let circumradius = size.width.min(size.height) as f32 * CELL_PX * 0.45;
    match shape_for(type_name) {
        Shape::Hexagon => GhostShape::Polygon {
            sides: 6,
            circumradius,
        },
        Shape::Fortress => GhostShape::Polygon {
            sides: 8,
            circumradius,
        },
        _ => GhostShape::Rect {
            extent: Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85,
        },
    }
}

// Colors of the always-on work visualization, one hue per verb — shared by the
// worker links, the job markers, and the construction progress bar so the same
// work reads the same everywhere.
const BUILD_WORK_COLOR: Color = Color::srgb(0.35, 0.55, 1.0);
const HARVEST_WORK_COLOR: Color = Color::srgb(0.85, 0.7, 0.2);
const REPAIR_WORK_COLOR: Color = Color::srgb(0.9, 0.5, 0.9);
const TRAIN_WORK_COLOR: Color = Color::srgb(0.3, 0.9, 0.9);
// Matches the HUD's research-button teal, so the bar and the button that
// started it read as the same work.
const RESEARCH_WORK_COLOR: Color = Color::srgb(0.35, 0.75, 0.65);
/// The dot color for passengers riding inside a transporter.
const PASSENGER_COLOR: Color = Color::srgb(0.95, 0.85, 0.35);
/// The bar of a running form change.
const MORPH_WORK_COLOR: Color = Color::srgb(0.85, 0.55, 0.25);
/// A repairer that cannot pay for this tick's work.
const STALLED_WORK_COLOR: Color = Color::srgb(1.0, 0.3, 0.25);

/// Draws a line from every visible worker to the job it is actively on — a site
/// being raised, a source being worked, or a patient being mended — so who is
/// doing what reads at a glance without the debug overlay (run in `Update`).
///
/// Walking toward a job draws nothing: the link appears when the work does. A
/// repairer that cannot pay for its work shows its link in the stalled color.
pub fn draw_work_links(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    // Anchored on the interpolated transforms rather than the stepped sim
    // positions, so the lines glide with the sprites they connect.
    workers: Query<
        (
            &Transform,
            &OrderQueueComponent,
            &Visibility,
            Option<&HarvestComponent>,
            Option<&BuildComponent>,
            Option<&RepairComponent>,
        ),
        Without<HiddenComponent>,
    >,
    targets: Query<(&EntityInfoComponent, &Transform), Without<HiddenComponent>>,
) {
    let entity_center = |id: SimulationId| {
        targets
            .iter()
            .find(|(info, _)| info.id() == id)
            .map(|(_, transform)| transform.translation.truncate())
    };

    for (transform, queue, visibility, harvest, build, repair) in &workers {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let Some(entry) = queue.0.front() else {
            continue;
        };
        let (end, color) = match &entry.order {
            Order::Build {
                type_name,
                position,
            } => {
                if build.is_none_or(|build| build.building.is_none()) {
                    continue;
                }
                let size = registry
                    .entity(type_name)
                    .and_then(|def| def.location)
                    .map_or(CellSize::ONE, |location| location.size());
                (
                    Some(world_center(FixedUVec2::from(CellPos::from(*position)), size).truncate()),
                    BUILD_WORK_COLOR,
                )
            }
            Order::Harvest { .. } => {
                let Some(source) = harvest.and_then(|harvest| harvest.harvesting) else {
                    continue;
                };
                (entity_center(source), HARVEST_WORK_COLOR)
            }
            Order::Repair { target } => {
                let Some(repair) = repair else {
                    continue;
                };
                let color = if repair.stalled > 0 {
                    STALLED_WORK_COLOR
                } else {
                    REPAIR_WORK_COLOR
                };
                (entity_center(*target), color)
            }
            Order::Move { .. }
            | Order::Attack { .. }
            | Order::AttackMove { .. }
            | Order::Patrol { .. }
            | Order::Follow { .. }
            | Order::Guard { .. }
            | Order::Train
            | Order::Research { .. }
            | Order::Morph { .. }
            | Order::Board { .. }
            | Order::Load { .. }
            | Order::Unload { .. }
            | Order::Die => continue,
        };
        let Some(end) = end else {
            continue;
        };
        let start = transform.translation.truncate();
        gizmos.line_2d(start, end, color);
        gizmos.circle_2d(end, CELL_PX * 0.18, color);
    }
}

/// Marks the jobs themselves: a ring in the verb's color around a source being
/// worked or an entity being mended, and a dot per crew member above it, so a
/// stacked crew is countable at a glance (run in `Update`). An unfinished site
/// shows its crew dots too; its own state is the translucent shape (see
/// [`tint_under_construction`]) and the progress bar (see [`draw_status_bars`]).
pub fn draw_work_markers(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    cameras: Query<&Transform, With<Camera2d>>,
    // Anchored on the interpolated transforms, so a ring follows a walking
    // patient instead of stepping cell to cell behind it.
    jobs: Query<
        (
            &EntityInfoComponent,
            &Transform,
            &Visibility,
            Option<&UnderHarvestComponent>,
            Option<&UnderRepairComponent>,
            Option<&UnderConstructionComponent>,
        ),
        Without<HiddenComponent>,
    >,
) {
    let camera = cameras.single().ok().cloned().unwrap_or_default();
    for (info, transform, visibility, harvest, repair, construction) in &jobs {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        if harvest.is_none() && repair.is_none() && construction.is_none() {
            continue;
        }
        let size = registry.def(info.type_id()).location.unwrap().size();
        let center = transform.translation.truncate();
        let radius = size.width.max(size.height) as f32 * CELL_PX * 0.55;

        let mut crew = 0;
        if let Some(harvest) = harvest {
            gizmos.circle_2d(center, radius, HARVEST_WORK_COLOR);
            crew += harvest.carriers.len();
        }
        if let Some(repair) = repair {
            gizmos.circle_2d(center, radius + 2.0, REPAIR_WORK_COLOR);
            crew += repair.repairers.len();
        }
        if let Some(construction) = construction {
            crew += construction.builders.len();
        }

        // The crew count reads as HUD: its offsets pass through the
        // camera's orientation and scale, staying screen-aligned whatever
        // the world's look.
        let anchored = |offset: Vec2| {
            center
                + (camera.rotation
                    * Vec3::new(offset.x * camera.scale.x, offset.y * camera.scale.y, 0.0))
                .truncate()
        };
        let top = size.height as f32 * CELL_PX / 2.0 + 13.0;
        let gap = 7.0;
        let left = -(crew.saturating_sub(1) as f32) * gap / 2.0;
        for i in 0..crew {
            gizmos.circle_2d(
                anchored(Vec2::new(left + i as f32 * gap, top)),
                2.5,
                Color::srgb(0.95, 0.95, 0.98),
            );
        }
    }
}

/// Draws slim bars over entities — energy, then health, then construction
/// progress while a site goes up, then training progress with a dot per queued
/// unit, then research progress, then a running form change's progress — for
/// whichever of those the entity has (run in `Update`). Whatever fog or hiding
/// keeps off screen stays bare.
pub fn draw_status_bars(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    cameras: Query<&Transform, With<Camera2d>>,
    // Anchored on the interpolated transforms, so the bars glide with the
    // sprites they sit over.
    query: Query<
        (
            &EntityInfoComponent,
            &Transform,
            &Visibility,
            Option<&HealthComponent>,
            Option<&StatsComponent>,
            Option<&EnergyComponent>,
            Option<&UnderConstructionComponent>,
            Option<&TrainQueueComponent>,
            Option<&TrainComponent>,
            Option<&ResearchComponent>,
            Option<&MorphComponent>,
            Option<&TransporterComponent>,
        ),
        Without<HiddenComponent>,
    >,
) {
    let camera = cameras.single().ok().cloned().unwrap_or_default();
    for (
        info,
        transform,
        visibility,
        health,
        stats,
        energy,
        construction,
        queue,
        train,
        research,
        morph,
        transporter,
    ) in &query
    {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        let center = transform.translation.truncate();
        let half_width = size.width as f32 * CELL_PX * 0.4;
        let mut y = size.height as f32 * CELL_PX / 2.0 + 4.0;

        // Bars read as HUD, not as paint on the ground: their offsets pass
        // through the camera's own orientation and scale, so they stay
        // screen-aligned whatever the world's look.
        let anchored = move |offset: Vec2| {
            center
                + (camera.rotation
                    * Vec3::new(offset.x * camera.scale.x, offset.y * camera.scale.y, 0.0))
                .truncate()
        };
        let bar = |gizmos: &mut Gizmos, fraction: f32, color: Color, y: f32| {
            let left = Vec2::new(-half_width, y);
            let right = Vec2::new(half_width, y);
            let fill = left.lerp(right, fraction.clamp(0.0, 1.0));
            // Two rows of lines stand in for a filled rect (gizmos have no fill).
            for row in [0.0, 1.0] {
                let offset = Vec2::new(0.0, row);
                gizmos.line_2d(
                    anchored(left + offset),
                    anchored(right + offset),
                    Color::srgba(0.0, 0.0, 0.0, 0.7),
                );
                if fraction > 0.0 {
                    gizmos.line_2d(anchored(left + offset), anchored(fill + offset), color);
                }
            }
        };

        if let (Some(energy), Some(stats)) = (energy, stats)
            && let Some(max) = stats.effective(EntityStatId::MAX_ENERGY)
            && max > FixedU64::ZERO
        {
            let fraction = (energy.current().to_num::<f32>() / max.to_num::<f32>()).min(1.0);
            bar(&mut gizmos, fraction, Color::srgb(0.45, 0.5, 1.0), y);
            y += 4.0;
        }

        if let (Some(health), Some(stats)) = (health, stats)
            && let Some(max) = stats.effective(EntityStatId::MAX_HEALTH)
            && max > FixedU64::ZERO
        {
            let fraction = (health.current().to_num::<f32>() / max.to_num::<f32>()).min(1.0);
            let color = Color::srgb(1.0 - fraction * 0.75, 0.15 + fraction * 0.75, 0.2);
            bar(&mut gizmos, fraction, color, y);
            y += 4.0;
        }

        if let Some(construction) = construction {
            let time = def.build_time.unwrap_or(1).max(1);
            let fraction = construction.progress as f32 / time as f32;
            bar(&mut gizmos, fraction, BUILD_WORK_COLOR, y);
            y += 4.0;
        }

        if let Some(research) = research {
            let time = registry
                .research_def(research.research)
                .map_or(1, |def| def.research_time)
                .max(1);
            let fraction = research.progress as f32 / time as f32;
            bar(&mut gizmos, fraction, RESEARCH_WORK_COLOR, y);
            y += 4.0;
        }

        if let Some(morph) = morph {
            // The change's length under the transition's own terms: a constant
            // is what it says, a stat reads the changing entity's effective
            // value — the same reading the simulation ticks against.
            let time = def
                .morphs
                .iter()
                .find(|transition| transition.into_type() == morph.type_name)
                .map(|transition| match transition.time() {
                    MorphTime::Constant(ticks) => ticks,
                    MorphTime::Stat(id) => stats
                        .and_then(|stats| stats.effective(id))
                        .map_or(0, |time| time.to_num::<u32>()),
                })
                .unwrap_or(1)
                .max(1);
            let fraction = morph.progress as f32 / time as f32;
            bar(&mut gizmos, fraction, MORPH_WORK_COLOR, y);
            y += 4.0;
        }

        if let Some(front) = queue.and_then(|queue| queue.0.front()) {
            let time = registry
                .entity(front)
                .and_then(|def| def.train_time)
                .unwrap_or(1)
                .max(1);
            let progress = train.map_or(0, |train| train.progress);
            bar(
                &mut gizmos,
                progress as f32 / time as f32,
                TRAIN_WORK_COLOR,
                y,
            );

            // A dot per queued unit above the bar, so the queue depth reads at
            // the trainer without selecting it.
            let queued = queue.map_or(0, |queue| queue.0.len());
            let gap = 7.0;
            let left = -(queued.saturating_sub(1) as f32) * gap / 2.0;
            for i in 0..queued {
                gizmos.circle_2d(
                    anchored(Vec2::new(left + i as f32 * gap, y + 5.0)),
                    2.5,
                    TRAIN_WORK_COLOR,
                );
            }
            y += 10.0;
        }

        // A dot per passenger, like the trainer's queue dots, so a manned
        // bunker or a loaded shelter reads at a glance.
        let aboard = transporter.map_or(0, |transporter| transporter.passengers.len());
        if aboard > 0 {
            let gap = 7.0;
            let left = -(aboard.saturating_sub(1) as f32) * gap / 2.0;
            for i in 0..aboard {
                gizmos.circle_2d(
                    anchored(Vec2::new(left + i as f32 * gap, y + 5.0)),
                    2.5,
                    PASSENGER_COLOR,
                );
            }
        }
    }
}

/// Fades a site to translucent while it is under construction and back to solid
/// when it finishes, so an unfinished building reads as one (run in `Update`).
pub fn tint_under_construction(
    mut materials: ResMut<Assets<ColorMaterial>>,
    query: Query<
        (
            &MeshMaterial2d<ColorMaterial>,
            Has<UnderConstructionComponent>,
        ),
        With<Renderable>,
    >,
) {
    for (material, under_construction) in &query {
        let alpha = if under_construction { 0.45 } else { 1.0 };
        // Read first, write only on a change: `get_mut` alone would mark every
        // material dirty every frame.
        if materials
            .get(&material.0)
            .is_some_and(|m| m.color.alpha() != alpha)
            && let Some(material) = materials.get_mut(&material.0)
        {
            material.color.set_alpha(alpha);
        }
    }
}
