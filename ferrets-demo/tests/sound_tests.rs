//! Which announcements the demo turns into sound, where the sound comes from,
//! and the ceiling that stops a seek or a large fight stacking every cue at once.

mod utils;

use bevy::{
    app::TaskPoolPlugin,
    asset::AssetPlugin,
    audio::{GlobalVolume, SpatialListener, Volume},
    ecs::system::RunSystemOnce,
    prelude::*,
    window::PrimaryWindow,
};
use ferrets_bevy_plugin::TickPacing;
use ferrets_content::{registry::ContentRegistry, skills::SkillId};
use ferrets_demo::{
    debug::{self, DebugState, DebugText},
    input::InputMode,
    render::{CELL_PX, FogReveal, ObserverPerspective, Smoothing},
    sound::{self, Muted, PlayingCue},
};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::{FixedU64, fixed_uvec2::FixedUVec2};
use ferrets_simulation::{
    events::{DeathCause, EventRecord, SimulationEvent},
    movement_model::MovementModel,
    simulation_id::SimulationId,
    spawn,
};

#[test]
fn kill_is_heard_where_it_happened() {
    let mut app = sound_app();
    let world = app.world_mut();
    let (victim, _) = spawn::create_entity(world, "grunt", utils::at_cell(6, 6), Some(1)).unwrap();
    let (_, killer) = spawn::create_entity(world, "grunt", utils::at_cell(5, 6), Some(0)).unwrap();
    spawn::despawn_entity(
        world,
        victim,
        DeathCause::Killed {
            by: killer,
            by_owner: Some(0),
        },
    );
    play(&mut app);

    assert_eq!(
        cue_cells(&mut app),
        vec![CellPos::new(6, 6)],
        "a death sounds from the cell it happened in, not from the listener"
    );
}

#[test]
fn cancelled_death_makes_no_sound() {
    let mut app = sound_app();
    let world = app.world_mut();
    let (site, _) = spawn::create_entity(world, "grunt", utils::at_cell(6, 6), Some(0)).unwrap();
    spawn::despawn_entity(world, site, DeathCause::Cancelled);
    play(&mut app);

    assert!(
        cue_cells(&mut app).is_empty(),
        "an owner calling something off is not an explosion"
    );
}

#[test]
fn burst_of_cues_stops_at_ceiling() {
    let mut app = sound_app();
    for step in 0..30u32 {
        let world = app.world_mut();
        let placed = spawn::create_entity(world, "grunt", utils::at_cell(4 + step, 8), Some(1));
        let Some((victim, _)) = placed else {
            continue;
        };
        spawn::despawn_entity(
            world,
            victim,
            DeathCause::Killed {
                by: SimulationId(0),
                by_owner: Some(0),
            },
        );
    }
    play(&mut app);

    assert_eq!(
        cue_cells(&mut app).len(),
        sound::MAX_CONCURRENT_CUES,
        "the burst fills the ceiling exactly and the rest is dropped"
    );
}

#[test]
fn cue_out_of_earshot_leaves_slot_for_near_one() {
    let mut app = sound_app();
    let world = app.world_mut();
    // More far-off hits than the ceiling holds, then one beside the listener.
    for step in 0..sound::MAX_CONCURRENT_CUES as u32 {
        world
            .resource_mut::<EventRecord>()
            .emit(hit_at(step, utils::at_cell(200 + step, 8)));
    }
    world
        .resource_mut::<EventRecord>()
        .emit(hit_at(99, utils::at_cell(5, 5)));
    play(&mut app);

    assert_eq!(
        cue_cells(&mut app),
        vec![CellPos::new(5, 5)],
        "hits nobody can hear do not use up the ceiling"
    );
}

#[test]
fn construction_completion_is_heard_at_building() {
    let mut app = sound_app();
    let world = app.world_mut();
    let (_, building) =
        spawn::create_entity(world, "grunt", utils::at_cell(7, 7), Some(0)).unwrap();
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::ConstructionCompleted {
            building,
            builder: SimulationId(0),
        });
    play(&mut app);

    assert_eq!(
        cue_cells(&mut app),
        vec![CellPos::new(7, 7)],
        "finishing a building sounds from the building"
    );
}

#[test]
fn enemy_construction_completion_is_not_heard() {
    let mut app = sound_app();
    let world = app.world_mut();
    let (_, building) =
        spawn::create_entity(world, "grunt", utils::at_cell(7, 7), Some(1)).unwrap();
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::ConstructionCompleted {
            building,
            builder: SimulationId(0),
        });
    play(&mut app);

    assert!(
        cue_cells(&mut app).is_empty(),
        "an enemy finishing a building is its milestone, not ours — even in plain sight"
    );
}

