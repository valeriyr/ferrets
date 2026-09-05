//! Procedurally generated sound cues, played where the simulation said things
//! happened.
//!
//! The waveforms are built in code; the demo carries no audio assets. Every cue
//! is a one-shot spawned at the announced position, placed by bevy's spatial
//! audio against a listener on the camera, and despawned when it finishes. Cues
//! too far from what the camera shows are not played at all.
//!
//! Cues come in two classes. Field sounds — hits, explosions, deliveries, casts,
//! form changes — follow the fog: anyone who can see the cell hears them.
//! Milestone sounds — research and construction completions, a player's own
//! skills — are progress reports, heard only on their owner's node; a node with
//! no player of its own (an observer) hears every player's.

use std::{sync::Arc, time::Duration};

use bevy::{
    audio::{
        AddAudioSource, AudioSink, AudioSinkPlayback, Decodable, GlobalVolume, PlaybackMode,
        PlaybackSettings, Source, SpatialAudioSink, SpatialListener, SpatialScale, Volume,
    },
    prelude::*,
    window::PrimaryWindow,
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_simulation::{
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, EventRecord, SimulationEvent},
    session::{GameSession, local_role::LocalRole, player_id::PlayerId},
    visibility::VisibilityGrid,
};

use crate::render::{self, FogReveal, ObserverPerspective, world_center};

/// Samples per second every generated cue is built at.
const SAMPLE_RATE: u32 = 44_100;

/// How loud cues play, relative to the generated waveform.
const CUE_VOLUME: f32 = 0.35;

/// How many cues may be sounding at once — a seek running many ticks inside one
/// frame, or one big fight, fills it in a single pass.
pub const MAX_CONCURRENT_CUES: usize = 12;

/// How far a cue carries, as a multiple of the width of what the camera shows —
/// silent beyond it, not faint.
const AUDIBLE_VIEWS: f32 = 2.0;

/// How many cells from the middle of the view a cue is one unit of falloff away.
///
/// Bevy attenuates by `1 / distance²`, clamped at full volume, so a cue inside
/// this stays as loud as one in the middle and beyond it fades. Scaled by the
/// zoom, so the fading edge follows what the camera shows.
const FALLOFF_CELLS: f32 = 10.0;

/// The gap between the listener's ears, in cells.
///
/// Panning depends on this only as a ratio against the distance to the cue, so it
/// sets how quickly a cue swings aside rather than how far it carries.
const EAR_GAP_CELLS: f32 = 4.0;

/// The cues the demo can play, one per kind of thing worth hearing.
#[derive(Clone, Copy)]
enum Cue {
    /// A hit landing on something.
    Hit,
    /// Something coming apart.
    Explosion,
    /// A load of resources reaching a stockpile.
    Delivery,
    /// A skill going off, heard at whoever cast it.
    Cast,
    /// That skill taking hold, heard at whatever it was applied to.
    Effect,
    /// A form change completing.
    Morph,
    /// A construction site finishing its building.
    Completed,
    /// A research topic finishing.
    Research,
}

impl Cue {
    /// The waveform this cue plays.
    fn waveform(self) -> Vec<f32> {
        match self {
            // A thud with a short noisy attack: a weapon connecting.
            Cue::Hit => tone(0.09, 220.0, 90.0, 0.55, 26.0),
            // Lower, longer and much noisier: something coming apart.
            Cue::Explosion => tone(0.35, 90.0, 40.0, 0.9, 9.0),
            // A short bright clink, barely any noise: a load banked.
            Cue::Delivery => tone(0.10, 880.0, 1180.0, 0.05, 24.0),
            // A rising chime: a skill going off.
            Cue::Cast => tone(0.22, 520.0, 900.0, 0.05, 11.0),
            // Softer and falling, so the landing answers the cast.
            Cue::Effect => tone(0.18, 660.0, 400.0, 0.04, 14.0),
            // A slow sweep down reads as a change of shape.
            Cue::Morph => tone(0.30, 700.0, 300.0, 0.08, 8.0),
            // A clean octave up: work ending on a high note.
            Cue::Completed => tone(0.30, 392.0, 784.0, 0.03, 7.0),
            // The longest and cleanest: a milestone rather than a thing in the
            // field.
            Cue::Research => tone(0.45, 440.0, 660.0, 0.02, 5.0),
        }
    }
}

/// A generated waveform, held as an asset so bevy's audio can play it.
#[derive(Asset, TypePath)]
pub struct GeneratedCue {
    /// One channel of samples in `-1.0..=1.0`, shared by every playback. The
    /// spatial sink takes mono and produces the stereo pair.
    samples: Arc<[f32]>,
}

impl Decodable for GeneratedCue {
    type DecoderItem = f32;
    type Decoder = CueSamples;

