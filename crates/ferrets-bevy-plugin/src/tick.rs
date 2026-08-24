//! The fixed tick loop's wall-clock cadence: how long a tick lasts, how far the
//! game is from keeping up with it, and driving the loop directly.
//!
//! The session's speed sets the cadence the game is *asking* for. What it gets is
//! that cadence or slower: a tick costing more than its share of the budget, a
//! peer's frames arriving with no room to spare, or a peer reporting that it
//! cannot hold the pace all throttle the loop, so the game runs uniformly slower
//! instead of hitching. None of it reaches the simulation — every duration in a
//! tick is counted in ticks, so cadence never enters a checksum.

use std::time::{Duration, Instant};

use bevy::{app::FixedMain, prelude::*};
use ferrets_math::FixedU64;
use ferrets_simulation::session::GameSession;

use crate::network::{FrameMargins, NetworkActive, PeerCapacities};

/// The shortest tick the cadence may be scaled down to, so an extreme speed
/// factor cannot ask for a zero-length timestep.
const MIN_TIMESTEP: Duration = Duration::from_nanos(1);
/// The share of a tick's budget its execution should occupy. Kept below one so
/// the spread of tick costs has room and whatever the game draws between ticks is
/// not starved: a tick that averages its whole budget stutters, because long and
/// short ticks do not cancel out.
pub const TARGET_LOAD: FixedU64 = FixedU64::lit("0.65");
/// The slowest the throttle may run the game, as a share of the cadence the game
/// installed. Expressed against that cadence rather than as a rate of its own, so
/// the engine still never states how long a tick is, and so the floor means the
/// same thing at every speed.
pub const MIN_NOMINAL_SHARE: FixedU64 = FixedU64::lit("0.1");
/// Weight of a new sample when folding it into the running tick cost.
const COST_SMOOTHING: FixedU64 = FixedU64::lit("0.15");
/// The throttle that changes nothing: the game runs at the speed it was asked
/// for. Also the ceiling, since that speed is one.
pub const NO_THROTTLE: FixedU64 = FixedU64::ONE;
/// The lead, in ticks, below which a peer's frames count as arriving late.
///
/// Steady transit latency lowers every frame's lead by the same amount without
/// ever threatening the loop — a frame that reliably lands a full tick before it
/// is needed never blocks anything, however far it travelled. So the healthy
/// range is not measured against the full send-ahead window: only a lead
/// shrinking under one tick — frames arriving barely before the loop consumes
/// them — means the sender (or its route) can no longer feed this node in time,
/// and easing the cadence off hands it more wall time per tick.
pub const MARGIN_HEADROOM: FixedU64 = FixedU64::ONE;
/// The most that another node's trouble may cost this one — whether its frames
/// arrive late or it reports that it cannot hold the pace.
///
/// Halving the cadence is generous accommodation; past that, the game is being
/// held hostage. A peer that cannot keep up even at this pace falls behind,
/// blocks the tick, and becomes the drop rule's business
/// ([`DropPolicy`](ferrets_simulation::session::drop_policy::DropPolicy) and
/// `detect_drops`, which have the grace window, the authority and the game's own
/// say) — so the throttle smooths what is survivable and refuses to decide what
/// is not. The deeper [`MIN_NOMINAL_SHARE`] floor is for this node's *own* cost,
/// where slowing down is the only recourse there is.
pub const MIN_PEER_THROTTLE: FixedU64 = FixedU64::lit("0.5");

/// The largest factor this layer will produce, and the longest tick it will
/// measure. Both are far past anything a game asks for, and keeping the
/// arithmetic under [`FixedU64`]'s ceiling is what lets it saturate rather than
/// wrap when a machine does something extraordinary.
pub const MAX_FACTOR: FixedU64 = FixedU64::lit("1024");
const MAX_MILLIS: FixedU64 = FixedU64::lit("60000");

const THOUSAND: FixedU64 = FixedU64::lit("1000");

/// The wall time one frame may spend inside a seek. What remains of the seek
/// carries over to the next frame, so the app keeps redrawing and reading input
/// while a long fast-forward runs instead of freezing until it lands.
const SEEK_FRAME_BUDGET: Duration = Duration::from_millis(30);

