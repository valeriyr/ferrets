#![allow(dead_code)]

use bevy::{ecs::entity::EntityNotSpawnedError, prelude::*};
use ferrets_bevy_plugin::{PendingInput, SimulationPlugin};
use ferrets_content::{
    costs,
    entity_buffs::{EntityBuffDef, EntityBuffId},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    morph::{MorphCancel, MorphPlacement, MorphTime, MorphTransition},
    player_buffs::PlayerBuffDef,
    registry::ContentRegistry,
    research::{ResearchDef, ResearchId},
    resource::{DepletionPolicy, HarvestData},
    skills::{EntityCastCost, PlayerCastEffect, SkillCaster, SkillDef},
    splash::SplashShape,
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
    transport::{BoardingPolicy, PassengerConduct, PassengerFate},
    work::WorkPresence,
};
use ferrets_geometry::{
    cell_pos::CellPos, cell_rect::CellRect, cell_size::CellSize, projection::Projection,
};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    mover_shape::MoverShape,
    nav_grid::{LayerId, NavGrid},
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
    },
    entity_def,
    input::{InputFrames, PlayerFrame},
    map::Map,
    movement_model::MovementModel,
    order::AttackTarget,
    resources::PlayerResources,
    selection::Selection,
    session::{
        GameSession,
        ai_hosting::AiHosting,
        authority::Authority,
        drop_policy::DropPolicy,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot},
        player_type::PlayerType,
    },
    simulation_id::SimulationId,
    spawn,
};

/// The single navigation layer the harness content declares.
pub const GROUND_LAYER: &str = "ground";
/// The id [`GROUND_LAYER`] resolves to — it is the first registered layer.
pub const GROUND: LayerId = LayerId::new(1);

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
            0,
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
        ),
    ));
    app.insert_resource(registry);
    app
}

