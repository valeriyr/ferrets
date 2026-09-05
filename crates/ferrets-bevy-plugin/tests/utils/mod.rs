#![allow(dead_code)]

use std::time::Duration;

use bevy::{app::FixedMain, ecs::entity::EntityNotSpawnedError, prelude::*};
use ferrets_bevy_plugin::{
    GameSet, NetworkPlugin, NominalTimestep, PendingInput, SimulationPlugin, TickPacing, map,
    replay,
};
use ferrets_content::{
    attack::{AttackDef, Delivery, Weapon},
    build::BuilderAttendance,
    costs,
    entity_buffs::{EntityBuffDef, EntityBuffId},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::FieldId,
    location::Solidity,
    morph::{MorphCancel, MorphPlacement, MorphTime, MorphTransition},
    player_buffs::PlayerBuffDef,
    projectile::{Aim, ProjectileDef},
    registry::ContentRegistry,
    research::{ResearchDef, ResearchId},
    resource::{DepletionPolicy, HarvestData},
    skills::{EntityCastCost, PlayerCastEffect, SkillCaster, SkillDef},
    splash::{SplashDef, SplashShape},
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    turret::{TurretDef, TurretFire, TurretMount, TurretStats, WeaponConduct},
    work::WorkPresence,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};

use ferrets_math::{
    FixedI64, FixedU64, facing::Facing, fixed_uvec2::FixedUVec2, fixed_vec2::FixedVec2,
};
use ferrets_network::{
    role::Role,
    roster::Roster,
    session::NetSession,
    transport::{NetworkTransport, loopback::LoopbackTransport},
};
use ferrets_pathfinder::{
    layer_mask::LayerMask,
    mover_shape::MoverShape,
    nav_grid::{LayerId, NavGrid},
};
use ferrets_replay::{
    buffer::SharedBuffer,
    header::{RecordedGame, ReplayHeader},
    recorder::Recorder,
};
use ferrets_simulation::{
    command::{PlayerCommand, SelectMode},
    components::{
        entity_info::EntityInfoComponent,
        entity_stats::StatsComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        order_queue::{CancelPolicy, OrderQueueComponent},
        owner::OwnerComponent,
        pending_reveal::PendingRevealComponent,
        train::TrainQueueComponent,
        transport::TransporterComponent,
        turret::TurretsComponent,
    },
    entity_def,
    events::{DeathCause, EventRecord, SimulationEvent, SpawnCause},
    fields::FieldGrid,
    input::{InputFrames, PlayerFrame},
    map::Map,
    movement_model::MovementModel,
    order::AttackTarget,
    resources::PlayerResources,
    selection::Selection,
    session::{
        GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy,
        finish_policy::FinishPolicy, local_role::LocalRole, player_id::PlayerId,
        player_slot::PlayerSlot, player_type::PlayerType,
    },
    simulation_id::SimulationId,
    skirmish::Skirmish,
    spawn::{self, FieldReach},
};

/// The single navigation layer the harness content declares.
pub const GROUND_LAYER: &str = "ground";
/// The id [`GROUND_LAYER`] resolves to — it is the first registered layer.
pub const GROUND: LayerId = LayerId::new(1);
/// The layer fliers occupy, registered by [`combat_app`] alone: only the tests
/// about which weapon may answer what need a second one.
pub const AIR_LAYER: &str = "air";
/// The id [`AIR_LAYER`] resolves to where it is registered.
pub const AIR: LayerId = LayerId::new(2);

/// A body weapon reaching `targets` that lands its hit where it stands — the
/// plainest one there is, for fixtures about anything but the weapon.
pub fn weapon(targets: impl Into<LayerMask>) -> AttackDef {
    AttackDef::new(Weapon::new(targets, Delivery::Instant, None))
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
    let mut registry = ContentRegistry::default();
    assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);

    let mut nav_grid = NavGrid::new(32, 32);
    nav_grid.add_layer(GROUND);

    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            LocalRole::Player(0),
            slots,
            "test",
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            FinishPolicy::Endless,
        ),
        Map::new(
            "test",
            Projection::Isometric,
            MovementModel::Cell,
            nav_grid,
            vec![],
            &[],
        ),
    ));
    app.insert_resource(registry);
    app
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
        }),
        MovementModel::Cell,
        Projection::Isometric,
    )
}