/// Whether the tick loop reacts to falling behind, and whether it takes part in
/// telling the peers about it. Insert to override; a game that says nothing gets
/// both.
#[derive(Resource)]
pub struct ThrottleConfig {
    /// Whether the cadence is lowered when a tick costs more than its budget, a
    /// peer's frames arrive with no room to spare, or a peer reports that it
    /// cannot hold the pace. Turned off, the game keeps the cadence it was asked
    /// for and falls behind in hitches instead of slowing down.
    pub enabled: bool,
    /// Whether this node publishes the speed it can sustain, and lets what peers
    /// publish cap its own cadence. Turned off it neither sends nor is
    /// constrained — for a game that would rather not spend control traffic on
    /// it, or would rather each node keep its own pace.
    pub share_capacity: bool,
}

impl Default for ThrottleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            share_capacity: true,
        }
    }
}

/// The cadence the game installed: the length of a tick at normal speed, which
/// the engine scales but never chooses.
///
/// Captured from `Time<Fixed>` the first time [`sync_fixed_timestep`] reads it.
/// That latch happens once, so a game that changes its tick rate after startup —
/// a lobby picking a rate per match, say — must insert this resource itself with
/// the new length; the scaled timestep the engine writes every frame is not a
/// cadence to re-derive it from.
#[derive(Resource, Default)]
pub struct NominalTimestep(pub Option<Duration>);

/// How the tick loop is being paced: what a tick is costing this node, and the
/// throttle that follows from it.
#[derive(Resource)]
pub struct TickPacing {
    /// When the step being measured began, and the tick the session stood at.
    start: Option<(Instant, u32)>,
    /// Smoothed wall time one tick takes to execute, in milliseconds. Seeded at
    /// zero: a game is presumed able to keep up until a tick proves otherwise.
    pub exec_millis: FixedU64,
    /// The throttle currently applied ([`NO_THROTTLE`] = the chosen speed in
    /// full).
    pub throttle: FixedU64,
}

impl Default for TickPacing {
    fn default() -> Self {
        Self {
            start: None,
            exec_millis: FixedU64::ZERO,
            throttle: NO_THROTTLE,
        }
    }
}

/// A marker that the current fixed step was driven by hand rather than by the
/// fixed loop — a seek, a single step, or a headless run.
#[derive(Resource)]
pub struct ManualTick;

/// A tick to fast-forward the session to, through a replay's recording or a live
/// local game's own simulation. Consumed by [`apply_seek`], which runs the
/// intervening ticks without presenting them, spread over as many frames as
/// [`SEEK_FRAME_BUDGET`] demands; the resource stands until the target is
/// reached, so its presence is also how "a seek is in progress" reads — and a
/// fresh request inserted mid-flight simply retargets it.
#[derive(Resource)]
pub struct Seek(pub u32);

/// The state of a seek in flight: whether the session was paused when it began,
/// captured on the seek's first frame and restored when it completes. Held
/// apart from [`Seek`] so a retarget mid-flight — a fresh request overwriting
/// that resource — cannot clobber it.
#[derive(Resource)]
pub(crate) struct SeekInProgress {
    paused_before: bool,
}

/// A request to advance exactly one tick while the session is paused — walking
/// through a moment tick by tick. Consumed by [`apply_step`].
#[derive(Resource)]
pub struct Step;

/// The fastest speed a tick costing `exec_millis` can be sustained at, against a
/// nominal tick length of `nominal_millis`: the factor whose budget the tick fills
/// to [`TARGET_LOAD`] and no further, capped at [`MAX_FACTOR`] so a tick too cheap
/// to measure cannot claim an absurd one.
///
/// Derived from the tick's cost alone, never from a throttle already in force — a
/// node publishing its throttled rate as its capability would ratchet the whole
/// game downwards, each peer's slowdown becoming the next one's evidence.
pub fn sustainable_factor(exec_millis: FixedU64, nominal_millis: FixedU64) -> FixedU64 {
    nominal_millis
        .saturating_mul(TARGET_LOAD)
        .saturating_div(exec_millis.max(FixedU64::DELTA))
        .min(MAX_FACTOR)
}

