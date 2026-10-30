#![allow(dead_code)]

use std::time::Duration;

use bevy::{app::FixedMain, ecs::entity::EntityNotSpawnedError, prelude::*};
use ferrets_bevy_plugin::{
    GameSet, NetworkPlugin, NominalTimestep, PendingInput, SimulationPlugin, TickPacing, map,
    replay,
};
use ferrets_content::{
    affiliation::Affiliation,
    annex::{AloneConduct, AnnexClaim, AnnexLife, AnnexWork},
    attack::{AttackDef, Delivery, Slain, Weapon},
    berths::BerthGroup,
    brood::{Lingering, OrphanFate},
    build::BuilderAttendance,
    cost::Cost,
    domain::{DomainDef, DomainId},
    dying::{Bequest, LeftBy},
    entity_buffs::{EntityBuffDef, EntityBuffId, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    field::FieldId,
    kinds::Kinds,
    layer::LayerLevel,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry, RevertCarry, ViaInterrupted,
    },
    player_buffs::PlayerBuffDef,
    pool::{Pool, PoolInitial},
    pool_def::{PoolId, PoolRole},
    pool_shift::PoolShift,
    price,
    projectile::{Aim, ProjectileDef},
    quantity::Quantity,
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    requirement::Requirement,
    research::{ResearchDef, ResearchId},
    resource::{Banking, DepletionPolicy, HarvestData},
    rise::{RisePool, RisePoolInitial},
    skills::{
        Casting, EntityCastEffect, PaymentPoint, PlayerCastEffect, Recipient, SkillCaster,
        SkillDef, SkillId,
    },
    splash::{SplashDef, SplashShape},
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
    terrains::Terrains,
    transport::{PassengerConduct, PassengerFate},
    turret::{TurretDef, TurretFire, TurretMount, TurretStats, WeaponConduct},
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};

use ferrets_math::{
    FixedI64, FixedU64, facing::Facing, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2,
};
use ferrets_nav_grid::{
    layer_id::LayerId, layer_mask::LayerMask, nav_grid::NavGrid, terrain_id::TerrainId,
};
use ferrets_network::{
    role::Role,
    roster::Roster,
    session::NetSession,
    transport::{NetworkTransport, loopback::LoopbackTransport},
};
use ferrets_replay::{
    buffer::SharedBuffer,
    header::{RecordedGame, ReplayHeader},
    recorder::Recorder,
};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode, SkillCasterRef, SkillTarget},
    components::{
        entity_info::EntityInfoComponent,
        entity_stats::StatsComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::OwnerComponent,
        pools,
        rally::RallyTarget,
        rise::{RiseAttendance, RiseOrigin, RisingComponent},
        train::TrainQueueComponent,
        transport::TransporterComponent,
        turret::TurretsComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent},
    fields::FieldGrid,
    game_loop::buffs::{self, Bearing},
    input::{InputFrames, PlayerFrame},
    map::Map,
    map_data::MapData,
    movement_model::MovementModel,
    order::{AttackTarget, Order},
    resources::PlayerResources,
    ruleset::{RemainsLimit, Ruleset},
    selection::Selection,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, local_role::LocalRole, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    skirmish::Skirmish,
    spawn::{self, Arrival, FieldReach},
    statistics::Statistics,
    visibility::{self, Senses, Sighting},
};

/// The name of the ground layer the harness content declares.
const GROUND_LAYER: &str = "ground";
/// The id [`GROUND_LAYER`] resolves to — it is the first registered layer.
pub const GROUND: LayerId = LayerId::new(1);
/// The name of the layer fliers occupy, registered by [`register_air`].
const AIR_LAYER: &str = "air";
/// The id [`AIR_LAYER`] resolves to where it is registered.
pub const AIR: LayerId = LayerId::new(2);
/// The terrain the harness content declares, and every harness map is made
/// of unless a test lays water.
pub const LAND: TerrainId = TerrainId::new(1);
/// The terrain [`register_dry`] adds.
const WATER: TerrainId = TerrainId::new(2);
/// The domain [`register_ground`] declares: the ground layer over every
/// terrain.
pub const GROUND_DOMAIN: DomainId = DomainId::new(0);
/// The domain [`register_air`] declares: the air layer over every terrain.
pub const AIR_DOMAIN: DomainId = DomainId::new(1);

/// Registers the ground layer, the land terrain and the [`GROUND_DOMAIN`].
fn register_ground(registry: &mut ContentRegistry) {
    assert_eq!(
        registry.register_layer(GROUND_LAYER, LayerLevel::At(1), &[]),
        GROUND
    );
    assert_eq!(registry.register_terrain("land"), LAND);
    assert_eq!(
        registry.register_domain("ground", DomainDef::new(GROUND, Terrains::Any, None)),
        GROUND_DOMAIN
    );
}

/// The layer of a domain the harness declares.
///
/// Panics for a domain the harness does not declare.
fn domain_layer(domain: DomainId) -> LayerId {
    match domain {
        GROUND_DOMAIN => GROUND,
        AIR_DOMAIN => AIR,
        other => panic!("the harness declares no domain {other:?}"),
    }
}

/// Registers the air layer and the [`AIR_DOMAIN`], after [`register_ground`].
pub fn register_air(registry: &mut ContentRegistry) {
    assert_eq!(
        registry.register_layer(AIR_LAYER, LayerLevel::At(2), &[]),
        AIR
    );
    assert_eq!(
        registry.register_domain("air", DomainDef::new(AIR, Terrains::Any, None)),
        AIR_DOMAIN
    );
}

/// Registers the [`WATER`] terrain and a domain on the ground layer over land
/// alone, after [`register_ground`], returning the domain.
fn register_dry(registry: &mut ContentRegistry) -> DomainId {
    assert_eq!(registry.register_terrain("water"), WATER);
    registry.register_domain(
        "dry",
        DomainDef::new(GROUND, Terrains::Only(LAND.into()), None),
    )
}

/// `def` moved into `domain`, keeping its footprint and the layers it
/// occupies.
pub fn in_domain(def: EntityTypeDef, domain: DomainId) -> EntityTypeDef {
    let location = def.location.expect("a harness type has a location");
    def.with_location(
        domain,
        location.size(),
        location.solidity(),
        location.occupies(),
    )
}

/// [`orders_app`] under `model` whose content also declares the
/// [`register_dry`] domain and a `landlubber` — the orders soldier in it — then
/// registers what `extra` adds with the dry domain in hand, on a map whose
/// `water` cells are water.
pub fn dry_orders_app(
    model: MovementModel,
    water: &[(u32, u32)],
    extra: impl FnOnce(&mut ContentRegistry, DomainId),
) -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    register_orders_content(&mut app);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let dry = register_dry(&mut registry);
        let soldier = registry
            .entity("soldier")
            .expect("the orders content declares a soldier")
            .clone();
        let mut landlubber = in_domain(soldier, dry);
        landlubber.name = "landlubber".to_string();
        registry.register(landlubber);
        extra(&mut registry, dry);
        registry.validate();
    }
    install_water_map(&mut app, model, water);
    start_game(&mut app);
    app
}

/// The cells of a 3×3 pool of water, (9,9)..(11,11).
pub fn water_pool() -> Vec<(u32, u32)> {
    (9..=11)
        .flat_map(|y| (9..=11).map(move |x| (x, y)))
        .collect()
}

/// Swaps the harness map for a 32×32 one under `model`, of land whose `water`
/// cells are water, serving the movers registered so far. Call once the
/// content is registered, before any spawns.
fn install_water_map(app: &mut App, model: MovementModel, water: &[(u32, u32)]) {
    let mut data = MapData::new("test", Projection::Isometric, 32, 32, "land");
    data.set_movement_model(model);
    for &cell in water {
        data.set_terrain(cell, "water");
    }
    let map = Map::from_data(&data, app.world().resource::<ContentRegistry>());
    map::install_map(app.world_mut(), map);
}

/// A body weapon reaching `targets` that lands its hit where it stands — the
/// plainest one there is, for fixtures about anything but the weapon.
pub fn weapon(targets: impl Into<LayerMask>) -> AttackDef {
    AttackDef::new(Weapon::new(
        targets,
        Delivery::Instant,
        None,
        Slain::Remains,
    ))
}

/// A one-cell solid mover named `name` in `domain`: half a cell a tick,
/// turning on the spot. No health, sight or weapon of its own.
pub fn walker(name: &str, domain: DomainId) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(domain, CellSize::ONE, Solidity::Solid, domain_layer(domain))
        .with_movement(
            FixedU64::from_num(0.5),
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
}

/// Creates an app with the simulation plugin on a 32×32 single-layer map,
/// with player slot `0` as the local player.
///
/// The session uses [`FinishPolicy::Endless`] so a lone or unpopulated slot is
/// never read as a win; a test that exercises the victory condition opts into
/// [`FinishPolicy::LastStanding`] with `set_finish_policy`. The caller registers
/// content and starts the session; the registry already declares
/// [`GROUND_LAYER`], matching the map's grid.
pub fn make_app(slots: Vec<PlayerSlot>) -> App {
    make_app_under(slots, Ruleset::new(RemainsLimit::Unbounded))
}

/// The same app under rules a test states for itself — a remains cap, say.
pub fn make_app_under(slots: Vec<PlayerSlot>, rules: Ruleset) -> App {
    make_app_seated(slots, rules, 0)
}

/// The same app with `local` the player whose commands the test issues.
pub fn make_app_as(slots: Vec<PlayerSlot>, local: PlayerId) -> App {
    make_app_seated(slots, Ruleset::new(RemainsLimit::Unbounded), local)
}

/// The app [`make_app_under`] builds, seated as `local`.
fn make_app_seated(slots: Vec<PlayerSlot>, rules: Ruleset, local: PlayerId) -> App {
    let mut registry = ContentRegistry::default();
    register_ground(&mut registry);

    let mut nav_grid = NavGrid::new(32, 32, |_| LAND);
    nav_grid.add_flat_layer(GROUND, 1);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            LocalRole::Player(local),
            slots,
            "test",
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
            rules,
        ),
        Map::new(
            "test",
            Projection::Isometric,
            MovementModel::Cell,
            nav_grid,
            vec![],
        ),
    ));
    app.insert_resource(registry);
    app
}

/// Kills `victim` outright, credited to `by`, with a weapon that leaves a body.
///
/// The announced death a test wants when the weapon is not its subject; one
/// that is about what a weapon leaves calls [`spawn::despawn_killed`] with the
/// [`Slain`] it means.
pub fn despawn_killed(
    world: &mut World,
    victim: Entity,
    by: SimulationId,
    by_owner: Option<PlayerId>,
) {
    spawn::despawn_killed(world, victim, by, by_owner, Slain::Remains);
}

/// `count` occupied human slots with contiguous ids and no team.
pub fn human_slots(count: u8) -> Vec<PlayerSlot> {
    (0..count)
        .map(|id| PlayerSlot::occupied(id, PlayerType::Human, None, None))
        .collect()
}

/// The tick the session has reached.
pub fn tick(app: &App) -> u32 {
    app.world().resource::<GameSession>().tick()
}

/// The nominal cadence the cadence suites install, standing in for the demo's
/// own 20 Hz.
pub const NOMINAL_HZ: f64 = 20.0;
/// The same cadence as a tick length, in the milliseconds the pacing counts in.
pub const NOMINAL_MILLIS: FixedU64 = FixedU64::lit("50");

/// Makes `app` measure `exec_millis` per tick against the nominal cadence — the
/// cost its throttle reacts to, and what it then reports to its peers.
pub fn set_tick_cost(app: &mut App, exec_millis: FixedU64) {
    app.world_mut().resource_mut::<NominalTimestep>().0 =
        Some(Duration::from_millis(NOMINAL_MILLIS.to_num()));
    app.world_mut().resource_mut::<TickPacing>().exec_millis = exec_millis;
}

/// Starts recording `app` into a fresh in-memory buffer, handed back so the
/// recording can be read once its ticks have run.
pub fn record_into(app: &mut App, header: &ReplayHeader) -> SharedBuffer {
    let buffer = SharedBuffer::default();
    let recorder = Recorder::new(buffer.clone(), header).expect("start recording");
    replay::recorder::install_per_game(app.world_mut(), recorder);
    buffer
}

/// A replay header for a skirmish on the harness map — its name and its rules
/// (see [`make_app`]), so a recording made in a test rebuilds into the same map
/// it was played on.
pub fn skirmish_header(slots: Vec<PlayerSlot>, finish_policy: FinishPolicy) -> ReplayHeader {
    ReplayHeader::new(
        RecordedGame::Skirmish(Skirmish {
            slots,
            map: "test".to_string(),
            finish_policy,
            rules: Ruleset::new(RemainsLimit::Unbounded),
        }),
        MovementModel::Cell,
        Projection::Isometric,
    )
}

/// A fixed-point value parsed from decimal digits.
pub fn fixed(text: &str) -> FixedU64 {
    FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a value"))
}

/// Each of `pools` held at its own initial while rising.
pub fn rise_initial_pools(pools: &[PoolId]) -> Vec<(PoolId, RisePool)> {
    pools
        .iter()
        .map(|pool| (*pool, RisePool::Initial))
        .collect()
}

/// A signed fixed-point value parsed from decimal digits.
pub fn signed_fixed(text: &str) -> FixedI64 {
    FixedI64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a signed value"))
}

/// A modifier adding `magnitude`, written as decimal digits, to `stat`.
pub fn flat(stat: EntityStatId, magnitude: &str) -> EntityModifier {
    EntityModifier {
        stat,
        op: ModifierOp::FlatAdd,
        magnitude: signed_fixed(magnitude),
    }
}

/// What `player`'s side makes of `entity` now.
pub fn sighting_of(app: &App, player: PlayerId, entity: Entity) -> Sighting {
    visibility::sighting(app.world(), player, entity, Senses::SeatDeclared)
}

