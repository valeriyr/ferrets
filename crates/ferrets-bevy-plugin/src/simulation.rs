//! The simulation's own per-game state: the stores the tick reads and writes,
//! sized to the session's slots and reset between games.

use bevy::prelude::*;
use ferrets_simulation::{
    control_groups::ControlGroups,
    entity_index::EntityIndex,
    events::EventRecord,
    game_loop::movement::MovePlanShare,
    impacts::PendingImpacts,
    input::{InputFrames, PlayerFrame, SYNC_LATENCY},
    player_buffs::PlayerBuffs,
    player_research::PlayerResearch,
    player_skills::PlayerSkills,
    player_stats::PlayerStats,
    resources::PlayerResources,
    selection::Selection,
    session::{GameSession, player_slot::PlayerSlot},
    simulation_id::SimulationIdGenerator,
    statistics::Statistics,
};

use crate::input::PendingInput;

/// (Re)installs the simulation's per-game state, called from
/// [`install_game_resources`](crate::install_game_resources) so no entry path
/// can forget it: sizes the per-player stores to the session's slots and clears
/// everything transient a previous game in the same app may have left behind.
pub(crate) fn install_per_game(world: &mut World) {
    let session = world.resource::<GameSession>();
    let player_count = session.slots().len();
    let frames = warmup_input_frames(session.slots());
    world.insert_resource(Selection::new(player_count));
    world.insert_resource(ControlGroups::new(player_count));
    world.insert_resource(PlayerResources::new(player_count));
    world.insert_resource(PlayerStats::new(player_count));
    world.insert_resource(PlayerSkills::new(player_count));
    world.insert_resource(PlayerBuffs::new(player_count));
    world.insert_resource(PlayerResearch::new(player_count));
    world.insert_resource(Statistics::new(player_count));
    world.insert_resource(frames);
    world.insert_resource(EntityIndex::default());
    world.insert_resource(SimulationIdGenerator::default());
    world.insert_resource(PendingImpacts::default());
    world.insert_resource(EventRecord::default());
    world.insert_resource(PendingInput::default());
    world.insert_resource(MovePlanShare::default());
}

/// Removes the simulation's per-game state when leaving a game, called from
/// [`teardown_game_resources`](crate::teardown_game_resources): despawns every
/// simulation entity and resets the stores and the session to their pre-game
/// state.
pub(crate) fn remove_per_game(world: &mut World) {
    let entities: Vec<Entity> = world
        .resource::<EntityIndex>()
        .all_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect();
    for entity in entities {
        world.despawn(entity);
    }
    // The pending session has no slots, so re-installing sizes every per-player
    // store to nothing and resets the rest. Done by reusing the installer rather
    // than by listing the stores again: the two lists had already drifted once,
    // leaving a finished game's tallies and stockpiles standing between games.
    world.insert_resource(GameSession::pending());
    install_per_game(world);
}

/// Builds the input queue with the lockstep warmup pre-seeded: ticks
/// `0..SYNC_LATENCY` can never be targeted by a source scheduling `SYNC_LATENCY`
/// ahead, so every occupied slot is recorded idle for them — otherwise the loop
/// would block at startup. Unoccupied slots get nothing: no tick requires their
/// input. Seeded identically on every peer, so it stays deterministic.
fn warmup_input_frames(slots: &[PlayerSlot]) -> InputFrames {
    let mut frames = InputFrames::new(slots.len());
    for tick in 0..SYNC_LATENCY {
        for slot in slots {
            if slot.player_type().is_some() {
                frames.push_frame(PlayerFrame::idle(slot.id(), tick));
            }
        }
    }
    frames
}
