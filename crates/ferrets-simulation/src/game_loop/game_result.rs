//! Victory condition: the last team (or lone player) with a building standing
//! wins, and a player whose buildings are all gone is defeated.

use std::collections::BTreeSet;

use bevy_ecs::world::World;

use crate::{
    components::{owner::OwnerComponent, tags::TagsComponent},
    entity_index::EntityIndex,
    session::{
        GameResult, GameSession, Winner,
        defeat_conduct::DefeatConduct,
        elimination_scope::EliminationScope,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot},
    },
};
use ferrets_content::tags;

/// Ends the session, under [`FinishPolicy::LastStanding`], once the players still
/// holding a building are all on one side.
///
/// A player stands while a standing building keeps them in the game — their
/// own under [`EliminationScope::Player`], any of their side's under
/// [`EliminationScope::Side`] — independent of any surviving units. A player
/// who stops standing is eliminated as of the next tick (under `Side`, a
/// falling side is eliminated whole, on one tick): every node derives the
/// same elimination from its own simulation, so from that tick on no node
/// requires the player's input — the survivors keep playing instead of
/// stalling on frames the eliminated player's node may never send.
/// The game ends when every survivor is allied with all the others — one side
/// left, winning as a team or as a lone player — or none survive (a draw).
/// While two unallied survivors stand the match continues, but the local
/// player is told of its own [`Defeat`](GameResult::Defeat) the moment its
/// whole side is out — under [`EliminationScope::Player`] a player may be
/// eliminated (out of input, watching) while its side fights on, and is not
/// defeated until the side falls. Under [`DefeatConduct::Spectate`] the
/// defeat finishes nothing: the node keeps simulating and the player watches
/// until the shared verdict arrives.
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
    let elimination = match session.finish_policy() {
        FinishPolicy::LastStanding { elimination } => elimination,
        FinishPolicy::Endless | FinishPolicy::Scripted => return,
    };
    if !session.is_active() {
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

    // The players a standing building keeps in the game this tick: holding
    // one themselves, or — under side scope — allied with a player who does.
    // A holder outside the accounting (an environment combatant) keeps no one
    // standing: it is on no team, so it is allied with no player.
    let session = world.resource::<GameSession>();
    let standing: BTreeSet<PlayerId> = occupied
        .iter()
        .copied()
        .filter(|&player| match elimination {
            EliminationScope::Player => with_building.contains(&player),
            EliminationScope::Side => with_building
                .iter()
                .any(|&holder| session.are_allied(player, holder)),
        })
        .collect();

    let mut session = world.resource_mut::<GameSession>();

    // A player no building keeps standing is eliminated as of the next tick:
    // this tick still executed its input on every node, the next requires
    // none of it.
    let eliminated_from = session.tick() + 1;
    for &player in &occupied {
        if !standing.contains(&player) && !session.is_player_out(player) {
            session.eliminate_player(player, eliminated_from);
        }
    }

    // A player survives while a building keeps it standing and it is not out
    // of the game — a building finished by a leftover order does not revive
    // an eliminated player.
    let survivors: Vec<PlayerId> = occupied
        .iter()
        .copied()
        .filter(|player| standing.contains(player) && !session.is_player_out(*player))
        .collect();

    // The local player is defeated once no survivor is on its side — itself
    // included — provided the node fields a player at all (an observer's
    // node has no side to lose) and it is not dropped: a dropped node's
    // ending is decided by the network layer (`Aborted`), never called a
    // defeat.
    let local_out = match session.local_player() {
        None => false,
        Some(local) => {
            occupied.contains(&local)
                && !session.is_player_dropped(local)
                && !survivors
                    .iter()
                    .any(|&survivor| session.are_allied(local, survivor))
        }
    };

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
        // Two or more unallied survivors: the match goes on. The local
        // player's defeat — its side out while others fight on — is answered
        // by this node's conduct: conclude at the losing frame, or keep
        // simulating so the player watches the match play out. Every other
        // node decides the same for its own local player.
        _ if local_out => match session.defeat_conduct() {
            DefeatConduct::Conclude => GameResult::Defeat,
            DefeatConduct::Spectate => return,
        },
        _ => return,
    };

    session.finish(result);
}