pub fn pos(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// A position part way across its cells, which only a continuous-model body can
/// hold (see [`continuous_orders_app`]). Written as decimals rather than floats,
/// so the position is the one the digits name.
pub fn part_way(x: &str, y: &str) -> FixedUVec2 {
    let coordinate =
        |text: &str| FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a position"));
    FixedUVec2::new(coordinate(x), coordinate(y))
}

/// The offset from one position to another, which unsigned positions cannot
/// hold themselves.
pub fn offset(from: FixedUVec2, to: FixedUVec2) -> FixedVec2 {
    FixedVec2::new(
        to.x.to_num::<FixedI64>() - from.x.to_num::<FixedI64>(),
        to.y.to_num::<FixedI64>() - from.y.to_num::<FixedI64>(),
    )
}

/// Creates an entity of `type_name` at `position` for `owner`, its field
/// sources at their initial reach, announcing nothing.
pub fn create_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
) -> Option<(Entity, SimulationId)> {
    spawn::create_entity(
        world,
        type_name,
        position,
        owner,
        FieldReach::Initial,
        Arrival::Standing { starts: &[] },
    )
}

/// Every announcement of every tick run so far, as a game system in [`GameSet`]
/// saw it. Suites filter the log for what they assert on.
#[derive(Resource, Default)]
pub struct Announced(pub Vec<SimulationEvent>);

/// Keeps every tick's announcements in [`Announced`] from here on: a game
/// system in [`GameSet`] reads the record before the tick retires it.
pub fn record_announcements(app: &mut App) {
    app.init_resource::<Announced>();
    app.add_systems(FixedLast, note_announced.in_set(GameSet));
}

fn note_announced(record: Res<EventRecord>, mut seen: ResMut<Announced>) {
    seen.0.extend(record.events().iter().cloned());
}

/// A fixture entity of `type_name` at `(x, y)` owned by `player`, created
/// without announcing it. Panics when the position cannot host the type.
pub fn create_owned(
    app: &mut App,
    type_name: &str,
    x: u32,
    y: u32,
    player: PlayerId,
) -> (Entity, SimulationId) {
    spawn::create_entity(
        app.world_mut(),
        type_name,
        pos(x, y),
        Some(player),
        FieldReach::Initial,
        Arrival::Standing { starts: &[] },
    )
    .unwrap_or_else(|| panic!("{type_name} fits at ({x}, {y})"))
}

/// A fixture entity of `type_name` at `(x, y)` owned by `player`, still
/// rising as a founded site, spawned without announcing it, with no work put
/// in and attended as `attendance` says.
/// Panics when the position cannot host the type.
pub fn create_rising(
    app: &mut App,
    type_name: &str,
    x: u32,
    y: u32,
    player: PlayerId,
    attendance: RiseAttendance,
) -> (Entity, SimulationId) {
    spawn::create_entity(
        app.world_mut(),
        type_name,
        pos(x, y),
        Some(player),
        FieldReach::Initial,
        Arrival::Rising {
            attendance,
            origin: RiseOrigin::Founded {
                payer: Some(player),
            },
        },
    )
    .unwrap_or_else(|| panic!("{type_name} fits at ({x}, {y})"))
}

/// A fixture entity of `type_name` at `(x, y)` owned by `player`, still
/// rising with no work put in, advancing itself on its own behalf.
pub fn create_unattended_rising(
    app: &mut App,
    type_name: &str,
    x: u32,
    y: u32,
    player: PlayerId,
) -> (Entity, SimulationId) {
    let (entity, id) = create_rising(app, type_name, x, y, player, RiseAttendance::Halted);
    app.world_mut()
        .get_mut::<RisingComponent>(entity)
        .expect("a rising entity carries its rising state")
        .attendance = RiseAttendance::Unattended { founder: id };
    (entity, id)
}

/// A fixture entity of `type_name` at `(x, y)` owned by `player`, still
/// rising with no work put in, worked by a crew nobody has joined yet.
pub fn create_crewed_rising(
    app: &mut App,
    type_name: &str,
    x: u32,
    y: u32,
    player: PlayerId,
) -> (Entity, SimulationId) {
    create_rising(
        app,
        type_name,
        x,
        y,
        player,
        RiseAttendance::Crew {
            workers: Default::default(),
        },
    )
}

/// One bequest of `entity_type`, handed on by any death that ends a life —
/// what most fixtures mean by "leaves a body".
pub fn leaves(entity_type: &str) -> Vec<Bequest> {
    vec![Bequest::new(entity_type, 1, LeftBy::Ordinary)]
}

/// Whether `player` covers the cell in `field`.
pub fn covered_by(app: &App, field: FieldId, x: u32, y: u32, player: PlayerId) -> bool {
    app.world()
        .resource::<FieldGrid>()
        .covered(field, CellPos::new(x, y))
        .contains(player)
}

/// Spawns `type_name` as the map would place it, its field at full reach.
pub fn place(app: &mut App, type_name: &str, x: u32, y: u32, player: PlayerId) -> Entity {
    spawn::spawn_placed(
        app.world_mut(),
        type_name,
        pos(x, y),
        Some(player),
        FieldReach::Full,
        &[],
    )
    .unwrap_or_else(|| panic!("{type_name} fits at ({x}, {y})"))
    .0
}

/// Removes the entity the way a mined-out node goes: no loss, no kill.
pub fn deplete(app: &mut App, entity: Entity) {
    spawn::despawn_entity(app.world_mut(), entity, DeathCause::Depleted);
}

pub fn push_command(app: &mut App, command: PlayerCommand) {
    app.world_mut().resource_mut::<PendingInput>().push(command);
}

/// Selects `id` for the local player, replacing the current selection — the
/// setup most order suites need before issuing a command.
pub fn select(app: &mut App, id: SimulationId) {
    push_command(
        app,
        PlayerCommand::SelectById {
            id,
            mode: SelectMode::Replace,
        },
    );
}

/// Ticks needed for a queued command to reach the simulation (see `SYNC_LATENCY`).
pub const APPLY: u32 = 3;

/// The local player's (player 0) current selection.
pub fn selection(app: &App) -> Vec<SimulationId> {
    app.world().resource::<Selection>().get(0).to_vec()
}

/// Two-player app for the selection and control-group suites: an armed unit, a
/// one-hit-kill `critter` (for group pruning), and a tagged `keep` building.
pub fn selection_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(1, [])
                .with_attack(weapon(GROUND), 10, 1, 3, 2, 1)
                .with_sight_range(5),
        );
        registry.register(
            EntityTypeDef::new("critter")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(1))
                .with_dying(1, []),
        );
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_tags(["building"]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App on the cell model with a walking 1×1 `soldier` and a parked 2×2
/// `wagon`. One human player, session started.
pub fn cell_crowd_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("wagon")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, []),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App with the form-change roster on the given movement model — a `whelp`
/// (1×1 mover, 30 hp) declaring four transitions: growing into the same-layer
/// 3×3 `giant`, the same growth into the 3×3 `brute` priced at 10 gold, an
/// instant committed change into the 10-hp `husk` paid in blood, and a timed
/// change into the poolless `wisp` (which changes back);
/// plus a `shrine` (2×2 building) that unroots into the same-size `golem`
/// mover. One human player, session started.
pub fn morph_app(model: MovementModel) -> App {
    morph_app_for(
        vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)],
        model,
    )
}

