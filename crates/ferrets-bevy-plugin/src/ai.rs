//! Bevy wiring for scripted AI players.
//!
//! Bridges live AI runtimes to the simulation each `FixedUpdate`:
//! [`supply_unmanned_input`] idles every locally-sourced AI slot with no brain,
//! and [`supply_ai_input`] runs each locally-sourced AI player's think on its
//! cadence, committing the returned commands into the input queue like any other
//! frame source. Which nodes source an AI slot is the session's
//! [`AiHosting`](ferrets_simulation::session::ai_hosting::AiHosting)
//! policy. Add the plugin for every game: idling costs nothing, and thinking
//! starts only once a game installs [`AiRuntimes`].

use std::collections::BTreeMap;

use bevy::ecs::world::EntityRef;
use bevy::prelude::*;
use ferrets_pathfinder::nav_pos::NavPos;
use ferrets_script::ai::AiRuntime;
use ferrets_script::ai::view::game::{EntityView, GameView};
use ferrets_simulation::{
    components::{
        build::UnderConstructionComponent,
        entity_info::EntityInfoComponent,
        health::HealthComponent,
        hidden::HiddenComponent,
        location::LocationComponent,
        order_queue::OrderQueueComponent,
        owner::OwnerComponent,
        resource::{ResourceCarrierComponent, ResourceSourceComponent},
        stance::StanceComponent,
        train::TrainQueueComponent,
    },
    entity_index::EntityIndex,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    map::Map,
    resources::PlayerResources,
    session::{GameSession, player_slot::PlayerId, player_type::PlayerType},
    simulation_id::SimulationId,
};

use crate::network::NetworkSession;
use crate::{SimulationSet, replay, session_is_active, session_is_not_paused, systems};

/// Ticks a think is offset per player id, spreading the work so co-hosted
/// brains do not all think on the same tick.
const THINK_STAGGER: u32 = 5;

/// The live AI runtimes, keyed by the player each one drives. A `NonSend`
/// resource because a script runtime is single-threaded; absent until a game
/// installs AI. The AI systems gate on [`AiActive`].
pub struct AiRuntimes(pub BTreeMap<PlayerId, Box<dyn AiRuntime>>);

/// A `Send` marker that [`AiRuntimes`] is installed. The AI systems gate on
/// this rather than on the runtimes directly, because run conditions may be
/// evaluated on worker threads where a `NonSend` resource must not be touched.
#[derive(Resource)]
pub struct AiActive;

/// Installs the AI runtimes: the `NonSend` map plus its `Send` marker. Call at
/// game start once the session's AI slots are known.
pub fn install_ai_runtimes(world: &mut World, runtimes: AiRuntimes) {
    world.insert_non_send_resource(runtimes);
    world.insert_resource(AiActive);
}

/// Removes the AI runtimes and their marker. Call at game teardown so a stale
/// brain never leaks into the next session.
pub fn remove_ai_runtimes(world: &mut World) {
    world.remove_non_send_resource::<AiRuntimes>();
    world.remove_resource::<AiActive>();
}

/// Whether this node is the session host. A local game's single node is its
/// own host.
pub fn is_session_host(world: &World) -> bool {
    world
        .get_non_send_resource::<NetworkSession>()
        .is_none_or(|net| net.0.is_host_node())
}

/// The AI players this node sources, with their races, in ascending id order.
/// Which nodes source an AI slot follows the session's hosting mode. An AI
/// that is out of the game needs no input for any tick, so its brain stops
/// thinking.
pub fn sourced_ai_players(world: &World) -> Vec<(PlayerId, String)> {
    let is_host = is_session_host(world);
    let session = world.resource::<GameSession>();
    session
        .slots()
        .iter()
        .filter(|slot| slot.player_type() == Some(PlayerType::Ai))
        .filter(|slot| !session.is_player_out(slot.id()))
        .filter(|slot| session.sources_locally(slot, is_host))
        .map(|slot| (slot.id(), slot.race().unwrap_or_default().to_string()))
        .collect()
}

/// Supplies input frames for scripted AI players and idles unmanned slots.
///
/// Requires [`SimulationPlugin`](crate::SimulationPlugin). Registers the AI
/// systems unconditionally; thinking starts only once [`AiRuntimes`] is
/// installed, so this plugin is safe to add for every game.
#[derive(Default)]
pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        // Both are frame sources, so they run before flush_input like net_receive
        // and supply_replay_input, and keep running while the session is blocked
        // (push_frame is idempotent; supply_ai_input additionally guards its
        // thinks). During replay playback the replay is the sole frame source,
        // so both are silenced.
        app.add_systems(
            FixedUpdate,
            supply_unmanned_input
                .in_set(SimulationSet)
                .before(systems::flush_input)
                .run_if(
                    session_is_active
                        .and(session_is_not_paused)
                        .and(not(resource_exists::<replay::ReplayPlayback>)),
                ),
        );
        app.add_systems(
            FixedUpdate,
            supply_ai_input
                .in_set(SimulationSet)
                .after(supply_unmanned_input)
                .before(systems::flush_input)
                .run_if(
                    session_is_active
                        .and(session_is_not_paused)
                        .and(not(resource_exists::<replay::ReplayPlayback>))
                        .and(resource_exists::<AiActive>),
                ),
        );
    }
}

