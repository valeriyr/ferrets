#![allow(dead_code)]

use bevy::ecs::entity::EntityNotSpawnedError;
use bevy::prelude::*;
use ferrets_bevy_plugin::{PendingInput, SimulationPlugin};
use ferrets_math::{FixedI64, FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_pathfinder::{
    astar,
    astar::Projection,
    nav_grid::{LayerId, NavGrid},
    nav_pos::NavPos,
    nav_size::NavSize,
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
    },
    content::{
        entity_buffs::{EntityBuffDef, EntityBuffId},
        entity_stats::EntityStatId,
        entity_type_def::EntityTypeDef,
        location::Solidity,
        player_buffs::PlayerBuffDef,
        registry::ContentRegistry,
        resource::{DepletionPolicy, HarvestData},
        skills::{PlayerCastEffect, SkillCaster, SkillDef},
        stack_rule::StackRule,
        stats::{EntityModifier, ModifierOp},
        work::WorkPresence,
    },
    entity_def,
    input::{InputFrames, PlayerFrame},
    map::Map,
    order::AttackTarget,
    resources::{self, PlayerResources},
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
        Map::new("test", Projection::Isometric, nav_grid, vec![]),
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
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(1, None)
                .with_attack(10, 1, 3, 2, 1)
                .with_sight_range(5),
        );
        registry.register(
            EntityTypeDef::new("critter")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(1)
                .with_dying(1, None),
        );
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_tags(["building"]),
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
pub fn cell_of(world: &mut World, entity: Entity) -> NavPos {
    NavPos::from(world.get::<LocationComponent>(entity).unwrap().position)
}

/// Marks or clears every cell of the map's ground layer, used to box a worker in.
pub fn set_all_cells_occupied(world: &mut World, occupied: bool) {
    let mut map = world.resource_mut::<Map>();
    let grid = map.nav_grid_mut();
    let (width, height) = (grid.width(), grid.height());
    for y in 0..height {
        for x in 0..width {
            grid.set_occupied(GROUND, NavPos::new(x, y), occupied);
        }
    }
}

/// Asserts `worker` is boxed in — hidden with its reveal queued — then frees `cell`
/// and checks the scheduled retry brings it back onto exactly that cell, dropping
/// both markers.
pub fn assert_reveal_deferred_then_lands_on(app: &mut App, worker: Entity, cell: NavPos) {
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
        .nav_grid_mut()
        .set_occupied(GROUND, cell, false);
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

/// Asserts `unit` stands within one cell of `building`'s footprint.
pub fn assert_adjacent_to_footprint(world: &mut World, unit: Entity, building: Entity) {
    let origin = cell_of(world, building);
    let size = entity_def::of(world, building).location.unwrap().size();
    let unit_cell = cell_of(world, unit);
    let nearest = NavPos::new(
        unit_cell.x.clamp(origin.x, origin.x + size.width - 1),
        unit_cell.y.clamp(origin.y, origin.y + size.height - 1),
    );
    assert!(
        astar::chebyshev(unit_cell, nearest) <= 1,
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

/// App with the combat content roster — an attacking soldier (50 hp, 3-tick
/// dying phase) and an immobile dummy that leaves decaying bones — one human
/// player, session started.
pub fn combat_app() -> App {
    let mut app = make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);

    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(50)
                .with_dying(3, None)
                .with_attack(10, 1, 1, 4, 2),
        );
        // Registered before `dummy`, which leaves it as a corpse.
        registry.register(
            EntityTypeDef::new("bones")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_dying(2, None),
        );
        registry.register(
            EntityTypeDef::new("dummy")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_health(20)
                .with_dying(3, Some("bones")),
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
                .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 20)])
                .with_build_time(10)
                .with_stat(EntityStatId::SUPPLY_PROVIDED, FixedU64::from_num(8)),
        );
        // Registered before `lodge`, which trains it.
        registry.register(
            EntityTypeDef::new("settler")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
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
                .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_trainer(["settler", "worker"]),
        );
        // Works from outside the site, so a camp going up stays observable.
        registry.register(
            EntityTypeDef::new("pioneer")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
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
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
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
                    cost: resources::cost([("gold", 10)]),
                    effect: PlayerCastEffect::ApplyBuff(drums_haste),
                },
            },
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
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
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 1, 4, 2)
                .with_cost([("gold", 30)])
                .with_train_time(4),
        );
        // Registered before `worker`, which builds it.
        registry.register(
            EntityTypeDef::new("depot")
                .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 50)])
                .with_build_time(6)
                .with_resource_storage(["gold", "wood"]),
        );
        registry.register(
            EntityTypeDef::new("worker")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
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
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["depot"], WorkPresence::Present),
        );
        // Same catalogue again, and any number of them can crowd one site.
        registry.register(
            EntityTypeDef::new("carpenter")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["depot"], WorkPresence::PresentStacking),
        );
        registry.register(
            EntityTypeDef::new("lumberjack")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::ONE)
                .with_resource_carrier([("wood", HarvestData::new(5, 2, WorkPresence::Present))]),
        );
        // Works a seam from three cells back, which says nothing about how close it
        // has to get to put the load down.
        registry.register(
            EntityTypeDef::new("prospector")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20)
                .with_dying(2, None)
                .with_stat(EntityStatId::HARVEST_RANGE, FixedU64::from_num(3))
                .with_resource_carrier([("gold", HarvestData::new(5, 2, WorkPresence::Present))]),
        );
        // Same trade as `lumberjack`, but a stand takes as many axes as turn up — and
        // a trip long enough to watch a crew form and break up while it lasts.
        registry.register(
            EntityTypeDef::new("logger")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
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
                .with_location(GROUND, NavSize::new(2, 2), Solidity::Solid)
                .with_health(100)
                .with_dying(2, None)
                .with_cost([("gold", 40)])
                .with_build_time(4)
                .with_trainer(["soldier"]),
        );
        registry.register(
            EntityTypeDef::new("mine")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("tree")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_resource_source("wood", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("geyser")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Persist),
        );
        registry.register(
            EntityTypeDef::new("ghost")
                .with_location(GROUND, NavSize::ONE, Solidity::Passable)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(20),
        );
        // A soldier variant that notices enemies well beyond its weapon range,
        // for the stance and auto-engagement suites.
        registry.register(
            EntityTypeDef::new("sentry")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 1, 5, 4, 2)
                // Sees farther than it auto-engages, so its circular vision
                // covers everything within acquisition range.
                .with_sight_range(8),
        );
        // A ranged sentry, for suites that need hits without an adjacent chaser.
        registry.register(
            EntityTypeDef::new("archer")
                .with_location(GROUND, NavSize::ONE, Solidity::Solid)
                .with_movement(FixedU64::from_num(0.5))
                .with_health(30)
                .with_dying(2, None)
                .with_attack(10, 3, 5, 4, 2)
                .with_sight_range(8),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
}