/// The throttle to run at, given what a tick costs here, the budget the chosen
/// speed allows it, that speed itself, the tightest lead peers' frames are
/// arriving with, and the slowest speed any peer says it can hold.
///
/// This node's own cost and the peers' shortfalls are floored differently, on
/// purpose: see [`MIN_NOMINAL_SHARE`] and [`MIN_PEER_THROTTLE`].
///
/// Never above [`NO_THROTTLE`], since the chosen speed is a ceiling, and never low enough
/// to take the game below [`MIN_NOMINAL_SHARE`] of its nominal cadence — except
/// where the chosen speed is already slower than that, which no throttle should
/// have to speed up.
pub fn throttle_for(
    exec_millis: FixedU64,
    budget_millis: FixedU64,
    speed: FixedU64,
    margin_ticks: Option<FixedU64>,
    peer_factor: Option<FixedU64>,
) -> FixedU64 {
    // What this node can afford, as a share of the cadence it was asked for.
    let affordable = sustainable_factor(exec_millis, budget_millis);
    // Frames arriving with less than a tick of headroom are the delivery side of
    // the same problem: easing off hands the late peer more wall time per tick.
    // Above the headroom the lead constrains nothing — steady transit latency
    // lowers it without ever threatening the loop (see [`MARGIN_HEADROOM`]).
    let delivered =
        margin_ticks.map_or(NO_THROTTLE, |margin| margin.saturating_div(MARGIN_HEADROOM));
    // A peer that cannot hold this cadence caps it for everybody, so the game
    // slows uniformly instead of hitching whenever that peer falls behind.
    let peer_capped = peer_factor.map_or(NO_THROTTLE, |peer| peer.saturating_div(speed));
    // Whatever the peers cost, only so far: past this the game stops
    // accommodating and lets the straggler block, which is what puts the decision
    // where it belongs.
    let peers = delivered.min(peer_capped).max(MIN_PEER_THROTTLE);

    let floor = MIN_NOMINAL_SHARE.saturating_div(speed).min(NO_THROTTLE);
    affordable.min(peers).clamp(floor, NO_THROTTLE)
}

/// Sets the fixed timestep from the session's speed and the throttle the last
/// ticks called for, against the cadence the game installed (run in [`First`]).
///
/// The budget every input is judged against is the *requested* timestep, not the
/// one currently in force — measuring against an already-throttled cadence would
/// make each slowdown the evidence for the next.
pub fn sync_fixed_timestep(
    session: Res<GameSession>,
    config: Res<ThrottleConfig>,
    mut fixed: ResMut<Time<Fixed>>,
    mut nominal: ResMut<NominalTimestep>,
    mut pacing: ResMut<TickPacing>,
    margins: Option<Res<FrameMargins>>,
    capacities: Option<Res<PeerCapacities>>,
) {
    let nominal = *nominal.0.get_or_insert(fixed.timestep());
    // Capped at the largest factor this layer will produce: the session accepts
    // any positive factor (the engine judges none), and an extreme one would
    // divide the tick down to `MIN_TIMESTEP`, which the fixed loop answers by
    // running that many steps in one frame — the app hangs. Past this the game
    // is not going faster in any useful sense anyway.
    let speed = session.speed().factor().min(MAX_FACTOR);
    // The tick length asked for, before any throttle. Dividing by the speed
    // lengthens it, which is what a slower speed and a throttle both do.
    let requested_millis = millis(nominal).saturating_div(speed);

    // Only a running game is paced. While it is paused, or blocked waiting for a
    // peer, no tick is advancing to spread out — and a stretched step would make
    // the drop rule's grace window (counted in blocked steps) take that many
    // times longer in wall time to expire.
    pacing.throttle = if config.enabled && session.is_advancing() {
        let margin = margins.and_then(|margins| margins.tightest(session.tick()));
        let peer_factor = if config.share_capacity {
            capacities
                .and_then(|capacities| capacities.tightest(session.tick()))
                .map(|capacity| capacity.factor())
        } else {
            None
        };
        throttle_for(
            pacing.exec_millis,
            requested_millis,
            speed,
            margin,
            peer_factor,
        )
    } else {
        NO_THROTTLE
    };

    let wanted = timestep_of(requested_millis.saturating_div(pacing.throttle)).max(MIN_TIMESTEP);
    if fixed.timestep() != wanted {
        fixed.set_timestep(wanted);
    }
}

/// Records the start of a fixed step, and the tick it started on (run in
/// [`FixedFirst`](bevy::app::FixedFirst)).
///
/// A step driven by hand is skipped: it runs back to back with no frame drawn
/// between, so what it costs says nothing about holding a cadence — folding a
/// seek's worth of them in would leave the game hitching afterwards on an
/// average that never included presenting anything. Whether a paced step is
/// worth measuring turns on whether it advances the tick, which nothing can
/// know until it has run, so that question is [`measure_tick`]'s.
pub fn mark_tick_start(
    mut pacing: ResMut<TickPacing>,
    session: Res<GameSession>,
    manual: Option<Res<ManualTick>>,
) {
    pacing.start = match manual {
        Some(_) => None,
        None => Some((Instant::now(), session.tick())),
    };
}

