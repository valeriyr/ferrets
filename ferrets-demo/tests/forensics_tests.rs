//! Scratch replay-forensics harness, run by hand against a recorded game:
//! rebuilds the skirmish from the replay header, replays it headless, and
//! reports movers that sat still while still under way.
//!
//! Run with: `FREP=replays/<stamp>.frep cargo test -p ferrets-demo --test
//! forensics_tests -- --ignored --nocapture`

use std::{collections::BTreeMap, fs::File, io::BufReader};

use bevy::prelude::*;
use ferrets_bevy_plugin::{ReplayPlugin, SimulationPlugin, install_replay_playback};
use ferrets_content::registry::ContentRegistry;
use ferrets_demo::setup;
use ferrets_geometry::projection::Projection;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_replay::{header::RecordedGame, replay::Replay};
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent, hidden::HiddenComponent, location::LocationComponent,
        movement::MoveComponent, order_queue::OrderQueueComponent,
    },
    entity_index::EntityIndex,
    map::Map,
    movement_model::MovementModel,
    session::{GameSession, ai_hosting::AiHosting, authority::Authority, drop_policy::DropPolicy},
    simulation_id::SimulationId,
};

/// One sampled tick of one entity's life.
struct Sample {
    tick: u32,
    position: FixedUVec2,
    queued: usize,
    front: Option<String>,
    /// Whether the entity was under way — carrying the movement state a walk
    /// runs on. An order that stands still by design, building or working a
    /// seam, drops it, which is what tells the two apart.
    under_way: bool,
}

