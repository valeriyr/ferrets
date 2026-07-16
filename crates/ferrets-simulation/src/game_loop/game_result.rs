//! Victory condition: the last team (or lone player) with a building standing
//! wins, and a player whose buildings are all gone is defeated.

use std::collections::BTreeSet;

use bevy_ecs::world::World;

use crate::{
    components::{owner::OwnerComponent, tags},
    entity_index::EntityIndex,
    session::{
        GameResult, GameSession, Winner,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot},
    },
};

/// Ends the session, under [`FinishPolicy::LastStanding`], once the players still
/// holding a building are all on one side.
///
/// A player survives while they own at least one standing building; losing the
/// last one is defeat, independent of any surviving units.
/// The game ends when every survivor is allied with all the others — one side
/// left, winning as a team or as a lone player — or none survive (a draw).
/// While two unallied survivors stand the match continues, but the local player
/// is told of its own [`Defeat`](GameResult::Defeat) the moment it is out.
/// Dropped players never count as survivors. A lineup with only one side — a
/// single team, or a lone player — has no opponent to outlast and so wins at
/// once; a game meant to run without a last-standing verdict uses
/// [`FinishPolicy::Endless`]. Under any other [`FinishPolicy`] this stands aside.
/// Runs at the end of a tick, after deaths have been resolved.
pub fn check(world: &mut World) {
    let session = world.resource::<GameSession>();
    if !session.is_active() || session.finish_policy() != FinishPolicy::LastStanding {
        return;
    }

    let occupied: Vec<PlayerId> = session
        .slots()
        .iter()
        .filter(|slot| slot.player_type().is_some())
        .map(|slot| slot.id())
        .collect();

    // Players that still hold at least one standing building this tick. A
    // building that has begun dying no longer counts — it is rubble.
    let index = world.resource::<EntityIndex>();
    let entities: Vec<_> = index
        .alive_entries()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect();
    let mut with_building: BTreeSet<PlayerId> = BTreeSet::new();
    for entity in entities {
        let entity_ref = world.entity(entity);
        let is_building = entity_ref
            .get::<tags::TagsComponent>()
            .is_some_and(|carried| carried.contains(tags::BUILDING));
        if is_building && let Some(owner) = entity_ref.get::<OwnerComponent>() {
            with_building.insert(owner.player());
        }
    }

    let session = world.resource::<GameSession>();
    // A player survives while it holds a building and has not dropped.
    let survivors: Vec<PlayerId> = occupied
        .iter()
        .copied()
        .filter(|player| with_building.contains(player) && !session.is_player_dropped(*player))
        .collect();

    // Read out the local outcome before taking the mutable borrow below.
    let local = session.local_player();
    let local_out = !survivors.contains(&local) && !session.is_player_dropped(local);

    let result = match survivors.as_slice() {
        // Everyone was wiped out on the same tick.
        [] => GameResult::Draw,
        // All survivors allied with the first → one side left. It wins as a team,
        // or as the lone player when that survivor is on no team (a teamless
        // player is allied with no one, so there can be only the one).
        [first, rest @ ..] if rest.iter().all(|p| session.are_allied(*first, *p)) => {
            let winner = match session.slot(*first).and_then(PlayerSlot::team) {
                Some(team) => Winner::Team(team),
                None => Winner::Player(*first),
            };
            GameResult::Victory { winner }
        }
        // Two or more unallied survivors: the match goes on. The local player
        // hears of its own defeat as soon as it is out; every other node decides
        // the same for its own local player.
        _ if local_out => GameResult::Defeat,
        _ => return,
    };

    world.resource_mut::<GameSession>().finish(result);
}