/// [`morph_app`] seated on `slots`.
pub fn morph_app_for(slots: Vec<PlayerSlot>, model: MovementModel) -> App {
    let mut app = make_app(slots);
    install_map(&mut app, Projection::Isometric, model);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register(
            EntityTypeDef::new("whelp")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_train_time(20)
                .with_morphs([
                    MorphTransition::new(
                        "giant",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(10),
                        MorphPlacement::Reserve,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        Vec::new(),
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    // The giant's growth, priced, into the brute.
                    MorphTransition::new(
                        "brute",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(10),
                        MorphPlacement::Reserve,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "boulder",
                        MorphCourse::direct(MorphInterrupted::Dies),
                        Quantity::Constant(0),
                        MorphPlacement::Revalidate,
                        MorphCancel::Committed,
                        MorphReason::Change,
                        Vec::new(),
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "ogre",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(10),
                        MorphPlacement::Reserve,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        Vec::new(),
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "husk",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(0),
                        MorphPlacement::Revalidate,
                        MorphCancel::Committed,
                        MorphReason::Change,
                        vec![Cost::Health(FixedU64::from_num(10))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "wisp",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(10),
                        MorphPlacement::Revalidate,
                        MorphCancel::Forfeit,
                        MorphReason::Change,
                        Vec::new(),
                        Vec::new(),
                        [],
                    ),
                    // Worn as a chrysalis on the way, paid, and refunded if
                    // the change ends early.
                    MorphTransition::new(
                        "wyrm",
                        MorphCourse::via(
                            "chrysalis",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::reverts([(
                                PoolId::HEALTH,
                                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                            )]),
                        ),
                        Quantity::Constant(10),
                        MorphPlacement::Revalidate,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                ]),
        );
        registry.register(
            EntityTypeDef::new("chrysalis")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("wyrm")
                .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(90))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("brute")
                .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.3"),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("giant")
                .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("boulder")
                .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("ogre")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("husk")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(10))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("wisp")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_morphs([MorphTransition::new(
                    "whelp",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Forfeit,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [],
                )]),
        );
        registry.register(
            EntityTypeDef::new("shrine")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_tags(["building"])
                // Trains the whelp, and its unrooted form does not: the one
                // role whose live state is pre-paid, so what happens to the
                // queue across the change is a contract worth pinning.
                .with_trainer(["whelp"])
                .with_morphs([MorphTransition::new(
                    "golem",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(10),
                    MorphPlacement::Reserve,
                    MorphCancel::Refundable,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        registry.register(
            EntityTypeDef::new("golem")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(50))
                .with_dying(2, [])
                .with_morphs([MorphTransition::new(
                    "shrine",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(10),
                    MorphPlacement::Reserve,
                    MorphCancel::Refundable,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// The brood group every breeder of [`brood_app`] offers: five points along
/// the row just below a 3×3 footprint, seating `slots` at once.
fn brood_berths(slots: usize) -> [(&'static str, BerthGroup); 1] {
    [(
        "brood",
        BerthGroup::new(
            [
                berth("0.5", "3.5"),
                berth("1.0", "3.5"),
                berth("1.5", "3.5"),
                berth("2.0", "3.5"),
                berth("2.5", "3.5"),
            ],
            slots,
        ),
    )]
}

/// A 3×3 building of [`brood_app`] with 300 health, providing 3 supply.
fn brood_building(name: &str) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
        .with_pool(Pool::health(300))
        .with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::from_num(3))
        .with_dying(2, [])
        .with_tags(["building"])
}

/// A 1×1 solid mover of [`brood_app`] with the given health.
fn brood_mover(name: &str, max_health: u32) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
        .with_movement(
            FixedU64::from_num(0.5),
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
        .with_pool(Pool::health(max_health))
        .with_dying(2, [])
}

/// A free, timed, refundable transition of [`brood_app`] into `into` after
/// 10 ticks, its ground judged again at the landing.
fn brood_change(into: &str) -> MorphTransition {
    MorphTransition::new(
        into,
        MorphCourse::direct(MorphInterrupted::Reverts),
        Quantity::Constant(10),
        MorphPlacement::Revalidate,
        MorphCancel::Refundable,
        MorphReason::Change,
        Vec::new(),
        Vec::new(),
        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
    )
}

/// Creates an app for the brood suite: breeders of a 3×3 footprint with five
/// brood berths along their southern foot, and what they bear.
///
/// - `hatch` seats four, breeds a `grub` every 10 ticks up to three, and its
///   grubs perish with it; it changes into `great_hatch` (opens with two)
///   through a `shell` (which seats but breeds nothing), into `bare_hatch`
///   (no berths) and into `tight_hatch` (two seats, limit two). `ready_hatch` is a hatch owing two
///   grubs when it stands, built by a `digger` in 20 ticks; `stat_hatch`
///   reads its period from the `brood_period` stat, 6. `rug_hatch` breeds as
///   a hatch and changes for 10 gold, refunded when called off, into a
///   `great_hatch` through a `rug_shell` lying underfoot (which seats but
///   breeds nothing).
/// - `grub` is passable, roams, and grows for 10 gold into a
///   `worker` (1 supply, counted as production) or, in 20 ticks, a `brute`
///   (2 supply, needs a `den`) inside a solid `egg`, both returning it when the growth ends
///   early, into a `flit` that dies instead, into a 3×3 `bulk` (or a
///   `hulk`, which dies when interrupted), or, reserving its ground, into a
///   3×3 standing `mound`.
/// - `linger_pen` seats two and breeds `piglet`s, which linger where they
///   are set down; it changes into `bare_pen` (no berths), as `inner_pen`
///   does, whose two berths lie on its middle cell. `reseat_pen` breeds
///   piglets it takes in from within three cells, and changes into `roomy_pen`
///   (the same seats and terms) through a seatless `pen_shell`. The great
///   hatch changes back into a `hatch`.
/// - `cart` is a hold; `sow` rides and breeds `hog`s into two seats.
/// - `delver` breeds an orbiting `mite` into two seats and builds a
///   `ready_hatch`, harvests a gold `lode` and mends from inside, banking at
///   a `larder`; `nurse` breeds a mite and mends seated in its patient's
///   brood berths.
/// - Buildings take two ticks to die, grubs one, movers two.
///
/// One human player, session started.
pub fn brood_app(model: MovementModel) -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    install_map(&mut app, Projection::Isometric, model);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        let brood_period = registry.register_entity_stat("brood_period", FixedU64::ONE);

        registry.register(
            brood_building("hatch")
                .with_berths(brood_berths(4))
                .with_breeder("grub", Quantity::Constant(10), 3, 0, OrphanFate::Perish)
                .with_morphs([
                    MorphTransition::new(
                        "great_hatch",
                        MorphCourse::via(
                            "shell",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::reverts([(
                                PoolId::HEALTH,
                                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                            )]),
                        ),
                        Quantity::Constant(10),
                        MorphPlacement::Revalidate,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        Vec::new(),
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    brood_change("bare_hatch"),
                    brood_change("tight_hatch"),
                ]),
        );
        registry.register(brood_building("shell").with_berths(brood_berths(4)));
        registry.register(
            brood_building("rug_hatch")
                .with_berths(brood_berths(4))
                .with_breeder("grub", Quantity::Constant(10), 3, 0, OrphanFate::Perish)
                .with_morphs([MorphTransition::new(
                    "great_hatch",
                    MorphCourse::via(
                        "rug_shell",
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                        ViaInterrupted::reverts([(
                            PoolId::HEALTH,
                            RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                        )]),
                    ),
                    Quantity::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Refundable,
                    MorphReason::Change,
                    vec![Cost::Resources(price::from([("gold", 10)]))],
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        registry.register(
            brood_building("rug_shell")
                .with_location(
                    GROUND_DOMAIN,
                    CellSize::new(3, 3),
                    Solidity::Underfoot,
                    GROUND,
                )
                .with_berths(brood_berths(4)),
        );
        registry.register(
            brood_building("great_hatch")
                .with_berths(brood_berths(4))
                .with_breeder("grub", Quantity::Constant(10), 3, 2, OrphanFate::Perish)
                .with_morphs([brood_change("hatch")]),
        );
        registry.register(brood_building("bare_hatch"));
        registry.register(
            brood_building("tight_hatch")
                .with_berths(brood_berths(2))
                .with_breeder("grub", Quantity::Constant(10), 2, 0, OrphanFate::Perish),
        );
        registry.register(
            brood_building("ready_hatch")
                .with_rise(20, rise_initial_pools(&[PoolId::HEALTH]))
                .with_berths(brood_berths(4))
                .with_breeder("grub", Quantity::Constant(10), 3, 2, OrphanFate::Perish),
        );
        registry.register(
            brood_building("stat_hatch")
                .with_stat(brood_period, FixedU64::from_num(6))
                .with_berths(brood_berths(4))
                .with_breeder(
                    "grub",
                    Quantity::Stat(brood_period),
                    3,
                    0,
                    OrphanFate::Perish,
                ),
        );
        registry.register(
            brood_building("linger_pen")
                .with_berths(brood_berths(2))
                .with_breeder(
                    "piglet",
                    Quantity::Constant(10),
                    2,
                    0,
                    OrphanFate::Linger(Lingering::Stay),
                )
                .with_morphs([brood_change("bare_pen")]),
        );
        registry.register(
            brood_building("inner_pen")
                .with_berths([(
                    "brood",
                    BerthGroup::new([berth("1.5", "1.5"), berth("1.5", "1.75")], 2),
                )])
                .with_breeder(
                    "piglet",
                    Quantity::Constant(10),
                    2,
                    0,
                    OrphanFate::Linger(Lingering::Stay),
                )
                .with_morphs([brood_change("bare_pen")]),
        );
        registry.register(brood_building("bare_pen"));
        let reseat = OrphanFate::Linger(Lingering::Reseat { distance: 3 });
        registry.register(
            brood_building("reseat_pen")
                .with_berths(brood_berths(2))
                .with_breeder("piglet", Quantity::Constant(10), 2, 0, reseat)
                .with_morphs([MorphTransition::new(
                    "roomy_pen",
                    MorphCourse::via(
                        "pen_shell",
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                        ViaInterrupted::reverts([(
                            PoolId::HEALTH,
                            RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                        )]),
                    ),
                    Quantity::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Refundable,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        registry.register(brood_building("pen_shell"));
        registry.register(
            brood_building("roomy_pen")
                .with_berths(brood_berths(2))
                .with_breeder("piglet", Quantity::Constant(10), 2, 0, reseat),
        );

        registry.register(
            EntityTypeDef::new("grub")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Passable, GROUND)
                .with_pool(Pool::health(25))
                .with_dying(1, [])
                .with_selection(1, None)
                .with_broodling(Attachment::new(
                    "brood",
                    BerthStance::Roaming {
                        speed: FixedU64::from_num(0.1),
                        dwell: 5,
                    },
                ))
                .with_morphs([
                    MorphTransition::new(
                        "worker",
                        MorphCourse::via(
                            "egg",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::reverts([(
                                PoolId::HEALTH,
                                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                            )]),
                        ),
                        Quantity::Constant(10),
                        MorphPlacement::Nearby,
                        MorphCancel::Refundable,
                        MorphReason::Production,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "brute",
                        MorphCourse::via(
                            "egg",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::reverts([(
                                PoolId::HEALTH,
                                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                            )]),
                        ),
                        Quantity::Constant(20),
                        MorphPlacement::Nearby,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        [Requirement::EntityType("den".to_string())],
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "mound",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(10),
                        MorphPlacement::Reserve,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "flit",
                        MorphCourse::via(
                            "egg",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::Dies,
                        ),
                        Quantity::Constant(10),
                        MorphPlacement::Nearby,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "bulk",
                        MorphCourse::via(
                            "egg",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::reverts([(
                                PoolId::HEALTH,
                                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                            )]),
                        ),
                        Quantity::Constant(10),
                        MorphPlacement::Nearby,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                    MorphTransition::new(
                        "hulk",
                        MorphCourse::via(
                            "egg",
                            [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                            ViaInterrupted::Dies,
                        ),
                        Quantity::Constant(10),
                        MorphPlacement::Nearby,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Resources(price::from([("gold", 10)]))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                ]),
        );
        for name in ["bulk", "hulk"] {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                    .with_movement(
                        FixedU64::from_num(0.3),
                        FixedU64::ONE,
                        FixedU64::ONE,
                        FixedU64::from_num(360),
                        FixedU64::from_num(360),
                    )
                    .with_pool(Pool::health(90))
                    .with_dying(2, []),
            );
        }
        registry.register(
            EntityTypeDef::new("egg")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(1, []),
        );
        registry.register(
            brood_mover("worker", 30).with_stat(EntityStatId::SUPPLY_COST, FixedU64::ONE),
        );
        registry.register(
            brood_mover("brute", 60).with_stat(EntityStatId::SUPPLY_COST, FixedU64::from_num(2)),
        );
        registry.register(brood_mover("flit", 20));
        registry.register(
            EntityTypeDef::new("mound")
                .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, []),
        );
        // A delayed cast on itself, for a cast held on a stranded broodling.
        let squeal = registry.register_skill(
            "squeal",
            SkillDef {
                cooldown: 10,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    casting: Casting::Delayed {
                        point: Quantity::Constant(6),
                        period: Quantity::Constant(8),
                        pays: PaymentPoint::AtLanding,
                    },
                    effect: EntityCastEffect::Damage {
                        amount: FixedU64::from_num(5),
                        on: Recipient::Caster,
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(
            EntityTypeDef::new("piglet")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_skills([squeal])
                .with_broodling(Attachment::new("brood", BerthStance::Still))
                .with_morphs([MorphTransition::new(
                    "worker",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Refundable,
                    MorphReason::Change,
                    vec![Cost::Resources(price::from([("gold", 10)]))],
                    vec![],
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        registry.register(
            EntityTypeDef::new("den")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_tags(["building"]),
        );
        registry.register(
            brood_mover("digger", 20)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["ready_hatch"],
                    BuilderAttendance::Crew(WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    }),
                ),
        );
        // A hold and a breeder that rides: one seating a brood boards nothing.
        registry.register(
            EntityTypeDef::new("cart")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_dying(2, [])
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
                .with_transporter(
                    Kinds::Any,
                    Affiliation::Own,
                    PassengerFate::Eject,
                    PassengerConduct::Shelter,
                ),
        );
        registry.register(
            brood_mover("sow", 40)
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
                .with_berths(brood_berths(2))
                .with_breeder("hog", Quantity::Constant(10), 2, 0, OrphanFate::Perish),
        );
        registry.register(
            EntityTypeDef::new("hog")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_broodling(Attachment::new("brood", BerthStance::Still)),
        );
        // Breeders that work from inside, or seated in a berth: one seating a
        // brood is refused the work, or gives it up at the job.
        let hidden = WorkPresence::Hidden {
            crew: CrewLimit::ONE,
        };
        registry.register(
            brood_mover("delver", 40)
                .with_sight_range(40)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_berths(brood_berths(2))
                .with_breeder("mite", Quantity::Constant(10), 1, 0, OrphanFate::Perish)
                .with_builder(["ready_hatch"], BuilderAttendance::Crew(hidden.clone()))
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(5, 5, 20, hidden.clone(), Banking::Direct, Kinds::Any),
                )])
                .with_repairer(
                    Kinds::Any,
                    RepairRate::PerTick(FixedU64::ONE),
                    hidden,
                    false,
                    RepairCost::Free,
                    None,
                ),
        );
        registry.register(
            brood_mover("nurse", 40)
                .with_sight_range(40)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_berths(brood_berths(2))
                .with_breeder("mite", Quantity::Constant(10), 1, 0, OrphanFate::Perish)
                .with_repairer(
                    Kinds::Any,
                    RepairRate::PerTick(FixedU64::ONE),
                    WorkPresence::Attached(Attachment::new("brood", BerthStance::Still)),
                    false,
                    RepairCost::Free,
                    None,
                ),
        );
        registry.register(
            EntityTypeDef::new("mite")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_broodling(Attachment::new(
                    "brood",
                    BerthStance::Orbit {
                        radius: fixed("0.25"),
                        period: 4,
                    },
                )),
        );
        registry.register(
            EntityTypeDef::new("lode")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_resource_source("gold", DepletionPolicy::Persist),
        );
        registry.register(
            EntityTypeDef::new("larder")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_resource_storage(["gold"]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// A berth point from decimal strings, in cells from the footprint's anchor.
pub fn berth(x: &str, y: &str) -> FixedVec2 {
    FixedVec2::new(signed_fixed(x), signed_fixed(y))
}

/// A 3×3 `core_vein` of gold on the ground, gone once drained, seating one
/// carrier at its middle cell.
pub fn core_vein() -> EntityTypeDef {
    EntityTypeDef::new("core_vein")
        .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
        .with_resource_source("gold", DepletionPolicy::Destroy)
        .with_berths([("rim", BerthGroup::new([berth("1.5", "1.5")], 1))])
}

/// Runs exactly `ticks` fixed updates.
///
/// The simulation is deterministic, so tests advance by a known tick count and
/// then assert the resulting state, rather than polling for a condition.
///
/// Non-local players have no network peer in tests, so an idle frame is recorded
/// for them every tick — otherwise lockstep would block waiting for their commands.
pub fn run_ticks(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        let world = app.world_mut();

        let (current_tick, local_player, players) = {
            let session = world.resource::<GameSession>();
            let players: Vec<_> = session.slots().iter().map(|slot| slot.id()).collect();
            (session.tick(), session.local_player(), players)
        };
        for player in players {
            if Some(player) != local_player {
                world
                    .resource_mut::<InputFrames>()
                    .push_frame(PlayerFrame::idle(player, current_tick));
            }
        }

        // The whole fixed step, not just `FixedUpdate`: the closing phases are
        // where a completed tick is recorded, tallied and retired, and a suite
        // that skipped them would not exercise anything a game hangs there.
        world.run_schedule(FixedMain);
    }
}

/// Runs `ticks` fixed updates feeding idle frames the way [`run_ticks`]
/// does, except that `player`'s frame at tick `at` carries `commands` — the
/// one way to issue commands as a non-local player, whose input never flows
/// through `PendingInput`.
pub fn run_ticks_commanding(
    app: &mut App,
    ticks: u32,
    player: PlayerId,
    at: u32,
    commands: Vec<PlayerCommand>,
) {
    let mut commands = Some(commands);
    for _ in 0..ticks {
        let world = app.world_mut();
        let (current_tick, local_player, players) = {
            let session = world.resource::<GameSession>();
            let players: Vec<PlayerId> = session.slots().iter().map(|slot| slot.id()).collect();
            (session.tick(), session.local_player(), players)
        };
        for other in players {
            if Some(other) == local_player {
                continue;
            }
            let frame = match commands.take_if(|_| other == player && current_tick == at) {
                Some(commands) => PlayerFrame {
                    player,
                    tick: current_tick,
                    commands,
                },
                None => PlayerFrame::idle(other, current_tick),
            };
            world.resource_mut::<InputFrames>().push_frame(frame);
        }
        world.run_schedule(FixedUpdate);
    }
}

/// Runs exactly `steps` fixed updates without synthesizing any input frames —
/// for suites whose registered frame sources already feed every slot.
pub fn run_steps(app: &mut App, steps: u32) {
    for _ in 0..steps {
        app.world_mut().run_schedule(FixedMain);
    }
}

pub fn order_queue_is_empty(world: &mut World, entity: Entity) -> bool {
    world
        .get::<OrderQueueComponent>(entity)
        .is_some_and(|q| q.front().is_none())
}

/// Force-cancels everything `entity` is doing, standing in for what takes an
/// order away rather than calls it off: a death, a field switching the entity
/// off, a transport pulling it aboard. Nothing is paid back.
///
/// Not routed through a command, because the ones that force a queue reach it
/// through the entity's own fate, and a worker off the map cannot be selected —
/// a hidden builder or a carrier down a mine is exactly what these suites need
/// to stop.
pub fn force_cancel_orders(world: &mut World, entity: Entity) {
    world
        .get_mut::<OrderQueueComponent>(entity)
        .expect("simulation entities carry an order queue")
        .cancel_all(CancelPolicy::Force);
}

/// Softly cancels everything `entity` is doing, standing in for the player
/// calling it off: each order answers for itself — a refundable change of form
/// gives its price back, and nothing else pays anything.
///
/// Not routed through `PlayerCommand::Stop`, because that reaches the selection
/// and a worker off the map cannot be selected.
pub fn soft_cancel_orders(world: &mut World, entity: Entity) {
    world
        .get_mut::<OrderQueueComponent>(entity)
        .expect("simulation entities carry an order queue")
        .cancel_all(CancelPolicy::Soft);
}

/// Pushes a Morph order into `type_name` onto the entity's queue.
pub fn order_morph(app: &mut App, entity: Entity, type_name: &str) {
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<OrderQueueComponent>()
        .expect("simulation entities carry an order queue")
        .push(
            Order::Morph {
                type_name: type_name.to_string(),
            },
            None,
        );
}

/// Pushes a Move order to `(x, y)` onto the entity's queue, behind whatever
/// it is doing.
pub fn order_move(app: &mut App, entity: Entity, x: u32, y: u32) {
    order_move_onto(app, entity, pos(x, y));
}

/// Gives `entity` a Move order onto the exact point `target`, as a click on
/// open ground does, and runs until it stands there, within `limit` ticks.
///
/// Panics if it does not get there in time.
pub fn walk_onto(app: &mut App, entity: Entity, target: FixedUVec2, limit: u32) {
    order_move_onto(app, entity, target);
    for _ in 0..limit {
        if position_of(app.world(), entity) == target {
            return;
        }
        run_ticks(app, 1);
    }
    assert_eq!(position_of(app.world(), entity), target, "the walk arrives");
}

/// Pushes a Move order onto the exact point `target` onto `entity`'s queue,
/// behind whatever it is doing.
pub fn order_move_onto(app: &mut App, entity: Entity, target: FixedUVec2) {
    app.world_mut()
        .get_mut::<OrderQueueComponent>(entity)
        .expect("a walker takes orders")
        .push(
            Order::Move {
                target,
                size: CellSize::ONE,
                range: 0,
            },
            None,
        );
}

/// Orders `trainer` to train one settler of [`supply_app`].
pub fn train_settler(app: &mut App, trainer: SimulationId) {
    push_command(
        app,
        PlayerCommand::TrainEntity {
            trainer,
            type_name: "settler".into(),
        },
    );
}

/// Asserts `entity` has been despawned: looking it up fails specifically because
/// its id is now invalid (its slot was freed and the generation bumped), naming
/// exactly that entity — not merely with some error.
pub fn assert_despawned(world: &mut World, entity: Entity) {
    match world.get_entity(entity) {
        Ok(_) => panic!("expected {entity:?} to be despawned, but it is still alive"),
        Err(EntityNotSpawnedError::Invalid(error)) => assert_eq!(error.entity, entity),
        Err(other) => panic!("expected {entity:?} despawned (invalid id), got {other:?}"),
    }
}

/// The cell the entity currently stands on.
pub fn cell_of(world: &World, entity: Entity) -> CellPos {
    CellPos::from(position_of(world, entity))
}

/// The entity's continuous position — sub-cell precise, where [`cell_of`]
/// floors it to a cell.
pub fn position_of(world: &World, entity: Entity) -> FixedUVec2 {
    world.get::<LocationComponent>(entity).unwrap().position
}

/// Which way the entity's body points. A gun the body carries keeps a bearing of
/// its own, which this is not.
pub fn facing_of(world: &World, entity: Entity) -> Facing {
    world.get::<LocationComponent>(entity).unwrap().facing
}

/// Where the entity's first mounted gun is trained.
pub fn bearing_of(world: &World, entity: Entity) -> Facing {
    world
        .get::<TurretsComponent>(entity)
        .expect("a turreted entity carries the bearings its guns are trained at")
        .0[0]
        .bearing
}

/// Marks `cell` of the ground layer statically held.
pub fn hold_cell(app: &mut App, cell: CellPos) {
    app.world_mut()
        .resource_mut::<Map>()
        .set_static_occupied(GROUND, cell, true);
}

/// Frees `cell` of the ground layer, statically held before.
pub fn free_cell(app: &mut App, cell: CellPos) {
    app.world_mut()
        .resource_mut::<Map>()
        .set_static_occupied(GROUND, cell, false);
}

/// Marks or clears every cell of the map's ground layer, used to box a worker in.
/// Cells already in the desired state — a standing building's — stay untouched,
/// since a static write must flip its cell.
pub fn set_all_cells_statically_occupied(world: &mut World, occupied: bool) {
    let mut map = world.resource_mut::<Map>();
    let (width, height) = (map.width(), map.height());
    for y in 0..height {
        for x in 0..width {
            let cell = CellPos::new(x, y);
            if map.nav_grid().is_statically_occupied_by(GROUND, cell) != occupied {
                map.set_static_occupied(GROUND, cell, occupied);
            }
        }
    }
}

/// Asserts `worker` is boxed in — hidden with its reveal queued — then frees `cell`
/// and checks the scheduled retry brings it back onto exactly that cell, dropping
/// both markers.
pub fn assert_reveal_deferred_then_lands_on(app: &mut App, worker: Entity, cell: CellPos) {
    assert!(
        entity_def::hidden(app.world(), worker),
        "a boxed-in worker stays off the map"
    );
    assert!(
        entity_def::pending_reveal(app.world(), worker),
        "with its reveal queued rather than forced"
    );

    app.world_mut()
        .resource_mut::<Map>()
        .set_static_occupied(GROUND, cell, false);
    run_ticks(app, 1);

    assert!(!entity_def::hidden(app.world(), worker));
    assert!(!entity_def::pending_reveal(app.world(), worker));
    assert_eq!(cell_of(app.world_mut(), worker), cell);
}

/// The entities of the given content type owned by `player`.
pub fn owned_of_type(world: &mut World, type_name: &str, player: PlayerId) -> Vec<Entity> {
    world
        .query::<(Entity, &EntityInfoComponent, &OwnerComponent)>()
        .iter(world)
        .filter(|(_, info, owner)| info.type_name() == type_name && owner.player() == player)
        .map(|(entity, ..)| entity)
        .collect()
}

/// The single entity of the given content type owned by `player`.
///
/// Panics when there is not exactly one.
pub fn single_owned_of_type(world: &mut World, type_name: &str, player: PlayerId) -> Entity {
    let entities = owned_of_type(world, type_name, player);
    assert_eq!(
        entities.len(),
        1,
        "expected exactly one {type_name} owned by player {player}"
    );
    entities[0]
}

/// Whether two entities stand within `distance` of each other, measured the
/// way the map itself measures — under its projection.
pub fn within(world: &mut World, a: Entity, b: Entity, distance: u32) -> bool {
    let (cell_a, cell_b) = (cell_of(world, a), cell_of(world, b));
    world
        .resource::<Map>()
        .projection()
        .in_range(cell_a, cell_b, distance)
}

/// Asserts `unit` stands beside `building`'s footprint: within one cell of
/// it, and not on it.
pub fn assert_adjacent_to_footprint(world: &mut World, unit: Entity, building: Entity) {
    let origin = cell_of(world, building);
    let size = entity_def::of(world, building).location.unwrap().size();
    let footprint = CellRect::new(origin, size);
    let unit_cell = cell_of(world, unit);
    assert!(
        world
            .resource::<Map>()
            .projection()
            .in_range_of_rect(unit_cell, footprint, 1)
            && !footprint.contains(unit_cell),
        "expected {unit_cell:?} beside the footprint at {origin:?}, not on it"
    );
}

/// Counts the entities of the given content type in the world.
pub fn count_of_type(world: &mut World, type_name: &str) -> usize {
    world
        .query::<&EntityInfoComponent>()
        .iter(world)
        .filter(|info| info.type_name() == type_name)
        .count()
}

/// Player 0's stockpile of gold.
pub fn gold(world: &World) -> u32 {
    world.resource::<PlayerResources>().amount(0, "gold")
}

/// Player 0's stockpile of wood.
pub fn wood(world: &World) -> u32 {
    world.resource::<PlayerResources>().amount(0, "wood")
}

/// Grants `amount` gold to player 0's stockpile.
pub fn grant_gold(app: &mut App, amount: u32) {
    app.world_mut()
        .resource_mut::<PlayerResources>()
        .add(0, "gold", amount);
}

/// The entity's exact remaining health, unrounded.
pub fn health(app: &App, entity: Entity) -> FixedU64 {
    entity_def::pool_value(app.world(), entity, PoolId::HEALTH)
        .expect("the entity carries a health pool")
}

/// The entity's remaining health as a whole number. Panics when the entity
/// has no health pool or its health is not whole.
pub fn health_as_u32(app: &App, entity: Entity) -> u32 {
    pool_as_u32(app, entity, PoolId::HEALTH)
}

/// The entity's remaining energy as a whole number. Panics when the entity
/// has no energy pool or its energy is not whole.
pub fn energy_as_u32(app: &App, entity: Entity) -> u32 {
    pool_as_u32(app, entity, PoolId::ENERGY)
}

/// What the entity's `pool` holds, as a whole number. Panics when the entity
/// has no such pool or its value is not whole.
fn pool_as_u32(app: &App, entity: Entity, pool: PoolId) -> u32 {
    let value = entity_def::pool_value(app.world(), entity, pool)
        .unwrap_or_else(|| panic!("the entity carries the {pool:?} pool"));
    assert!(
        value.frac() == FixedU64::ZERO,
        "the entity's {pool:?} {value} is a whole number"
    );
    value.to_num::<u32>()
}

/// Applies the buff `id` to `entity`, as a fixture whose buff fits its
/// target. Panics when `entity` cannot carry it.
pub fn apply_buff(world: &mut World, entity: Entity, id: EntityBuffId) {
    match buffs::apply_entity_buff(world, entity, id) {
        Bearing::Borne => {}
        Bearing::Uncarried => panic!("apply_buff is given a buff its target carries"),
    }
}

/// Switches `entity` off with a `dazed` buff lasting `ticks`, registered for
/// the purpose.
pub fn disable_for(app: &mut App, entity: Entity, ticks: u32) {
    let dazed = app
        .world_mut()
        .resource_mut::<ContentRegistry>()
        .register_entity_buff(
            "dazed",
            EntityBuffDef {
                effects: vec![EntityEffect::Disable],
                lasting: Lasting::For(ticks),
                stack_rule: StackRule::Refresh,
                interrupted_by: Vec::new(),
            },
        );
    apply_buff(app.world_mut(), entity, dazed);
}

/// Removes `amount` health points directly, standing in for damage taken.
pub fn wound(app: &mut App, entity: Entity, amount: &str) {
    pools::drain(app.world_mut(), entity, PoolId::HEALTH, fixed(amount));
}

/// Selects `attacker` for the local player and orders it to attack `target`,
/// flushing whatever it was doing.
pub fn attack(app: &mut App, attacker: SimulationId, target: SimulationId) {
    select(app, attacker);
    push_command(
        app,
        PlayerCommand::Attack {
            target: AttackTarget::Entity(target),
            flush: true,
        },
    );
}

/// Registers a single-modifier entity buff, refreshing on re-application:
/// `stat` moved by `magnitude` per `op`, for `duration` ticks (`None` is
/// permanent). On a pool's maximum it clamps the pool.
pub fn register_entity_buff(
    app: &mut App,
    name: &str,
    stat: EntityStatId,
    op: ModifierOp,
    magnitude: &str,
    duration: Option<u32>,
) -> EntityBuffId {
    let modifiers = vec![EntityModifier {
        stat,
        op,
        magnitude: signed_fixed(magnitude),
    }];
    let modifiers = match app.world().resource::<ContentRegistry>().pool_role_of(stat) {
        Some((_, PoolRole::Maximum)) => EntityModifiers::PoolMaximums {
            modifiers,
            pool_shift: PoolShift::Clamp,
        },
        Some((_, PoolRole::Regen | PoolRole::Drain)) | None => EntityModifiers::Stats(modifiers),
    };
    app.world_mut()
        .resource_mut::<ContentRegistry>()
        .register_entity_buff(
            name,
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(modifiers)],
                lasting: match duration {
                    Some(ticks) => Lasting::For(ticks),
                    None => Lasting::Forever,
                },
                stack_rule: StackRule::Refresh,
                interrupted_by: Vec::new(),
            },
        )
}

/// The entity's damage stat after the tick's modifier fold — what the buff and
/// skill suites compare before and after applying an effect.
pub fn effective_damage(app: &App, entity: Entity) -> FixedU64 {
    app.world()
        .get::<StatsComponent>(entity)
        .unwrap()
        .effective(EntityStatId::DAMAGE)
        .unwrap()
}

/// Starts the game on the test's content: builds the map's hierarchy for the
/// movers it registered, as a game's map is built, then starts the session.
pub fn start_game(app: &mut App) {
    serve_movers(app);
    ferrets_bevy_plugin::start_game(app.world_mut());
}

/// Swaps the harness map for a fresh 32×32 one with the given projection and
/// movement model, its hierarchy built for the registered movers. The fresh
/// grid holds none of the marks of bodies spawned before it.
pub fn install_map(app: &mut App, projection: Projection, model: MovementModel) {
    let mut grid = NavGrid::new(32, 32, |_| LAND);
    grid.add_flat_layer(GROUND, 1);
    map::install_map(
        app.world_mut(),
        Map::new("test", projection, model, grid, vec![]),
    );
    serve_movers(app);
}

/// Side of [`install_choke_point_map`]'s map. Several clusters across, so a walk
/// over it is planned as a corridor of real crossings; a map only a cluster or
/// two wide comes back as one flat segment however it is walled.
pub const CHOKE_POINT_SIZE: u32 = 96;

/// The rows left open in [`install_choke_point_map`]'s wall.
pub const CHOKE_POINT_GAP: std::ops::RangeInclusive<u32> = 8..=9;

/// Swaps the harness map for a cell-model one split by a wall with a single gap,
/// so crossing it can only be planned as a corridor through that gap. An open
/// field is planned as one flat segment and never changes legs, which is the
/// whole thing a corridor test needs to exercise. Call before any spawns.
pub fn install_choke_point_map(app: &mut App) {
    let mut grid = NavGrid::new(CHOKE_POINT_SIZE, CHOKE_POINT_SIZE, |_| LAND);
    grid.add_flat_layer(GROUND, 1);
    for y in 0..CHOKE_POINT_SIZE {
        if !CHOKE_POINT_GAP.contains(&y) {
            grid.set_occupied_by(GROUND, CellPos::new(CHOKE_POINT_SIZE / 2, y), true);
        }
    }
    map::install_map(
        app.world_mut(),
        Map::new(
            "test",
            Projection::Isometric,
            MovementModel::Cell,
            grid,
            vec![],
        ),
    );
    serve_movers(app);
}

/// App with the combat content roster — an attacking soldier (50 hp, 3-tick
/// dying phase) and an immobile dummy that leaves decaying bones — two human
/// players so a target can be owned and hostile, session started.
pub fn combat_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(8)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(50))
                .with_dying(3, [])
                .with_attack(weapon(GROUND), 10, 1, 1, 4, 2),
        );
        // A gun on a turret: it never moves and never turns, and what comes round
        // is the weapon, slowly, through a narrow arc. It notices further than it
        // shoots, so it starts coming round while a target is still closing.
        // Sight covers its notice: naming a target — by an order or by its own
        // hunting — first requires seeing it through the fog grid.
        let keep_gun = registry.register_turret(
            "keep_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("bastion")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(200))
                .with_sight_range(14)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(3))
                .with_stat(EntityStatId::ATTACK_ARC, FixedU64::from_num(60))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(30))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(8))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(12))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(4))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(1))
                .with_turrets([TurretMount::new(
                    keep_gun,
                    CellPos::new(0, 0),
                    CellSize::new(2, 2),
                )]),
        );
        // A gun on wheels: it walks like a unit and aims like a turret, which is
        // the one combination where a hull's heading and a gun's bearing must not
        // be the same value.
        let wagon_gun = registry.register_turret(
            "wagon_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("gun_wagon")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(40))
                .with_sight_range(10)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(30))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(8))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
                .with_turrets([TurretMount::new(
                    wagon_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        // The same gun on wheels, authored to fight while the wheels are under
        // orders: this is the one gun in the fixtures that does not stop to
        // shoot, so it is what firing on the move is read against.
        let rolling_gun = registry.register_turret(
            "rolling_gun_mount",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
                TurretStats::default(),
                WeaponConduct::OnTheMove,
            ),
        );
        registry.register(
            EntityTypeDef::new("rolling_gun")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(40))
                // Sight wider than the range it engages at, so what it engages —
                // and what it is ordered onto across the map — is something it
                // can see: naming a target reads the fog grid.
                .with_sight_range(14)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(30))
                .with_stat(EntityStatId::ATTACK_ARC, FixedU64::from_num(60))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(8))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
                .with_turrets([TurretMount::new(
                    rolling_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        // The same again, throwing a shell at where it aims rather than at what it
        // aims at — the one kind of weapon that can be pointed at bare ground, and
        // so the one that can be told to shoot a place while a gun is fighting on
        // the move.
        let lob = registry.register_projectile(
            "lob",
            ProjectileDef::new(FixedU64::from_num(2), Aim::Position),
        );
        let rolling_lob = registry.register_turret(
            "rolling_lob",
            TurretDef::new(
                Weapon::new(
                    GROUND,
                    Delivery::Projectile(lob),
                    Some(SplashDef::new(
                        SplashShape::Circular,
                        vec![(1, FixedU64::ONE)],
                        GROUND,
                        true,
                    )),
                    Slain::Remains,
                ),
                TurretStats::default(),
                WeaponConduct::OnTheMove,
            ),
        );
        registry.register(
            EntityTypeDef::new("rolling_mortar")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(40))
                .with_sight_range(10)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(360))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(6))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(8))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
                .with_turrets([TurretMount::new(
                    rolling_lob,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        // A keep with a gun at each corner, all reading the same numbers: what
        // several guns on one body do about several attackers is the question
        // they exist to answer. Its guns come round at once, so a test about
        // targets is not also a test about turning.
        let keeps: Vec<(&str, TurretFire)> = vec![
            ("spreading_keep", TurretFire::Spread),
            ("focused_keep", TurretFire::Focus),
        ];
        for (name, fire) in keeps {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(GROUND_DOMAIN, CellSize::new(5, 5), Solidity::Solid, GROUND)
                    .with_pool(Pool::health(300))
                    .with_sight_range(14)
                    .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(360))
                    .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
                    .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(8))
                    .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(10))
                    .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
                    .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
                    .with_turrets([
                        TurretMount::new(keep_gun, CellPos::new(0, 0), CellSize::new(2, 2)),
                        TurretMount::new(keep_gun, CellPos::new(3, 0), CellSize::new(2, 2)),
                        TurretMount::new(keep_gun, CellPos::new(0, 3), CellSize::new(2, 2)),
                        TurretMount::new(keep_gun, CellPos::new(3, 3), CellSize::new(2, 2)),
                    ])
                    .with_turret_fire(fire),
            );
        }
        register_air(&mut registry);
        // A gun that answers only what flies, for the body that carries one
        // alongside a weapon of its own.
        let flak = registry.register_turret(
            "flak",
            TurretDef::new(
                Weapon::new(AIR, Delivery::Instant, None, Slain::Remains),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        // Bodies that point a weapon and carry a gun as well — the fixtures where
        // both kinds fight at once. The gunship's gun answers the same ground its
        // own weapon does; the flak post's answers only the air its weapon cannot.
        for (name, gun) in [("gunship", keep_gun), ("flak_post", flak)] {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                    .with_pool(Pool::health(60))
                    .with_sight_range(10)
                    .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(360))
                    .with_attack(weapon(GROUND), 10, 4, 8, 6, 3)
                    .with_turrets([TurretMount::new(gun, CellPos::new(0, 0), CellSize::ONE)]),
            );
        }
        // Something that flies, answerable only on the air layer.
        registry.register(
            EntityTypeDef::new("kite")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Passable, GROUND)
                .with_targetable(AIR)
                .with_pool(Pool::health(30))
                .with_dying(3, []),
        );
        // A keep with one gun on its far corner, throwing something slow enough to
        // watch: where a shot leaves from is only visible while it is in the air.
        let corner_gun = registry.register_turret(
            "corner_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Projectile(lob), None, Slain::Remains),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("shell_keep")
                .with_location(GROUND_DOMAIN, CellSize::new(5, 5), Solidity::Solid, GROUND)
                .with_pool(Pool::health(300))
                .with_sight_range(14)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(360))
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(8))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
                .with_turrets([TurretMount::new(
                    corner_gun,
                    CellPos::new(3, 3),
                    CellSize::new(2, 2),
                )]),
        );
        // A body pointing a short spear beside a far-reaching gun: the gun reads
        // a range stat of its own, four times the spear's, so an order arriving
        // at the body's longest reach has not put its own weapon in range.
        let gun_range = registry.register_entity_stat("gun_range", FixedU64::ONE);
        let long_gun = registry.register_turret(
            "long_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
                TurretStats {
                    range: gun_range,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("longarm")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_sight_range(12)
                .with_attack(weapon(GROUND), 10, 2, 8, 6, 3)
                .with_stat(gun_range, FixedU64::from_num(8))
                .with_turrets([TurretMount::new(
                    long_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        // A body throwing at bodies beside a gun throwing at places, and the
        // mirror of it: an ordered bare cell binds only the weapon whose shots
        // are sent to one.
        registry.register(
            EntityTypeDef::new("bombardier")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_sight_range(12)
                .with_attack(weapon(GROUND), 10, 4, 8, 6, 3)
                .with_turrets([TurretMount::new(
                    corner_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        registry.register(
            EntityTypeDef::new("battery")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_sight_range(12)
                .with_attack(
                    AttackDef::new(Weapon::new(
                        GROUND,
                        Delivery::Projectile(lob),
                        None,
                        Slain::Remains,
                    )),
                    10,
                    4,
                    8,
                    6,
                    3,
                )
                .with_turrets([TurretMount::new(
                    keep_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        // A mover fighting only from a gun that names its own acquisition stat,
        // so the type legally declares no acquire_range: what an attack-move
        // stops for has to be asked of every weapon rather than of the body.
        let prowl_notice = registry.register_entity_stat("prowl_notice", FixedU64::ONE);
        let prowl_gun = registry.register_turret(
            "prowl_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
                TurretStats {
                    acquire_range: prowl_notice,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("prowler")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(40))
                .with_sight_range(10)
                .with_stat(EntityStatId::DAMAGE, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(4))
                .with_stat(prowl_notice, FixedU64::from_num(6))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(6))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(3))
                .with_turrets([TurretMount::new(
                    prowl_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        // A short spear under a long anti-air gun, on wheels: what an ordered
        // attack closes to has to be asked of the weapon that can serve the
        // target, or the long gun's reach would park the body out of the spear's.
        let anti_air_range = registry.register_entity_stat("anti_air_range", FixedU64::ONE);
        let anti_air_gun = registry.register_turret(
            "anti_air_gun",
            TurretDef::new(
                Weapon::new(AIR, Delivery::Instant, None, Slain::Remains),
                TurretStats {
                    range: anti_air_range,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("escort")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(40))
                .with_sight_range(12)
                .with_attack(weapon(GROUND), 10, 2, 8, 6, 3)
                .with_stat(anti_air_range, FixedU64::from_num(10))
                .with_turrets([TurretMount::new(
                    anti_air_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        // Something to shoot at that outlasts the shooting: four guns on one keep
        // kill a dummy before a test can look at what they were working.
        registry.register(
            EntityTypeDef::new("hulk")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(500))
                .with_dying(3, []),
        );
        // Registered before `dummy`, which leaves it as a body: tagged remains
        // and lying there for the lifetime it carries, like every corpse.
        registry.register(
            EntityTypeDef::new("bones")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Passable, GROUND)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(400)),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(20))
                .with_dying(3, leaves("bones")),
        );
        // A wide attacker, so a chase threads a 2x2 chaser footprint: reach
        // is rect to rect, measured from its nearest edge.
        registry.register(
            EntityTypeDef::new("ballista")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_sight_range(12)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(80))
                .with_dying(3, [])
                .with_attack(weapon(GROUND), 10, 2, 2, 4, 2),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();

    start_game(&mut app);
    app
}

/// App with the economy content roster — soldier, worker, lumberjack, barracks,
/// depot, the gold/wood sources, and a passable ghost — two human players,
/// session started. Used by the order suites that exercise production, harvest,
/// movement, follow, and command dispatch.
pub fn orders_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    register_orders_content(&mut app);
    start_game(&mut app);
    app
}

/// [`orders_app`] on a continuous-model map, where a body's position is any
/// point rather than a lattice one — what an off-lattice reach reads as is only
/// a question under this model.
pub fn continuous_orders_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    register_orders_content(&mut app);
    start_game(&mut app);
    app
}

/// App with the economy roster plus the supply roster — a `camp` that provides
/// supply, a `settler` that costs it, a `lodge` that trains settlers and
/// workers, and a `pioneer` that raises camps — one human player, session
/// started.
///
/// No other type in the roster carries a supply stat, so headroom comes only
/// from standing camps and the player's `max_supply` ceiling.
pub fn supply_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    register_orders_content(&mut app);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        // Registered before `pioneer`, which builds it.
        registry.register(
            EntityTypeDef::new("camp")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_price([("gold", 20)])
                .with_rise(10, rise_initial_pools(&[PoolId::HEALTH]))
                .with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::from_num(8)),
        );
        // Registered before `lodge`, which trains it.
        registry.register(
            EntityTypeDef::new("settler")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_train_time(10)
                .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ONE),
        );
        // Also trains the supply-free `worker`, so the gate's exemption for
        // costless types can be watched from the same trainer.
        registry.register(
            EntityTypeDef::new("lodge")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_trainer(["settler", "worker"]),
        );
        // Works from outside the site, so a camp still rising stays observable.
        registry.register(
            EntityTypeDef::new("pioneer")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["camp"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                ),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App with the player-effects roster — a `runner` whose speed owner-wide
/// modifiers move, and a `drums` player skill (+100% speed for 10 ticks,
/// 20-tick cooldown, 10 gold) — two human players, session started.
pub fn player_effects_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register(
            EntityTypeDef::new("runner")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, []),
        );
        let drums_haste = registry.register_player_buff(
            "drums_haste",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::from_num(1),
                }])],
                duration: Some(10),
                stack_rule: StackRule::Refresh,
            },
        );
        registry.register_skill(
            "drums",
            SkillDef {
                cooldown: 20,
                caster: SkillCaster::Player {
                    price: price::from([("gold", 10)]),
                    effect: PlayerCastEffect::ApplyBuff(drums_haste),
                },
                requires: Vec::new(),
            },
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App with the research roster — a `lab` (tagged "workshop") hosting the
/// `smithing` upgrade (a permanent +5 damage army-wide buff) and the `tactics`
/// unlock that requires it, plus a `guardhouse` training a free `pikeman`, a
/// `halberdier` gated on the `smithing` research, and a `knight` gated on the
/// "workshop" tag — one human player, session started.
pub fn research_app() -> App {
    research_app_seating(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)])
}

/// [`research_app`] seating `slots`.
pub fn research_app_seating(slots: Vec<PlayerSlot>) -> App {
    let mut app = make_app(slots);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register_tag("workshop");
        let sharp_blades = registry.register_player_buff(
            "sharp_blades",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                    stat: EntityStatId::DAMAGE,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(5),
                }])],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        let smithing = registry.register_research(
            "smithing",
            ResearchDef::new(
                price::from([("gold", 30)]),
                10,
                Some(sharp_blades),
                Vec::new(),
            ),
        );
        let tactics = registry.register_research(
            "tactics",
            ResearchDef::new(
                price::from([("gold", 20)]),
                10,
                None,
                [Requirement::Research(smithing)],
            ),
        );
        // Gated by nothing, so a lab can hold it behind another topic and a
        // cancel can reach an entry that has not started.
        let masonry = registry.register_research(
            "masonry",
            ResearchDef::new(price::from([("gold", 20)]), 10, None, Vec::new()),
        );
        let soldier = |name: &str| {
            EntityTypeDef::new(name)
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_attack(weapon(GROUND), 10, 1, 1, 4, 2)
                .with_price([("gold", 10)])
                .with_train_time(5)
        };
        registry.register(soldier("pikeman"));
        registry.register(soldier("halberdier").with_requires([Requirement::Research(smithing)]));
        registry
            .register(soldier("knight").with_requires([Requirement::Tag("workshop".to_string())]));
        // Either research admits the crossbowman: the one `any` in the fixture.
        registry.register(soldier("crossbowman").with_requires([Requirement::Any(vec![
            Requirement::Research(tactics),
            Requirement::Research(masonry),
        ])]));
        registry.register(
            EntityTypeDef::new("lab")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_researcher([smithing, tactics, masonry])
                .with_tags(["workshop"]),
        );
        registry.register(
            EntityTypeDef::new("guardhouse")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_trainer(["pikeman", "halberdier", "knight", "crossbowman"]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App with the annex roster — a `keep` that raises the annexes its two docks
/// take and lifts off into `keep_aloft`, a `tower` whose dock is on its other
/// side (so two primaries can offer one cell), and the annexes themselves: the
/// `lookout` that researches and stands idle with no primary, the `beacon`
/// razed the moment it loses one, the `mast` that keeps working while it
/// fades, the `spire` bound to its owner alone and the `mooring` its allies
/// share. A `sentry` may only be trained by a keep with a lookout docked.
///
/// Players 0 and 2 are allied; player 1 is the rival, session started.
pub fn annex_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(0)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(2, PlayerType::Human, None, Some(0)),
    ]);
    // The air layer the lifted-off form lives on, over the same ground.
    {
        let mut grid = NavGrid::new(32, 32, |_| LAND);
        grid.add_flat_layer(GROUND, 1);
        grid.add_flat_layer(AIR, 2);
        map::install_map(
            app.world_mut(),
            Map::new(
                "test",
                Projection::Isometric,
                MovementModel::Cell,
                grid,
                vec![],
            ),
        );
    }
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        register_air(&mut registry);
        let signals = registry.register_research(
            "signals",
            ResearchDef::new(price::from([("gold", 10)]), 20, None, Vec::new()),
        );
        let annexes = [
            "lookout",
            "watchpost",
            "beacon",
            "mast",
            "spire",
            "mooring",
            "hub",
        ];
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(200))
                .with_sight_range(8)
                .with_dying(2, [])
                .with_tags(["building"])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    annexes,
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                )
                // Two docks, so which one an annex fills is recorded and not
                // merely counted: east of the keep, and south of it.
                .with_docks([
                    (CellPos::new(2, 0), Kinds::types(annexes)),
                    (CellPos::new(0, 2), Kinds::types(annexes)),
                ])
                .with_trainer(["sentry", "runner"])
                .with_morphs([MorphTransition::new(
                    "keep_aloft",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(4),
                    MorphPlacement::Revalidate,
                    MorphCancel::Committed,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        registry.register(
            EntityTypeDef::new("keep_aloft")
                .with_location(AIR_DOMAIN, CellSize::new(2, 2), Solidity::Solid, AIR)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(200))
                .with_sight_range(8)
                .with_dying(2, [])
                .with_tags(["building"])
                .with_morphs([MorphTransition::new(
                    "keep",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(4),
                    MorphPlacement::Reserve,
                    MorphCancel::Committed,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        // Its dock is below it, so a tower two cells up offers the same cell a
        // keep two cells to the left does.
        registry.register(
            EntityTypeDef::new("tower")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(150))
                .with_sight_range(8)
                .with_dying(2, [])
                .with_tags(["building"])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["lookout"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                )
                .with_docks([(CellPos::new(0, 2), Kinds::types(["lookout"]))]),
        );
        registry.register(
            EntityTypeDef::new("lookout")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(40))
                .with_sight_range(6)
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(
                    4,
                    [(
                        PoolId::HEALTH,
                        RisePool::FollowsWork(RisePoolInitial::Share(fixed("0.25"))),
                    )],
                )
                .with_researcher([signals])
                .with_trainer(["signaler"])
                .with_annex(
                    AloneConduct::Standing {
                        work: AnnexWork::Idles,
                        life: AnnexLife::Endures,
                    },
                    AnnexClaim::Seized,
                ),
        );
        // Twelve ticks to raise, where every other annex takes four. Command
        // latency is three, so a test that has to watch a site part-raised —
        // held behind a queue, or halted by a lift-off — needs one whose work
        // outlasts the commands it sends.
        registry.register(
            EntityTypeDef::new("watchpost")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(40))
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(12, rise_initial_pools(&[PoolId::HEALTH]))
                .with_annex(
                    AloneConduct::Standing {
                        work: AnnexWork::Idles,
                        life: AnnexLife::Endures,
                    },
                    AnnexClaim::Seized,
                ),
        );
        registry.register(
            EntityTypeDef::new("beacon")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_annex(AloneConduct::Razed, AnnexClaim::Bound),
        );
        // Ten health and two a tick: five ticks alone and it is gone. The
        // drain is declared at zero for the fade to raise.
        registry.register(
            EntityTypeDef::new("mast")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(10))
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_annex(
                    AloneConduct::Standing {
                        work: AnnexWork::Works,
                        life: AnnexLife::Fades {
                            per_tick: FixedU64::from_num(2),
                        },
                    },
                    AnnexClaim::Seized,
                ),
        );
        // Bound to whoever raised it: a rival's building docks with it never,
        // an ally's without taking it, and it stands either way.
        registry.register(
            EntityTypeDef::new("spire")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_annex(
                    AloneConduct::Standing {
                        work: AnnexWork::Works,
                        life: AnnexLife::Endures,
                    },
                    AnnexClaim::Bound,
                ),
        );
        // The same, but its own side's buildings are welcome: an ally's keep
        // docks with it, and it stays whose it was.
        registry.register(
            EntityTypeDef::new("mooring")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_annex(
                    AloneConduct::Standing {
                        work: AnnexWork::Works,
                        life: AnnexLife::Endures,
                    },
                    AnnexClaim::Allied,
                ),
        );
        // An annex that is a primary in its own turn: it stands in a keep's
        // dock and offers one of its own, so a chain of seized annexes can be
        // handed over at once.
        registry.register(
            EntityTypeDef::new("hub")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["relay"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                )
                .with_docks([(CellPos::new(1, 0), Kinds::types(["relay"]))])
                .with_annex(
                    AloneConduct::Standing {
                        work: AnnexWork::Works,
                        life: AnnexLife::Endures,
                    },
                    AnnexClaim::Seized,
                ),
        );
        registry.register(
            EntityTypeDef::new("relay")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(60))
                .with_dying(2, [])
                .with_tags(["building"])
                .with_price([("gold", 10)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_annex(
                    AloneConduct::Standing {
                        work: AnnexWork::Works,
                        life: AnnexLife::Endures,
                    },
                    AnnexClaim::Seized,
                ),
        );
        // A plain unit the keep trains, asking nothing of its docks.
        registry.register(
            EntityTypeDef::new("runner")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                // Shorter than the lookout's rise time, so a unit that trained
                // alongside the annex instead of behind it would be out while
                // the annex was still rising.
                .with_train_time(2),
        );
        // What the lookout trains: slow enough that an entry is still in the
        // queue when a primary lifts off and a rival lands in its place.
        registry.register(
            walker("signaler", GROUND_DOMAIN)
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_train_time(40),
        );
        registry.register(
            EntityTypeDef::new("sentry")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_train_time(4)
                .with_requires([Requirement::Annexed("lookout".to_string())]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App with the transport roster — riflemen and grunts (tagged "infantry",
/// cargo sizes 1 and 2), an untransportable `civilian`, an own-only `wagon`
/// and an allies-welcome `ferry` (capacity 4, metered loading/unloading, cargo
/// destroyed with them), an immobile `bunker` carrying riflemen by type name
/// whose passengers fight and are ejected on death, and a splashing `bombard`
/// — players 0 and 1 on one team against player 2, session started.
pub fn transport_app() -> App {
    let mut app = make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(1, PlayerType::Human, None, Some(1)),
        PlayerSlot::occupied(2, PlayerType::Human, None, Some(2)),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register_tag("infantry");
        registry.register(
            EntityTypeDef::new("rifleman")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_attack(weapon(GROUND), 10, 3, 5, 4, 2)
                .with_sight_range(8)
                .with_tags(["infantry"])
                .with_price([("gold", 10)])
                .with_train_time(5)
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
        );
        // A trainer and researcher that rides, for production carried across
        // a ride.
        let drill = registry.register_research(
            "drill",
            ResearchDef::new(price::from([("gold", 10)]), 10, None, Vec::new()),
        );
        registry.register(
            EntityTypeDef::new("drillmaster")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_sight_range(8)
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
                .with_trainer(["rifleman"])
                .with_researcher([drill]),
        );
        registry.register(
            EntityTypeDef::new("grunt")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::from_num(2)),
        );
        registry.register(
            EntityTypeDef::new("civilian")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_tags(["infantry"]),
        );
        let carrier = |name: &str, boarding: Affiliation| {
            EntityTypeDef::new(name)
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, [])
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::from_num(3))
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::from_num(2))
                .with_transporter(
                    Kinds::tags(["infantry"]),
                    boarding,
                    PassengerFate::Destroy,
                    PassengerConduct::Shelter,
                )
        };
        registry.register(carrier("wagon", Affiliation::Own));
        // A holder that works from inside a seam, for riders inside a holder
        // inside a job.
        registry.register(
            EntityTypeDef::new("ore_seam")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_resource_source("gold", DepletionPolicy::Persist),
        );
        registry.register(
            carrier("hauler", Affiliation::Own)
                .with_sight_range(8)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        20,
                        WorkPresence::Hidden {
                            crew: CrewLimit::ONE,
                        },
                        Banking::Carried,
                        Kinds::types(["ore_seam"]),
                    ),
                )]),
        );
        registry.register(carrier("ferry", Affiliation::Allied));
        registry.register(carrier("raft", Affiliation::Anyone));
        // A mender that works from inside its patient, for a job that walks.
        registry.register(
            EntityTypeDef::new("surgeon")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_sight_range(8)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_repairer(
                    Kinds::tags(["infantry"]),
                    RepairRate::PerTick(FixedU64::ONE),
                    WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    },
                    false,
                    RepairCost::Free,
                    None,
                ),
        );
        // A mender that rides, for a boarding that takes a worker off its job
        // and for a patient that leaves its mender.
        registry.register(
            EntityTypeDef::new("orderly")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_sight_range(8)
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_repairer(
                    Kinds::Any,
                    RepairRate::PerTick(FixedU64::from_num(5)),
                    WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    },
                    false,
                    RepairCost::Free,
                    None,
                ),
        );
        // An archer whose arrows fly a cell a tick and follow their target.
        let arrow =
            registry.register_projectile("arrow", ProjectileDef::new(FixedU64::ONE, Aim::Entity));
        registry.register(
            EntityTypeDef::new("archer")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_sight_range(10)
                .with_attack(
                    AttackDef::new(Weapon::new(
                        GROUND,
                        Delivery::Projectile(arrow),
                        None,
                        Slain::Remains,
                    )),
                    10,
                    8,
                    8,
                    5,
                    2,
                ),
        );
        // The archer's twin whose shot also bursts, half its damage a cell
        // around where it lands, friend or foe.
        let stone =
            registry.register_projectile("stone", ProjectileDef::new(FixedU64::ONE, Aim::Entity));
        registry.register(
            EntityTypeDef::new("slinger")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_sight_range(10)
                .with_attack(
                    AttackDef::new(Weapon::new(
                        GROUND,
                        Delivery::Projectile(stone),
                        Some(SplashDef::new(
                            SplashShape::Circular,
                            vec![(1, fixed("0.5"))],
                            GROUND,
                            true,
                        )),
                        Slain::Remains,
                    )),
                    10,
                    8,
                    8,
                    5,
                    2,
                ),
        );
        // A rider in a committed change of form that dies when interrupted.
        registry.register(
            EntityTypeDef::new("moth")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("pupa")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
                .with_morphs([MorphTransition::new(
                    "moth",
                    MorphCourse::direct(MorphInterrupted::Dies),
                    Quantity::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Committed,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        // A fighting holder that mends from inside its patient, so it goes off
        // the map with passengers aboard.
        registry.register(
            EntityTypeDef::new("ambulance")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, [])
                .with_sight_range(8)
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(2))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
                .with_stat(EntityStatId::REPAIR_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::REPAIR_SPEED, FixedU64::ONE)
                .with_transporter(
                    Kinds::tags(["infantry"]),
                    Affiliation::Own,
                    PassengerFate::Eject,
                    PassengerConduct::Fight,
                )
                .with_repairer(
                    Kinds::types(["bunker"]),
                    RepairRate::PerTick(FixedU64::from_num(5)),
                    WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    },
                    false,
                    RepairCost::Free,
                    None,
                ),
        );
        // A rider whose only weapon is a turret, for the rule that a passenger
        // fights with what it points itself: a turret is mounted on a body that
        // stands somewhere, and a passenger stands nowhere.
        // Its gun reads a stat of its own, the way content declares one for a
        // second weapon: a body weapon's numbers are not there to be read.
        let rider_damage = registry.register_entity_stat("rider_damage", FixedU64::ZERO);
        let rider_gun = registry.register_turret(
            "rider_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None, Slain::Remains),
                TurretStats {
                    damage: rider_damage,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("gun_rider")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_sight_range(8)
                .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(360))
                .with_stat(rider_damage, FixedU64::from_num(10))
                .with_stat(EntityStatId::ATTACK_RANGE, FixedU64::from_num(3))
                .with_stat(EntityStatId::ACQUIRE_RANGE, FixedU64::from_num(5))
                .with_stat(EntityStatId::ATTACK_PERIOD, FixedU64::from_num(4))
                .with_stat(EntityStatId::DAMAGE_POINT, FixedU64::from_num(2))
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE)
                .with_turrets([TurretMount::new(
                    rider_gun,
                    CellPos::new(0, 0),
                    CellSize::ONE,
                )]),
        );
        registry.register(
            EntityTypeDef::new("bunker")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(200))
                .with_dying(2, [])
                .with_sight_range(8)
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
                .with_transporter(
                    Kinds::types(["rifleman", "gun_rider"]),
                    Affiliation::Own,
                    PassengerFate::Eject,
                    PassengerConduct::Fight,
                ),
        );
        registry.register(
            EntityTypeDef::new("bombard")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(40))
                .with_dying(2, [])
                .with_sight_range(10)
                .with_attack(
                    AttackDef::new(Weapon::new(
                        GROUND,
                        Delivery::Instant,
                        Some(SplashDef::new(
                            SplashShape::Circular,
                            vec![(2, FixedU64::from_num(0.5))],
                            GROUND,
                            true,
                        )),
                        Slain::Remains,
                    )),
                    20,
                    6,
                    6,
                    4,
                    2,
                ),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App on a continuous-model map with a solid 3x3 `keep` and a `runner`
/// (speed 0.3, radius 0.5), session started.
pub fn corner_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100)),
        );
        registry.register(
            EntityTypeDef::new("runner")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20)),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// App on the continuous model with two movers that differ only in whether they