pub fn pos(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// Spawns one entity of `type_name` at `(x, y)` owned by `player`, panicking when
/// the spawn is refused.
pub fn spawn_owned(
    app: &mut App,
    type_name: &str,
    x: u32,
    y: u32,
    player: PlayerId,
) -> (Entity, SimulationId) {
    spawn::spawn_entity(app.world_mut(), type_name, pos(x, y), Some(player))
        .unwrap_or_else(|| panic!("{type_name} spawns"))
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(1, None)
                .with_attack(10, 1, 3, 2, 1)
                .with_targets(GROUND)
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
        app.world_mut().insert_resource(Map::with_hierarchy_shapes(
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("wagon")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_movement(FixedU64::from_num(0.3), FixedU64::ONE)
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_cost([("gold", 10)])
                .with_train_time(20)
                .with_morphs([
                    MorphTransition::new(
                        "giant",
                        MorphTime::Constant(10),
                        MorphPlacement::Reserve,
                        MorphCancel::Refundable,
                        Vec::new(),
                        Vec::<String>::new(),
                    ),
                    MorphTransition::new(
                        "husk",
                        MorphTime::Constant(0),
                        MorphPlacement::Revalidate,
                        MorphCancel::Committed,
                        vec![EntityCastCost::Health(FixedU64::from_num(10))],
                        Vec::<String>::new(),
                    ),
                    MorphTransition::new(
                        "wisp",
                        MorphTime::Constant(10),
                        MorphPlacement::Revalidate,
                        MorphCancel::Forfeit,
                        Vec::new(),
                        Vec::<String>::new(),
                    ),
                ]),
        );
        registry.register(
            EntityTypeDef::new("giant")
                .with_location(GROUND, CellSize::new(3, 3), Solidity::Solid)
                .with_movement(FixedU64::from_num(0.3), FixedU64::ONE)
                .with_health(60)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("husk")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(10)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("wisp")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_morphs([MorphTransition::new(
                    "whelp",
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
                .with_movement(FixedU64::from_num(0.3), FixedU64::ONE)
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
            if player != local_player {
                world
                    .resource_mut::<InputFrames>()
                    .push_frame(PlayerFrame::idle(player, current_tick));
            }
        }

        world.run_schedule(FixedUpdate);
    }
}

/// Runs exactly `steps` fixed updates without synthesizing any input frames —
/// for suites whose registered frame sources already feed every slot.
pub fn run_steps(app: &mut App, steps: u32) {
    for _ in 0..steps {
        app.world_mut().run_schedule(FixedUpdate);
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
pub fn cell_of(world: &mut World, entity: Entity) -> CellPos {
    CellPos::from(position_of(world, entity))
}

/// The entity's continuous position — sub-cell precise, where [`cell_of`]
/// floors it to a cell.
pub fn position_of(world: &mut World, entity: Entity) -> FixedUVec2 {
    world.get::<LocationComponent>(entity).unwrap().position
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
pub fn wound(app: &mut App, entity: Entity, amount: f64) {
    app.world_mut()
        .get_mut::<HealthComponent>(entity)
        .unwrap()
        .apply_damage(FixedU64::from_num(amount));
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
    magnitude: f64,
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
                    magnitude: FixedI64::from_num(magnitude),
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
    app.world_mut().insert_resource(Map::with_hierarchy_shapes(
        "test",
        projection,
        model,
        grid,
        vec![],
        &[MoverShape::point(GROUND)],
    ));
}

/// App with the combat content roster — an attacking soldier (50 hp, 3-tick
/// dying phase) and an immobile dummy that leaves decaying bones — one human
/// player, session started.
pub fn combat_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(50)
                .with_dying(3, None)
                .with_attack(10, 1, 1, 4, 2)
                .with_targets(GROUND),
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::ONE)
                .with_health(80)
                .with_dying(3, None)
                .with_attack(10, 2, 2, 4, 2)
                .with_targets(GROUND),
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["camp"], WorkPresence::Present),
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 1, 4, 2)
                .with_targets(GROUND)
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 3, 5, 4, 2)
                .with_targets(GROUND)
                .with_sight_range(8)
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::ONE),
        );
        registry.register(
            EntityTypeDef::new("grunt")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_tags(["infantry"])
                .with_stat(EntityStatId::CARGO_SIZE, FixedU64::from_num(2)),
        );
        registry.register(
            EntityTypeDef::new("civilian")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_tags(["infantry"]),
        );
        let carrier = |name: &str, boarding: BoardingPolicy| {
            EntityTypeDef::new(name)
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
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
                    ["rifleman"],
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
                .with_attack(20, 6, 6, 4, 2)
                .with_targets(GROUND)
                .with_sight_range(10)
                .with_splash(
                    SplashShape::Circular,
                    vec![(2, FixedU64::from_num(0.5))],
                    GROUND,
                    true,
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
                .with_movement(FixedU64::from_num(0.3), FixedU64::from_num(0.5))
                .with_health(20),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
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
pub fn register_orders_content(app: &mut App) {
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register_resource("gold");
        registry.register_resource("wood");
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 1, 4, 2)
                .with_targets(GROUND)
                .with_cost([("gold", 30)])
                .with_train_time(4),
        );
        // A wide continuous mover: 2x2 footprint with the largest legal body
        // circle (radius = half the narrow side), for mixed-size contact and
        // claim tests.
        registry.register(
            EntityTypeDef::new("wagon")
                .with_location(GROUND, CellSize::new(2, 2), Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::ONE)
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_cost([("gold", 10)])
                .with_train_time(2)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_builder(["depot"], WorkPresence::Hidden)
                .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Hidden))]),
        );
        // Same catalogue as `worker`, but it works from outside the site — the pair
        // is what makes `WorkPresence` observable.
        registry.register(
            EntityTypeDef::new("mason")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["depot"], WorkPresence::Present),
        );
        // Same catalogue again, and any number of them can crowd one site.
        registry.register(
            EntityTypeDef::new("carpenter")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["depot"], WorkPresence::PresentStacking),
        );
        registry.register(
            EntityTypeDef::new("lumberjack")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
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
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(20),
        );
        // A soldier variant that notices enemies well beyond its weapon range,
        // for the stance and auto-engagement suites.
        registry.register(
            EntityTypeDef::new("sentry")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 5, 4, 2)
                .with_targets(GROUND)
                // Sees farther than it auto-engages, so its circular vision
                // covers everything within acquisition range.
                .with_sight_range(8),
        );
        // A ranged sentry, for suites that need hits without an adjacent chaser.
        registry.register(
            EntityTypeDef::new("archer")
                .with_location(GROUND, CellSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5), FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 3, 5, 4, 2)
                .with_targets(GROUND)
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