/// Folds the step's execution time into the running tick cost (run in
/// [`FixedLast`](bevy::app::FixedLast)).
///
/// Only a step that advanced the tick says anything about what a tick costs.
/// One spent paused, or blocked waiting for a peer, executes almost nothing and
/// would drag the cost down to nothing — leaving the game to hitch on resume
/// until the average recovered.
pub fn measure_tick(mut pacing: ResMut<TickPacing>, session: Res<GameSession>) {
    let Some((start, began_at)) = pacing.start.take() else {
        return;
    };
    if session.tick() == began_at {
        return;
    }
    let sample = millis(start.elapsed()).min(MAX_MILLIS);
    pacing.exec_millis = smooth(pacing.exec_millis, sample, COST_SMOOTHING);
}

/// Runs one fixed tick immediately, in the order the fixed loop would, advancing
/// as far as the session's state allows.
///
/// The fixed schedules run exactly as the loop's own step runs them — through
/// `FixedMain`, so a schedule the game added to the fixed order is not skipped —
/// but outside the loop's timekeeping: `Time<Fixed>` does not advance and the
/// generic `Time` is not switched to the fixed clock, so a system in a fixed
/// schedule must not read wall time (which the simulation, counting everything
/// in ticks, never does).
///
/// Panics on a world without the main schedules (Bevy's default `App` has them):
/// a tick that silently ran nothing would read as a stuck session.
pub fn run_tick(world: &mut World) {
    world.insert_resource(ManualTick);
    world.run_schedule(FixedMain);
    world.remove_resource::<ManualTick>();
}

/// Runs one fixed tick with the pause lifted for its duration, leaving the
/// session paused afterwards. A no-op while the session is not paused.
pub fn run_tick_while_paused(world: &mut World) {
    if !world.resource::<GameSession>().is_paused() {
        return;
    }
    world.resource_mut::<GameSession>().set_paused(false);
    run_tick(world);
    world.resource_mut::<GameSession>().set_paused(true);
}

/// Runs fixed ticks until the session reaches `target`, returning the tick it
/// ended on. Gives up on a tick that advanced nothing: a blocked, paused or
/// finished session never reaches a tick ahead of it, and would otherwise spin
/// forever.
pub fn run_until_tick(world: &mut World, target: u32) -> u32 {
    world.insert_resource(ManualTick);
    let reached = loop {
        let tick = world.resource::<GameSession>().tick();
        if tick >= target {
            break tick;
        }
        world.run_schedule(FixedMain);
        if world.resource::<GameSession>().tick() == tick {
            break tick;
        }
    };
    world.remove_resource::<ManualTick>();
    reached
}