/// line up before walking: `nimble` comes round as it goes, while `ponderous`
/// declares a pivot angle and so plants its feet for anything past a right angle.
/// Both come round slowly enough to watch, and the ponderous one slowly enough
/// that a turn outlasts the stall clock several times over. One human player,
/// session started.
pub fn turning_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    install_map(&mut app, Projection::Isometric, MovementModel::Continuous);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("nimble")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.25),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(9),
                    FixedU64::from_num(18),
                )
                .with_pool(Pool::health(20)),
        );
        registry.register(
            EntityTypeDef::new("ponderous")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.25),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(9),
                    FixedU64::ONE,
                )
                .with_stat(EntityStatId::PIVOT_ANGLE, FixedU64::from_num(90))
                .with_pool(Pool::health(20)),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    start_game(&mut app);
    app
}

/// [`turning_app`]'s movers on the cell model, where a crossing is a claim rather
/// than a free walk.
pub fn turning_cell_app() -> App {
    let mut app = turning_app();
    install_map(&mut app, Projection::Isometric, MovementModel::Cell);
    app
}

/// The ids currently riding inside `holder`.
pub fn passengers_of(world: &World, holder: Entity) -> Vec<SimulationId> {
    world
        .get::<TransporterComponent>(holder)
        .map(|transporter| transporter.passengers.iter().copied().collect())
        .unwrap_or_default()
}

