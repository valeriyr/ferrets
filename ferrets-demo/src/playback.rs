//! Rebuilding a recorded game so it can be replayed, windowed or without a
//! window.
//!
//! A recording names the game it captured, not the world it produced, so
//! replaying one means rebuilding that game from the demo's own content: the
//! session it was played under, the map it was played on, and the scene it
//! started from. The engine cannot do this on its own — the content and the maps
//! are the game's. Both entry paths resolve a recording here, so the windowed
//! viewer and the headless runner cannot rebuild different games from the same
//! file.

use bevy::prelude::*;
use ferrets_bevy_plugin::{ReplayPlugin, SimulationPlugin};
use ferrets_content::registry::ContentRegistry;
use ferrets_replay::{
    header::{RecordedGame, ReplayHeader},
    replay::Replay,
};
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::{
    map::Map,
    map_data::MapData,
    scenario::Scenario,
    session::{
        GameSession,
        ai_hosting::AiHosting,
        authority::Authority,
        drop_policy::DropPolicy,
        finish_policy::FinishPolicy,
        local_role::LocalRole,
        player_slot::{self, PlayerSlot},
    },
};

use crate::{ai, map, scenario, scenario::CurrentScenario, setup};

/// What a recording resolves to in the demo's own content: everything needed to
/// rebuild the recorded game.
pub struct ResolvedGame {
    /// The session slots the recorded game seated.
    pub slots: Vec<PlayerSlot>,
    /// The name of the map it was played on.
    pub map_name: String,
    /// The finish rule its ticks ran under. Unlike the choices only the frame
    /// sources read, the tick itself reads this one, so it is the recorded
    /// game's own.
    pub finish_policy: FinishPolicy,
    /// The map as data, under the recorded movement model and projection.
    pub data: MapData,
    /// The mission a scenario recording names; `None` for a skirmish.
    pub scenario: Option<Scenario>,
}

/// A rebuilt recording: the app to run it in, and the last tick it recorded —
/// `None` for a recording holding no completed ticks at all.
pub struct Rebuilt {
    pub app: App,
    pub last_tick: Option<u32>,
}

/// Resolves `header`'s game against the demo's own content — a scenario
/// recording names a scenario the demo must know; a skirmish recording carries
/// its own definition and only needs its map. The rules the game was played
/// under come from the header, so nothing depends on how a menu happens to be
/// set. Errs on a scenario or map this build does not know.
pub fn resolve(header: &ReplayHeader, registry: &ContentRegistry) -> Result<ResolvedGame, String> {
    let model = header.movement_model;
    let projection = header.projection;
    let (slots, map_name, finish_policy, data, scenario) = match header.game.clone() {
        RecordedGame::Scenario(name) => {
            let mission = scenario::builtin_mission(projection, model);
            if name != mission.name {
                return Err(format!("the replay needs unknown scenario '{name}'"));
            }
            (
                player_slot::scenario_slots(&mission, ai::environment_vision(registry)),
                mission.map.name().to_string(),
                FinishPolicy::Scripted,
                mission.map.clone(),
                Some(mission),
            )
        }
        RecordedGame::Skirmish(skirmish) => {
            let Some(mut data) = map::by_name(&skirmish.map) else {
                return Err(format!("the replay needs unknown map '{}'", skirmish.map));
            };
            data.set_movement_model(model);
            data.set_projection(projection);
            (
                skirmish.slots,
                skirmish.map,
                skirmish.finish_policy,
                data,
                None,
            )
        }
    };

    Ok(ResolvedGame {
        slots,
        map_name,
        finish_policy,
        data,
        scenario,
    })
}

/// The playback session for a resolved game. The viewer is an observer — a
/// node with no local player, exactly like a watcher of the live game — so
/// no result of the recorded players' can ever be its own; the recording is
/// the sole frame source, so the choices that only the net control plane and
/// the AI frame sources read — the authority, its hosting mode, the drop
/// policy — never come into play.
pub fn session(resolved: &ResolvedGame) -> GameSession {
    GameSession::configured(
        LocalRole::Observer,
        resolved.slots.clone(),
        resolved.map_name.clone(),
        Authority::Host {
            ai_hosting: AiHosting::Replicated,
        },
        DropPolicy::Automatic,
        resolved.finish_policy,
    )
}

/// Rebuilds `replay`'s game into a headless app with the recording installed as
/// the sole frame source, ready to be advanced tick by tick.
pub fn rebuild(replay: Replay) -> Result<Rebuilt, String> {
    let registry = content::load(&LuaEngine, crate::content::CONTENT)
        .map_err(|error| format!("demo content failed to load: {error}"))?;
    let resolved = resolve(replay.header(), &registry)?;
    let last_tick = replay.last_tick();
    let game_map = Map::from_data(&resolved.data, &registry);
    let session = session(&resolved);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(SimulationPlugin::new(session, game_map));
    app.add_plugins(ReplayPlugin);
    *app.world_mut().resource_mut::<ContentRegistry>() = registry;
    ferrets_bevy_plugin::install_game_resources(app.world_mut());
    ferrets_bevy_plugin::replay::playback::install_per_game(app.world_mut(), replay);

    // The same scene the recorded game started from, rebuilt at tick 0 by the
    // very spawners the demo uses, so a headless rebuild cannot drift from the
    // scene a player would see. No scenario runtime is installed: the recording
    // is the sole authority, and the win/loss check stands down during playback.
    match resolved.scenario {
        Some(mission) => {
            app.world_mut().insert_resource(CurrentScenario(mission));
            scenario::spawn_scenario_scene(app.world_mut());
        }
        None => setup::spawn_demo_scene(app.world_mut()),
    }

    Ok(Rebuilt { app, last_tick })
}
