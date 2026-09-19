//! Renders simulation entities as placeholder colored shapes: triangles for
//! combat units, circles for workers, and squares for buildings and resources.
//!
//! Render components are attached directly to the simulation entities, so they
//! despawn automatically with them. Positions are interpolated between the
//! previous and current tick against the fixed-step overstep, so motion is
//! smooth and stays locked to the simulation cadence (it can never outrun it).
//! Unit shapes rotate to point in their facing direction.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use ferrets_content::{
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    quantity::Quantity,
    registry::ContentRegistry,
    resource::ResourceSourceDef,
    skills::{Casting, SkillCaster},
    tags,
    turret::TurretMount,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{
    FixedU64,
    facing::{self, Facing},
    fixed_uvec2::FixedUVec2,
};
use ferrets_simulation::{
    command::SkillTarget,
    components::{
        annex::{AnnexComponent, Docking},
        attached::AttachedComponent,
        brood::{BredComponent, BroodComponent},
        build::{BuildComponent, SiteWork, UnderConstructionComponent},
        cast::{CastComponent, CastStage},
        dying::{DyingComponent, RemainsComponent},
        energy::EnergyComponent,
        entity_info::EntityInfoComponent,
        entity_stats::StatsComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        lifetime::LifetimeComponent,
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
        turret::TurretsComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{EventRecord, SimulationEvent, SpawnCause},
    fields::FieldGrid,
    impacts::PendingImpacts,
    order::Order,
    selection::Selection,
    session::{GameSession, local_role::LocalRole, player_id::PlayerId},
    simulation_id::SimulationId,
    visibility::{CellVisibility, VisibilityGrid},
    watches::Watches,
};

use crate::{map, scenario::CurrentScenario, states::InGameUi};

/// The one colour every projectile in flight is drawn in, so shape alone tells the
/// kinds apart.
const SHOT_COLOR: Color = Color::srgb(1.0, 0.82, 0.4);

/// The colour a body's own look is traced in.
pub const LOOK_COLOR: Color = Color::srgb(1.0, 1.0, 0.4);

/// The colour a gun's bearing is traced in.
///
/// A different hue from [`LOOK_COLOR`] because the two answer different questions:
/// one is where the body points, the other where the weapon is trained, and on a
/// building only the second ever moves. Warm rather than cool — the keep that
/// carries four of them sits in water, which a cool line would sink into.
pub const BEARING_COLOR: Color = Color::srgb(1.0, 0.45, 0.25);
/// What a body on the ground is drawn in: grey, and fading as it decays.
pub const REMAINS_COLOR: Color = Color::srgba(0.45, 0.44, 0.42, 1.0);

/// Screen pixels per grid cell.
pub const CELL_PX: f32 = 32.0;

/// The interpolated render position from the previous tick.
#[derive(Component)]
pub struct PrevPos(Vec3);

impl PrevPos {
    /// The point interpolation starts from this frame.
    pub fn anchor(&self) -> Vec3 {
        self.0
    }
}

/// The look an entity held at the previous tick, the bearing interpolation
/// starts from this frame.
#[derive(Component)]
pub struct PrevFacing(Facing);

impl PrevFacing {
    /// The bearing interpolation starts from this frame.
    pub fn anchor(&self) -> Facing {
        self.0
    }
}

/// The look an entity is drawn at this frame: part way from the previous tick's
/// bearing to this one's.
///
/// Its own value rather than the sprite's rotation, because the two are not the
/// same question: everything with a facing has a drawn look, while only a
/// [`Directional`] entity turns its body to show it — a building stands square
/// however it is looking.
#[derive(Component)]
pub struct DrawnFacing(Facing);

impl DrawnFacing {
    /// The bearing the sprite is drawn looking along.
    pub fn bearing(&self) -> Facing {
        self.0
    }
}

/// Where each turret was trained at the previous tick, the bearings their own
/// interpolations start from.
#[derive(Component)]
pub struct PrevBearings(Vec<Facing>);

/// Where each turret is drawn trained this frame.
///
/// A second look, beside the body's, rather than a choice between the two: a gun
/// that comes round while its hull drives somewhere else is two directions at
/// once, and one interpolated value could only show one of them — which for a
/// turreted mover would spin the hull with the gun.
///
/// Carried only where there are guns that bear on their own, so its presence is
/// the question already answered, and one entry per gun in mounted order.
#[derive(Component)]
pub struct DrawnBearings(Vec<Facing>);

impl DrawnBearings {
    /// The bearings the guns are drawn trained along, in mounted order.
    pub fn bearings(&self) -> &[Facing] {
        &self.0
    }
}

/// Marks an entity that already has its render components attached.
#[derive(Component)]
pub struct Renderable;

/// Marks a renderable whose body sprite is drawn along its look rather than square
/// to the map — carried by anything that can walk, since a body's look is where it
/// is going. Buildings and resources stay axis-aligned, gun and all.
#[derive(Component)]
pub struct Directional;

/// A per-cell fog overlay sprite, darkened by the local team's visibility.
#[derive(Component)]
pub struct FogTile {
    x: u32,
    y: u32,
}

/// A per-cell field overlay sprite, tinted by what covers its cell: anyone's
/// creep, or the viewed player's own power.
#[derive(Component)]
pub struct FieldTile {
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
    /// A circle inside the footprint — the round buildings.
    Circle { circumradius: f32 },
}

/// The last-seen appearance of a scouted enemy building, kept so it can be drawn
/// as a dimmed ghost while its cell is remembered but out of sight. Render-only
/// and local to this client; persists (stale) even after the real building dies
/// in the fog, until the cell is seen again.
struct GhostSprite {
    origin: (u32, u32),
    center: Vec2,
    /// The footprint remembered under the outline, for consumers that draw the
    /// remembered building as cells rather than as a shape.
    size: CellSize,
    shape: GhostShape,
}

/// Last-seen enemy buildings, keyed by [`SimulationId`] (see [`GhostSprite`]).
#[derive(Resource, Default)]
pub struct Ghosts(HashMap<SimulationId, GhostSprite>);

impl Ghosts {
    /// Where each remembered building stood and how much ground it covered.
    pub(crate) fn remembered(&self) -> impl Iterator<Item = ((u32, u32), CellSize)> {
        self.0.values().map(|ghost| (ghost.origin, ghost.size))
    }
}

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

/// The fog a watching node looks through: everything (the caster's default),
/// or one player's own view — flipping between what each player knows. Read
/// only on a node with no local player; a player's node has exactly one
/// honest perspective, its own.
#[derive(Resource, Default)]
pub struct ObserverPerspective(pub Option<PlayerId>);

/// Cycles the watching perspective on `Tab`: everything → each side's view
/// in seat order → everything. Sides, not players — vision is shared within
/// a team, so every member's view is the same view, and each side appears
/// once, represented by its first-seated member. Deliberately ungated: who
/// actually sees through the choice is enforced where it is read
/// ([`perspective`]) — on a playing node, whose fog is always its own, the
/// keypress writes a choice nothing consults.
pub fn perspective_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    mut watch: ResMut<ObserverPerspective>,
) {
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }
    let sides = side_representatives(&session);
    watch.0 = match watch.0 {
        None => sides.first().copied(),
        Some(current) => sides
            .iter()
            .copied()
            .skip_while(|&side| side != current)
            .nth(1),
    };
}

/// One player per side, in seat order: the first-seated member of each team,
/// and every teamless player as its own side — the sides the watching
/// perspective cycles through.
fn side_representatives(session: &GameSession) -> Vec<PlayerId> {
    let mut sides: Vec<PlayerId> = Vec::new();
    for slot in session.player_slots() {
        let seen = sides.iter().any(|&member| {
            session
                .slot(member)
                .is_some_and(|held| held.team().is_some() && held.team() == slot.team())
        });
        if !seen {
            sides.push(slot.id());
        }
    }
    sides
}

/// Drops everything the view kept for the game just left, on entering a new
/// one. All of it keys off a tick or a simulation id, and both restart with a
/// new game.
pub fn reset_per_game(
    mut watch: ResMut<ObserverPerspective>,
    mut inspected: ResMut<crate::input::Inspected>,
    mut ghosts: ResMut<Ghosts>,
    mut pulses: ResMut<SkillPulses>,
    mut puffs: ResMut<Puffs>,
) {
    watch.0 = None;
    inspected.0.clear();
    ghosts.0.clear();
    pulses.active.clear();
    puffs.active.clear();
}

/// The knowledge this node watches the map through: the local player's own
/// team vision — or, on a node with no local player, whatever
/// [`ObserverPerspective`] holds: everything, or one chosen player's view.
/// A player, eliminated or not, only ever sees what its side sees; the
/// map-wide view and the flipping belong to the observer alone.
pub fn perspective(
    session: &GameSession,
    watch: &ObserverPerspective,
    fog: &VisibilityGrid,
    x: u32,
    y: u32,
) -> CellVisibility {
    match (session.local_player(), watch.0) {
        (Some(local), _) => fog.visibility_to(session, local, x, y),
        (None, Some(player)) => fog.visibility_to(session, player, x, y),
        (None, None) => CellVisibility::Visible,
    }
}

/// Whether this node currently sees the cell (see [`perspective`]).
pub fn sees(
    session: &GameSession,
    watch: &ObserverPerspective,
    fog: &VisibilityGrid,
    x: u32,
    y: u32,
) -> bool {
    perspective(session, watch, fog, x, y) == CellVisibility::Visible
}