    fn decoder(&self) -> Self::Decoder {
        CueSamples {
            samples: self.samples.clone(),
            next: 0,
        }
    }
}

/// Plays a [`GeneratedCue`]'s samples once, in mono.
pub struct CueSamples {
    samples: Arc<[f32]>,
    next: usize,
}

impl Iterator for CueSamples {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.samples.get(self.next).copied();
        self.next += 1;
        sample
    }
}

impl Source for CueSamples {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.len().saturating_sub(self.next))
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(
            self.samples.len() as f32 / SAMPLE_RATE as f32,
        ))
    }
}

/// Every cue's waveform, built once at startup.
#[derive(Resource)]
pub struct Cues {
    hit: Handle<GeneratedCue>,
    explosion: Handle<GeneratedCue>,
    delivery: Handle<GeneratedCue>,
    cast: Handle<GeneratedCue>,
    effect: Handle<GeneratedCue>,
    morph: Handle<GeneratedCue>,
    completed: Handle<GeneratedCue>,
    research: Handle<GeneratedCue>,
}

impl Cues {
    /// The waveform `cue` plays.
    fn handle(&self, cue: Cue) -> Handle<GeneratedCue> {
        match cue {
            Cue::Hit => self.hit.clone(),
            Cue::Explosion => self.explosion.clone(),
            Cue::Delivery => self.delivery.clone(),
            Cue::Cast => self.cast.clone(),
            Cue::Effect => self.effect.clone(),
            Cue::Morph => self.morph.clone(),
            Cue::Completed => self.completed.clone(),
            Cue::Research => self.research.clone(),
        }
    }
}

/// Whether cues are silenced. Silent until asked for.
#[derive(Resource)]
pub struct Muted(pub bool);

impl Default for Muted {
    fn default() -> Self {
        Self(true)
    }
}

/// Silences or restores every cue, following [`Muted`] — one already sounding is
/// cut off with the rest.
pub fn apply_mute(muted: Res<Muted>, mut volume: ResMut<GlobalVolume>) {
    if !muted.is_changed() && !volume.is_added() {
        return;
    }
    volume.volume = if muted.0 {
        Volume::SILENT
    } else {
        Volume::Linear(1.0)
    };
}

/// Toggles [`Muted`] on the mute key.
pub fn mute_input(keys: Res<ButtonInput<KeyCode>>, mut muted: ResMut<Muted>) {
    if keys.just_pressed(KeyCode::KeyM) {
        muted.0 = !muted.0;
    }
}

/// Marks a one-shot cue entity, so finished ones can be cleared.
#[derive(Component)]
pub struct PlayingCue;

/// Marks a cue seen once with no sink yet, so the next look can tell "not wired
/// up yet" from "never will be".
///
/// Never removed: once a sink arrives the marker is no longer read, and it goes
/// down with the one-shot entity it rides.
#[derive(Component)]
struct AwaitingSink;

/// Registers the generated waveforms as a playable source, builds them, and
/// clears the one-shots bevy has finished with.
///
/// The audio backend itself comes from `DefaultPlugins`, which carries
/// `AudioPlugin` whenever the `bevy_audio` feature is on.
pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<GeneratedCue>()
            .init_resource::<Muted>()
            .add_systems(Startup, build_cues)
            .add_systems(Update, (mute_input, apply_mute, despawn_finished_cues));
    }
}

/// Builds every waveform and stores the handles.
pub fn build_cues(mut commands: Commands, mut assets: ResMut<Assets<GeneratedCue>>) {
    let mut built = |cue: Cue| {
        assets.add(GeneratedCue {
            samples: cue.waveform().into(),
        })
    };
    commands.insert_resource(Cues {
        hit: built(Cue::Hit),
        explosion: built(Cue::Explosion),
        delivery: built(Cue::Delivery),
        cast: built(Cue::Cast),
        effect: built(Cue::Effect),
        morph: built(Cue::Morph),
        completed: built(Cue::Completed),
        research: built(Cue::Research),
    });
}