/// Runs until `holder` carries exactly `count` passengers, within `limit` ticks.
pub fn run_until_aboard(app: &mut App, holder: Entity, count: usize, limit: u32) {
    for _ in 0..limit {
        if passengers_of(app.world(), holder).len() == count {
            return;
        }
        run_ticks(app, 1);
    }
    assert_eq!(
        passengers_of(app.world(), holder).len(),
        count,
        "expected {count} passengers within {limit} ticks"
    );
}

/// Orders `holder` to take `target` in.
pub fn load(app: &mut App, holder: SimulationId, target: SimulationId) {
    push_command(
        app,
        PlayerCommand::Load {
            holder,
            target,
            flush: true,
        },
    );
}

/// Issues an unload command for `holder` as the local player.
pub fn unload(app: &mut App, holder: SimulationId, at: Option<FixedUVec2>) {
    push_command(
        app,
        PlayerCommand::Unload {
            holder,
            at,
            flush: true,
        },
    );
}

/// Sets `entity`'s rally point to `(x, y)` for player 0, and lets the command
/// land.
pub fn set_rally(app: &mut App, entity: SimulationId, x: u32, y: u32) {
    set_rally_to(app, entity, RallyTarget::Position(pos(x, y)));
    run_ticks(app, APPLY);
}