/// Whether `player` is the local player or one of its allies — `false` on a
/// node with no local player, which is on nobody's side.
pub fn allied_with_local(session: &GameSession, player: PlayerId) -> bool {
    session
        .local_player()
        .is_some_and(|local| session.are_allied(local, player))
}

/// Whether the drawing smooths what the simulation hands it — the walk between
/// two ticks' positions, and the turn between their two looks.
///
/// Cleared, every sprite sits exactly where its tick put it and looks exactly
/// where the tick says: twenty jumps a second, which is no way to play but the
/// only way to see what the simulation actually did. Presentation only, like
/// [`FogReveal`] — the sprites move differently, nothing else does.
#[derive(Resource)]
pub struct Smoothing(pub bool);

impl Default for Smoothing {
    fn default() -> Self {
        Self(true)
    }
}

/// Toggles the smoothing (see [`Smoothing`]) on the `I` key.
pub fn toggle_smoothing(keys: Res<ButtonInput<KeyCode>>, mut smoothing: ResMut<Smoothing>) {
    if keys.just_pressed(KeyCode::KeyI) {
        smoothing.0 = !smoothing.0;
    }
}

/// World-space center of a footprint, in pixels (Bevy y points up, sim y down).
pub(crate) fn world_center(position: FixedUVec2, size: CellSize) -> Vec3 {
    world_point(FixedUVec2::new(
        position.x + FixedU64::from_num(size.width) / 2,
        position.y + FixedU64::from_num(size.height) / 2,
    ))
}

/// Where a simulation point is drawn.
pub(crate) fn world_point(position: FixedUVec2) -> Vec3 {
    Vec3::new(
        position.x.to_num::<f32>() * CELL_PX,
        -position.y.to_num::<f32>() * CELL_PX,
        1.0,
    )
}

/// How far up the screen an airborne sprite is drawn from the cell it is over,
/// and how far its shadow sits below it.
const AIR_LIFT_PX: f32 = CELL_PX * 0.9;

/// Whether a type flies: it occupies the air *alone*. Something that holds
/// the air on top of a surface — a fortress tall enough to wall the sky —
/// stands on that surface and casts no flight shadow.
fn airborne(registry: &ContentRegistry, def: &EntityTypeDef) -> bool {
    match (registry.layer(map::AIR), def.location) {
        (Some(air), Some(location)) => location.occupation() == *air,
        _ => false,
    }
}

/// Where in the stack a type is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawLayer {
    /// Lifted up the screen and drawn over everything on the ground.
    Air,
    /// Drawn over what stands on the ground — a worker sitting in a tree shows
    /// on the tree, not behind it.
    Walking,
    /// Drawn where it stands, under everything that moves.
    Standing,
}

impl DrawLayer {
    /// The layer a type of `def` is drawn in.
    fn of(registry: &ContentRegistry, def: &EntityTypeDef) -> Self {
        match (airborne(registry, def), def.can_move()) {
            (true, _) => DrawLayer::Air,
            (false, true) => DrawLayer::Walking,
            (false, false) => DrawLayer::Standing,
        }
    }

    /// The draw offset of the layer.
    fn offset(self) -> Vec3 {
        match self {
            DrawLayer::Air => Vec3::new(0.0, AIR_LIFT_PX, 1.0),
            DrawLayer::Walking => Vec3::new(0.0, 0.0, 0.5),
            DrawLayer::Standing => Vec3::ZERO,
        }
    }
}

/// The draw offset for a type, by the layer it is drawn in.
///
/// Altitude is presentation only — the simulation knows nothing about it, and a
/// flier's position is the cell it is over. The lift is what makes an air unit
/// crossing a lake or a keep read as passing above it rather than through it.
pub(crate) fn lift(registry: &ContentRegistry, def: &EntityTypeDef) -> Vec3 {
    DrawLayer::of(registry, def).offset()
}

pub(crate) fn color_for(
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
        Some(player) => player_color(player),
        None => Color::srgb(0.75, 0.7, 0.4), // neutral — tan
    }
}

/// The color a player's presence is drawn in, wherever it appears — blips,
/// selection rings, vision overlays.
pub(crate) fn player_color(player: PlayerId) -> Color {
    match player {
        0 => Color::srgb(0.35, 0.55, 1.0), // blue
        1 => Color::srgb(1.0, 0.35, 0.35), // red
        2 => Color::srgb(0.4, 0.8, 0.4),   // green
        3 => Color::srgb(0.7, 0.4, 0.9),   // purple
        _ => Color::srgb(0.75, 0.7, 0.4),  // beyond the demo's seats — tan
    }
}

/// The placeholder shape used for an entity type.
enum Shape {
    /// The archer — a triangle that points where it faces.
    Triangle,
    /// Melee fighters — a diamond.
    Diamond,
    /// The raised dead — a darkened diamond wearing pale ribs across it.
    Skeleton,
    /// The swarm's spawn — a small wedge with a pale bud on its nose.
    Spawn,
    /// Siege units (those whose hits burst) — a pentagon.
    Pentagon,
    /// Workers, and the big rock — a circle.
    Circle,
    /// Larvae — a small disc with a darker core.
    Grub,
    /// Ships — a hull with a triangular bow pointing where it faces.
    Ship,
    /// Unit production — a hexagon, distinct from the main hall.
    Hexagon,
    /// The sea fortress — an octagonal wall. What sits on it is drawn from the
    /// guns it mounts, not from the shape.
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
    /// The overlord — a bloated sac wider than it is long, a lighter crown on
    /// its back and tendrils trailing beneath.
    Overlord,
    /// The hive — a dome with a lighter cap and four dark spires over its
    /// front and flanks.
    Hive,
    /// The war wagon — a hull between two wheel bars. The wheels are what make
    /// its heading readable on a body whose gun does not share it; the gun itself
    /// is drawn from its mount, like every other.
    WarWagon,
    /// The siege works — an octagon, distinct from the camp's hexagon.
    Octagon,
    /// The watch tower — a square base bearing a lighter lookout platform.
    WatchTower,
    /// Armed towers — the watch tower's silhouette with a darker four-point
    /// turret on the lookout.
    GuardTower,
    /// The spirit tower — a base bearing a pale orb in a halo, with nothing
    /// solid on the lookout at all.
    SpiritTower,
    /// The nerubian tower — a base bearing a dark body crouched on four legs.
    NerubianTower,
    /// Growths — the creep tumor and the cocoon — a disc bearing a darker core.
    Pod,
    /// The pylon — a square base bearing a lighter crystal, a diamond standing
    /// on its point.
    Pylon,
    /// The entangled mine — a mine's square bound by a ring of the owner's
    /// roots, with a darker shaft at its heart.
    EntangledMine,
    /// The comsat station — a base bearing a lighter dish.
    Dish,
    /// The tech lab — a base bearing a darker vent bar across it.
    Lab,
    /// The refinery — a base bearing a holding tank, with the shaft head at
    /// its heart and a pipe over one corner.
    Refinery,
    /// The tank — a hull with a barrel down its nose, which is what makes a
    /// gun that only fires ahead readable; planted, it wears a ring and a
    /// brace on each flank, and no barrel at all.
    Tank {
        /// Whether this is the planted form.
        sieged: bool,
    },
    /// The undead hall grown once — a keep between two dark spires under a
    /// pale lintel.
    Halls,
    /// The undead hall grown twice — a keep with dark towers at its corners
    /// and a pale keep standing at its heart.
    Citadel,
    /// Main buildings and resource sources — a square.
    Square,
}

/// Picks a shape from the entity type name. Add new types here.
fn shape_for(type_name: &str) -> Shape {
    match type_name {
        "peasant" | "peon" | "drone" | "probe" | "wisp" | "scv" | "acolyte" => Shape::Circle,
        "grunt" | "swarmling" | "ravager" | "zealot" | "huntress" | "ghoul" => Shape::Diamond,
        "skeleton" => Shape::Skeleton,
        "hatchling" => Shape::Spawn,
        "archer" | "marine" => Shape::Triangle,
        "mortar" => Shape::Pentagon,
        "medic" | "shaman" | "necromancer" => Shape::Cross,
        "ship" => Shape::Ship,
        "training_camp"
        | "war_camp"
        | "spawning_pit"
        | "gateway"
        | "barracks"
        | "barracks_aloft"
        | "crypt"
        | "temple_of_the_damned" => Shape::Hexagon,
        "sea_fortress" => Shape::Fortress,
        "gryphon" => Shape::Gryphon { aloft: false },
        "gryphon_aloft" => Shape::Gryphon { aloft: true },
        "zeppelin" => Shape::Zeppelin,
        "overlord" => Shape::Overlord,
        "hive" => Shape::Hive,
        "war_wagon" => Shape::WarWagon,
        "siege_works" | "factory" | "factory_aloft" => Shape::Octagon,
        "watch_tower" => Shape::WatchTower,
        "guard_tower" | "photon_cannon" => Shape::GuardTower,
        "spirit_tower" => Shape::SpiritTower,
        "nerubian_tower" => Shape::NerubianTower,
        "tumor" | "cocoon" | "hive_cocoon" | "egg" => Shape::Pod,
        "larva" => Shape::Grub,
        "pylon" => Shape::Pylon,
        // The elves' ancients wear the shapes of the buildings they stand in
        // for, rooted or walking; the roots an uprooted one carries are added
        // apart from the shape (see `uprooted`).
        "ancient_of_war" | "ancient_of_war_uprooted" => Shape::Hexagon,
        "ancient_protector" | "ancient_protector_uprooted" => Shape::GuardTower,
        "entangled_mine" | "haunted_mine" => Shape::EntangledMine,
        // The terran annexes and the tank. A lifted-off structure keeps its
        // grounded shape: the lift and its shadow are what say it is flying.
        "comsat_station" => Shape::Dish,
        "tech_lab" => Shape::Lab,
        "refinery" => Shape::Refinery,
        "tank" => Shape::Tank { sieged: false },
        "siege_tank" => Shape::Tank { sieged: true },
        "big_rock" => Shape::Circle,
        // The undead hall wears its tier: the necropolis is a square like any
        // first hall, and each form it grows into has one of its own — worn
        // from the moment it starts rising, so a hall never reads as a form it
        // has left behind.
        "halls_of_the_dead" | "halls_of_the_dead_rising" => Shape::Halls,
        "black_citadel" | "black_citadel_rising" => Shape::Citadel,
        _ => Shape::Square,
    }
}