/// Supplies idle frames for every locally-sourced AI slot while no
/// [`AiRuntimes`] is installed, so a failed script degrades to an idle AI
/// instead of stalling lockstep. Unoccupied slots and players out of the game
/// need nothing: no tick requires their input.
pub fn supply_unmanned_input(
    mut frames: ResMut<InputFrames>,
    session: Res<GameSession>,
    net: Option<NonSend<NetworkSession>>,
    ai_active: Option<Res<AiActive>>,
) {
    let is_host = net.is_none_or(|net| net.0.is_host_node());
    let target = session.tick() + SYNC_LATENCY;

    for slot in session.slots() {
        if session.is_player_out(slot.id()) || !session.sources_locally(slot, is_host) {
            continue;
        }
        let unmanned = slot.player_type() == Some(PlayerType::Ai) && ai_active.is_none();
        if unmanned {
            frames.push_frame(PlayerFrame::idle(slot.id(), target));
        }
    }
}

/// The AI players' frame source: runs each locally-sourced AI's think on its
/// cadence and commits the returned commands; every other tick gets an idle
/// frame so lockstep stays fed.
///
/// A think runs at most once per (player, tick): while the session is blocked
/// this system reruns with the tick frozen, and re-running a think would
/// advance the script's persistent state a node-local number of times — the
/// committed-frame check short-circuits the reruns. A think error is logged and
/// treated as an idle frame; it is identical on every node that computes this
/// AI, so the session keeps running.
pub fn supply_ai_input(world: &mut World) {
    let session = world.resource::<GameSession>();
    let tick = session.tick();
    let target = tick + SYNC_LATENCY;
    let ai_players = sourced_ai_players(world);

    // Take the runtimes out of the world so the view can borrow it while the
    // Lua state is called into.
    let Some(mut runtimes) = world.remove_non_send_resource::<AiRuntimes>() else {
        return;
    };
    for (player, race) in ai_players {
        if world.resource::<InputFrames>().has_frame(player, target) {
            continue;
        }
        let commands = match runtimes.0.get_mut(&player) {
            Some(runtime) if is_think_tick(tick, player, runtime.period()) => {
                let view = game_view(world, player, &race);
                match runtime.think(&view) {
                    Ok(commands) => commands,
                    Err(error) => {
                        eprintln!("ai think failed for player {player} at tick {tick}: {error}");
                        Vec::new()
                    }
                }
            }
            _ => Vec::new(),
        };
        world.resource_mut::<InputFrames>().push_frame(PlayerFrame {
            player,
            tick: target,
            commands,
        });
    }
    world.insert_non_send_resource(runtimes);
}

/// Whether `player`'s brain thinks on `tick`. A pure function of the tick,
/// player id, and declared period, so every node computes the same cadence.
fn is_think_tick(tick: u32, player: PlayerId, period: u32) -> bool {
    (tick + u32::from(player) * THINK_STAGGER).is_multiple_of(period)
}

/// Snapshots everything `player`'s brain observes this tick. Entity lists are
/// in ascending simulation-id order; only integers, strings, and booleans are
/// captured, so the snapshot is identical on every node with identical state.
pub fn game_view(world: &World, player: PlayerId, race: &str) -> GameView {
    let map = world.resource::<Map>();
    let resources = world
        .resource::<PlayerResources>()
        .iter()
        .filter(|(owner, _, _)| *owner == player)
        .map(|(_, kind, amount)| (kind.to_string(), amount))
        .collect();

    let session = world.resource::<GameSession>();
    let mut my_entities = Vec::new();
    let mut ally_entities = Vec::new();
    let mut enemy_entities = Vec::new();
    let mut neutral_entities = Vec::new();
    for (id, entity) in world.resource::<EntityIndex>().alive_entries() {
        let entity_ref = world.entity(entity);
        let owner = entity_ref
            .get::<OwnerComponent>()
            .map(|owner| owner.player());
        // A brain keeps seeing its own hidden entities (a worker inside a mine
        // still counts toward its economy); other players' hidden entities are
        // omitted, matching what its commands could target.
        let hidden = entity_ref.contains::<HiddenComponent>();
        if hidden && owner != Some(player) {
            continue;
        }
        let view = entity_view(&entity_ref, id, hidden);
        match owner {
            Some(owner) if owner == player => my_entities.push(view),
            Some(owner) if session.are_allied(player, owner) => ally_entities.push(view),
            Some(_) => enemy_entities.push(view),
            None => neutral_entities.push(view),
        }
    }

    GameView {
        tick: session.tick(),
        player: u32::from(player),
        race: race.to_string(),
        map_width: map.width(),
        map_height: map.height(),
        resources,
        my_entities,
        ally_entities,
        enemy_entities,
        neutral_entities,
    }
}

/// Snapshots one entity to its integer view.
fn entity_view(entity: &EntityRef, id: SimulationId, hidden: bool) -> EntityView {
    let cell = entity
        .get::<LocationComponent>()
        .map_or(NavPos::new(0, 0), |location| {
            NavPos::from(location.position)
        });
    EntityView {
        id: id.0,
        type_name: entity
            .get::<EntityInfoComponent>()
            .map_or_else(String::new, |info| info.type_name().to_string()),
        x: cell.x,
        y: cell.y,
        health: entity.get::<HealthComponent>().map(|h| h.current()),
        idle: entity
            .get::<OrderQueueComponent>()
            .is_none_or(|queue| queue.front().is_none()),
        hidden,
        carrying: entity
            .get::<ResourceCarrierComponent>()
            .and_then(|carrier| carrier.kind.clone().map(|kind| (kind, carrier.amount))),
        train_queue: entity
            .get::<TrainQueueComponent>()
            .map_or_else(Vec::new, |queue| queue.0.iter().cloned().collect()),
        under_construction: entity.contains::<UnderConstructionComponent>(),
        stance: entity
            .get::<StanceComponent>()
            .map(|stance| stance.0.name().to_string()),
        resource_amount: entity
            .get::<ResourceSourceComponent>()
            .map(|source| source.amount),
    }
}