/// Builds one cue: a sine sweeping from `from_hz` to `to_hz` over `seconds`,
/// mixed with `noise` and shaped by an exponential decay of rate `decay`.
///
/// The noise comes from a fixed counter rather than a random source, so a cue
/// sounds the same on every run and every machine.
fn tone(seconds: f32, from_hz: f32, to_hz: f32, noise: f32, decay: f32) -> Vec<f32> {
    let count = (seconds * SAMPLE_RATE as f32) as usize;
    let mut samples = Vec::with_capacity(count);
    let mut phase = 0.0f32;
    let mut grit: u32 = 0x2545_f491;
    for index in 0..count {
        let progress = index as f32 / count as f32;
        let hertz = from_hz + (to_hz - from_hz) * progress;
        phase += std::f32::consts::TAU * hertz / SAMPLE_RATE as f32;
        // xorshift, so the grain is the same on every run and every machine.
        grit ^= grit << 13;
        grit ^= grit >> 17;
        grit ^= grit << 5;
        let hiss = (grit as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let envelope = (-decay * progress).exp();
        samples.push((phase.sin() * (1.0 - noise) + hiss * noise) * envelope * CUE_VOLUME);
    }
    samples
}

/// Where the view is centred, how wide it is in world units, and the zoom that
/// made it that wide.
///
/// `None` while there is no camera to hear through.
fn view(world: &mut World) -> Option<(Vec2, f32, f32)> {
    let width = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .iter(world)
        .next()
        .map(Window::width)?;
    let (transform, projection) = world
        .query_filtered::<(&GlobalTransform, &Projection), With<Camera2d>>()
        .iter(world)
        .next()?;
    let zoom = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };
    Some((transform.translation().truncate(), width * zoom, zoom))
}

/// Whether a cue at `place` is close enough to the middle of the view to be
/// heard, given a view `view_width` wide.
///
/// A cut-off rather than an ever-fainter sound: things happening a couple of
/// screens away are not the player's concern, and a field of barely-audible
/// noise is worse than quiet.
pub fn is_audible(place: Vec3, middle: Vec2, view_width: f32) -> bool {
    place.truncate().distance(middle) < view_width * AUDIBLE_VIEWS
}

/// The listener a camera carries, so cues pan and fade around whatever the
/// player is looking at.
///
/// Spawned with the camera rather than added to it afterwards: a listener that
/// arrives late leaves bevy falling back to a default pair of ears at the world
/// origin, and every cue on the map then leans the same way.
pub fn listener() -> SpatialListener {
    SpatialListener::new(EAR_GAP_CELLS * render::CELL_PX)
}

/// How much world distance one unit of bevy's falloff covers, at a given zoom.
///
/// Follows the zoom so the fading edge stays at the edge of the view: showing
/// more ground should not make everything on it equally loud.
pub fn falloff_scale(zoom: f32) -> SpatialScale {
    SpatialScale::new(1.0 / (FALLOFF_CELLS * render::CELL_PX * zoom))
}

/// Plays a cue for each thing the tick announced that is worth hearing (run once
/// per tick, in the game's slot).
///
/// A placed cue over a cell the viewer cannot see is skipped, so an unseen fight
/// makes no noise; one out of earshot is skipped too. Neither counts against
/// [`MAX_CONCURRENT_CUES`], which caps only what actually sounds. A cue with no
/// place — a player's own skill, research granted outright — plays flat.
pub fn play_cues(world: &mut World) {
    if !world.contains_resource::<Cues>() {
        return;
    }
    if world.resource::<EventRecord>().events().is_empty() {
        return;
    }
    let mut sounding = world
        .query_filtered::<(), With<PlayingCue>>()
        .iter(world)
        .count();
    let Some((middle, view_width, zoom)) = view(world) else {
        return;
    };

    let local = world.resource::<GameSession>().local_role();
    let mut wanted: Vec<(Cue, Option<FixedUVec2>)> = Vec::new();
    for event in world.resource::<EventRecord>().events() {
        cues_for(world, local, event, &mut wanted);
    }

    for (cue, position) in wanted {
        if sounding >= MAX_CONCURRENT_CUES {
            break;
        }
        match position {
            Some(position) => {
                let cell = CellPos::from(position);
                let visible = world.resource::<FogReveal>().0
                    || render::sees(
                        world.resource::<GameSession>(),
                        world.resource::<ObserverPerspective>(),
                        world.resource::<VisibilityGrid>(),
                        cell.x,
                        cell.y,
                    );
                if !visible {
                    continue;
                }
                let at = world_center(position, CellSize::ONE);
                if !is_audible(at, middle, view_width) {
                    continue;
                }
                let handle = world.resource::<Cues>().handle(cue);
                world.spawn((
                    PlayingCue,
                    AudioPlayer(handle),
                    PlaybackSettings {
                        mode: PlaybackMode::Despawn,
                        spatial: true,
                        spatial_scale: Some(falloff_scale(zoom)),
                        ..default()
                    },
                    Transform::from_translation(at),
                ));
            }
            None => {
                let handle = world.resource::<Cues>().handle(cue);
                world.spawn((
                    PlayingCue,
                    AudioPlayer(handle),
                    PlaybackSettings {
                        mode: PlaybackMode::Despawn,
                        ..default()
                    },
                ));
            }
        }
        sounding += 1;
    }
}