/// Whether the type is one of the elves' ancients on the move: it keeps its
/// building's shape and carries its roots along, four darker squares at the
/// corners.
fn uprooted(type_name: &str) -> bool {
    matches!(
        type_name,
        "tree_of_life_uprooted" | "ancient_of_war_uprooted" | "ancient_protector_uprooted"
    )
}

/// Attaches a placeholder shape to any simulation entity that lacks one.
///
/// The shape comes from [`shape_for`]. Anything that can walk also gets a
/// [`Directional`] marker, so its body is drawn along its look, and anything with a
/// gun of its own gets that gun's own drawn bearing beside it.
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
            Option<&TurretsComponent>,
            Option<&RemainsComponent>,
        ),
        Without<Renderable>,
    >,
) {
    for (entity, info, location, owner, turret, corpse) in &query {
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        let center = world_center(location.position, size) + lift(&registry, def);
        // Remains wear whoever fell there: the fallen type's silhouette, greyed
        // and drawn small, so a field of bodies reads as bodies rather than as
        // a crowd of whatever they were.
        let (shape, color, scale): (Shape, Color, f32) = match corpse {
            Some(corpse) => (
                shape_for(registry.def(corpse.of).name.as_str()),
                REMAINS_COLOR,
                0.6,
            ),
            None => (
                shape_for(info.type_name()),
                color_for(owner, def.resource_source.as_ref(), &session),
                1.0,
            ),
        };
        let radius = size.width.min(size.height) as f32 * CELL_PX * 0.45 * scale;

        let mut entity = commands.entity(entity);
        match shape {
            Shape::Triangle => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 3))),
                    MeshMaterial2d(materials.add(color)),
                ));
            }
            Shape::Diamond => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 4))),
                    MeshMaterial2d(materials.add(color)),
                ));
            }
            Shape::Pentagon => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 5))),
                    MeshMaterial2d(materials.add(color)),
                ));
            }
            Shape::Circle => {
                entity.insert((
                    Mesh2d(meshes.add(Circle::new(radius))),
                    MeshMaterial2d(materials.add(color)),
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
                // An octagonal wall. What sits on it is drawn from its mounts.
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 8))),
                    MeshMaterial2d(materials.add(color)),
                ));
            }
            Shape::Cross => {
                // A disc with a lighter cross laid over it, so a medic reads as
                // support at a glance rather than as another combat silhouette.
                let arm = Vec2::new(radius * 1.1, radius * 0.36);
                let mark = color.lighter(0.35);
                entity.insert((
                    Mesh2d(meshes.add(Circle::new(radius))),
                    MeshMaterial2d(materials.add(color)),
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
            Shape::Overlord => {
                // The sac leads with its broad side; the crown sits high on
                // the back and three tendrils hang off the rear.
                let tendril = Vec2::new(radius * 0.12, radius * 0.7);
                entity.insert((
                    Mesh2d(meshes.add(Ellipse::new(radius * 0.95, radius * 0.7))),
                    MeshMaterial2d(materials.add(color)),
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Ellipse::new(radius * 0.45, radius * 0.3))),
                        MeshMaterial2d(materials.add(color.lighter(0.25))),
                        Transform::from_translation(Vec3::new(0.0, radius * 0.15, 0.1)),
                    ));
                    for lane in [-0.5, 0.0, 0.5] {
                        parent.spawn((
                            Sprite::from_color(color.darker(0.2), tendril),
                            Transform::from_translation(Vec3::new(
                                radius * lane,
                                -radius * 0.85,
                                -0.1,
                            )),
                        ));
                    }
                });
            }
            Shape::Hive => {
                // The dome fills the footprint, the cap sits high on it and
                // the spires lean out over the front and the flanks.
                let spire = Vec2::new(radius * 0.16, radius * 0.5);
                entity.insert((
                    Mesh2d(meshes.add(Ellipse::new(radius * 0.95, radius * 0.8))),
                    MeshMaterial2d(materials.add(color)),
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Ellipse::new(radius * 0.5, radius * 0.35))),
                        MeshMaterial2d(materials.add(color.lighter(0.2))),
                        Transform::from_translation(Vec3::new(0.0, radius * 0.15, 0.1)),
                    ));
                    for (lane, lift) in [(-0.6, 0.25), (-0.22, 0.55), (0.22, 0.55), (0.6, 0.25)] {
                        parent.spawn((
                            Sprite::from_color(color.darker(0.3), spire),
                            Transform::from_translation(Vec3::new(
                                radius * lane,
                                radius * lift,
                                0.2,
                            )),
                        ));
                    }
                });
            }
            Shape::WarWagon => {
                // A hull longer than it is wide, with wheels down each flank —
                // what makes the heading readable on a body whose gun does not
                // share it. The gun itself is drawn from its mount.
                let hull = Vec2::new(radius * 1.05, radius * 1.45);
                let wheel = Vec2::new(radius * 0.26, radius * 1.55);
                entity.insert(Sprite::from_color(color, hull));
                entity.with_children(|parent| {
                    for flank in [-1.0, 1.0] {
                        parent.spawn((
                            Sprite::from_color(color.darker(0.3), wheel),
                            Transform::from_translation(Vec3::new(flank * radius * 0.62, 0.0, 0.1)),
                        ));
                    }
                });
            }
            Shape::Octagon => {
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 8))),
                    MeshMaterial2d(materials.add(color)),
                ));
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
            Shape::Spawn => {
                // A wedge two thirds of a swarmling, tipped with a pale bud:
                // plainly less than the thing it was too young to become.
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius * 0.7, 3))),
                    MeshMaterial2d(materials.add(color)),
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.18))),
                        MeshMaterial2d(materials.add(color.lighter(0.4))),
                        Transform::from_translation(Vec3::new(0.0, radius * 0.3, 0.1)),
                    ));
                });
            }
            Shape::Halls => {
                // A keep between two spires with a pale lintel across their
                // heads: the hall that has grown once.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                let spire = Vec2::new(radius * 0.22, radius * 1.1);
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    for side in [-0.62, 0.62] {
                        parent.spawn((
                            Sprite::from_color(color.darker(0.3), spire),
                            Transform::from_translation(Vec3::new(radius * side, 0.0, 0.1)),
                        ));
                    }
                    parent.spawn((
                        Sprite::from_color(
                            color.lighter(0.3),
                            Vec2::new(radius * 1.55, radius * 0.2),
                        ),
                        Transform::from_translation(Vec3::new(0.0, radius * 0.5, 0.2)),
                    ));
                });
            }
            Shape::Citadel => {
                // Towers at the corners about a keep standing on its point:
                // the hall grown all the way, and the one that shoots.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                let tower = Vec2::splat(radius * 0.38);
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    for corner in [(-0.62, -0.62), (-0.62, 0.62), (0.62, -0.62), (0.62, 0.62)] {
                        parent.spawn((
                            Sprite::from_color(color.darker(0.35), tower),
                            Transform::from_translation(Vec3::new(
                                radius * corner.0,
                                radius * corner.1,
                                0.1,
                            )),
                        ));
                    }
                    parent.spawn((
                        Mesh2d(meshes.add(RegularPolygon::new(radius * 0.5, 4))),
                        MeshMaterial2d(materials.add(color.lighter(0.3))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.2)),
                    ));
                });
            }
            Shape::Skeleton => {
                // The diamond every melee fighter wears, darkened and ribbed:
                // what walked back up off the field reads as bone at a glance.
                let bone = color.lighter(0.45);
                entity.insert((
                    Mesh2d(meshes.add(RegularPolygon::new(radius, 4))),
                    MeshMaterial2d(materials.add(color.darker(0.2))),
                ));
                entity.with_children(|parent| {
                    for (lane, span) in [(0.32, 0.5), (0.0, 0.85), (-0.32, 0.5)] {
                        parent.spawn((
                            Sprite::from_color(bone, Vec2::new(radius * span, radius * 0.14)),
                            Transform::from_translation(Vec3::new(0.0, radius * lane, 0.1)),
                        ));
                    }
                });
            }
            Shape::SpiritTower => {
                // The tower's base under an orb in a halo: what shoots is a
                // spirit, so nothing sits on the lookout.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.62))),
                        MeshMaterial2d(materials.add(color.lighter(0.2))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.32))),
                        MeshMaterial2d(materials.add(color.lighter(0.55))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.2)),
                    ));
                });
            }
            Shape::NerubianTower => {
                // The base under a dark body on four legs: the heavier gun,
                // and flat — nothing of it points up.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                let leg = Vec2::new(radius * 0.16, radius * 1.35);
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    for quarter in 0..4 {
                        let angle = std::f32::consts::FRAC_PI_4
                            + quarter as f32 * std::f32::consts::FRAC_PI_2;
                        parent.spawn((
                            Sprite::from_color(color.darker(0.25), leg),
                            Transform::from_translation(Vec3::new(0.0, 0.0, 0.1))
                                .with_rotation(Quat::from_rotation_z(angle)),
                        ));
                    }
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.42))),
                        MeshMaterial2d(materials.add(color.darker(0.4))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.2)),
                    ));
                });
            }
            Shape::Grub => {
                // A small disc with a darker core: a crawler at a building's
                // foot, half the size of a worker.
                entity.insert((
                    Mesh2d(meshes.add(Circle::new(radius * 0.5))),
                    MeshMaterial2d(materials.add(color)),
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.2))),
                        MeshMaterial2d(materials.add(color.darker(0.15))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::Pod => {
                // A disc with a darker core: a growth on the ground rather than
                // a structure standing on it.
                entity.insert((
                    Mesh2d(meshes.add(Circle::new(radius))),
                    MeshMaterial2d(materials.add(color)),
                ));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.45))),
                        MeshMaterial2d(materials.add(color.darker(0.15))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::Pylon => {
                // A square base with a lighter crystal standing on its point,
                // taller than it is wide.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Rhombus::new(radius * 0.7, radius * 1.4))),
                        MeshMaterial2d(materials.add(color.lighter(0.25))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::EntangledMine => {
                // A mine, in a mine's colour, bound by a ring of the owner's
                // roots round its middle — nothing like the corner roots a
                // walking ancient carries — with a darker shaft at its heart.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                let grip = owner.map_or(color.darker(0.2), |owner| player_color(owner.player()));
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Annulus::new(radius * 0.62, radius * 0.8))),
                        MeshMaterial2d(materials.add(grip)),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.3))),
                        MeshMaterial2d(materials.add(color.darker(0.3))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::Dish => {
                // A base with a lighter dish on it, and the mast fixed at
                // its top: the station has no speed, so it is drawn square to
                // the map and nothing about it may assert a heading.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.5))),
                        MeshMaterial2d(materials.add(color.lighter(0.3))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                    parent.spawn((
                        Sprite::from_color(color.darker(0.25), Vec2::new(radius * 0.16, radius)),
                        Transform::from_translation(Vec3::new(0.0, radius * 0.45, 0.2)),
                    ));
                });
            }
            Shape::Refinery => {
                // A holding tank on the base, with the shaft head at its
                // heart in the same darker ring an entangled mine wears: the
                // seam is still down there, and the gold comes up through
                // this.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                let pipe = owner.map_or(color.darker(0.25), |owner| player_color(owner.player()));
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Mesh2d(meshes.add(Circle::new(radius * 0.62))),
                        MeshMaterial2d(materials.add(color.lighter(0.2))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                    parent.spawn((
                        Mesh2d(meshes.add(Annulus::new(radius * 0.26, radius * 0.38))),
                        MeshMaterial2d(materials.add(color.darker(0.35))),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.2)),
                    ));
                    // The pipe the load leaves by, over one corner of the
                    // base, in the colour of whoever draws through it: every
                    // other part is the seam's own gold, which would leave a
                    // rival's refinery reading as unclaimed ground.
                    parent.spawn((
                        Sprite::from_color(pipe, Vec2::new(radius * 0.9, radius * 0.18)),
                        Transform::from_translation(Vec3::new(radius * 0.45, -radius * 0.6, 0.2)),
                    ));
                });
            }
            Shape::Lab => {
                // A base with a darker vent bar across it.
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
                entity.with_children(|parent| {
                    parent.spawn((
                        Sprite::from_color(
                            color.darker(0.3),
                            Vec2::new(radius * 1.1, radius * 0.3),
                        ),
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    ));
                });
            }
            Shape::Tank { sieged } => {
                // Rolling, the hull is square to its heading and the barrel
                // runs out of its nose: where the barrel points is where the
                // gun can fire.
                //
                // Planted, it draws no barrel. A form with no speed is drawn
                // square to the map whatever its look, so a barrel would
                // assert a heading the sieged gun does not have — it traverses
                // and answers whatever comes into reach from any side. What
                // says "planted" instead is the brace on each flank and the
                // ring its gun turns in.
                let hull = Vec2::new(radius * 1.1, radius * 1.2);
                entity.insert(Sprite::from_color(color, hull));
                entity.with_children(|parent| {
                    if sieged {
                        parent.spawn((
                            Mesh2d(meshes.add(Annulus::new(radius * 0.34, radius * 0.46))),
                            MeshMaterial2d(materials.add(color.lighter(0.2))),
                            Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                        ));
                        for flank in [-1.0, 1.0] {
                            parent.spawn((
                                Sprite::from_color(
                                    color.darker(0.3),
                                    Vec2::new(radius * 0.3, radius * 0.9),
                                ),
                                Transform::from_translation(Vec3::new(
                                    flank * radius * 0.7,
                                    0.0,
                                    0.1,
                                )),
                            ));
                        }
                    } else {
                        let barrel = Vec2::new(radius * 0.22, radius);
                        parent.spawn((
                            Sprite::from_color(color.lighter(0.2), barrel),
                            Transform::from_translation(Vec3::new(0.0, barrel.y * 0.55, 0.1)),
                        ));
                    }
                });
            }
            Shape::Square => {
                let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
                entity.insert(Sprite::from_color(color, px));
            }
        }
        // An ancient on the move carries its roots: four darker squares at the
        // corners of its footprint, which is what tells a walker from a stand
        // that wears the same shape.
        if uprooted(info.type_name()) {
            let px = Vec2::new(size.width as f32, size.height as f32) * CELL_PX * 0.85;
            let root = Vec2::splat(px.x * 0.22);
            entity.with_children(|parent| {
                for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    parent.spawn((
                        Sprite::from_color(color.darker(0.15), root),
                        Transform::from_translation(Vec3::new(
                            dx * px.x * 0.45,
                            dy * px.y * 0.45,
                            0.1,
                        )),
                    ));
                }
            });
        }
        // Every gun the body carries, drawn where it is mounted: a disc, lighter
        // than the body, round because a shape with a front of its own would be
        // turned by the hull it rides on rather than by where it is trained. Where
        // it points is the bearing line (see `draw_facing`).
        for mount in &def.turrets {
            let gun = meshes.add(Circle::new(
                mount.size().width.min(mount.size().height) as f32 * CELL_PX * 0.3,
            ));
            let gun_color = materials.add(color.lighter(0.25));
            let at = mount_offset(def, mount);
            entity.with_children(|parent| {
                parent.spawn((
                    Mesh2d(gun),
                    MeshMaterial2d(gun_color),
                    Transform::from_translation(at.extend(0.2)),
                ));
            });
        }

        // A body that can walk is drawn facing where it walks; one that cannot is
        // drawn square to the map, however its look moves — a tower aiming turns
        // its facing without turning its walls.
        //
        // Drawn already facing its look, or a spawn — and a form that swaps its
        // silhouette — would turn from wherever the identity rotation points.
        let mut transform = Transform::from_translation(center);
        if def.can_move() {
            transform.rotation = facing_rotation(location.facing);
            entity.insert(Directional);
        }
        entity.insert((
            transform,
            PrevPos(center),
            PrevFacing(location.facing),
            DrawnFacing(location.facing),
            Renderable,
        ));
        // A gun that bears on its own is drawn along its own look, from the
        // bearing it is mounted at.
        if let Some(TurretsComponent(turrets)) = turret {
            let bearings: Vec<Facing> = turrets.iter().map(|turret| turret.bearing).collect();
            entity.insert((PrevBearings(bearings.clone()), DrawnBearings(bearings)));
        }
    }
}