#[test]
fn enemy_research_at_visible_lab_is_not_heard() {
    let mut app = sound_app();
    let research = app
        .world()
        .resource::<ContentRegistry>()
        .research("iron_weapons")
        .expect("demo content declares iron_weapons");
    let world = app.world_mut();
    let (_, lab) = spawn::create_entity(world, "grunt", utils::at_cell(7, 7), Some(1)).unwrap();
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::ResearchCompleted {
            player: 1,
            research,
            researcher: Some(lab),
        });
    play(&mut app);

    assert!(
        cue_cells(&mut app).is_empty(),
        "seeing the lab does not mean seeing what finished inside it"
    );
}

//
// ─── Casts ───────────────────────────────────────────────────────────────
//

#[test]
fn cast_across_map_is_heard_at_both_ends() {
    let mut app = sound_app();
    let skill = battle_focus(&app);
    let world = app.world_mut();
    let (_, mage) = spawn::create_entity(world, "grunt", utils::at_cell(4, 4), Some(0)).unwrap();
    let (_, victim) = spawn::create_entity(world, "grunt", utils::at_cell(20, 4), Some(1)).unwrap();
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::SkillCast {
            caster: mage,
            target: victim,
            skill,
        });
    play(&mut app);

    assert_eq!(
        cue_cells(&mut app),
        vec![CellPos::new(4, 4), CellPos::new(20, 4)],
        "the cast sounds at its caster and its effect at what it landed on"
    );
}

#[test]
fn self_cast_is_heard_once() {
    let mut app = sound_app();
    let skill = battle_focus(&app);
    let world = app.world_mut();
    let (_, mage) = spawn::create_entity(world, "grunt", utils::at_cell(4, 4), Some(0)).unwrap();
    world
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::SkillCast {
            caster: mage,
            target: mage,
            skill,
        });
    play(&mut app);

    assert_eq!(
        cue_cells(&mut app),
        vec![CellPos::new(4, 4)],
        "caster and target being the same place is one sound, not two"
    );
}

#[test]
fn cue_beyond_two_views_is_not_heard() {
    let width = 40.0 * 32.0;
    assert!(
        sound::is_audible(Vec3::new(width * 1.9, 0.0, 0.0), Vec2::ZERO, width),
        "just inside earshot still sounds"
    );
    assert!(
        !sound::is_audible(Vec3::new(width * 2.0, 0.0, 0.0), Vec2::ZERO, width),
        "two view widths out is silence, not a sound too faint to place"
    );
}

#[test]
fn earshot_follows_where_camera_looks() {
    let width = 40.0 * 32.0;
    let far_off = Vec3::new(width * 3.0, 0.0, 0.0);
    assert!(
        !sound::is_audible(far_off, Vec2::ZERO, width),
        "out of earshot of one view"
    );
    assert!(
        sound::is_audible(far_off, far_off.truncate(), width),
        "and within it once the camera is looking there"
    );
}

#[test]
fn falloff_widens_as_view_zooms_out() {
    let close = sound::falloff_scale(1.0).0.x;
    let far = sound::falloff_scale(2.0).0.x;
    assert!(
        far < close,
        "a wider view reaches further before a cue fades: {far} vs {close}"
    );
}

#[test]
fn camera_carries_listener_from_spawn() {
    let mut app = App::new();
    app.add_systems(Startup, spawn_camera_with_listener);
    app.update();

    let mut cameras = app.world_mut().query::<(&Camera2d, &SpatialListener)>();
    assert_eq!(
        cameras.iter(app.world()).count(),
        1,
        "without ears on the camera every cue leans the same way, off the world origin"
    );
}

//
// ─── Player milestones ────────────────────────────────────────────────────
//

#[test]
fn own_player_milestone_is_heard_flat() {
    let mut app = sound_app();
    let skill = battle_focus(&app);
    app.world_mut()
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::PlayerSkillCast { player: 0, skill });
    play(&mut app);

    assert_eq!(
        cue_count(&mut app),
        1,
        "the local player's own milestone sounds"
    );
}

#[test]
fn enemy_player_milestone_is_not_heard() {
    let mut app = sound_app();
    let skill = battle_focus(&app);
    app.world_mut()
        .resource_mut::<EventRecord>()
        .emit(SimulationEvent::PlayerSkillCast { player: 1, skill });
    play(&mut app);

    assert_eq!(
        cue_count(&mut app),
        0,
        "an enemy milestone stays as unheard as it is unseen"
    );
}

//
// ─── Muting ──────────────────────────────────────────────────────────────
//

#[test]
fn sound_starts_muted() {
    assert!(
        Muted::default().0,
        "the demo waits to be asked before making noise"
    );
}

#[test]
fn mute_key_toggles_both_ways() {
    let mut app = App::new();
    app.init_resource::<Muted>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_systems(Update, sound::mute_input);

    press_mute(&mut app);
    assert!(!app.world().resource::<Muted>().0, "the key unmutes");

    press_mute(&mut app);
    assert!(app.world().resource::<Muted>().0, "and mutes again");
}