/// A value written as decimal digits rather than a float, so the number the
/// digits name is the one under test.
pub fn fixed(text: &str) -> FixedU64 {
    FixedU64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a value"))
}

/// The same, where the value can point downwards.
pub fn signed_fixed(text: &str) -> FixedI64 {
    FixedI64::from_str(text).unwrap_or_else(|_| panic!("'{text}' is a signed value"))
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
    spawn::create_entity(world, type_name, position, owner, FieldReach::Initial)
}

/// Like [`create_entity`], announcing the spawn with `cause`.
pub fn spawn_entity(
    world: &mut World,
    type_name: &str,
    position: FixedUVec2,
    owner: Option<PlayerId>,
    cause: ferrets_simulation::events::SpawnCause,
) -> Option<(Entity, SimulationId)> {
    spawn::spawn_entity(
        world,
        type_name,
        position,
        owner,
        cause,
        FieldReach::Initial,
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
    )
    .unwrap_or_else(|| panic!("{type_name} fits at ({x}, {y})"))
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
    spawn::spawn_entity(
        app.world_mut(),
        type_name,
        pos(x, y),
        Some(player),
        SpawnCause::Placed,
        FieldReach::Full,
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(1, None)
                .with_attack(weapon(GROUND), 10, 1, 3, 2, 1)
                .with_sight_range(5),
        );
        registry.register(
            EntityTypeDef::new("critter")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(1)
                .with_dying(1, None),
        );
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_tags(["building"]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// App on the cell model with hierarchy abstractions for both a walking 1×1
/// `soldier` and a parked 2×2 `wagon` — the blocked-crossing rungs only fire
/// when the plan was made claim-blind, which takes the hierarchy. One human
/// player, session started.
pub fn cell_crowd_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut grid = NavGrid::new(32, 32);
        grid.add_layer(GROUND);
        app.world_mut().insert_resource(Map::new(
            "test",
            Projection::Isometric,
            MovementModel::Cell,
            grid,
            vec![],
            &[
                MoverShape::point(GROUND),
                MoverShape::new(GROUND, CellSize::new(2, 2)),
            ],
        ));
    }
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("wagon")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(60)
                .with_dying(2, None),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// App with the form-change roster on the given movement model — a `whelp`
/// (1×1 mover, 30 hp) declaring three transitions: growing into the same-layer
/// 3×3 `giant`, an instant committed change into the 10-hp `husk` paid in
/// blood, and a timed change into the poolless `wisp` (which changes back);
/// plus a `shrine` (2×2 building) that unroots into the same-size `golem`
/// mover. One human player, session started.
pub fn morph_app(model: MovementModel) -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    install_map(&mut app, Projection::Isometric, model);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register(
            EntityTypeDef::new("whelp")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
                .with_cost([("gold", 10)])
                .with_train_time(20)
                .with_morphs([
                    MorphTransition::new(
                        "giant",
                        None,
                        MorphTime::Constant(10),
                        MorphPlacement::Reserve,
                        MorphCancel::Refundable,
                        Vec::new(),
                        Vec::<String>::new(),
                    ),
                    MorphTransition::new(
                        "husk",
                        None,
                        MorphTime::Constant(0),
                        MorphPlacement::Revalidate,
                        MorphCancel::Committed,
                        vec![EntityCastCost::Health(FixedU64::from_num(10))],
                        Vec::<String>::new(),
                    ),
                    MorphTransition::new(
                        "wisp",
                        None,
                        MorphTime::Constant(10),
                        MorphPlacement::Revalidate,
                        MorphCancel::Forfeit,
                        Vec::new(),
                        Vec::<String>::new(),
                    ),
                    // Worn as a chrysalis on the way, paid, and refunded if
                    // the change ends early.
                    MorphTransition::new(
                        "wyrm",
                        Some("chrysalis"),
                        MorphTime::Constant(10),
                        MorphPlacement::Revalidate,
                        MorphCancel::Refundable,
                        vec![EntityCastCost::Resources(costs::cost([("gold", 10)]))],
                        Vec::<String>::new(),
                    ),
                ]),
        );
        registry.register(
            EntityTypeDef::new("chrysalis")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(60)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("wyrm")
                .with_location(GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(90)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("giant")
                .with_location(GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(60)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("husk")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(10)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("wisp")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_morphs([MorphTransition::new(
                    "whelp",
                    None,
                    MorphTime::Constant(10),
                    MorphPlacement::Revalidate,
                    MorphCancel::Forfeit,
                    Vec::new(),
                    Vec::<String>::new(),
                )]),
        );
        registry.register(
            EntityTypeDef::new("shrine")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_tags(["building"])
                // Trains the whelp, and its unrooted form does not: the one
                // role whose live state is pre-paid, so what happens to the
                // queue across the change is a contract worth pinning.
                .with_trainer(["whelp"])
                .with_morphs([MorphTransition::new(
                    "golem",
                    None,
                    MorphTime::Constant(10),
                    MorphPlacement::Reserve,
                    MorphCancel::Refundable,
                    Vec::new(),
                    Vec::<String>::new(),
                )]),
        );
        registry.register(
            EntityTypeDef::new("golem")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(50)
                .with_dying(2, None),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
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

/// Force-cancels everything `entity` is doing, standing in for a stop command.
///
/// Not routed through `PlayerCommand::Stop`, because that reaches the selection and a
/// worker off the map cannot be selected — a hidden builder or a carrier down a mine is
/// exactly what these suites need to stop.
pub fn stop_orders(world: &mut World, entity: Entity) {
    world
        .get_mut::<OrderQueueComponent>(entity)
        .expect("simulation entities carry an order queue")
        .cancel_all(CancelPolicy::Force);
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
        app.world().get::<HiddenComponent>(worker).is_some(),
        "a boxed-in worker stays off the map"
    );
    assert!(
        app.world().get::<PendingRevealComponent>(worker).is_some(),
        "with its reveal queued rather than forced"
    );

    app.world_mut()
        .resource_mut::<Map>()
        .set_static_occupied(GROUND, cell, false);
    run_ticks(app, 1);

    assert!(app.world().get::<HiddenComponent>(worker).is_none());
    assert!(app.world().get::<PendingRevealComponent>(worker).is_none());
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

/// Asserts `unit` stands within one cell of `building`'s footprint.
pub fn assert_adjacent_to_footprint(world: &mut World, unit: Entity, building: Entity) {
    let origin = cell_of(world, building);
    let size = entity_def::of(world, building).location.unwrap().size();
    let unit_cell = cell_of(world, unit);
    assert!(
        world.resource::<Map>().projection().in_range_of_rect(
            unit_cell,
            CellRect::new(origin, size),
            1
        ),
        "expected {unit_cell:?} adjacent to the footprint at {origin:?}"
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

/// The entity's displayed health points, `0` once it is dead or gone.
pub fn health(app: &App, entity: Entity) -> u32 {
    app.world()
        .get::<HealthComponent>(entity)
        .map_or(0, HealthComponent::displayed)
}

/// The entity's exact remaining health, unrounded.
pub fn current_health(app: &App, entity: Entity) -> FixedU64 {
    app.world()
        .get::<HealthComponent>(entity)
        .unwrap()
        .current()
}

/// Removes `amount` health points directly, standing in for damage taken.
pub fn wound(app: &mut App, entity: Entity, amount: &str) {
    app.world_mut()
        .get_mut::<HealthComponent>(entity)
        .unwrap()
        .drain(fixed(amount));
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
/// permanent).
pub fn register_entity_buff(
    app: &mut App,
    name: &str,
    stat: EntityStatId,
    op: ModifierOp,
    magnitude: &str,
    duration: Option<u32>,
) -> EntityBuffId {
    app.world_mut()
        .resource_mut::<ContentRegistry>()
        .register_entity_buff(
            name,
            EntityBuffDef {
                modifiers: vec![EntityModifier {
                    stat,
                    op,
                    magnitude: signed_fixed(magnitude),
                }],
                duration,
                stack_rule: StackRule::Refresh,
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

/// Swaps the harness map for a 32×32 one with the given projection and
/// movement model, a ground hierarchy included. Call before any spawns.
pub fn install_map(app: &mut App, projection: Projection, model: MovementModel) {
    let mut grid = NavGrid::new(32, 32);
    grid.add_layer(GROUND);
    map::install_map(
        app.world_mut(),
        Map::new(
            "test",
            projection,
            model,
            grid,
            vec![],
            &[MoverShape::point(GROUND)],
        ),
    );
}

/// Side of [`install_chokepoint_map`]'s map. Several clusters across, so a walk
/// over it is planned as a corridor of real crossings; a map only a cluster or
/// two wide comes back as one flat segment however it is walled.
pub const CHOKEPOINT_SIZE: u32 = 96;

/// The rows left open in [`install_chokepoint_map`]'s wall.
pub const CHOKEPOINT_GAP: std::ops::RangeInclusive<u32> = 8..=9;

/// Swaps the harness map for a cell-model one split by a wall with a single gap,
/// so crossing it can only be planned as a corridor through that gap. An open
/// field is planned as one flat segment and never changes legs, which is the
/// whole thing a corridor test needs to exercise. Call before any spawns.
pub fn install_chokepoint_map(app: &mut App) {
    let mut grid = NavGrid::new(CHOKEPOINT_SIZE, CHOKEPOINT_SIZE);
    grid.add_layer(GROUND);
    for y in 0..CHOKEPOINT_SIZE {
        if !CHOKEPOINT_GAP.contains(&y) {
            grid.set_occupied(GROUND, CellPos::new(CHOKEPOINT_SIZE / 2, y), true);
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
            &[MoverShape::point(GROUND)],
        ),
    );
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(8)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(50)
                .with_dying(3, None)
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
                Weapon::new(GROUND, Delivery::Instant, None),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("bastion")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(200)
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
                Weapon::new(GROUND, Delivery::Instant, None),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("gun_wagon")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(40)
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
                Weapon::new(GROUND, Delivery::Instant, None),
                TurretStats::default(),
                WeaponConduct::OnTheMove,
            ),
        );
        registry.register(
            EntityTypeDef::new("rolling_gun")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(40)
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
                ),
                TurretStats::default(),
                WeaponConduct::OnTheMove,
            ),
        );
        registry.register(
            EntityTypeDef::new("rolling_mortar")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(40)
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
                    .with_location(GROUND, CellSize::new(5, 5), Solidity::Solid)
                    .with_health(300)
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
        assert_eq!(registry.register_layer(AIR_LAYER), AIR);
        // A gun that answers only what flies, for the body that carries one
        // alongside a weapon of its own.
        let flak = registry.register_turret(
            "flak",
            TurretDef::new(
                Weapon::new(AIR, Delivery::Instant, None),
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
                    .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                    .with_health(60)
                    .with_sight_range(10)
                    .with_stat(EntityStatId::AIM_RATE, FixedU64::from_num(360))
                    .with_attack(weapon(GROUND), 10, 4, 8, 6, 3)
                    .with_turrets([TurretMount::new(gun, CellPos::new(0, 0), CellSize::ONE)]),
            );
        }
        // Something that flies, answerable only on the air layer.
        registry.register(
            EntityTypeDef::new("kite")
                .with_location(GROUND, CellSize::ONE, Solidity::Passable)
                .with_targetable(AIR)
                .with_health(30)
                .with_dying(3, None),
        );
        // A keep with one gun on its far corner, throwing something slow enough to
        // watch: where a shot leaves from is only visible while it is in the air.
        let corner_gun = registry.register_turret(
            "corner_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Projectile(lob), None),
                TurretStats::default(),
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("shell_keep")
                .with_location(GROUND, CellSize::new(5, 5), Solidity::Solid)
                .with_health(300)
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
        let gun_range = registry.register_entity_stat("gun_range");
        let long_gun = registry.register_turret(
            "long_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None),
                TurretStats {
                    range: gun_range,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("longarm")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(60)
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(60)
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(60)
                .with_sight_range(12)
                .with_attack(
                    AttackDef::new(Weapon::new(GROUND, Delivery::Projectile(lob), None)),
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
        let prowl_notice = registry.register_entity_stat("prowl_notice");
        let prowl_gun = registry.register_turret(
            "prowl_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None),
                TurretStats {
                    acquire_range: prowl_notice,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("prowler")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(40)
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
        let anti_air_range = registry.register_entity_stat("anti_air_range");
        let anti_air_gun = registry.register_turret(
            "anti_air_gun",
            TurretDef::new(
                Weapon::new(AIR, Delivery::Instant, None),
                TurretStats {
                    range: anti_air_range,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("escort")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(40)
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(500)
                .with_dying(3, None),
        );
        // Registered before `dummy`, which leaves it as a corpse.
        registry.register(
            EntityTypeDef::new("bones")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(3, Some("bones")),
        );
        // A wide attacker, so a chase threads a 2x2 chaser footprint: reach
        // is rect to rect, measured from its nearest edge.
        registry.register(
            EntityTypeDef::new("ballista")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_sight_range(12)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(80)
                .with_dying(3, None)
                .with_attack(weapon(GROUND), 10, 2, 2, 4, 2),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();

    app.world_mut().resource_mut::<GameSession>().start();
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
    app.world_mut().resource_mut::<GameSession>().start();
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
    app.world_mut().resource_mut::<GameSession>().start();
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
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 20)])
                .with_build_time(10)
                .with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::from_num(8)),
        );
        // Registered before `lodge`, which trains it.
        registry.register(
            EntityTypeDef::new("settler")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_cost([("gold", 10)])
                .with_train_time(10)
                .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ONE),
        );
        // Also trains the supply-free `worker`, so the gate's exemption for
        // costless types can be watched from the same trainer.
        registry.register(
            EntityTypeDef::new("lodge")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_trainer(["settler", "worker"]),
        );
        // Works from outside the site, so a camp going up stays observable.
        registry.register(
            EntityTypeDef::new("pioneer")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["camp"], BuilderAttendance::Crew(WorkPresence::Present)),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None),
        );
        let drums_haste = registry.register_player_buff(
            "drums_haste",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifier {
                    stat: EntityStatId::SPEED,
                    op: ModifierOp::PercentAdd,
                    magnitude: FixedI64::from_num(1),
                }],
                duration: Some(10),
                stack_rule: StackRule::Refresh,
            },
        );
        registry.register_skill(
            "drums",
            SkillDef {
                cooldown: 20,
                caster: SkillCaster::Player {
                    cost: costs::cost([("gold", 10)]),
                    effect: PlayerCastEffect::ApplyBuff(drums_haste),
                },
                requires: Vec::new(),
            },
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// App with the research roster — a `lab` (tagged "workshop") hosting the
/// `smithing` upgrade (a permanent +5 damage army-wide buff) and the `tactics`
/// unlock that requires it, plus a `guardhouse` training a free `pikeman`, a
/// `halberdier` gated on the `smithing` research, and a `knight` gated on the
/// "workshop" tag — one human player, session started.
pub fn research_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register_tag("workshop");
        let sharp_blades = registry.register_player_buff(
            "sharp_blades",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifier {
                    stat: EntityStatId::DAMAGE,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(5),
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        let smithing = registry.register_research(
            "smithing",
            ResearchDef::new(
                costs::cost([("gold", 30)]),
                10,
                Some(sharp_blades),
                Vec::<String>::new(),
            ),
        );
        let tactics = registry.register_research(
            "tactics",
            ResearchDef::new(costs::cost([("gold", 20)]), 10, None, ["smithing"]),
        );
        let soldier = |name: &str| {
            EntityTypeDef::new(name)
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
                .with_attack(weapon(GROUND), 10, 1, 1, 4, 2)
                .with_cost([("gold", 10)])
                .with_train_time(5)
        };
        registry.register(soldier("pikeman"));
        registry.register(soldier("halberdier").with_requires(["smithing"]));
        registry.register(soldier("knight").with_requires(["workshop"]));
        registry.register(
            EntityTypeDef::new("lab")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_researcher([smithing, tactics])
                .with_tags(["workshop"]),
        );
        registry.register(
            EntityTypeDef::new("guardhouse")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_trainer(["pikeman", "halberdier", "knight"]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
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
        registry.register_tag("infantry");
        registry.register(
            EntityTypeDef::new("rifleman")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
                .with_attack(weapon(GROUND), 10, 3, 5, 4, 2)
                .with_sight_range(8)
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
        );
        registry.register(
            EntityTypeDef::new("grunt")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::from_num(2)),
        );
        registry.register(
            EntityTypeDef::new("civilian")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_tags(["infantry"]),
        );
        let carrier = |name: &str, boarding: BoardingPolicy| {
            EntityTypeDef::new(name)
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(60)
                .with_dying(2, None)
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::from_num(2))
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::from_num(3))
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::from_num(2))
                .with_transporter(
                    ["infantry"],
                    boarding,
                    PassengerFate::Destroy,
                    PassengerConduct::Shelter,
                )
        };
        registry.register(carrier("wagon", BoardingPolicy::Own));
        registry.register(carrier("ferry", BoardingPolicy::Allies));
        // A rider whose only weapon is a turret, for the rule that a passenger
        // fights with what it points itself: a turret is mounted on a body that
        // stands somewhere, and a passenger stands nowhere.
        // Its gun reads a stat of its own, the way content declares one for a
        // second weapon: a body weapon's numbers are not there to be read.
        let rider_damage = registry.register_entity_stat("rider_damage");
        let rider_gun = registry.register_turret(
            "rider_gun",
            TurretDef::new(
                Weapon::new(GROUND, Delivery::Instant, None),
                TurretStats {
                    damage: rider_damage,
                    ..TurretStats::default()
                },
                WeaponConduct::Halts,
            ),
        );
        registry.register(
            EntityTypeDef::new("gun_rider")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
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
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(200)
                .with_dying(2, None)
                .with_sight_range(8)
                .with_stat(EntityStatId::CARGO_CAPACITY, FixedU64::from_num(4))
                .with_stat(EntityStatId::LOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::UNLOAD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::LOAD_PERIOD, FixedU64::ZERO)
                .with_stat(EntityStatId::UNLOAD_PERIOD, FixedU64::ZERO)
                .with_transporter(
                    ["rifleman", "gun_rider"],
                    BoardingPolicy::Own,
                    PassengerFate::Eject,
                    PassengerConduct::Fight,
                ),
        );
        registry.register(
            EntityTypeDef::new("bombard")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(40)
                .with_dying(2, None)
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
    app.world_mut().resource_mut::<GameSession>().start();
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
                .with_location(GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_health(100),
        );
        registry.register(
            EntityTypeDef::new("runner")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.3),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.25),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(9),
                    FixedU64::from_num(18),
                )
                .with_health(20),
        );
        registry.register(
            EntityTypeDef::new("ponderous")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.25),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(9),
                    FixedU64::ONE,
                )
                .with_stat(EntityStatId::PIVOT_ANGLE, FixedU64::from_num(90))
                .with_health(20),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
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

/// Issues an unload command for `transport` as the local player.
pub fn unload(app: &mut App, transport: SimulationId, at: Option<FixedUVec2>) {
    push_command(
        app,
        PlayerCommand::Unload {
            transport,
            at,
            flush: true,
        },
    );
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

/// The handle the given research name resolves to in the app's registry.
pub fn research_id(app: &App, name: &str) -> ResearchId {
    app.world()
        .resource::<ContentRegistry>()
        .research(name)
        .unwrap_or_else(|| panic!("research '{name}' is registered"))
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
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
                .with_attack(weapon(GROUND), 10, 1, 1, 4, 2)
                .with_cost([("gold", 30)])
                .with_train_time(4),
        );
        // A wide continuous mover: 2x2 footprint with the largest legal body
        // circle (radius = half the narrow side), for mixed-size contact and
        // claim tests.
        registry.register(
            EntityTypeDef::new("wagon")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(60)
                .with_dying(2, None),
        );
        // A heavy continuous mover on a soldier's footprint, for contact tests
        // that need weight told apart from size.
        registry.register(
            EntityTypeDef::new("ox")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(4),
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(60)
                .with_dying(2, None),
        );
        // Registered before `worker`, which builds it.
        registry.register(
            EntityTypeDef::new("depot")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 50)])
                .with_build_time(6)
                .with_resource_storage(["gold", "wood"]),
        );
        registry.register(
            EntityTypeDef::new("worker")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_cost([("gold", 10)])
                .with_train_time(2)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_builder(["depot"], BuilderAttendance::Crew(WorkPresence::Hidden))
                .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Hidden))]),
        );
        // Same catalogue as `worker`, but it works from outside the site — the pair
        // is what makes `WorkPresence` observable.
        registry.register(
            EntityTypeDef::new("mason")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["depot"], BuilderAttendance::Crew(WorkPresence::Present)),
        );
        // Same catalogue again, and any number of them can crowd one site.
        registry.register(
            EntityTypeDef::new("carpenter")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["depot"],
                    BuilderAttendance::Crew(WorkPresence::PresentStacking),
                ),
        );
        // Same catalogue once more, but it only places the site: the depot
        // advances itself from there.
        registry.register(
            EntityTypeDef::new("architect")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["depot"], BuilderAttendance::Unattended),
        );
        // Same catalogue, and the site is what becomes of it: placing a depot
        // spends the larva.
        registry.register(
            EntityTypeDef::new("larva")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::SUPPLY_COST, FixedU64::ONE)
                .with_builder(["depot"], BuilderAttendance::Consumed),
        );
        registry.register(
            EntityTypeDef::new("lumberjack")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([("wood", HarvestData::new(5, 2, WorkPresence::Present))]),
        );
        // Works a seam from three cells back, which says nothing about how close it
        // has to get to put the load down.
        registry.register(
            EntityTypeDef::new("prospector")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::from_num(3))
                .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Present))]),
        );
        // Same trade as `lumberjack`, but a stand takes as many axes as turn up — and
        // a trip long enough to watch a crew form and break up while it lasts.
        registry.register(
            EntityTypeDef::new("logger")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([(
                    "wood",
                    HarvestData::new(5, 8, WorkPresence::PresentStacking),
                )]),
        );
        registry.register(
            EntityTypeDef::new("barracks")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_sight_range(8)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 40)])
                .with_build_time(4)
                .with_trainer(["soldier"]),
        );
        // A plain obstacle, for walling sources off.
        registry.register(
            EntityTypeDef::new("boulder")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_health(50),
        );
        // Carries either resource, so only an order's kind lock keeps a wood
        // trip off the gold.
        registry.register(
            EntityTypeDef::new("forager")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_sight_range(40)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([
                    ("gold", HarvestData::new(5, 2, WorkPresence::Present)),
                    ("wood", HarvestData::new(5, 2, WorkPresence::Present)),
                ]),
        );
        registry.register(
            EntityTypeDef::new("mine")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("tree")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_resource_source("wood", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("geyser")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Persist),
        );
        registry.register(
            EntityTypeDef::new("ghost")
                .with_location(GROUND, CellSize::ONE, Solidity::Passable)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(20),
        );
        // A soldier variant that notices enemies well beyond its weapon range,
        // for the stance and auto-engagement suites.
        registry.register(
            EntityTypeDef::new("sentry")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
                .with_attack(weapon(GROUND), 10, 1, 5, 4, 2)
                // Sees farther than it auto-engages, so its circular vision
                // covers everything within acquisition range.
                .with_sight_range(8),
        );
        // A ranged sentry, for suites that need hits without an adjacent chaser.
        registry.register(
            EntityTypeDef::new("archer")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(
                    FixedU64::from_num(0.5),
                    FixedU64::from_num(0.5),
                    FixedU64::ONE,
                    FixedU64::from_num(360),
                    FixedU64::from_num(360),
                )
                .with_health(30)
                .with_dying(2, None)
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

    let mut nav_grid = NavGrid::new(32, 32);
    nav_grid.add_layer(GROUND);
    let session = GameSession::configured(
        local,
        slots,
        "test",
        authority,
        DropPolicy::Automatic,
        FinishPolicy::Endless,
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
            &[],
        ),
    ));
    app.add_plugins(NetworkPlugin);
    // Supplies idle frames for AI slots with no installed runtime, as in a
    // real game; a no-op for the all-human rosters.
    app.add_plugins(ferrets_bevy_plugin::ai::AiPlugin);
    ferrets_bevy_plugin::install_network_session(app.world_mut(), net);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        assert_eq!(registry.register_layer(GROUND_LAYER), GROUND);
        registry.register(harness_soldier());
        registry.register(harness_base());
        registry.validate();
    }
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// The one mobile entity type the harness games use. Armed, so a game can
/// destroy a building through the command pipeline; nothing attacks unordered.
pub fn harness_soldier() -> EntityTypeDef {
    EntityTypeDef::new("soldier")
        .with_location(GROUND, CellSize::ONE, Solidity::Solid)
        .with_sight_range(8)
        .with_movement(
            FixedU64::from_num(0.5),
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
        .with_health(30)
        .with_dying(2, None)
        .with_attack(weapon(GROUND), 10, 1, 1, 4, 2)
}

/// A standing building — the presence the `LastStanding` rule counts. Immobile,
/// destructible, no combat of its own.
pub fn harness_base() -> EntityTypeDef {
    EntityTypeDef::new("base")
        .with_location(GROUND, CellSize::ONE, Solidity::Solid)
        .with_health(30)
        .with_dying(2, None)
        .with_tags(["building"])
}
