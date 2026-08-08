//! Victory condition under `LastStanding`: the last team (or lone player) with a
//! building standing wins, a player whose buildings are all gone is defeated, and
//! the local player hears of its own elimination even while other teams fight on.
//! Under `Endless` the session never ends on its own.

mod utils;

use bevy::prelude::{App, Entity};
use ferrets_geometry::cell_size::CellSize;
use ferrets_simulation::{
    content::{entity_type_def::EntityTypeDef, location::Solidity, registry::ContentRegistry},
    session::{
        GameResult, GameSession, Winner,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot, TeamId},
        player_type::PlayerType,
    },
    spawn,
};

use utils::GROUND;

#[test]
fn last_standing_wins_when_only_opponent_base_is_destroyed() {
    let (mut app, bases) = bases_app(&[None, None]);

    // Both players hold a base, so the game is in progress.
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);

    // Player 1's base falls — player 0 is the last one standing and wins.
    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        })
    );
}

#[test]
fn surviving_unit_does_not_save_player_whose_last_building_falls() {
    let (mut app, bases) = bases_app(&[None, None]);

    // Player 1 keeps a soldier on the field, but only buildings count.
    let world = app.world_mut();
    spawn::spawn_entity(world, "soldier", utils::pos(20, 20), Some(1)).unwrap();
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);

    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        })
    );
}

#[test]
fn endless_never_finishes_even_when_a_base_is_destroyed() {
    let (mut app, bases) = bases_app(&[None, None]);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::Endless);

    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);
}

#[test]
fn dropped_player_is_excluded_so_game_resolves_among_rest() {
    let (mut app, bases) = bases_app(&[None, None, None]);

    // All three hold a base → in progress.
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);

    // Player 2 drops; its base lingers but it no longer counts as a survivor,
    // and players 0 and 1 are still in it.
    let tick = app.world().resource::<GameSession>().tick();
    app.world_mut()
        .resource_mut::<GameSession>()
        .drop_player(2, tick);
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);

    // Player 1's base falls; only player 0 remains, even though player 2's base
    // still stands on the map.
    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        })
    );
}

#[test]
fn local_player_is_eliminated_when_its_base_falls_while_others_fight() {
    // A free-for-all: the local player 0 against two others, no teams.
    let (mut app, bases) = bases_app(&[None, None, None]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);

    // The local player's base is destroyed while players 1 and 2 keep theirs.
    // Two teams remain, so there is no winner yet — but the local player learns
    // of its own defeat at once, instead of spectating forever.
    destroy(&mut app, bases[0]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), Some(GameResult::Defeat));
}

#[test]
fn building_less_player_is_eliminated_from_next_tick() {
    // Free-for-all: player 1's base falls while players 0 and 2 fight on. The
    // elimination is derived from the simulation itself, so from the next tick
    // on no input is required from player 1 — the survivors keep playing
    // without waiting for frames its node will never send, and without a drop.
    let (mut app, bases) = bases_app(&[None, None, None]);
    utils::run_ticks(&mut app, 1);

    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);

    let session = app.world().resource::<GameSession>();
    assert_eq!(session.result(), None);
    assert!(session.is_player_eliminated(1));
    assert!(!session.is_player_dropped(1));
    assert_eq!(session.required_players(session.tick()), vec![0, 2]);
}

#[test]
fn eliminated_player_regaining_building_stays_out() {
    // Elimination is permanent: a building finished by a leftover order after
    // the defeat does not bring the player back into the match, and cannot win
    // it for them.
    let (mut app, bases) = bases_app(&[None, None, None]);
    utils::run_ticks(&mut app, 1);

    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);
    assert!(
        app.world()
            .resource::<GameSession>()
            .is_player_eliminated(1)
    );

    // A new building appears for the eliminated player — as a build order
    // still in flight at the defeat would place one.
    spawn::spawn_entity(app.world_mut(), "base", utils::pos(14, 2), Some(1)).expect("late base");
    destroy(&mut app, bases[2]);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        })
    );
}

#[test]
fn allied_team_wins_when_the_other_team_is_eliminated() {
    // Two-on-two: players 0 and 1 on team 1, players 2 and 3 on team 2.
    let (mut app, bases) = bases_app(&[Some(1), Some(1), Some(2), Some(2)]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);

    // Team 2's bases both fall; team 1 wins as a team.
    destroy(&mut app, bases[2]);
    destroy(&mut app, bases[3]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Team(1)
        })
    );
}

#[test]
fn one_team_standing_wins_when_member_has_fallen() {
    // Two-on-two: destroy one whole team plus one member of the other.
    let (mut app, bases) = bases_app(&[Some(1), Some(1), Some(2), Some(2)]);
    utils::run_ticks(&mut app, 1);

    destroy(&mut app, bases[1]);
    destroy(&mut app, bases[2]);
    destroy(&mut app, bases[3]);
    utils::run_ticks(&mut app, 1);
    // Only team 1 remains, through player 0.
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Team(1)
        })
    );
}

#[test]
fn all_allied_lineup_wins_at_once() {
    // Both players on the same team: with no opposing side they are already the
    // last team standing, so the team wins immediately. (A game meant to run on
    // without a verdict uses the Endless policy.)
    let (mut app, _) = bases_app(&[Some(1), Some(1)]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Team(1)
        })
    );
}