/// Orders `entity`'s rally point set to `target` for player 0.
pub fn set_rally_to(app: &mut App, entity: SimulationId, target: RallyTarget) {
    push_command(
        app,
        PlayerCommand::SetRallyPoint {
            entity,
            target: Some(target),
        },
    );
}

/// Where the last order `entity` holds moves it, if that order is a move.
pub fn move_target(app: &App, entity: Entity) -> Option<FixedUVec2> {
    match entity_def::orders(app.world(), entity).last() {
        Some(Order::Move { target, .. }) => Some(*target),
        Some(_) | None => None,
    }
}

/// Selects `unit` and right-clicks `target` — the send-to-entity intent that
/// resolves to boarding when the target is a transporter with room.
pub fn send_to(app: &mut App, unit: SimulationId, target: SimulationId) {
    select(app, unit);
    push_command(
        app,
        PlayerCommand::SendToEntity {
            target,
            flush: true,
        },
    );
}

/// Orders `unit` to board `target`, selecting it first.
pub fn board(app: &mut App, unit: SimulationId, target: SimulationId) {
    select(app, unit);
    push_command(
        app,
        PlayerCommand::Board {
            target,
            flush: true,
        },
    );
}

/// A one-cell solid building named `name` with 100 health.
pub fn building(name: &str) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
        .with_pool(Pool::health(100))
        .with_dying(1, [])
}