/// Strips the render components of any entity whose type was rewritten or whose
/// owner changed, so [`attach_sprites`] rebuilds it — a gryphon that lands must
/// stop wearing its flight silhouette, and a seized annex must stop wearing the
/// colour of the player who lost it.
///
/// `Changed` also fires the tick a component is added, but a freshly spawned
/// entity is not yet [`Renderable`], so only real rewrites pass the filter.
pub fn refresh_changed_sprites(
    mut commands: Commands,
    query: Query<
        Entity,
        (
            Or<(Changed<EntityInfoComponent>, Changed<OwnerComponent>)>,
            With<Renderable>,
        ),
    >,
) {
    for entity in &query {
        commands
            .entity(entity)
            .remove::<(
                Renderable,
                Directional,
                PrevPos,
                PrevFacing,
                DrawnFacing,
                PrevBearings,
                DrawnBearings,
                Mesh2d,
                MeshMaterial2d<ColorMaterial>,
                Sprite,
            )>()
            .despawn_related::<Children>();
    }
}

/// A bearing as a Z rotation for a shape that points `+Y` at rest.
///
/// A bearing runs clockwise from north and a Bevy rotation runs anticlockwise, so
/// one is the other negated.
pub fn facing_rotation(facing: Facing) -> Quat {
    Quat::from_rotation_z(-radians(facing))
}

