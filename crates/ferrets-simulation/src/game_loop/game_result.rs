//! Victory condition: a player wins when every opponent's entities are gone.

use std::collections::BTreeSet;

use bevy_ecs::world::World;

use crate::{
    components::owner::OwnerComponent,
    entity_index::EntityIndex,
    session::{GameResult, GameSession, finish_policy::FinishPolicy, player_slot::PlayerId},
};

/// Ends the session, under [`FinishPolicy::LastStanding`], once at most one
/// occupied player still has entities on the field.
///
/// Counts each player's remaining entities — both alive and still dying, so a
/// player counts as present until their last entity has finished its death and
/// despawned. The game ends only when two or more players are occupied: the lone
/// survivor among them wins, and if none survive it is a draw. Dropped players are
/// excluded from the survivors (they are still counted as occupied, so a 2-player
/// game whose other side dropped still resolves), but kept on the map idle. Under
/// [`FinishPolicy::Endless`] it never ends. Runs at the end of a tick, after
/// deaths have been resolved.
pub fn check(world: &mut World) {
    let session = world.resource::<GameSession>();
    if !session.is_active() || session.finish_policy() == FinishPolicy::Endless {
        return;
    }

    let occupied: Vec<PlayerId> = session
        .slots()
        .iter()
        .filter(|slot| slot.player_type().is_some())
        .map(|slot| slot.id())
        .collect();
    if occupied.len() < 2 {
        return;
    }

    // Players that still have at least one entity this tick, dying ones included.
    let mut surviving: BTreeSet<PlayerId> = BTreeSet::new();
    let index = world.resource::<EntityIndex>();
    let entities: Vec<_> = index
        .alive_entries()
        .into_iter()
        .chain(index.dying_entries())
        .map(|(_, entity)| entity)
        .collect();
    for entity in entities {
        if let Some(owner) = world.entity(entity).get::<OwnerComponent>() {
            surviving.insert(owner.player());
        }
    }

    // A dropped player is never a survivor (its lingering idle units don't count),
    // but it stays in `occupied` above so a 2-player game still meets the
    // two-occupied bar and resolves to the other player.
    let remaining: Vec<PlayerId> = occupied
        .into_iter()
        .filter(|player| surviving.contains(player) && !session.is_player_dropped(*player))
        .collect();

    let result = match remaining.as_slice() {
        [winner] => GameResult::Victory { winner: *winner },
        [] => GameResult::Draw,
        // Two or more players still in the game.
        _ => return,
    };

    world.resource_mut::<GameSession>().finish(result);
}
