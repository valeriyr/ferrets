//! Scratch replay-forensics harness, run by hand against a recorded game:
//! rebuilds the skirmish from the replay header, replays it headless, and
//! reports movers that sat still while still under way, sprites that faced
//! somewhere other than the step they took, and walks whose step swung from one
//! tick to the next.
//!
//! Run with: `FREP=replays/<stamp>.frep cargo test -p ferrets-demo --test
//! forensics_tests -- --ignored --nocapture`

use std::{collections::BTreeMap, fs::File, io::BufReader};

use bevy::prelude::*;
use ferrets_demo::playback;
use ferrets_math::{
    FixedI64, FixedU64,
    facing::{self, Facing},
    fixed_uvec2::FixedUVec2,
    fixed_vec2::FixedVec2,
};
use ferrets_replay::replay::Replay;
use ferrets_simulation::{
    components::{
        entity_info::EntityInfoComponent, hidden::HiddenComponent, location::LocationComponent,
        movement::MoveComponent, order_queue::OrderQueueComponent, owner::OwnerComponent,
        resource::ResourceCarrierComponent,
    },
    entity_index::EntityIndex,
    session::{GameSession, player_slot::PlayerId},
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
    /// The slot the entity answers to, so a report can be read per player.
    owner: Option<PlayerId>,
    /// The look direction written this tick, to hold against the step actually
    /// taken: a sprite points along this, so the two parting company is a unit
    /// facing one way and walking another.
    facing: Facing,
}

#[test]
#[ignore = "manual forensics harness; set FREP to a .frep path"]
fn replay_forensics() {
    let path = std::env::var("FREP").expect("set FREP to a .frep path");
    let replay = Replay::read(BufReader::new(File::open(&path).expect("replay opens")))
        .expect("replay reads");
    let mut rebuilt = playback::rebuild(replay).expect("the replay's game rebuilds");
    let last = rebuilt.last_tick.expect("the recording holds ticks");
    let app = &mut rebuilt.app;
    println!("replay {path}: {last} ticks");

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
        ferrets_bevy_plugin::run_tick(app.world_mut());
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
            let facing = entity_ref
                .get::<LocationComponent>()
                .map(|location| location.facing);
            let carried = entity_ref
                .get::<ResourceCarrierComponent>()
                .map(|carrier| format!("{:?} x{}", carrier.kind, carrier.amount));
            let form = entity_ref
                .get::<EntityInfoComponent>()
                .map(|info| info.type_name().to_string());
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
            let owner = entity_ref
                .get::<OwnerComponent>()
                .map(|owner| owner.player());
            println!(
                "t{tick} owner {owner:?} form {form:?} pos {position:?} facing {facing:?} carried {carried:?} hidden {hidden} front {front:?} move {movement:?}"
            );
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
                    owner: entity_ref
                        .get::<OwnerComponent>()
                        .map(|owner| owner.player()),
                    facing: location.facing,
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
            let owner = track.first().and_then(|sample| sample.owner);
            println!("track {name} {id:?} owner {owner:?}:");
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

    // Facing flick: the look the sprite is drawn at disagrees with the step the
    // body took, so the unit walks one way while pointing another.
    for (id, (name, track)) in &tracks {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for pair in track.windows(2) {
            let (before, after) = (&pair[0], &pair[1]);
            if after.tick != before.tick + 1 || !after.under_way {
                continue;
            }
            let Some(stepped) = stepped_sector(offset(before.position, after.position)) else {
                continue;
            };
            if stepped == sector(after.facing) {
                continue;
            }
            match runs.last_mut() {
                Some(run) if run.1 + 1 == after.tick => run.1 = after.tick,
                _ => runs.push((after.tick, after.tick)),
            }
        }
        for (from, to) in runs {
            println!("FLICK {name} {id:?}: ticks {from}..{to}, looked off the step taken");
        }
    }

    // Wobble: the step itself swinging from one tick to the next, whatever the
    // facing does with it. A walk that alternates between two directions reads
    // as a unit jinking on the spot — and since a unit looks along its step,
    // its sprite swings with it.
    for (id, (name, track)) in &tracks {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for window in track.windows(3) {
            let [before, between, after] = window else {
                continue;
            };
            if after.tick != before.tick + 2 || !after.under_way {
                continue;
            }
            let Some(was) = stepped_sector(offset(before.position, between.position)) else {
                continue;
            };
            let Some(now) = stepped_sector(offset(between.position, after.position)) else {
                continue;
            };
            // Neighboring directions are the gentle turn any walk makes; two
            // apart is a swing of a right angle or more.
            if turns_apart(was, now) < 2 {
                continue;
            }
            match runs.last_mut() {
                Some(run) if run.1 + 1 >= after.tick => run.1 = after.tick,
                _ => runs.push((after.tick, after.tick)),
            }
        }
        for (from, to) in runs {
            println!("WOBBLE {name} {id:?}: ticks {from}..{to}, the step swung a right angle");
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

/// The offset from one sampled position to the next, which unsigned positions
/// cannot hold themselves.
fn offset(from: FixedUVec2, to: FixedUVec2) -> FixedVec2 {
    FixedVec2::new(
        to.x.to_num::<FixedI64>() - from.x.to_num::<FixedI64>(),
        to.y.to_num::<FixedI64>() - from.y.to_num::<FixedI64>(),
    )
}

/// The shortest step whose direction the eye can read, in cells. Below it the
/// body is being nudged by its neighbours rather than walking, and which way it
/// looks while that happens says nothing.
const READABLE_STEP: FixedI64 = FixedI64::lit("0.05");

/// Which of the eight sprite directions a bearing falls in, numbered clockwise
/// from north.
///
/// The eight are what a viewer can tell apart, so they are also the resolution
/// worth reporting: finer than that would rank differences the screen cannot
/// show.
fn sector(facing: Facing) -> u8 {
    let eighth = facing::PER_TURN / 8;
    ((facing.to_bits() as u32 + eighth / 2) % facing::PER_TURN / eighth) as u8
}

/// The sector a step was taken along, or `None` when the step is too short to
/// point anywhere — below that the body is being nudged by its neighbours rather
/// than walking, and which way it goes while that happens says nothing.
fn stepped_sector(direction: FixedVec2) -> Option<u8> {
    if direction.x.abs() < READABLE_STEP && direction.y.abs() < READABLE_STEP {
        return None;
    }
    Facing::of(direction).map(sector)
}

/// How many sprite directions apart two looks are, the short way round the
/// eight.
fn turns_apart(one: u8, other: u8) -> u8 {
    let apart = one.abs_diff(other);
    apart.min(8 - apart)
}