/// Which way a shape drawn at `facing` points — the direction its facing line
/// traces.
pub fn facing_line(facing: Facing) -> Vec2 {
    let bearing = radians(facing);
    Vec2::new(bearing.sin(), bearing.cos())
}

/// A bearing in radians clockwise from north.
fn radians(facing: Facing) -> f32 {
    facing.to_bits() as f32 / facing::PER_TURN as f32 * std::f32::consts::TAU
}

/// The look drawn `alpha` of the way from `prev` to `curr`, the short way round.
///
/// The simulation's own two bearings and nothing else: a look that changes by no
/// more than the body's turn rate each tick is already smooth, so the drawing has
/// only to fill in between the ticks — the same job it does for position.
fn between(prev: Facing, curr: Facing, alpha: f32) -> Facing {
    let step = (prev.difference(curr) as f32 * alpha) as i32;
    Facing::from_bits(prev.to_bits().wrapping_add(step as i16 as u16))
}

/// Snapshots each sprite's current sim position as the interpolation start, run
/// before the simulation advances (`FixedPreUpdate`).
pub fn record_prev(
    registry: Res<ContentRegistry>,
    mut query: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &mut PrevPos,
        &mut PrevFacing,
        Option<(&TurretsComponent, &mut PrevBearings)>,
    )>,
) {
    for (info, location, mut prev, mut looked, turrets) in &mut query {
        prev.0 = draw_anchor(&registry, registry.def(info.type_id()), location.position);
        looked.0 = location.facing;
        if let Some((TurretsComponent(turrets), mut previous)) = turrets {
            previous.0 = turrets.iter().map(|turret| turret.bearing).collect();
        }
    }
}

/// Where a drawn entity's sprite sits for a position: its footprint's centre,
/// lifted if it flies. Only entities the sprite attachment accepted carry a
/// [`PrevPos`], and it takes none without a footprint.
fn draw_anchor(registry: &ContentRegistry, def: &EntityTypeDef, position: FixedUVec2) -> Vec3 {
    world_center(position, def.location.unwrap().size()) + lift(registry, def)
}

/// Snaps a set-down entity's drawing — where it interpolates from, and the
/// look it is drawn at — to where it was set down: one reappearing from off
/// the map, one stepping onto its job, and one stepping back off it.
///
/// Each is a discontinuity, not motion: the entity is set down at a cell
/// chosen for it, facing whatever it faces now, and easing into either slides
/// or swings it into place instead. A distance rule cannot tell the two apart
/// — stepping out of a mine puts a worker beside the cell it went in by, well
/// under any threshold a real step has to clear — so the set-down itself is
/// the signal.
pub fn snap_revealed(
    registry: Res<ContentRegistry>,
    mut revealed: RemovedComponents<HiddenComponent>,
    mut detached: RemovedComponents<AttachedComponent>,
    attached: Query<Entity, Added<AttachedComponent>>,
    mut query: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &mut PrevPos,
        &mut PrevFacing,
        &mut DrawnFacing,
        &mut Transform,
        Option<&Directional>,
        Option<(&TurretsComponent, &mut PrevBearings, &mut DrawnBearings)>,
    )>,
) {
    let set_down: Vec<Entity> = revealed
        .read()
        .chain(detached.read())
        .chain(attached.iter())
        .collect();
    for entity in set_down {
        let Ok((
            info,
            location,
            mut prev,
            mut looked,
            mut drawn,
            mut transform,
            directional,
            turret,
        )) = query.get_mut(entity)
        else {
            continue;
        };
        prev.0 = draw_anchor(&registry, registry.def(info.type_id()), location.position);
        looked.0 = location.facing;
        drawn.0 = looked.0;
        if directional.is_some() {
            transform.rotation = facing_rotation(drawn.0);
        }
        // A gun is set down trained where it was set down too.
        if let Some((TurretsComponent(turrets), mut previous, mut shown)) = turret {
            previous.0 = turrets.iter().map(|turret| turret.bearing).collect();
            shown.0 = previous.0.clone();
        }
    }
}