#[test]
fn draw_when_last_teams_fall_together() {
    let (mut app, bases) = bases_app(&[None, None]);
    utils::run_ticks(&mut app, 1);

    destroy(&mut app, bases[0]);
    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), Some(GameResult::Draw));
}

//
// ─── Environment slots ────────────────────────────────────────────────────────
//

#[test]
fn environment_base_does_not_block_victory() {
    // Two unallied players plus an environment combatant holding its own base.
    let (mut app, bases, _) = bases_app_with_environment(&[None, None]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(result(&app), None);

    // Player 1 falls. The environment's base still stands, yet player 0 wins —
    // an environment is not surviving opposition.
    destroy(&mut app, bases[1]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        })
    );
}

#[test]
fn environment_without_building_is_not_eliminated() {
    // The environment slot loses its only building; the building-less
    // elimination sweep must pass it by, and it keeps feeding input.
    let (mut app, _, environment_base) = bases_app_with_environment(&[None, None]);
    let environment = environment_id(&[None, None]);
    destroy(&mut app, environment_base);
    utils::run_ticks(&mut app, 3);

    let session = app.world().resource::<GameSession>();
    assert_eq!(session.result(), None);
    assert!(!session.is_player_eliminated(environment));
    assert!(
        session
            .required_players(session.tick())
            .contains(&environment)
    );
}

#[test]
fn lone_player_beside_environment_wins_at_once() {
    // A single lobby player has no opposing side to outlast — the environment
    // does not count as one — so the lineup wins immediately, like any other
    // one-sided lineup under `LastStanding`.
    let (mut app, _, _) = bases_app_with_environment(&[None]);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        result(&app),
        Some(GameResult::Victory {
            winner: Winner::Player(0)
        })
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A started `LastStanding` app seating one player per entry in `teams` (each a
/// team id, or `None` for no team), with the local player at slot `0`. The roster
/// is one building type, `base`, and one `soldier`; every player is given a base,
/// and the bases are returned indexed by player id.
fn bases_app(teams: &[Option<TeamId>]) -> (App, Vec<Entity>) {
    let slots = teams
        .iter()
        .enumerate()
        .map(|(id, team)| PlayerSlot::occupied(id as PlayerId, PlayerType::Human, None, *team))
        .collect();
    let mut app = utils::make_app(slots);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding);
    register_bases_content(&mut app);
    let bases = {
        let world = app.world_mut();
        teams
            .iter()
            .enumerate()
            .map(|(id, _)| {
                let (entity, _) = spawn::spawn_entity(
                    world,
                    "base",
                    utils::pos(2 + id as u32 * 4, 2),
                    Some(id as PlayerId),
                )
                .expect("base placement");
                entity
            })
            .collect()
    };
    app.world_mut().resource_mut::<GameSession>().start();
    (app, bases)
}

/// Like [`bases_app`], with one extra environment AI slot seated after the
/// lobby players, holding its own base (returned last).
fn bases_app_with_environment(teams: &[Option<TeamId>]) -> (App, Vec<Entity>, Entity) {
    let environment = environment_id(teams);
    let mut slots: Vec<PlayerSlot> = teams
        .iter()
        .enumerate()
        .map(|(id, team)| PlayerSlot::occupied(id as PlayerId, PlayerType::Human, None, *team))
        .collect();
    slots.push(PlayerSlot::environment(environment));

    let mut app = utils::make_app(slots);
    app.world_mut()
        .resource_mut::<GameSession>()
        .set_finish_policy(FinishPolicy::LastStanding);
    register_bases_content(&mut app);

    let world = app.world_mut();
    let bases: Vec<Entity> = teams
        .iter()
        .enumerate()
        .map(|(id, _)| {
            let (entity, _) = spawn::spawn_entity(
                world,
                "base",
                utils::pos(2 + id as u32 * 4, 2),
                Some(id as PlayerId),
            )
            .expect("base placement");
            entity
        })
        .collect();
    let (environment_base, _) =
        spawn::spawn_entity(world, "base", utils::pos(2, 8), Some(environment))
            .expect("environment base placement");

    app.world_mut().resource_mut::<GameSession>().start();
    (app, bases, environment_base)
}

/// The slot id the environment player takes: the one past the lobby players.
fn environment_id(teams: &[Option<TeamId>]) -> PlayerId {
    teams.len() as PlayerId
}

/// Registers the roster the suite uses: one building type, `base`, and one
/// non-building `soldier`.
fn register_bases_content(app: &mut App) {
    let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
    registry.register(
        EntityTypeDef::new("base")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(30)
            .with_dying(2, None)
            .with_tags(["building"]),
    );
    registry.register(
        EntityTypeDef::new("soldier")
            .with_location(GROUND, CellSize::ONE, Solidity::Solid)
            .with_health(30)
            .with_dying(2, None),
    );
    registry.validate();
}

/// Starts the given entity's dying phase, taking it out of the standing-building
/// count on the next check.
fn destroy(app: &mut App, entity: Entity) {
    spawn::destroy_entity(app.world_mut(), entity);
}

fn result(app: &App) -> Option<GameResult> {
    app.world().resource::<GameSession>().result()
}