/// How many of `entity_type` player 0 has produced.
pub fn produced(app: &App, entity_type: EntityTypeId) -> u32 {
    app.world()
        .resource::<Statistics>()
        .player(0)
        .produced(entity_type)
}

/// Whether the entity `id` is alive: neither dying nor gone.
pub fn alive(app: &App, id: SimulationId) -> bool {
    app.world().resource::<EntityIndex>().alive(id).is_some()
}

/// The work put into the rising `entity`, or `None` once it has risen.
pub fn rise_progress(app: &App, entity: Entity) -> Option<u32> {
    app.world()
        .get::<RisingComponent>(entity)
        .map(|rising| rising.progress)
}

/// Orders `trainer` to train one unit of `type_name`.
pub fn train_entity(app: &mut App, trainer: SimulationId, type_name: &str) {
    push_command(
        app,
        PlayerCommand::TrainEntity {
            trainer,
            type_name: type_name.into(),
        },
    );
}

/// Orders `warper` to warp a `type_name` in anchored at `(x, y)`.
pub fn warp_in(app: &mut App, warper: SimulationId, type_name: &str, x: u32, y: u32) {
    push_command(
        app,
        PlayerCommand::WarpIn {
            warper,
            type_name: type_name.into(),
            at: pos(x, y),
        },
    );
}

/// Orders `builder` to raise `type_name` at `(x, y)`, behind whatever it has
/// queued.
pub fn build_entity(app: &mut App, builder: SimulationId, type_name: &str, x: u32, y: u32) {
    push_command(
        app,
        PlayerCommand::BuildEntity {
            builder,
            type_name: type_name.into(),
            position: pos(x, y),
            flush: false,
        },
    );
}

/// Commands `entity`'s change into `type_name` the way a player does:
/// selects it and issues the morph command, so the executor judges it.
pub fn command_morph(app: &mut App, entity: SimulationId, type_name: &str) {
    select(app, entity);
    push_command(
        app,
        PlayerCommand::Morph {
            type_name: type_name.to_string(),
            flush: true,
        },
    );
}

/// The handle the given research name resolves to in the app's registry.
pub fn research_id(app: &App, name: &str) -> ResearchId {
    app.world()
        .resource::<ContentRegistry>()
        .research(name)
        .unwrap_or_else(|| panic!("research '{name}' is registered"))
}

/// The handle the given skill name resolves to in the app's registry.
pub fn skill_id(app: &App, name: &str) -> SkillId {
    app.world()
        .resource::<ContentRegistry>()
        .skill(name)
        .unwrap_or_else(|| panic!("skill '{name}' is registered"))
}

/// Has the local player cast the named skill from `caster` at `target`; the
/// cast lands once the command's input delay has run.
pub fn use_skill(app: &mut App, skill: &str, caster: SkillCasterRef, target: Option<SkillTarget>) {
    let skill = skill_id(app, skill);
    push_command(
        app,
        PlayerCommand::UseSkill {
            skill,
            caster,
            target,
        },
    );
}

/// Casts `skill` from `caster` at nothing but itself, as the local player.
pub fn cast_self(app: &mut App, caster: SimulationId, skill: &str) {
    use_skill(app, skill, SkillCasterRef::Entity(caster), None);
}

/// Casts `skill` from `caster` at the cell `(x, y)`, as the local player.
pub fn cast_at(app: &mut App, caster: SimulationId, skill: &str, x: u32, y: u32) {
    use_skill(
        app,
        skill,
        SkillCasterRef::Entity(caster),
        Some(SkillTarget::Position(pos(x, y))),
    );
}

/// Casts `skill` from `caster` at `target`, as the local player.
pub fn cast_on(app: &mut App, caster: SimulationId, skill: &str, target: SimulationId) {
    use_skill(
        app,
        skill,
        SkillCasterRef::Entity(caster),
        Some(SkillTarget::Entity(target)),
    );
}

/// The entity's speed stat after the tick's modifier fold — what the
/// player-effect suites compare before and after an owner-wide modifier.
pub fn effective_speed(app: &App, entity: Entity) -> FixedU64 {
    app.world()
        .get::<StatsComponent>(entity)
        .unwrap()
        .effective(EntityStatId::SPEED)
        .unwrap()
}

/// The number of units waiting in `entity`'s training queue.
pub fn train_queue_len(world: &World, entity: Entity) -> usize {
    world
        .get::<TrainQueueComponent>(entity)
        .map_or(0, |queue| queue.0.len())
}