/// Interpolates each sprite between its previous and current sim position by the
/// fixed-step overstep, turns it toward its look at [`TURN_RATE`], and hides
/// off-map entities (run in `Update`).
///
/// Both the walk and the turn are what [`Smoothing`] switches off, leaving each
/// sprite on the tick's own position and look.
pub fn interpolate_sprites(
    fixed: Res<Time<Fixed>>,
    smoothing: Res<Smoothing>,
    session: Res<GameSession>,
    registry: Res<ContentRegistry>,
    fog: Res<VisibilityGrid>,
    watch: Res<ObserverPerspective>,
    reveal: Res<FogReveal>,
    mut query: Query<(
        &EntityInfoComponent,
        &LocationComponent,
        &PrevPos,
        &PrevFacing,
        &mut DrawnFacing,
        &mut Transform,
        &mut Visibility,
        Option<&HiddenComponent>,
        Option<&OwnerComponent>,
        Option<&Directional>,
        Option<(&TurretsComponent, &PrevBearings, &mut DrawnBearings)>,
    )>,
) {
    // Unsmoothed, the sprite is drawn on the tick's own position rather than
    // part way from the last one — which is what `alpha` at rest means.
    let alpha = if smoothing.0 {
        fixed.overstep_fraction().clamp(0.0, 1.0)
    } else {
        1.0
    };
    for (
        info,
        location,
        prev,
        looked,
        mut drawn,
        mut transform,
        mut visibility,
        hidden,
        owner,
        directional,
        turret,
    ) in &mut query
    {
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        let curr = world_center(location.position, size) + lift(&registry, def);
        // Snap rather than slide across teleports/reveals.
        transform.translation = if prev.0.distance(curr) > 1.5 * CELL_PX {
            curr
        } else {
            prev.0.lerp(curr, alpha)
        };
        drawn.0 = between(looked.0, location.facing, alpha);
        if directional.is_some() {
            transform.rotation = facing_rotation(drawn.0);
        }
        // The gun's own look, filled in between its own two ticks: a hull driving
        // one way while its gun comes round another is two directions at once, and
        // each is interpolated from where it was.
        if let Some((TurretsComponent(turrets), previous, mut shown)) = turret {
            shown.0 = turrets
                .iter()
                .zip(previous.0.iter())
                .map(|(turret, &was)| between(was, turret.bearing, alpha))
                .collect();
        }
        // Own and allied entities always draw; an enemy or neutral one is hidden
        // while its cell is not in the local team's vision (fog of war). Building
        // ghosts (draw_ghosts) stand in for last-seen enemy structures.
        let fogged = !reveal.0
            && match owner {
                Some(owner) if allied_with_local(&session, owner.player()) => false,
                _ => !sees(
                    &session,
                    &watch,
                    &fog,
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
        if !airborne(&registry, def) {
            continue;
        }
        let size = def.location.unwrap().size();
        let ground = transform.translation.truncate() - Vec2::new(0.0, AIR_LIFT_PX);
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
    inspected: Res<crate::input::Inspected>,
    registry: Res<ContentRegistry>,
    query: Query<
        (&EntityInfoComponent, &Transform, &Visibility),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    // A playing node rings its own selection. A defeated player rings only
    // what it inspects — every player's selection is enemy attention, and a
    // player, eliminated or not, knows only what its side knows. An observer
    // rings its inspection first, then every player's selection in that
    // player's color — what each player is doing is half of what a caster
    // narrates.
    let rings: Vec<(&[SimulationId], Color)> = match session.local_role() {
        LocalRole::Player(local) if session.is_player_live(local) => {
            vec![(selection.get(local), Color::srgb(0.2, 1.0, 0.4))]
        }
        LocalRole::Player(_) => vec![(&inspected.0[..], Color::srgb(0.95, 0.95, 1.0))],
        LocalRole::Observer => std::iter::once((&inspected.0[..], Color::srgb(0.95, 0.95, 1.0)))
            .chain(
                session
                    .player_slots()
                    .map(|slot| (selection.get(slot.id()), player_color(slot.id()))),
            )
            .collect(),
    };
    if rings.iter().all(|(selected, ..)| selected.is_empty()) {
        return;
    }
    for (info, transform, visibility) in &query {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        // Several watchers can hold the same entity — the inspection plus a
        // player's selection, or two players targeting one unit. Equal radii
        // would overlap into one circle showing only the last color, so each
        // ring after the first steps outward and they read as concentric.
        let mut rings_drawn = 0;
        for (selected, color) in &rings {
            if selected.contains(&info.id()) {
                let size = registry.def(info.type_id()).location.unwrap().size();
                // Larger than the sprite so the ring isn't hidden behind it.
                let radius = size.width.max(size.height) as f32
                    * CELL_PX
                    * (0.7 + rings_drawn as f32 * 0.12);
                gizmos.circle_2d(transform.translation.truncate(), radius, *color);
                rings_drawn += 1;
            }
        }
    }
}

/// How long a cast pulse stays on screen, in ticks — 0.45 s at the nominal
/// cadence. Counted in ticks because it marks something the simulation did: at
/// any game speed, and under any throttle, the ring covers the same stretch of
/// the game rather than the same stretch of wall time.
const PULSE_TICKS: u32 = 9;

/// The moment the frame sees, in ticks, with the fixed step's overstep folded in
/// so a ring grows smoothly between ticks rather than in steps.
fn pulse_now(session: &GameSession, fixed: &Time<Fixed>) -> f32 {
    session.tick() as f32 + fixed.overstep_fraction()
}

/// How far through its [`PULSE_TICKS`] life a pulse started at `started` has
/// come, in `0.0..=1.0` — `None` once it has run out.
fn pulse_progress(now: f32, started: u32) -> Option<f32> {
    let age = now - started as f32;
    (age <= PULSE_TICKS as f32).then(|| (age / PULSE_TICKS as f32).clamp(0.0, 1.0))
}

/// Ring pulses marking skills that have just been cast.
#[derive(Resource, Default)]
pub struct SkillPulses {
    /// Live pulses: what a skill landed on, and the tick its cast was
    /// announced on.
    active: Vec<(SkillTarget, u32)>,
}

/// Starts a pulse for every skill the simulation announced this tick, and drops
/// the expired ones (run once per tick, in the game's slot).
///
/// A refused cast — too little energy, or still cooling down — is never
/// announced and draws nothing. Pruning here keeps the store bounded across a
/// seek, which runs many ticks before anything draws.
pub fn collect_skill_pulses(
    record: Res<EventRecord>,
    session: Res<GameSession>,
    mut pulses: ResMut<SkillPulses>,
) {
    let tick = session.tick();
    pulses
        .active
        .retain(|&(_, started)| tick.saturating_sub(started) <= PULSE_TICKS);
    for event in record.events() {
        if let SimulationEvent::SkillCast { target, .. } = event {
            pulses.active.push((*target, tick));
        }
    }
}

/// What a brief place-marker is standing for.
#[derive(Clone, Copy)]
enum PuffKind {
    /// A death left something standing here — a body, or whatever burst out of
    /// the dying thing.
    Bequeathed,
    /// Something went off the map here — boarded, or stepped inside.
    Hidden,
    /// Something came back onto the map here.
    Revealed,
}

impl PuffKind {
    /// The ring's colour, so the three read apart at a glance.
    fn color(self, fade: f32) -> Color {
        match self {
            PuffKind::Bequeathed => Color::srgba(0.55, 0.5, 0.45, fade),
            PuffKind::Hidden => Color::srgba(0.45, 0.6, 0.85, fade),
            PuffKind::Revealed => Color::srgba(0.6, 0.85, 0.5, fade),
        }
    }
}

/// Brief rings marking announcements whose subject may already be dying or off
/// the map, drawn from where the subject stood.
#[derive(Resource, Default)]
pub struct Puffs {
    active: Vec<(FixedUVec2, CellSize, PuffKind, u32)>,
}

/// Starts a marker for each remains, hiding and reveal the tick announced, and
/// drops the expired ones (run once per tick, in the game's slot).
pub fn collect_puffs(world: &mut World) {
    let tick = world.resource::<GameSession>().tick();
    world.resource_scope(|world, mut puffs: Mut<Puffs>| {
        puffs
            .active
            .retain(|&(_, _, _, started)| tick.saturating_sub(started) <= PULSE_TICKS);
        for event in world.resource::<EventRecord>().events() {
            let marked = match event {
                SimulationEvent::EntitySpawned {
                    entity,
                    cause: SpawnCause::Bequeathed { .. },
                } => Some((*entity, PuffKind::Bequeathed)),
                SimulationEvent::EntityHidden { entity } => Some((*entity, PuffKind::Hidden)),
                SimulationEvent::EntityRevealed { entity } => Some((*entity, PuffKind::Revealed)),
                _ => None,
            };
            // The announcement names its subject; where it stands and how much
            // ground it covers come from the subject itself. What a death leaves
            // may be a body, which begins its life dying, so the lookup has to
            // accept that stage as well as the standing one.
            if let Some((id, kind)) = marked
                && let Some(entity) = world.resource::<EntityIndex>().any(id)
            {
                let (position, size) = entity_def::footprint(world, entity);
                puffs.active.push((position, size, kind, tick));
            }
        }
    });
}

/// Draws each live marker as a ring that grows and fades (run in `Update`).
///
/// A marker over a cell the viewer cannot see is skipped.
pub fn draw_puffs(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    fixed: Res<Time<Fixed>>,
    grid: Res<VisibilityGrid>,
    watch: Res<ObserverPerspective>,
    reveal: Res<FogReveal>,
    mut puffs: ResMut<Puffs>,
) {
    let now = pulse_now(&session, &fixed);
    puffs
        .active
        .retain(|&(_, _, _, started)| pulse_progress(now, started).is_some());

    for &(position, size, kind, started) in &puffs.active {
        let cell = CellPos::from(position);
        if !reveal.0 && !sees(&session, &watch, &grid, cell.x, cell.y) {
            continue;
        }
        let Some(progress) = pulse_progress(now, started) else {
            continue;
        };
        // Grows with the footprint, so a marker reads as belonging to what stood
        // there rather than always being unit-sized.
        let span = size.width.max(size.height) as f32;
        let radius = CELL_PX * span * (0.2 + 0.4 * progress);
        // Centred on the footprint like everything else drawn from a position: a
        // position is a footprint's corner, so drawing on it sits up and to the
        // left of what it marks.
        gizmos.circle_2d(
            world_center(position, size).truncate(),
            radius,
            kind.color(1.0 - progress),
        );
    }
}

/// Draws an expanding ring on what a skill landed on for a moment after it
/// goes off (run in `Update`).
///
/// The ring marks the aim rather than the caster: the thing struck, healed or
/// buffed, and the ground itself for a cast that landed on a cell — which is
/// what a body spent by a raise leaves behind to mark. An aim the local player
/// cannot see draws nothing.
pub fn draw_skill_pulses(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    fixed: Res<Time<Fixed>>,
    grid: Res<VisibilityGrid>,
    watch: Res<ObserverPerspective>,
    reveal: Res<FogReveal>,
    mut pulses: ResMut<SkillPulses>,
    rendered: Query<
        (&EntityInfoComponent, &Transform, &Visibility),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    let now = pulse_now(&session, &fixed);
    pulses
        .active
        .retain(|&(_, started)| pulse_progress(now, started).is_some());

    for &(target, started) in &pulses.active {
        let at = match target {
            SkillTarget::Entity(id) => rendered
                .iter()
                .find(|(info, _, visibility)| {
                    info.id() == id && !matches!(visibility, Visibility::Hidden)
                })
                .map(|(_, transform, _)| transform.translation),
            SkillTarget::Position(position) => {
                let cell = CellPos::from(position);
                (reveal.0 || sees(&session, &watch, &grid, cell.x, cell.y))
                    .then(|| world_center(position, CellSize::ONE))
            }
        };
        let Some(at) = at else {
            continue;
        };
        // Expand and fade over the pulse's life.
        let Some(progress) = pulse_progress(now, started) else {
            continue;
        };
        let radius = CELL_PX * (0.35 + 0.55 * progress);
        gizmos.circle_2d(
            at.truncate(),
            radius,
            Color::srgba(1.0, 0.85, 0.35, 1.0 - progress),
        );
    }
}

/// Draws the ring a caster closes while it works at a cast (run in `Update`).
///
/// A cast that is not instant holds its caster still for the ticks before the
/// effect lands: the ring draws inward over that stretch and closes on the
/// tick it goes off, so the work shows and not only its result. A caster the
/// local player cannot see draws nothing.
pub fn draw_casts(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    casters: Query<
        (
            &CastComponent,
            &OrderQueueComponent,
            Option<&StatsComponent>,
            &Transform,
            &Visibility,
        ),
        (With<Renderable>, Without<HiddenComponent>),
    >,
) {
    for (cast, orders, stats, transform, visibility) in &casters {
        // Only while the caster is still working at it: past its point the
        // cast has gone off, and what is left is the caster standing.
        match cast.stage {
            CastStage::Working if cast.phase > 0 => {}
            CastStage::Working | CastStage::Holding => continue,
        }
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let Some(point) = cast_point(&registry, orders, stats) else {
            continue;
        };
        let progress = (cast.phase as f32 / point as f32).clamp(0.0, 1.0);
        gizmos.circle_2d(
            transform.translation.truncate(),
            CELL_PX * (0.95 - 0.5 * progress),
            Color::srgba(0.72, 0.45, 1.0, 0.25 + 0.6 * progress),
        );
    }
}

/// The tick of a cast's work the effect lands on, or `None` when what is being
/// cast lands the moment it is ordered.
fn cast_point(
    registry: &ContentRegistry,
    orders: &OrderQueueComponent,
    stats: Option<&StatsComponent>,
) -> Option<u32> {
    let (skill, _) = orders
        .0
        .iter()
        .find_map(|entry| entry.order.cast_params())?;
    let SkillCaster::Entity { casting, .. } = &registry.skill_def(skill)?.caster else {
        return None;
    };
    match casting {
        Casting::Instant => None,
        Casting::Delayed { point, .. } => match point {
            Quantity::Constant(ticks) => Some(*ticks),
            Quantity::Stat(stat) => stats?.effective_as_u32(*stat),
        },
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
    watch: Res<ObserverPerspective>,
    reveal: Res<FogReveal>,
    targets: Query<(&EntityInfoComponent, &Transform, Option<&HiddenComponent>), With<Renderable>>,
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

        // Drawn from where the simulation released it, which is where the weapon
        // that fired it sits: the middle of a body for its own weapon, the corner a
        // turret is mounted on for that gun's, and the holder for a garrisoned
        // archer whose own bearer stands nowhere.
        let from = world_point(shot.origin);
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
        if !reveal.0 && !sees(&session, &watch, &grid, cell.x, cell.y) {
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

    // Rally lines are own-units UI: a node with no local player owns nothing.
    let Some(local) = session.local_player() else {
        return;
    };
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

/// Draws a short line from each entity's center along the look it is drawn at
/// (Update): where a body points, or where a gun is trained, each in its own
/// colour ([`LOOK_COLOR`], [`BEARING_COLOR`]).
///
/// The drawn look rather than the simulation's own: between two ticks the sprite
/// is part way from one bearing to the next, and a line taken straight from the
/// simulation would run ahead of the shape it is meant to belong to — which, on a
/// placeholder shape, is the direction cue the eye actually follows. Unsmoothed
/// the two are the same thing, so the line still reports the tick exactly when
/// that is what is wanted.
pub fn draw_facing(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    query: Query<
        (
            &EntityInfoComponent,
            &DrawnFacing,
            &Transform,
            &Visibility,
            Option<&Directional>,
            Option<&DrawnBearings>,
        ),
        Without<HiddenComponent>,
    >,
) {
    for (info, drawn, transform, visibility, directional, guns) in &query {
        // Don't trace a unit hidden by fog (interpolate_sprites set its visibility).
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let def = registry.def(info.type_id());
        let size = def.location.unwrap().size();
        let length = size.width.min(size.height) as f32 * CELL_PX * 0.6;
        let center = transform.translation.truncate();
        // A body that turns traces where it goes, from its own middle.
        if directional.is_some() {
            let line = facing_line(drawn.bearing());
            gizmos.line_2d(center, center + line * length, LOOK_COLOR);
        }
        // Each gun traces where it is trained, from where it sits: a hull driving
        // one way with its guns round another shows every one of them, and a keep
        // with a gun at each corner shows four lines from four corners.
        //
        // Scaled to the gun rather than to the body that carries it, so four short
        // lines read as four guns instead of as one shape with spines.
        let Some(guns) = guns else { continue };
        for (mount, &bearing) in def.turrets.iter().zip(guns.bearings()) {
            let at = center + mounted_at(transform, def, mount);
            let reach = mount.size().width.min(mount.size().height) as f32 * CELL_PX * 0.45;
            gizmos.line_2d(at, at + facing_line(bearing) * reach, BEARING_COLOR);
        }
    }
}

/// Where a mounted gun sits on the drawn body, as an offset from its middle.
fn mount_offset(def: &EntityTypeDef, mount: &TurretMount) -> Vec2 {
    let footprint = def.location.unwrap().size();
    let middle = |at: u32, span: u32, whole: u32| {
        (at as f32 + span as f32 / 2.0 - whole as f32 / 2.0) * CELL_PX
    };
    Vec2::new(
        middle(mount.origin().x, mount.size().width, footprint.width),
        -middle(mount.origin().y, mount.size().height, footprint.height),
    )
}

/// The same offset taken through the body's own rotation, so a gun on a hull that
/// turns rides round with it.
fn mounted_at(transform: &Transform, def: &EntityTypeDef, mount: &TurretMount) -> Vec2 {
    let offset = mount_offset(def, mount).extend(0.0);
    (transform.rotation * offset).truncate()
}

/// The tile color for a terrain, by content name.
pub(crate) fn terrain_color(terrain: &str) -> Color {
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
    let Some(map) = map::opened(&session, scenario.as_deref()) else {
        return;
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

    // Field overlay between the terrain and the fog (z 0.25), and the fog
    // overlay above it but below entities (z 0.5): one tile per cell each,
    // updated every frame from the grids.
    for y in 0..map.height() {
        for x in 0..map.width() {
            let center = Vec3::new(
                (x as f32 + 0.5) * CELL_PX,
                -(y as f32 + 0.5) * CELL_PX,
                0.25,
            );
            commands.spawn((
                FieldTile { x, y },
                Sprite::from_color(Color::NONE, Vec2::splat(CELL_PX)),
                Transform::from_translation(center),
                InGameUi,
            ));
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

/// The tint a covered cell is drawn in, by field name: creep and blight are
/// shown whoever's they are, power only where it is the viewed player's own.
pub(crate) const CREEP_TINT: Color = Color::srgba(0.55, 0.2, 0.65, 0.5);
pub(crate) const POWER_TINT: Color = Color::srgba(0.25, 0.55, 1.0, 0.28);
pub(crate) const BLIGHT_TINT: Color = Color::srgba(0.16, 0.16, 0.17, 0.6);

/// The player whose own fields this node draws: the local player, or the
/// side an observer is watching.
pub(crate) fn viewed_player(
    session: &GameSession,
    watch: &ObserverPerspective,
) -> Option<PlayerId> {
    session.local_player().or(watch.0)
}

/// Tints each field tile by what covers its cell.
pub fn update_field_overlay(
    session: Res<GameSession>,
    registry: Res<ContentRegistry>,
    fields: Res<FieldGrid>,
    watch: Res<ObserverPerspective>,
    mut tiles: Query<(&FieldTile, &mut Sprite)>,
) {
    let creep = registry.field("creep");
    let power = registry.field("power");
    let blight = registry.field("blight");
    let viewed = viewed_player(&session, &watch);
    for (tile, mut sprite) in &mut tiles {
        let cell = CellPos::new(tile.x, tile.y);
        let color = if creep.is_some_and(|creep| !fields.covered(creep, cell).is_empty()) {
            CREEP_TINT
        } else if blight.is_some_and(|blight| !fields.covered(blight, cell).is_empty()) {
            BLIGHT_TINT
        } else if let (Some(power), Some(player)) = (power, viewed)
            && fields.covered(power, cell).contains(player)
        {
            POWER_TINT
        } else {
            Color::NONE
        };
        // Write only on change, as the fog does.
        if sprite.color != color {
            sprite.color = color;
        }
    }
}

/// Darkens each fog tile by the local team's knowledge of its cell: black when
/// unexplored, dimmed when explored-but-unseen, clear when visible.
pub fn update_fog_overlay(
    session: Res<GameSession>,
    fog: Res<VisibilityGrid>,
    watch: Res<ObserverPerspective>,
    reveal: Res<FogReveal>,
    mut tiles: Query<(&FogTile, &mut Sprite)>,
) {
    for (tile, mut sprite) in &mut tiles {
        let alpha = if reveal.0 {
            0.0
        } else {
            match perspective(&session, &watch, &fog, tile.x, tile.y) {
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
    watch: Res<ObserverPerspective>,
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
    let mut alive = HashSet::new();
    for (info, location, tags, owner) in &buildings {
        let own_team = owner.is_some_and(|o| allied_with_local(&session, o.player()));
        if own_team || !tags.contains(tags::BUILDING) {
            continue;
        }
        alive.insert(info.id());
        let (x, y) = (
            location.position.x.to_num::<u32>(),
            location.position.y.to_num::<u32>(),
        );
        if sees(&session, &watch, &fog, x, y) {
            let size = registry.def(info.type_id()).location.unwrap().size();
            ghosts.0.insert(
                info.id(),
                GhostSprite {
                    origin: (x, y),
                    center: world_center(location.position, size).truncate(),
                    size,
                    shape: ghost_shape(info.type_name(), size),
                },
            );
        }
    }

    ghosts.0.retain(|id, ghost| {
        match perspective(&session, &watch, &fog, ghost.origin.0, ghost.origin.1) {
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
                        GhostShape::Circle { circumradius } => {
                            gizmos.circle_2d(iso, *circumradius, color);
                        }
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
        Shape::Fortress | Shape::Octagon => GhostShape::Polygon {
            sides: 8,
            circumradius,
        },
        Shape::Circle | Shape::Pod | Shape::Grub | Shape::Hive => {
            GhostShape::Circle { circumradius }
        }
        // Everything else is ghosted by its footprint: the shapes that are
        // squares outright, and the ones whose outline is a square with
        // something drawn on top of it.
        Shape::Square
        | Shape::Triangle
        | Shape::Diamond
        | Shape::Skeleton
        | Shape::Spawn
        | Shape::Halls
        | Shape::Citadel
        | Shape::Pentagon
        | Shape::Ship
        | Shape::Cross
        | Shape::Gryphon { .. }
        | Shape::Zeppelin
        | Shape::Overlord
        | Shape::WarWagon
        | Shape::WatchTower
        | Shape::GuardTower
        | Shape::SpiritTower
        | Shape::NerubianTower
        | Shape::Pylon
        | Shape::EntangledMine
        | Shape::Dish
        | Shape::Lab
        | Shape::Refinery
        | Shape::Tank { .. } => GhostShape::Rect {
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
/// The ring around a site its crew left standing, waiting for a builder.
const HALTED_SITE_COLOR: Color = Color::srgb(0.6, 0.6, 0.65);
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

/// The patch a watch is holding open.
const WATCH_PATCH_COLOR: Color = Color::srgb(0.55, 0.85, 1.0);

/// The coupling between an annex and the primary it stands with.
const ANNEX_BOND_COLOR: Color = Color::srgb(1.0, 0.62, 0.18);

/// The tie between a broodling and the breeder that bred it.
const BROOD_TIE_COLOR: Color = Color::srgb(0.75, 0.45, 0.95);

/// A breeder's progress toward its next birth.
const BROOD_WORK_COLOR: Color = Color::srgb(0.75, 0.45, 0.95);

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
            | Order::Cast { .. }
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

/// Rings each patch of map a watch is holding open, for as long as it holds
/// (run in `Update`).
///
/// Without it a scan cast over ground already in sight leaves no mark at all,
/// and a scan is the one skill whose whole effect is what it lets the caster
/// see. Only the watches this node is on the side of are drawn: another
/// player's reveal is not the viewer's to know about.
pub fn draw_watch_patches(
    mut gizmos: Gizmos,
    session: Res<GameSession>,
    watch: Res<ObserverPerspective>,
    watches: Res<Watches>,
) {
    for held in watches.in_force() {
        let shown = match (session.local_player(), watch.0) {
            (Some(_), _) => allied_with_local(&session, held.player),
            (None, Some(player)) => session.are_allied(player, held.player),
            (None, None) => true,
        };
        if !shown {
            continue;
        }
        let center = world_center(FixedUVec2::from(held.center), CellSize::ONE).truncate();
        // Half a cell beyond the radius, so the ring encloses the outermost
        // cells the sweep reveals instead of cutting through their centres:
        // the reveal runs from the patch's whole 1x1 footprint, as reach does.
        gizmos.circle_2d(
            center,
            (held.radius as f32 + 0.5) * CELL_PX,
            WATCH_PATCH_COLOR,
        );
    }
}

/// Draws each visible broodling's tie to its breeder (run in `Update`): a line
/// from the broodling to the breeder, whether it sits in the breeder's berths
/// or has stepped out onto the ground to change.
pub fn draw_brood_ties(
    mut gizmos: Gizmos,
    // Anchored on the interpolated transforms, so the tie glides with the
    // bodies it joins.
    broodlings: Query<(&BredComponent, &Transform, &Visibility), Without<HiddenComponent>>,
    breeders: Query<(&EntityInfoComponent, &Transform, &Visibility), Without<HiddenComponent>>,
) {
    for (bred, transform, visibility) in &broodlings {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let Some(end) = breeders
            .iter()
            .find(|(breeder, _, visible)| {
                breeder.id() == bred.by && !matches!(visible, Visibility::Hidden)
            })
            .map(|(_, transform, _)| transform.translation.truncate())
        else {
            continue;
        };
        gizmos.line_2d(transform.translation.truncate(), end, BROOD_TIE_COLOR);
    }
}

/// Draws which primary an annex stands with (run in `Update`): docked, a
/// coupling to the primary whose dock it holds, with a plug at its own end;
/// alone, the same plug in the halted color inside a halted ring.
///
/// The two states read apart at a glance in one vocabulary: a lab bound to its
/// factory is tied to it, and one a lift-off left behind is unplugged and
/// ringed. A coupling needs both ends on screen, so an annex whose primary is
/// under fog draws its plug alone. An annex still going up draws neither: the
/// site's own markers have it until it stands.
pub fn draw_annex_bonds(
    mut gizmos: Gizmos,
    registry: Res<ContentRegistry>,
    // Anchored on the interpolated transforms, so the coupling glides with the
    // buildings it ties together rather than stepping behind them.
    annexes: Query<
        (
            &EntityInfoComponent,
            &AnnexComponent,
            &Transform,
            &Visibility,
        ),
        (
            Without<HiddenComponent>,
            Without<UnderConstructionComponent>,
        ),
    >,
    primaries: Query<(&EntityInfoComponent, &Transform, &Visibility), Without<HiddenComponent>>,
) {
    for (info, annex, transform, visibility) in &annexes {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let start = transform.translation.truncate();
        let plug = CELL_PX * 0.18;
        match annex.docked_to {
            Docking::Primary(primary_id) => {
                gizmos.circle_2d(start, plug, ANNEX_BOND_COLOR);
                if let Some(end) = primaries
                    .iter()
                    .find(|(primary, _, visible)| {
                        primary.id() == primary_id && !matches!(visible, Visibility::Hidden)
                    })
                    .map(|(_, transform, _)| transform.translation.truncate())
                {
                    gizmos.line_2d(start, end, ANNEX_BOND_COLOR);
                }
            }
            Docking::Alone => {
                let size = registry.def(info.type_id()).location.unwrap().size();
                let radius = size.width.max(size.height) as f32 * CELL_PX * 0.55;
                gizmos.circle_2d(start, plug, HALTED_SITE_COLOR);
                gizmos.circle_2d(start, radius + 4.0, HALTED_SITE_COLOR);
            }
        }
    }
}

/// Marks the jobs themselves: a ring in the verb's color around a source being
/// worked or an entity being mended, and a dot per crew member above it, so a
/// stacked crew is countable at a glance (run in `Update`). An unfinished site
/// shows its crew dots too, and a grey ring while it stands halted; its own
/// state is the translucent shape (see [`tint_under_construction`]) and the
/// progress bar (see [`draw_status_bars`]).
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
            match &construction.work {
                SiteWork::Crew { builders } => crew += builders.len(),
                SiteWork::Halted => {
                    gizmos.circle_2d(center, radius + 4.0, HALTED_SITE_COLOR);
                }
                SiteWork::Unattended { .. } => {}
            }
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

/// Draws slim bars over entities — energy, then health, then what is left of a
/// timed life, then construction
/// progress while a site goes up, then training progress with a dot per queued
/// unit, then research progress, then a running form change's progress, then
/// a breeder's progress toward its next birth — for whichever of those the
/// entity has (run in `Update`). Whatever fog or hiding
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
            Option<&LifetimeComponent>,
            Option<&UnderConstructionComponent>,
            Option<&TrainQueueComponent>,
            Option<&TrainComponent>,
            Option<&ResearchComponent>,
            Option<&MorphComponent>,
            Option<&TransporterComponent>,
            Option<&BroodComponent>,
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
        lifetime,
        construction,
        queue,
        train,
        research,
        morph,
        transporter,
        brood,
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

        // What a summon has left of its time, under its health: a thin bar
        // draining as it ages.
        if let (Some(lifetime), Some(stats)) = (lifetime, stats)
            && let Some(limit) = stats.effective_as_u32(EntityStatId::LIFETIME)
            && limit > 0
        {
            let left = limit.saturating_sub(lifetime.age);
            bar(
                &mut gizmos,
                left as f32 / limit as f32,
                Color::srgb(0.75, 0.75, 0.85),
                y,
            );
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
            // The change's length under the transition's own terms, read from
            // the form the change started from — the one worn now may be an
            // interim form that declares none: a constant is what it says, a
            // stat reads the changing entity's effective value — the same
            // reading the simulation ticks against.
            let time = registry
                .def(morph.from)
                .morphs
                .iter()
                .find(|transition| transition.into_type() == morph.into)
                .map(|transition| match transition.time() {
                    Quantity::Constant(ticks) => ticks,
                    Quantity::Stat(id) => stats
                        .and_then(|stats| stats.effective(id))
                        .map_or(0, |time| time.to_num::<u32>()),
                })
                .unwrap_or(1)
                .max(1);
            let fraction = morph.progress as f32 / time as f32;
            bar(&mut gizmos, fraction, MORPH_WORK_COLOR, y);
            y += 4.0;
        }

        if let (Some(brood), Some(breeder)) = (brood, def.breeder.as_ref()) {
            // The wait for the next birth, under the breeding form's own
            // terms; a brood at its limit holds its progress, and the bar
            // holds with it.
            let period = match breeder.period() {
                Quantity::Constant(ticks) => ticks,
                Quantity::Stat(id) => stats
                    .and_then(|stats| stats.effective(id))
                    .map_or(0, |time| time.to_num::<u32>()),
            }
            .max(1);
            let fraction = brood.progress as f32 / period as f32;
            bar(&mut gizmos, fraction, BROOD_WORK_COLOR, y);
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

/// Fades every body on the ground as it decays, so a fresh corpse reads as
/// fresher than one about to go (run in `Update`).
///
/// A site going up is tinted by the same means ([`tint_under_construction`]);
/// the two never meet on one entity, since remains are never built.
pub fn fade_remains(
    registry: Res<ContentRegistry>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    query: Query<
        (
            &MeshMaterial2d<ColorMaterial>,
            &EntityInfoComponent,
            &DyingComponent,
            Option<&Children>,
        ),
        (With<Renderable>, With<RemainsComponent>),
    >,
    parts: Query<&MeshMaterial2d<ColorMaterial>, Without<RemainsComponent>>,
) {
    /// How faint the last tick of a body is drawn.
    const FAINTEST: f32 = 0.2;

    for (material, info, dying, children) in &query {
        // A body's countdown is its `lifetime`, the stat the simulation seeded
        // it from — not the dying time every other death runs down.
        let decay = registry
            .def(info.type_id())
            .base_stat_as_u32(EntityStatId::LIFETIME)
            .unwrap_or(1)
            .max(1);
        let left = dying.ticks_remaining as f32 / decay as f32;
        let alpha = FAINTEST + (1.0 - FAINTEST) * left.clamp(0.0, 1.0);
        // The parts a silhouette is drawn from carry their own materials — a
        // medic's cross, a tower's turret — and a body fades whole or it reads
        // as half a corpse.
        let drawn = children
            .into_iter()
            .flatten()
            .filter_map(|&child| parts.get(child).ok())
            .chain(std::iter::once(material));
        for material in drawn {
            // Read first, write only on a change, as the construction tint does.
            if materials
                .get(&material.0)
                .is_some_and(|m| m.color.alpha() != alpha)
                && let Some(material) = materials.get_mut(&material.0)
            {
                material.color.set_alpha(alpha);
            }
        }
    }
}