#[test]
fn muting_silences_global_volume() {
    let mut app = App::new();
    app.init_resource::<Muted>()
        .init_resource::<GlobalVolume>()
        .add_systems(Update, sound::apply_mute);
    app.update();
    assert_eq!(
        app.world().resource::<GlobalVolume>().volume,
        Volume::SILENT,
        "starting muted leaves nothing audible"
    );

    app.world_mut().resource_mut::<Muted>().0 = false;
    app.update();
    assert_eq!(
        app.world().resource::<GlobalVolume>().volume,
        Volume::Linear(1.0),
        "and unmuting restores it"
    );
}

#[test]
fn debug_readout_reports_whether_sound_is_on() {
    let mut app = readout_app();
    let readout = app.world_mut().spawn((DebugText, Text::new(""))).id();

    show_readout(&mut app);
    assert!(
        readout_text(&mut app, readout).contains("sound off"),
        "muted is what the readout says while nothing can be heard: {:?}",
        readout_text(&mut app, readout)
    );

    app.world_mut().resource_mut::<Muted>().0 = false;
    show_readout(&mut app);
    assert!(
        readout_text(&mut app, readout).contains("sound on"),
        "and it follows the toggle: {:?}",
        readout_text(&mut app, readout)
    );
}

//
// ─── Helpers ─────────────────────────────────────────────────────────────
//

/// A demo map app with the pieces the cue systems read, and every waveform
/// built. Fog is revealed, so a test asserts about cues rather than sight.
fn sound_app() -> App {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()));
    app.init_asset::<sound::GeneratedCue>()
        .init_resource::<ObserverPerspective>()
        .insert_resource(FogReveal(true));
    app.world_mut()
        .run_system_once(sound::build_cues)
        .expect("the waveforms are built");
    // A cue is placed against what the camera shows, so the view has to exist.
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.world_mut()
        .spawn((Camera2d, Transform::default(), sound::listener()));
    app
}

/// Runs the tick's cue systems over whatever the record holds.
fn play(app: &mut App) {
    app.world_mut()
        .run_system_once(sound::play_cues)
        .expect("the placed cues play");
}

/// How many cues are sounding, placed or flat.
fn cue_count(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<(), With<PlayingCue>>()
        .iter(app.world())
        .count()
}

/// A hit announced at `position`, from nobody in particular — cue staging needs
/// only the place.
fn hit_at(victim: u32, position: FixedUVec2) -> SimulationEvent {
    SimulationEvent::DamageLanded {
        target: SimulationId(victim),
        target_owner: Some(1),
        attacker: SimulationId(0),
        attacker_owner: Some(0),
        amount: FixedU64::ONE,
        position,
    }
}

/// One fresh press of the mute key, then a frame.
///
/// Reset first because the bare test app carries no input plugin to release the
/// key between frames, and a key already held is not pressed again.
fn press_mute(app: &mut App) {
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.reset_all();
    keys.press(KeyCode::KeyM);
    app.update();
}

/// The cell each playing cue sounds from.
fn cue_cells(app: &mut App) -> Vec<CellPos> {
    let mut query = app.world_mut().query::<(&Transform, &PlayingCue)>();
    let mut cells: Vec<CellPos> = query
        .iter(app.world())
        .map(|(transform, _)| {
            let at = transform.translation;
            CellPos::new((at.x / CELL_PX) as u32, (-at.y / CELL_PX) as u32)
        })
        .collect();
    cells.sort_by_key(|cell| (cell.x, cell.y));
    cells
}

/// A demo map app carrying what the debug readout reads.
fn readout_app() -> App {
    let mut app = utils::demo_map_app(MovementModel::Cell);
    app.init_resource::<Muted>()
        .init_resource::<Smoothing>()
        .init_resource::<InputMode>()
        .init_resource::<DebugState>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Time<Fixed>>()
        .init_resource::<TickPacing>();
    app
}

/// Fills in the debug readout.
fn show_readout(app: &mut App) {
    app.world_mut()
        .run_system_once(debug::debug_readout)
        .expect("the readout is written");
}

/// What the debug readout currently reads.
fn readout_text(app: &mut App, readout: Entity) -> String {
    app.world()
        .entity(readout)
        .get::<Text>()
        .expect("a readout")
        .0
        .clone()
}

/// Spawns the demo's camera exactly as the game does, listener and all.
fn spawn_camera_with_listener(mut commands: Commands) {
    commands.spawn((Camera2d, Transform::default(), sound::listener()));
}

/// The demo's `battle_focus` skill, for an announcement that needs one.
fn battle_focus(app: &App) -> SkillId {
    app.world()
        .resource::<ContentRegistry>()
        .skill("battle_focus")
        .expect("demo content declares battle_focus")
}