#[test]
#[ignore = "manual forensics harness; set FREP to a .frep path"]
fn replay_forensics() {
    let path = std::env::var("FREP").expect("set FREP to a .frep path");
    let replay = Replay::read(BufReader::new(File::open(&path).expect("replay opens")))
        .expect("replay reads");
    let RecordedGame::Skirmish(skirmish) = replay.header().game.clone() else {
        panic!("this harness rebuilds skirmish recordings only");
    };
    let last = replay.last_tick().unwrap_or(0);
    println!("replay {path}: map '{}', {} ticks", skirmish.map, last);

    let mut data = ferrets_demo::map::by_name(&skirmish.map).expect("the replay's map is known");
    // The header does not record settings (a known deferred item), so the
    // demo defaults are assumed; a recording made under other settings
    // rebuilds wrong and shows up as the checksum mismatch printed below,
    // not as a simulation bug.
    data.set_movement_model(MovementModel::Continuous);
    data.set_projection(Projection::Isometric);
    let registry = content::load(&LuaEngine, ferrets_demo::content::CONTENT).expect("demo content");
    let game_map = Map::from_data(&data, &registry);

    let viewer = skirmish
        .slots
        .iter()
        .find(|slot| slot.player_type().is_some())
        .map_or(0, |slot| slot.id());
    let mut app = App::new();
    app.add_plugins(SimulationPlugin::new(
        GameSession::configured(
            viewer,
            skirmish.slots.clone(),
            skirmish.map.clone(),
            Authority::Host {
                ai_hosting: AiHosting::Replicated,
            },
            DropPolicy::Automatic,
            skirmish.finish_policy,
        ),
        game_map,
    ));
    app.add_plugins(ReplayPlugin);
    *app.world_mut().resource_mut::<ContentRegistry>() = registry;
    install_replay_playback(app.world_mut(), replay);
    setup::spawn_demo_scene(app.world_mut());

    // FOCUS=<simulation id> dumps that entity's full movement state per tick;
    // FOCUS_FROM/FOCUS_TO bound the dump's tick range.
    let focus: Option<u32> = std::env::var("FOCUS").ok().and_then(|id| id.parse().ok());
    let focus_from: u32 = std::env::var("FOCUS_FROM")
        .ok()
        .and_then(|tick| tick.parse().ok())
        .unwrap_or(0);
    let focus_to: u32 = std::env::var("FOCUS_TO")
        .ok()
        .and_then(|tick| tick.parse().ok())
        .unwrap_or(u32::MAX);

    let mut tracks: BTreeMap<SimulationId, (String, Vec<Sample>)> = BTreeMap::new();
    for _ in 0..last + 10 {
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().run_schedule(FixedLast);
        let world = app.world_mut();
        let tick = world.resource::<GameSession>().tick();
        if let Some(focus) = focus
            && (focus_from..=focus_to).contains(&tick)
            && let Some(entity) = world.resource::<EntityIndex>().alive(SimulationId(focus))
        {
            let entity_ref = world.entity(entity);
            let position = entity_ref
                .get::<LocationComponent>()
                .map(|location| location.position);
            let hidden = entity_ref.get::<HiddenComponent>().is_some();
            let front = entity_ref
                .get::<OrderQueueComponent>()
                .and_then(|queue| queue.0.front().map(|entry| format!("{:?}", entry.order)));
            let movement = entity_ref
                .get::<MoveComponent>()
                .map(|movement| {
                    format!(
                        "path {:?} corridor {} plan {:?} frustration {} wait {} best {} regaining {} avoid {} detoured {}",
                        movement.path,
                        movement.corridor.len(),
                        movement.plan,
                        movement.frustration,
                        movement.wait_ticks,
                        movement.best_distance,
                        movement.regaining,
                        movement.avoid_claims,
                        movement.detoured,
                    )
                });
            println!("t{tick} pos {position:?} hidden {hidden} front {front:?} move {movement:?}");
        }
        let entries = world.resource::<EntityIndex>().alive_entries();
        for (id, entity) in entries {
            let entity_ref = world.entity(entity);
            let Some(location) = entity_ref.get::<LocationComponent>() else {
                continue;
            };
            let Some(info) = entity_ref.get::<EntityInfoComponent>() else {
                continue;
            };
            let (queued, front) =
                entity_ref
                    .get::<OrderQueueComponent>()
                    .map_or((0, None), |queue| {
                        (
                            queue.0.len(),
                            queue.0.front().map(|entry| format!("{:?}", entry.order)),
                        )
                    });
            tracks
                .entry(id)
                .or_insert_with(|| (info.type_name().to_string(), Vec::new()))
                .1
                .push(Sample {
                    tick,
                    position: location.position,
                    queued,
                    front,
                    under_way: entity_ref.get::<MoveComponent>().is_some(),
                });
        }
    }

    let playback = app
        .world()
        .resource::<ferrets_bevy_plugin::ReplayPlayback>();
    println!(
        "playback done: {}, checksum mismatch: {:?}",
        playback.is_done(),
        playback.mismatch()
    );

    // TYPE=<substring> prints every 10th sample of matching entities' tracks.
    if let Ok(wanted) = std::env::var("TYPE") {
        for (id, (name, track)) in &tracks {
            if !name.contains(&wanted) {
                continue;
            }
            println!("track {name} {id:?}:");
            for sample in track.iter().step_by(10) {
                println!(
                    "  t{} ({}, {}) queued {} front {:?}",
                    sample.tick,
                    sample.position.x,
                    sample.position.y,
                    sample.queued,
                    sample.front.as_deref().unwrap_or("none"),
                );
            }
        }
    }

    // Stuck: >= 40 consecutive sampled ticks under way and < 0.05 net movement
    // from the window's start. Being under way is what makes it a fault rather
    // than a job: a builder raising a site and a worker at a seam both stand
    // still for far longer than this with an order in hand, and neither is
    // stuck.
    let threshold = FixedU64::from_num(0.05);
    for (id, (name, track)) in &tracks {
        let mut start = 0;
        while start < track.len() {
            if track[start].queued == 0 || !track[start].under_way {
                start += 1;
                continue;
            }
            let mut end = start;
            while end + 1 < track.len()
                && track[end + 1].queued > 0
                && track[end + 1].under_way
                && track[start].position.distance(track[end + 1].position) < threshold
            {
                end += 1;
            }
            if track[end].tick.saturating_sub(track[start].tick) >= 40 {
                println!(
                    "STUCK {name} {id:?}: ticks {}..{} at ({}, {}), front order {:?}",
                    track[start].tick,
                    track[end].tick,
                    track[start].position.x,
                    track[start].position.y,
                    track[start].front.as_deref().unwrap_or("none"),
                );
            }
            start = end + 1;
        }
    }
}