/// The cues one announcement is worth, appended to `out` with the place each
/// should come from — `None` for one that concerns a player rather than a place,
/// which plays flat.
///
/// A place is read off whichever entity the announcement names, except for a
/// death, which carries its own: the victim is dying by now and its remains may
/// already stand on the cell.
///
/// A cast is heard at whoever made it and, in its own voice, at whatever it was
/// applied to. A self-cast is one place and so one sound.
///
/// The milestone cues — a research finishing, a construction completing — are
/// appended only when they belong to `local` (see [`own_milestone`]).
fn cues_for(
    world: &World,
    local: LocalRole,
    event: &SimulationEvent,
    out: &mut Vec<(Cue, Option<FixedUVec2>)>,
) {
    let at = |id| {
        let entity = world.resource::<EntityIndex>().any(id)?;
        Some(entity_def::position(world, entity))
    };
    match event {
        SimulationEvent::DamageLanded { position, .. } => out.push((Cue::Hit, Some(*position))),
        // Only a death an enemy caused: a cancelled site or a mined-out node
        // going away is not an explosion.
        SimulationEvent::EntityDied {
            position,
            cause: DeathCause::Killed { .. },
            ..
        } => out.push((Cue::Explosion, Some(*position))),
        SimulationEvent::EntityMorphed { entity, .. } => {
            out.extend(at(*entity).map(|place| (Cue::Morph, Some(place))));
        }
        SimulationEvent::ConstructionCompleted { building, .. } => {
            if let Some(entity) = world.resource::<EntityIndex>().any(*building)
                && own_milestone(local, entity_def::owner(world, entity))
            {
                out.push((Cue::Completed, Some(entity_def::position(world, entity))));
            }
        }
        SimulationEvent::ResourcesGathered { storage, .. } => {
            out.extend(at(*storage).map(|place| (Cue::Delivery, Some(place))));
        }
        // Placed at whatever worked it; a granted one has nothing to place it
        // on and plays flat.
        SimulationEvent::ResearchCompleted {
            researcher, player, ..
        } => {
            if own_milestone(local, Some(*player)) {
                let place = researcher.and_then(&at);
                out.push((Cue::Research, place));
            }
        }
        SimulationEvent::SkillCast { caster, target, .. } => {
            let from = at(*caster);
            let onto = at(*target);
            out.extend(from.map(|place| (Cue::Cast, Some(place))));
            if onto != from {
                out.extend(onto.map(|place| (Cue::Effect, Some(place))));
            }
        }
        SimulationEvent::PlayerSkillCast { player, .. } => {
            if own_milestone(local, Some(*player)) {
                out.push((Cue::Cast, None));
            }
        }
        SimulationEvent::EntityDied { .. }
        | SimulationEvent::EntitySpawned { .. }
        | SimulationEvent::EntityHidden { .. }
        | SimulationEvent::EntityRevealed { .. }
        | SimulationEvent::ResourcesSpent { .. }
        | SimulationEvent::ResourcesRefunded { .. } => {}
    }
}

/// Whether a milestone belonging to `player` is this node's to hear.
///
/// An observer hears every player's, and a milestone with no owner has no side
/// to keep it from.
fn own_milestone(local: LocalRole, player: Option<PlayerId>) -> bool {
    match local {
        LocalRole::Observer => true,
        LocalRole::Player(own) => match player {
            Some(player) => player == own,
            None => true,
        },
    }
}

/// Clears cue entities bevy has finished with, so a long game does not pile them
/// up. [`PlaybackMode::Despawn`] handles most; this catches a finished flat or
/// spatial sink it missed, and a cue that never received a sink at all (no audio
/// device, for instance) — recognised by still having none a frame after it was
/// marked.
fn despawn_finished_cues(
    mut commands: Commands,
    cues: Query<
        (
            Entity,
            Option<&AudioSink>,
            Option<&SpatialAudioSink>,
            Has<AwaitingSink>,
        ),
        With<PlayingCue>,
    >,
) {
    for (entity, flat, spatial, marked) in cues.iter() {
        let finished = match (flat, spatial) {
            (Some(sink), _) => sink.empty(),
            (None, Some(sink)) => sink.empty(),
            // Bevy wires sinks up in the frame a cue is spawned; one still
            // sinkless a frame after being marked will never get one.
            (None, None) => marked,
        };
        if finished {
            commands.entity(entity).despawn();
        } else if flat.is_none() && spatial.is_none() {
            commands.entity(entity).insert(AwaitingSink);
        }
    }
}

/// Drops every cue still playing when a game ends, so nothing carries into the
/// next one.
pub fn reset_per_game(mut commands: Commands, cues: Query<Entity, With<PlayingCue>>) {
    for entity in cues.iter() {
        commands.entity(entity).despawn();
    }
}