/// Advances a requested [`Seek`] (run in `PreUpdate`, ahead of the frame's own
/// fixed loop, so nothing this frame draws or snapshots straddles the jump).
///
/// Runs ticks up to [`SEEK_FRAME_BUDGET`] of wall time, holding the session
/// paused between frames so the regular fixed loop cannot advance past the
/// target uncontrolled, and restores the pause state the seek began under once
/// the target is reached. Gives up — target reached or not — on a tick that
/// advances nothing, so a finished session or an exhausted recording ends the
/// seek instead of pinning it.
///
/// A seek is discarded whole on a networked session — running ahead would leave
/// this node ticks past its peers, and past the staleness guard on control
/// proposals — and on a finished replay: past the recorded end there is nothing
/// left to seek through, only unrecorded simulation. The guards live here so no
/// frontend can get them wrong.
pub fn apply_seek(world: &mut World) {
    let Some(Seek(target)) = world.remove_resource::<Seek>() else {
        return;
    };
    // A networked session must not run ahead of its peers, and a session that
    // cannot advance has nothing to seek through — a played-out recording blocks,
    // so this covers it without asking the replay anything.
    if world.get_resource::<NetworkActive>().is_some() || !can_advance(world) {
        // A seek already in flight holds the session paused between its frames,
        // so becoming ineligible mid-flight must put back the pause state it
        // began under — otherwise a game the player was watching at speed is
        // left frozen by machinery that has given up.
        if let Some(in_progress) = world.remove_resource::<SeekInProgress>() {
            world
                .resource_mut::<GameSession>()
                .set_paused(in_progress.paused_before);
        }
        return;
    }
    let paused_before = match world.get_resource::<SeekInProgress>() {
        Some(in_progress) => in_progress.paused_before,
        None => {
            let paused_before = world.resource::<GameSession>().is_paused();
            world.insert_resource(SeekInProgress { paused_before });
            paused_before
        }
    };
    world.resource_mut::<GameSession>().set_paused(false);

    let deadline = Instant::now() + SEEK_FRAME_BUDGET;
    world.insert_resource(ManualTick);
    let reached = loop {
        let tick = world.resource::<GameSession>().tick();
        if tick >= target {
            break true;
        }
        world.run_schedule(FixedMain);
        if world.resource::<GameSession>().tick() == tick {
            // No progress: done as far as this session can go.
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
    };
    world.remove_resource::<ManualTick>();

    if reached {
        world.remove_resource::<SeekInProgress>();
        if paused_before {
            world.resource_mut::<GameSession>().set_paused(true);
        }
    } else {
        world.resource_mut::<GameSession>().set_paused(true);
        world.insert_resource(Seek(target));
    }
}

/// Advances one tick for a requested [`Step`] (run in `PreUpdate`). A no-op —
/// though still consumed — unless the session is paused. Refused whole on a
/// networked session (a step would leave this node a tick ahead of its peers)
/// and on a finished replay (past the recorded end there is nothing left to
/// step into) — the guards live here so no frontend can get them wrong.
pub fn apply_step(world: &mut World) {
    if world.remove_resource::<Step>().is_none() {
        return;
    }
    if world.get_resource::<NetworkActive>().is_some() || !can_advance(world) {
        return;
    }
    run_tick_while_paused(world);
}

/// Whether the session could advance if it were unpaused: running or paused, but
/// not blocked and not finished. A step or a seek lifts a pause; it cannot
/// conjure input a frame source does not have, nor restart a finished game.
fn can_advance(world: &World) -> bool {
    let session = world.resource::<GameSession>();
    session.is_active() && !session.is_blocked()
}

/// (Re)installs the cadence's per-game state, called from
/// [`install_game_resources`](crate::install_game_resources) so no entry path
/// can forget it. What a tick cost belongs to the game that ran it, and a step
/// or seek requested in the last game must not drive this one.
pub(crate) fn install_per_game(world: &mut World) {
    world.insert_resource(TickPacing::default());
    remove_per_game(world);
}

/// Removes the pending step and seek requests when leaving a game, called from
/// [`teardown_game_resources`](crate::teardown_game_resources): a request left
/// standing would drive the next game's first frames.
pub(crate) fn remove_per_game(world: &mut World) {
    world.remove_resource::<Step>();
    world.remove_resource::<Seek>();
    world.remove_resource::<SeekInProgress>();
}

/// Folds a new sample into a running exponentially-smoothed value, `weight`
/// being the share the sample takes. Saturating throughout, so an extraordinary
/// sample clips instead of wrapping.
pub(crate) fn smooth(running: FixedU64, sample: FixedU64, weight: FixedU64) -> FixedU64 {
    running
        .saturating_mul(FixedU64::ONE - weight)
        .saturating_add(sample.saturating_mul(weight))
}

/// A duration in milliseconds, as far as this layer will count.
///
/// Fixed-point throughout: the cadence is wall-clock business and never reaches a
/// tick, but computing it in [`FixedU64`] keeps a class of mistake off the table —
/// a value that leaked into the simulation later would at least be the *same*
/// wrong value on every node, a loud bug rather than a desync.
pub(crate) fn millis(duration: Duration) -> FixedU64 {
    FixedU64::saturating_from_num(duration.as_nanos() / 1_000).saturating_div(THOUSAND)
}

/// Milliseconds back to a tick length. Scaling the bit pattern rather than the
/// value keeps a long tick — a heavily throttled one, or a very slow speed — from
/// saturating on the way out.
fn timestep_of(millis: FixedU64) -> Duration {
    let nanos = (millis.to_bits() as u128 * 1_000_000) >> FixedU64::FRAC_NBITS;
    Duration::from_nanos(nanos.min(u64::MAX as u128) as u64)
}