/// Registers the economy content roster ([`orders_app`]'s) and validates it.
/// The mobile types see the whole harness map (sight 40): these suites
/// exercise orders and economy, not scouting — fog has its own suites, and a
/// fogged target would refuse the very commands under test.
pub fn register_orders_content(app: &mut App) {
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register_resource("wood");
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_attack(weapon(GROUND), 10, 1, 1, 4, 2)
                .with_price([("gold", 30)])
                .with_train_time(4),
        );
        // A wide continuous mover: 2x2 footprint with the largest legal body
        // circle (radius = half the narrow side), for mixed-size contact and
        // claim tests.
        registry.register(
            EntityTypeDef::new("wagon")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, []),
        );
        // A heavy continuous mover on a soldier's footprint, for contact tests
        // that need weight told apart from size.
        registry.register(
            EntityTypeDef::new("ox")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(4),
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(60))
                .with_dying(2, []),
        );
        // A stall that sees one cell further once it has risen.
        let opened = registry.register_entity_buff(
            "opened",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::Stats(vec![flat(
                    EntityStatId::SIGHT_RANGE,
                    "1",
                )]))],
                lasting: Lasting::While(Requirement::Risen),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        // A rampart whose site starts at a quarter of its 400 health and
        // gains the rest over its ten ticks of work.
        registry.register(
            EntityTypeDef::new("rampart")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(400))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_rise(
                    10,
                    [(
                        PoolId::HEALTH,
                        RisePool::FollowsWork(RisePoolInitial::Share(fixed("0.25"))),
                    )],
                ),
        );
        // A flare tower whose site has no energy; the finished tower gains it at
        // a quarter of 60.
        registry.register(
            EntityTypeDef::new("flare_tower")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_pool(Pool::builtin(
                    PoolId::ENERGY,
                    FixedU64::from_num(60),
                    FixedU64::ZERO,
                    FixedU64::ZERO,
                    PoolInitial::Share(fixed("0.25")),
                ))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_rise(
                    4,
                    [
                        (PoolId::HEALTH, RisePool::Initial),
                        (PoolId::ENERGY, RisePool::GainedOnRisen),
                    ],
                ),
        );
        // A cistern whose site raises its energy from half of 80 with the
        // work, while the energy also regenerates a point a tick.
        registry.register(
            EntityTypeDef::new("cistern")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_pool(Pool::builtin(
                    PoolId::ENERGY,
                    FixedU64::from_num(80),
                    FixedU64::ONE,
                    FixedU64::ZERO,
                    PoolInitial::Full,
                ))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_rise(
                    4,
                    [
                        (PoolId::HEALTH, RisePool::Initial),
                        (
                            PoolId::ENERGY,
                            RisePool::FollowsWork(RisePoolInitial::Share(fixed("0.5"))),
                        ),
                    ],
                ),
        );
        registry.register(
            EntityTypeDef::new("kiosk")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(50))
                .with_sight_range(2)
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_rise(6, rise_initial_pools(&[PoolId::HEALTH]))
                .with_passives([opened]),
        );
        // Registered before `worker`, which builds it.
        registry.register(
            EntityTypeDef::new("depot")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_price([("gold", 50)])
                .with_rise(6, rise_initial_pools(&[PoolId::HEALTH]))
                .with_resource_storage(["gold", "wood"])
                .with_berths([(
                    "rim",
                    BerthGroup::new(
                        [
                            berth("0.5", "0.5"),
                            berth("1.5", "0.5"),
                            berth("0.5", "1.5"),
                            berth("1.5", "1.5"),
                        ],
                        4,
                    ),
                )]),
        );
        registry.register(
            EntityTypeDef::new("worker")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_price([("gold", 10)])
                .with_train_time(2)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_builder(
                    ["depot", "kiosk", "rampart"],
                    BuilderAttendance::Crew(WorkPresence::Hidden {
                        crew: CrewLimit::ONE,
                    }),
                )
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        2,
                        WorkPresence::Hidden {
                            crew: CrewLimit::ONE,
                        },
                        Banking::Carried,
                        Kinds::Any,
                    ),
                )]),
        );
        // Same catalogue as `worker`, but it works from outside the site — the pair
        // is what makes `WorkPresence` observable.
        registry.register(
            EntityTypeDef::new("mason")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["depot"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                ),
        );
        // Same catalogue again, and any number of them can crowd one site.
        registry.register(
            EntityTypeDef::new("carpenter")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["depot"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::Unlimited,
                    }),
                ),
        );
        // Same catalogue, and it works on the site itself: on the map and open
        // to attack, but holding no cells.
        registry.register(
            EntityTypeDef::new("roofer")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["depot"],
                    BuilderAttendance::Crew(WorkPresence::Attached(Attachment::new(
                        "rim",
                        BerthStance::Still,
                    ))),
                ),
        );
        // Same catalogue once more, but it only places the site: the depot
        // advances itself from there.
        registry.register(
            EntityTypeDef::new("architect")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["depot"], BuilderAttendance::Unattended),
        );
        // Same catalogue, and the site is what becomes of it: placing a depot
        // spends the larva.
        registry.register(
            EntityTypeDef::new("larva")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ONE)
                .with_builder(["depot"], BuilderAttendance::Consumed),
        );
        registry.register(
            EntityTypeDef::new("lumberjack")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "wood",
                    HarvestData::new(
                        5,
                        5,
                        2,
                        WorkPresence::Present {
                            crew: CrewLimit::ONE,
                        },
                        Banking::Carried,
                        Kinds::Any,
                    ),
                )]),
        );
        // Works a seam from three cells back, which says nothing about how close it
        // has to get to put the load down.
        registry.register(
            EntityTypeDef::new("prospector")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::from_num(3))
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        2,
                        WorkPresence::Present {
                            crew: CrewLimit::ONE,
                        },
                        Banking::Carried,
                        Kinds::Any,
                    ),
                )]),
        );
        // Same trade as `lumberjack`, but a stand takes as many axes as turn up — and
        // a trip long enough to watch a crew form and break up while it lasts.
        registry.register(
            EntityTypeDef::new("logger")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "wood",
                    HarvestData::new(
                        5,
                        5,
                        8,
                        WorkPresence::Present {
                            crew: CrewLimit::Unlimited,
                        },
                        Banking::Carried,
                        Kinds::Any,
                    ),
                )]),
        );
        registry.register(
            EntityTypeDef::new("barracks")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_sight_range(8)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_price([("gold", 40)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_trainer(["soldier"]),
        );
        // Twenty ticks to train, where the soldier takes four: long enough that
        // a cancel aimed at the entry in progress lands while it is still under
        // way, and that the entry behind it can be watched starting over.
        registry.register(
            walker("recruit", GROUND_DOMAIN)
                .with_pool(Pool::health(30))
                .with_dying(1, [])
                .with_price([("gold", 30)])
                .with_train_time(20),
        );
        registry.register(
            EntityTypeDef::new("academy")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_sight_range(8)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_price([("gold", 40)])
                .with_rise(4, rise_initial_pools(&[PoolId::HEALTH]))
                .with_trainer(["recruit"]),
        );
        // A plain obstacle, for walling sources off.
        registry.register(
            EntityTypeDef::new("boulder")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(50)),
        );
        // Carries either resource, so only an order's kind lock keeps a wood
        // trip off the gold.
        registry.register(
            EntityTypeDef::new("forager")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([
                    (
                        "gold",
                        HarvestData::new(
                            5,
                            5,
                            2,
                            WorkPresence::Present {
                                crew: CrewLimit::ONE,
                            },
                            Banking::Carried,
                            Kinds::Any,
                        ),
                    ),
                    (
                        "wood",
                        HarvestData::new(
                            5,
                            5,
                            2,
                            WorkPresence::Present {
                                crew: CrewLimit::ONE,
                            },
                            Banking::Carried,
                            Kinds::Any,
                        ),
                    ),
                ]),
        );
        // Banks where it stands: sits in a tree alone, drawing wood without
        // felling it, or on a mine with any number of others, draining it.
        registry.register(
            EntityTypeDef::new("sylph")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["shaft_house", "pump_house", "well_house"],
                    BuilderAttendance::Unattended,
                )
                .with_resource_carrier([
                    (
                        "wood",
                        HarvestData::new(
                            5,
                            0,
                            2,
                            WorkPresence::Attached(Attachment::new("canopy", BerthStance::Still)),
                            Banking::Direct,
                            Kinds::Any,
                        ),
                    ),
                    (
                        "gold",
                        HarvestData::new(
                            5,
                            5,
                            2,
                            WorkPresence::Attached(Attachment::new("rim", BerthStance::Still)),
                            Banking::Direct,
                            Kinds::Any,
                        ),
                    ),
                ]),
        );
        // A sylph that never sits still: two ticks at a berth, then a quarter
        // of a cell a tick straight across to another free one.
        registry.register(
            EntityTypeDef::new("roamer")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        2,
                        WorkPresence::Attached(Attachment::new(
                            "rim",
                            BerthStance::Roaming {
                                speed: FixedU64::from_num(0.25),
                                dwell: 2,
                            },
                        )),
                        Banking::Direct,
                        Kinds::Any,
                    ),
                )]),
        );
        // A sylph that walks the rim: two ticks at a berth, then a quarter of a
        // cell a tick along the loop to the next free one.
        registry.register(
            EntityTypeDef::new("circler")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        2,
                        WorkPresence::Attached(Attachment::new(
                            "rim",
                            BerthStance::Circling {
                                speed: FixedU64::from_num(0.25),
                                dwell: 2,
                            },
                        )),
                        Banking::Direct,
                        Kinds::Any,
                    ),
                )]),
        );
        // A sylph that hovers: it circles its canopy berth a quarter turn a
        // tick, a quarter of a cell out.
        registry.register(
            EntityTypeDef::new("hoverer")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "wood",
                    HarvestData::new(
                        5,
                        0,
                        2,
                        WorkPresence::Attached(Attachment::new(
                            "canopy",
                            BerthStance::Orbit {
                                radius: FixedU64::from_num(0.25),
                                period: 4,
                            },
                        )),
                        Banking::Direct,
                        Kinds::Any,
                    ),
                )]),
        );
        // Banks a load of five while taking two out of the seam.
        registry.register(
            EntityTypeDef::new("tapper")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        2,
                        2,
                        WorkPresence::Attached(Attachment::new("rim", BerthStance::Still)),
                        Banking::Direct,
                        Kinds::Any,
                    ),
                )]),
        );
        // Two diggers at a time work one seam, each of them inside it: the
        // count is the carrier's, and no seat is involved.
        registry.register(
            EntityTypeDef::new("paired_digger")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        20,
                        WorkPresence::Hidden {
                            crew: CrewLimit::limit(2),
                        },
                        Banking::Carried,
                        Kinds::Any,
                    ),
                )]),
        );
        // `paired_digger`'s twin but for the crew it allows: one. The pair is
        // what makes a crew of mixed terms observable, and the same twenty-tick
        // trip keeps both of them on the seam while it is asked about.
        registry.register(
            EntityTypeDef::new("lone_digger")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        20,
                        WorkPresence::Hidden {
                            crew: CrewLimit::ONE,
                        },
                        Banking::Carried,
                        Kinds::Any,
                    ),
                )]),
        );
        // Banks what it takes straight from inside the seam, for a source
        // drained to death by the carrier working inside it.
        registry.register(
            EntityTypeDef::new("siphon")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        20,
                        WorkPresence::Hidden {
                            crew: CrewLimit::ONE,
                        },
                        Banking::Direct,
                        Kinds::Any,
                    ),
                )]),
        );
        // Works gold out of a geyser and nothing else, however much a bare
        // seam holds.
        registry.register(
            EntityTypeDef::new("geyser_tapper")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20))
                .with_dying(2, [])
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "gold",
                    HarvestData::new(
                        5,
                        5,
                        20,
                        WorkPresence::Hidden {
                            crew: CrewLimit::ONE,
                        },
                        Banking::Carried,
                        Kinds::types(["geyser"]),
                    ),
                )]),
        );
        // A bare seam: nothing sits on it, so an attached carrier is turned
        // away, and a shaft house may be raised over it.
        registry.register(
            EntityTypeDef::new("mine")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        // A spring claims no cell; a well house is raised over one.
        registry.register(
            EntityTypeDef::new("spring")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Passable, GROUND)
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("well_house")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_price([("gold", 20)])
                .with_rise(20, rise_initial_pools(&[PoolId::HEALTH]))
                .with_tags(["building"])
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_overbuilds("spring"),
        );
        // A seam that walks, for a carrier inside a source that moves on.
        registry.register(
            EntityTypeDef::new("roaming_seam")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    fixed("0.5"),
                    fixed("0.5"),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_resource_source("gold", DepletionPolicy::Persist),
        );
        // A wide seam with three spots — two down its west edge and one at its
        // south-east, so a loop has a corner to turn and a crossing has a
        // diagonal — seating two workers.
        registry.register(
            EntityTypeDef::new("lode")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_berths([(
                    "rim",
                    BerthGroup::new(
                        [
                            berth("0.5", "0.5"),
                            berth("0.5", "1.5"),
                            berth("1.5", "1.5"),
                        ],
                        2,
                    ),
                )]),
        );
        // A wide seam with four spots at its corners in loop order — north-west,
        // north-east, south-east, south-west — all seated, for the way a
        // circling crew spreads round it.
        registry.register(
            EntityTypeDef::new("ring")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_berths([(
                    "rim",
                    BerthGroup::new(
                        [
                            berth("0.5", "0.5"),
                            berth("1.5", "0.5"),
                            berth("1.5", "1.5"),
                            berth("0.5", "1.5"),
                        ],
                        4,
                    ),
                )]),
        );
        // A wide seam with two berths on its west edge.
        registry.register(
            EntityTypeDef::new("vein")
                .with_location(GROUND_DOMAIN, CellSize::new(2, 2), Solidity::Solid, GROUND)
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_berths([(
                    "rim",
                    BerthGroup::new([berth("0.5", "0.5"), berth("0.5", "1.5")], 2),
                )]),
        );
        // Raised over a mine: takes its gold, seats one carrier, and gives the
        // mine back when it falls with gold still in it.
        registry.register(
            EntityTypeDef::new("shaft_house")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_price([("gold", 20)])
                .with_rise(20, rise_initial_pools(&[PoolId::HEALTH]))
                .with_tags(["building"])
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_overbuilds("mine")
                .with_berths([("rim", BerthGroup::new([berth("0.5", "0.5")], 1))])
                .with_morphs([MorphTransition::new(
                    "walking_shaft",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(4),
                    MorphPlacement::Reserve,
                    MorphCancel::Committed,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        // The form a shaft house takes when it uproots.
        registry.register(
            EntityTypeDef::new("walking_shaft")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                // Back to standing, as an uprooted ancient roots again: the
                // ground it settles on is reserved when the change starts.
                .with_morphs([MorphTransition::new(
                    "shaft_house",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(4),
                    MorphPlacement::Reserve,
                    MorphCancel::Committed,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        // Raised over a geyser, which stays on the map when emptied: what the
        // pump house gives back when it falls drained.
        registry.register(
            EntityTypeDef::new("pump_house")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_pool(Pool::health(100))
                .with_dying(2, [])
                .with_price([("gold", 20)])
                .with_rise(20, rise_initial_pools(&[PoolId::HEALTH]))
                .with_tags(["building"])
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_overbuilds("geyser")
                .with_berths([("rim", BerthGroup::new([berth("0.5", "0.5")], 1))])
                .with_morphs([MorphTransition::new(
                    "walking_pump",
                    MorphCourse::direct(MorphInterrupted::Reverts),
                    Quantity::Constant(4),
                    MorphPlacement::Reserve,
                    MorphCancel::Committed,
                    MorphReason::Change,
                    Vec::new(),
                    Vec::new(),
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                )]),
        );
        // The form a pump house takes when it uproots: wider than the house, so
        // the walk recenters it away from the cell the house was raised over,
        // and it draws nothing itself.
        registry.register(
            EntityTypeDef::new("walking_pump")
                .with_location(GROUND_DOMAIN, CellSize::new(3, 3), Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(100))
                .with_dying(2, []),
        );
        registry.register(
            EntityTypeDef::new("tree")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_resource_source("wood", DepletionPolicy::Destroy)
                .with_berths([("canopy", BerthGroup::new([berth("0.5", "0.5")], 1))]),
        );
        registry.register(
            EntityTypeDef::new("geyser")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_resource_source("gold", DepletionPolicy::Persist)
                .with_berths([("rim", BerthGroup::new([berth("0.5", "0.5")], 1))]),
        );
        registry.register(
            EntityTypeDef::new("ghost")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Passable, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(20)),
        );
        // A soldier variant that notices enemies well beyond its weapon range,
        // for the stance and auto-engagement suites.
        registry.register(
            EntityTypeDef::new("sentry")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_attack(weapon(GROUND), 10, 1, 5, 4, 2)
                // Sees farther than it auto-engages, so its circular vision
                // covers everything within acquisition range.
                .with_sight_range(8),
        );
        // A ranged sentry, for suites that need hits without an adjacent chaser.
        registry.register(
            EntityTypeDef::new("archer")
                .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_pool(Pool::health(30))
                .with_dying(2, [])
                .with_attack(weapon(GROUND), 10, 3, 5, 4, 2)
                .with_sight_range(8),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
}

/// A position pinned to the bit — captured from a probe run and asserted
/// exactly ever after: any drift is a lockstep desync.
pub fn position_bits(x: u64, y: u64) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_bits(x), FixedU64::from_bits(y))
}

/// Builds a networked app of `players` Human slots, whose local slot matches the
/// transport's peer. `players` is the roster a lobby would have agreed (slots
/// `0..players`), passed in rather than inferred from connectivity.
pub fn net_app(transport: LoopbackTransport, players: usize) -> App {
    net_app_with_roster(transport, Roster::new((0..players as u64).collect()))
}

/// Like [`net_app`], with an explicit roster (e.g. a slot whose peer will
/// never speak).
pub fn net_app_with_roster(transport: LoopbackTransport, roster: Roster) -> App {
    net_app_configured(
        transport,
        roster,
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
    )
}

/// Like [`net_app_with_roster`], with an explicit decision authority.
pub fn net_app_configured(
    transport: LoopbackTransport,
    roster: Roster,
    authority: Authority,
) -> App {
    let slots = (0..roster.len())
        .map(|i| PlayerSlot::occupied(i as u8, PlayerType::Human, None, None))
        .collect();
    net_app_with_slots(transport, roster, authority, slots)
}

/// Like [`net_app_configured`], with explicit session slots (e.g. allied ones).
pub fn net_app_with_slots(
    transport: LoopbackTransport,
    roster: Roster,
    authority: Authority,
    slots: Vec<PlayerSlot>,
) -> App {
    // A local peer outside the roster is an observer's node: it watches.
    let local = match roster.player_of(transport.local_peer()) {
        Some(player) => LocalRole::Player(player),
        None => LocalRole::Observer,
    };
    // Peer 0 is the host node, as the lobby would assign.
    let net = NetSession::over_shared(Box::new(transport), Role::Peer, roster);
    assert_eq!(net.gameplay_ref().local_player(), local.player());

    let mut nav_grid = NavGrid::new(32, 32, |_| LAND);
    nav_grid.add_flat_layer(GROUND, 1);
    let session = GameSession::configured(
        local,
        slots,
        "test",
        authority,
        DropPolicy::Automatic,
        FinishPolicy::Endless,
        Ruleset::new(RemainsLimit::Unbounded),
    );

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        session,
        Map::new(
            "test",
            Projection::Isometric,
            MovementModel::Cell,
            nav_grid,
            vec![],
        ),
    ));
    app.add_plugins(NetworkPlugin);
    // Supplies idle frames for AI slots with no installed runtime, as in a
    // real game; a no-op for the all-human rosters.
    app.add_plugins(ferrets_bevy_plugin::ai::AiPlugin);
    ferrets_bevy_plugin::install_network_session(app.world_mut(), net);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        register_ground(&mut registry);
        registry.register(harness_soldier());
        registry.register(harness_base());
        registry.validate();
    }
    start_game(&mut app);
    app
}

/// The one mobile entity type the harness games use. Armed, so a game can
/// destroy a building through the command pipeline; nothing attacks unordered.
pub fn harness_soldier() -> EntityTypeDef {
    EntityTypeDef::new("soldier")
        .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
        .with_sight_range(8)
        .with_movement(
            FixedU64::from_num(0.5),
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
        .with_pool(Pool::health(30))
        .with_dying(2, [])
        .with_attack(weapon(GROUND), 10, 1, 1, 4, 2)
}

/// A standing building — the presence the `LastStanding` rule counts. Immobile,
/// destructible, no combat of its own.
pub fn harness_base() -> EntityTypeDef {
    EntityTypeDef::new("base")
        .with_location(GROUND_DOMAIN, CellSize::ONE, Solidity::Solid, GROUND)
        .with_pool(Pool::health(30))
        .with_dying(2, [])
        .with_tags(["building"])
}

/// Builds the map's hierarchy for every mover shape the registry declares, as a
/// game's map is built from its content.
fn serve_movers(app: &mut App) {
    app.world_mut().resource_scope(|world, mut map: Mut<Map>| {
        map.serve_movers(world.resource::<ContentRegistry>())
    });
}
