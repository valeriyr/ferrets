//! Victory condition: the last team (or lone player) with a building standing
//! wins, and a player whose buildings are all gone is defeated.

use std::collections::BTreeSet;

use bevy_ecs::world::World;

use crate::{
    components::{owner::OwnerComponent, tags::TagsComponent},
    entity_index::EntityIndex,
    session::{
        GameResult, GameSession, Winner,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot},
    },
};
use ferrets_content::tags;

/// Ends the session, under [`FinishPolicy::LastStanding`], once the players still
/// holding a building are all on one side.
///
/// A player survives while they own at least one standing building; losing the
/// last one is defeat, independent of any surviving units. A defeated player is
/// eliminated as of the next tick: every node derives the same elimination from
/// its own simulation, so from that tick on no node requires the player's input
/// — the survivors keep playing instead of stalling on frames the defeated
/// player's node will never send.
/// The game ends when every survivor is allied with all the others — one side
/// left, winning as a team or as a lone player — or none survive (a draw).
/// While two unallied survivors stand the match continues, but the local player
/// is told of its own [`Defeat`](GameResult::Defeat) the moment it is out.
/// A player whose drop has taken effect never counts as a survivor (a drop
/// decided for a tick still ahead leaves them playing until it arrives), and
/// an eliminated player stays out even if a leftover order finishes a new
/// building for them. A lineup with only one side — a
/// single team, or a lone player — has no opponent to outlast and so wins at
/// once; a game meant to run without a last-standing verdict uses
/// [`FinishPolicy::Endless`]. Under any other [`FinishPolicy`] this stands aside.
///
/// Only [`Participation::Player`](crate::session::player_slot::Participation)
/// slots enter the accounting: an environment combatant is never eliminated,
/// never survives as a side, and never blocks another side's victory — it is
/// part of the environment, and its buildings count for no one.
///
/// Runs at the end of a tick, after deaths have been resolved.
pub fn check(world: &mut World) {
    let session = world.resource::<GameSession>();
    if !session.is_active() || session.finish_policy() != FinishPolicy::LastStanding {
        return;
    }

    let occupied: Vec<PlayerId> = session.player_slots().map(PlayerSlot::id).collect();

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
            .get::<TagsComponent>()
            .is_some_and(|carried| carried.contains(tags::BUILDING));
        if is_building && let Some(owner) = entity_ref.get::<OwnerComponent>() {
            with_building.insert(owner.player());
        }
    }

    let mut session = world.resource_mut::<GameSession>();

    // A player out of buildings is eliminated as of the next tick: this tick
    // still executed its input on every node, the next requires none of it.
    let eliminated_from = session.tick() + 1;
    for &player in &occupied {
        if !with_building.contains(&player) && !session.is_player_out(player) {
            session.eliminate_player(player, eliminated_from);
        }
    }

    // A player survives while it holds a building and is not out of the game —
    // a building finished by a leftover order does not revive an eliminated
    // player.
    let survivors: Vec<PlayerId> = occupied
        .iter()
        .copied()
        .filter(|player| with_building.contains(player) && !session.is_player_out(*player))
        .collect();

    let local = session.local_player();
    let local_out = !survivors.contains(&local) && !session.is_player_out(local);

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

    session.finish(result);
}
