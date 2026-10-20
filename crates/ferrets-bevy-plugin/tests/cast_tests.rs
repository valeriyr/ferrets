//! Casting at a body: the Remains aim, the Summon effect, and the Cast order
//! that walks a caster into its reach before either happens.

use bevy::prelude::*;
use ferrets_content::{
    affiliation::Affiliation,
    attack::Slain,
    cost::Cost,
    detection::Detection,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::{EntityTypeDef, EntityTypeId},
    field::{
        Emission, FieldAction, FieldCoverage, FieldDecay, FieldDef, FieldGrowth, FieldLayer,
        FieldPlacement, FieldSourceDef, FieldVision,
    },
    kinds::Kinds,
    landing::Landing,
    location::Solidity,
    player_buffs::PlayerBuffDef,
    pool::Pool,
    pool_def::PoolId,
    price,
    quantity::Quantity,
    registry::ContentRegistry,
    resource::DepletionPolicy,
    skills::{
        Casting, EntityCastEffect, PaymentPoint, Reach, Recipient, Refund, SkillCaster, SkillDef,
        SummonArrival, SummonAt, Whom,
    },
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{
    buffs_store::Term,
    command::{PlayerCommand, SkillTarget},
    components::{
        entity_info::EntityInfoComponent, entity_skills::SkillsComponent, pools,
        resource::ResourceSourceComponent, rise::RiseOrigin,
    },
    cooldown::Cooldown,
    entity_def,
    entity_index::EntityIndex,
    events::{DeathCause, SimulationEvent, SpawnCause},
    game_loop::{damage, fields},
    order::Order,
    player_buffs::PlayerBuffs,
    session::{player_id::PlayerId, player_slot::PlayerSlot, player_type::PlayerType},
    simulation_id::SimulationId,
    spawn,
};

mod utils;

//
// ─── Raising from a body ────────────────────────────────────────────────────
//

#[test]
fn raise_spends_body_and_sets_down_what_it_makes() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_none(),
        "the body the cast was aimed at is spent"
    );
    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "exactly the two the skill names stand up"
    );
    assert!(
        owners(&app, "skeleton").all(|owner| owner == Some(0)),
        "what a caster raises is the caster's own"
    );
}

#[test]
fn raise_reads_its_count_off_caster() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_host", body);
    utils::run_ticks(&mut app, 5);

    // `host_size` 3.5, truncated: three whole skeletons.
    assert_eq!(count_of(&app, "skeleton"), 3);
}

#[test]
fn raise_announces_body_it_spent() {
    let mut app = app();
    utils::record_announcements(&mut app);
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    let announced = &app.world().resource::<utils::Announced>().0;
    assert!(
        announced.iter().any(|event| matches!(
            event,
            SimulationEvent::RemainsSpent { remains, position, .. }
                if *remains == body && *position == utils::pos(6, 5)
        )),
        "spending a body is announced, so a game can play it"
    );
    assert!(
        announced.iter().any(|event| matches!(
            event,
            SimulationEvent::EntitySpawned {
                cause: SpawnCause::Raised { from, .. },
                ..
            } if *from == body
        )),
        "what comes up remembers the body it came out of"
    );
}

#[test]
fn raising_is_not_production() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "the raise happened, so the tally below is about what it counts as"
    );
    let skeleton = app
        .world()
        .resource::<ContentRegistry>()
        .type_id("skeleton")
        .expect("the type is registered");
    assert_eq!(
        utils::produced(&app, skeleton),
        0,
        "a spell that lasts a while is not a unit anyone produced"
    );
}

#[test]
fn raise_aimed_at_living_is_refused() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let (_, standing) = utils::create_owned(&mut app, "soldier", 6, 5, 1);

    raise(&mut app, necromancer, standing);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a raise names a body, and what still stands is not one"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "a refused cast pays nothing"
    );
}

#[test]
fn summon_with_nowhere_to_stand_pays_nothing() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // The colossus is wider than the map, so no cell can hold one: the cast is
    // refused before anything is spent.
    utils::cast_on(&mut app, necromancer, "raise_colossus", body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "colossus"),
        0,
        "a summon with nowhere to stand raises nothing"
    );
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_some(),
        "the body is still lying there"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "nothing was paid for the cast that did not happen"
    );
}

#[test]
fn summon_sets_down_only_where_summoned_types_placement_rules_admit() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);
    let blight = app
        .world()
        .resource::<ContentRegistry>()
        .field("blight")
        .expect("blight is registered");
    fields::apply_action(
        app.world_mut(),
        1,
        blight,
        CellPos::new(6, 5),
        0,
        FieldAction::Cover,
    );

    utils::cast_on(&mut app, necromancer, "raise_ghouls", body);
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let mut cells: Vec<CellPos> = utils::owned_of_type(app.world_mut(), "ghoul", 0)
        .into_iter()
        .map(|ghoul| utils::cell_of(app.world(), ghoul))
        .collect();
    cells.sort_unstable_by_key(|cell| (cell.y, cell.x));
    // Unblighted, the two would rise at (6, 4) and on the body's own cell,
    // (6, 5); blight holds (6, 5), so the second takes (7, 5).
    assert_eq!(cells, vec![CellPos::new(6, 4), CellPos::new(7, 5)]);
}

#[test]
fn summon_over_supply_ceiling_pays_nothing() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // Two wardens cost two supply each and the player provides none, so half a
    // summon is not offered: the cast does nothing at all.
    utils::cast_on(&mut app, necromancer, "raise_wardens", body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(count_of(&app, "warden"), 0, "half a summon is not offered");
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_some(),
        "the body is still lying there"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "and nothing was paid for it"
    );
}

//
// ─── The walk into reach ────────────────────────────────────────────────────
//

#[test]
fn caster_walks_into_its_reach_before_casting() {
    let mut app = app();
    let necromancer = caster(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    raise(&mut app, necromancer, body);
    // Twelve cells away with a reach of four: long past the tick an unranged
    // caster would have cast on, and still nothing has been raised.
    utils::run_ticks(&mut app, 5);
    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a caster out of reach walks rather than casting across the map"
    );

    utils::run_ticks(&mut app, 120);
    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "having closed to its reach, it casts"
    );
}

#[test]
fn body_gone_before_caster_arrives_ends_order() {
    let mut app = app();
    let necromancer = caster(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);
    // Someone else's necromancer got there first.
    let other = caster(&mut app, 13, 5);
    raise(&mut app, other, body);
    utils::run_ticks(&mut app, 120);

    let entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(necromancer)
        .expect("the caster is still standing");
    assert!(
        utils::order_queue_is_empty(app.world_mut(), entity),
        "an aim that is gone finishes the order rather than holding the caster"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "and the caster paid nothing for the body it never reached"
    );
}

#[test]
fn caster_without_reach_casts_where_it_stands() {
    let mut app = app();
    // The acolyte's raise declares no reach, so it lands at once however far
    // off the body is.
    let acolyte = summoner(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    utils::cast_on(&mut app, acolyte, "raise_where_it_stands", body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "an unranged cast is instant, as every cast was before reaches existed"
    );
}

#[test]
fn body_in_fog_cannot_be_raised() {
    let mut app = app();
    // The acolyte sees twenty cells and casts where it stands, so what stops
    // this raise is the fog over the body and nothing else.
    let acolyte = summoner(&mut app, 2, 2);
    let body = fallen(&mut app, 29, 29);

    utils::cast_on(&mut app, acolyte, "raise_where_it_stands", body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a cast named at what the fog hides is a cast the player could not have known to make"
    );
    assert_eq!(
        energy_as_u32(&app, acolyte),
        100,
        "and it paid nothing for it"
    );
}

#[test]
fn rising_summon_arrives_rising_on_its_own() {
    let mut app = app();
    let acolyte = summoner(&mut app, 5, 5);

    utils::cast_self(&mut app, acolyte, "raise_mound");
    utils::run_ticks(&mut app, utils::APPLY + 1);

    let mound = utils::single_owned_of_type(app.world_mut(), "mound", 0);
    let raise_mound = utils::skill_id(&app, "raise_mound");
    assert_eq!(
        entity_def::rise_origin(app.world(), mound),
        Some(RiseOrigin::Summoned { skill: raise_mound })
    );
    utils::run_ticks(&mut app, MOUND_RISE_TIME);
    assert!(!entity_def::rising(app.world(), mound));
}

#[test]
fn rising_summon_is_not_canceled() {
    let mut app = app();
    let acolyte = summoner(&mut app, 5, 5);
    utils::cast_self(&mut app, acolyte, "raise_mound");
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let mound = utils::single_owned_of_type(app.world_mut(), "mound", 0);
    let mound_id = entity_def::simulation_id(app.world(), mound);

    // A summon is a cast that has landed already: nothing calls it off.
    utils::push_command(&mut app, PlayerCommand::CancelBuild { site: mound_id });
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(utils::alive(&app, mound_id));
    assert!(entity_def::rising(app.world(), mound));
}

#[test]
fn rising_summon_counts_toward_no_production() {
    let mut app = app();
    let acolyte = summoner(&mut app, 5, 5);
    utils::cast_self(&mut app, acolyte, "raise_mound");
    utils::run_ticks(&mut app, utils::APPLY + 1 + MOUND_RISE_TIME);

    let mound = utils::single_owned_of_type(app.world_mut(), "mound", 0);
    assert!(!entity_def::rising(app.world(), mound));
    let mound_type = entity_def::type_id(app.world(), mound);
    assert_eq!(utils::produced(&app, mound_type), 0);
}

#[test]
fn summon_lands_only_inside_its_field_rules() {
    let mut app = app();
    utils::create_owned(&mut app, "lantern", 10, 10, 0);
    let acolyte = summoner(&mut app, 2, 2);
    utils::run_ticks(&mut app, 1);

    // (20, 20) lies outside the lantern's glow of two cells; (10, 12) inside.
    utils::cast_at(&mut app, acolyte, "glow_mound", 20, 20);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(count_of(&app, "mound"), 0);

    utils::cast_at(&mut app, acolyte, "glow_mound", 10, 12);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(count_of(&app, "mound"), 1);
}

#[test]
fn rising_summon_losing_its_field_is_lost_unrefunded() {
    let mut app = app();
    let (lantern, _) = utils::create_owned(&mut app, "lantern", 10, 10, 0);
    let acolyte = summoner(&mut app, 2, 2);
    utils::run_ticks(&mut app, 1);
    utils::cast_at(&mut app, acolyte, "glow_mound", 10, 12);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let mound = utils::single_owned_of_type(app.world_mut(), "mound", 0);
    let mound_id = entity_def::simulation_id(app.world(), mound);
    assert!(entity_def::rising(app.world(), mound));

    utils::record_announcements(&mut app);
    spawn::despawn_entity(app.world_mut(), lantern, DeathCause::Decayed);
    utils::run_ticks(&mut app, 1);

    // Ended before it had risen: neither a loss nor a kill.
    let announced = &app.world().resource::<utils::Announced>().0;
    assert!(announced.iter().any(|event| matches!(
        event,
        SimulationEvent::EntityDied { entity, cause: DeathCause::Canceled, .. } if *entity == mound_id
    )));

    assert!(!utils::alive(&app, mound_id));
    // Paid 30 of 100 and given nothing back.
    assert_eq!(energy_as_u32(&app, acolyte), 70);
}

#[test]
fn summon_raised_over_source_covers_it_and_finds_none_far_off() {
    let mut app = app();
    let (seam, seam_id) = utils::create_entity(app.world_mut(), "seam", utils::pos(10, 10), None)
        .expect("the seam is registered");
    app.world_mut()
        .get_mut::<ResourceSourceComponent>(seam)
        .expect("a seam is a resource source")
        .amount = 40;
    let acolyte = summoner(&mut app, 2, 2);
    utils::run_ticks(&mut app, 1);

    // No seam within the placement search of (28, 28): nothing is summoned.
    utils::cast_at(&mut app, acolyte, "raise_spire", 28, 28);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(count_of(&app, "spire"), 0);

    utils::record_announcements(&mut app);
    utils::cast_at(&mut app, acolyte, "raise_spire", 10, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let spire = utils::single_owned_of_type(app.world_mut(), "spire", 0);
    assert_eq!(utils::cell_of(app.world(), spire), CellPos::new(10, 10));
    assert!(entity_def::rising(app.world(), spire));
    // The seam is covered, not merely gone: it dies as overbuilt and the spire
    // holds the 40 it had left.
    let announced = &app.world().resource::<utils::Announced>().0;
    assert!(announced.iter().any(|event| matches!(
        event,
        SimulationEvent::EntityDied { entity, cause: DeathCause::Overbuilt, .. }
            if *entity == seam_id
    )));
    assert_eq!(
        app.world()
            .get::<ResourceSourceComponent>(spire)
            .expect("a spire is a resource source")
            .amount,
        40
    );
}

#[test]
fn summon_over_spring_walker_stands_on_raises_nothing_until_walker_dies() {
    let mut app = app();
    utils::create_entity(app.world_mut(), "spring", utils::pos(10, 10), None)
        .expect("the spring is registered");
    // The spring holds no ground of its own, so a soldier stands on it.
    let (soldier, _) = utils::create_owned(&mut app, "soldier", 10, 10, 1);
    let acolyte = summoner(&mut app, 2, 2);
    utils::run_ticks(&mut app, 1);

    utils::cast_at(&mut app, acolyte, "raise_well", 10, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(count_of(&app, "well"), 0);
    assert_eq!(count_of(&app, "spring"), 1);

    spawn::despawn_entity(app.world_mut(), soldier, DeathCause::Decayed);
    utils::cast_at(&mut app, acolyte, "raise_well", 10, 10);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    let well = utils::single_owned_of_type(app.world_mut(), "well", 0);
    assert_eq!(utils::cell_of(app.world(), well), CellPos::new(10, 10));
    assert_eq!(count_of(&app, "spring"), 0);
}

#[test]
fn sighted_summon_lands_only_where_its_side_sees() {
    let mut app = app();
    // The acolyte sees twenty cells: (10, 2) lies in sight, (29, 29) in fog.
    let acolyte = summoner(&mut app, 2, 2);

    utils::cast_at(&mut app, acolyte, "sight_mound", 29, 29);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(count_of(&app, "mound"), 0);

    utils::cast_at(&mut app, acolyte, "sight_mound", 10, 2);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(count_of(&app, "mound"), 1);
}

#[test]
fn summon_with_room_for_only_one_raises_none() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // Every cell on the map held but the one beside the body, so the search
    // around it finds exactly one place to stand where the raise needs two.
    // A body lies on held ground, so this leaves the aim itself untouched.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::free_cell(&mut app, CellPos::new(7, 5));

    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 5);

    assert_eq!(count_of(&app, "skeleton"), 0, "half a raise is not offered");
    assert!(
        app.world()
            .resource::<EntityIndex>()
            .remains(body)
            .is_some(),
        "the body is still lying there"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "and nothing was paid for it"
    );
}

//
// ─── Working at a cast ──────────────────────────────────────────────────────
//

#[test]
fn worked_cast_lands_on_its_point() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_slowly", body);
    // The command waits out the input delay (utils::APPLY = 3), and the tick
    // it applies on is the first tick of work: the skeletons rise on the fifth
    // of those, the seventh from here — 3 + 5 - 1.
    utils::run_ticks(&mut app, 6);

    assert_eq!(count_of(&app, "skeleton"), 0, "the work is not done yet");
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "and nothing is spent while it is under way"
    );

    utils::run_ticks(&mut app, 1);

    assert_eq!(count_of(&app, "skeleton"), 2, "the fifth tick raises them");
    assert_eq!(
        energy_as_u32(&app, necromancer),
        60,
        "and pays for them then: 100 - 40"
    );
}

#[test]
fn worked_cast_walks_in_before_its_work_starts() {
    let mut app = app();
    let necromancer = caster(&mut app, 2, 5);
    let body = fallen(&mut app, 14, 5);

    // Twelve cells out with a reach of four: the work cannot start until the
    // walk is over, so the cast lands far later than its five-tick point.
    utils::cast_on(&mut app, necromancer, "raise_slowly", body);
    utils::run_ticks(&mut app, 12);

    assert_eq!(
        count_of(&app, "skeleton"),
        0,
        "a caster still walking is a caster not yet working"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "and it pays for the cast at its point, not for setting off"
    );

    utils::run_ticks(&mut app, 120);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "having closed, it works and raises"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        60,
        "paying then: 100 - 40"
    );
}

#[test]
fn caster_stands_through_what_is_left_of_its_cast() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_slowly", body);
    // Landed on the seventh tick, with a period of eight: three more to stand
    // through before the order is done with it.
    utils::run_ticks(&mut app, 8);

    assert_eq!(count_of(&app, "skeleton"), 2, "the cast has landed");
    assert!(
        !orders_of(&app, necromancer).is_empty(),
        "and the caster is still at it"
    );

    utils::run_ticks(&mut app, 2);

    assert!(
        orders_of(&app, necromancer).is_empty(),
        "the cast lets go once its period is out"
    );
}

#[test]
fn point_pulled_under_work_already_done_lands_at_once() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_by_stat", body);
    // Four ticks of work against a point of ten, then a buff pulls the point
    // to two — under the work already done. A cast has no next cycle to fire
    // on, so it lands the very next tick rather than being skipped.
    utils::run_ticks(&mut app, utils::APPLY + 3);
    assert_eq!(count_of(&app, "skeleton"), 0, "the work is not done yet");

    apply_buff(&mut app, "quickened_rites");
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "a point that moved under the phase already worked lands on the next tick"
    );
}

#[test]
fn fractional_cast_point_rounds_up() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // A point of 10 − 0.75 = 9.25 ticks, read as 10: not landed after the
    // ninth tick of work, landed on the tenth.
    apply_buff(&mut app, "trimmed_rites");
    utils::cast_on(&mut app, necromancer, "raise_by_stat", body);
    utils::run_ticks(&mut app, utils::APPLY + 8);
    assert_eq!(count_of(&app, "skeleton"), 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(count_of(&app, "skeleton"), 2);
}

#[test]
fn cast_landing_past_its_period_frees_caster_at_once() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // A buff draws the point out to twenty, past the twelve-tick period the
    // skill is freed at — content may not author that, but a stat may reach
    // it. The cast lands on the twentieth tick of work with the period long
    // gone, so the caster is free the same tick it casts.
    apply_buff(&mut app, "drawn_out_rites");
    utils::cast_on(&mut app, necromancer, "raise_by_stat", body);
    utils::run_ticks(&mut app, utils::APPLY + 19);

    assert_eq!(count_of(&app, "skeleton"), 2, "the cast landed");
    assert!(
        orders_of(&app, necromancer).is_empty(),
        "and nothing holds the caster past a period its point outran"
    );
}

#[test]
fn cast_lands_once_though_its_cooldown_comes_round_first() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    // Five ticks to the point, eight to the end, and two of cooldown: the
    // skill is ready again while the caster is still standing through its own
    // cast, so only the cast's own stage stops it raising twice.
    utils::cast_on(&mut app, necromancer, "raise_slowly", body);
    utils::run_ticks(&mut app, utils::APPLY + 8);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "a cast lands once, whatever the phase does after it"
    );
}

#[test]
fn cast_that_kills_its_caster_ends_as_death() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);

    // The queue is out of the world while an order runs, so a caster the cast
    // killed would keep a dying phase nothing counts down unless the order
    // itself says it died.
    utils::cast_self(&mut app, necromancer, "self_immolate");
    utils::run_ticks(&mut app, utils::APPLY + 8);

    assert!(
        app.world()
            .resource::<EntityIndex>()
            .any(necromancer)
            .is_none(),
        "a caster its own cast killed leaves the world rather than lingering dying"
    );
}

#[test]
fn skill_pressed_on_cooldown_leaves_caster_at_its_work() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    // Both bodies first: laying one costs ticks, and what this test is about
    // is how few of them pass between the raise and the press that follows it.
    let body = fallen(&mut app, 6, 5);
    let second = fallen(&mut app, 7, 5);

    // One raise, then a walk, then the same skill pressed while it is still
    // cooling — three ticks of a ten-tick cooldown gone. The walk must
    // survive: a cast that will not happen is no reason to drop what the
    // caster was doing.
    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, utils::APPLY + 1);
    utils::select(&mut app, necromancer);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 12),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY);

    raise(&mut app, necromancer, second);
    utils::run_ticks(&mut app, utils::APPLY);

    assert!(
        orders_of(&app, necromancer)
            .iter()
            .any(|order| matches!(order, Order::Move { .. })),
        "the walk the caster was on outlives a cast that never starts"
    );
}

#[test]
fn cast_cut_short_before_its_point_costs_nothing() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_slowly", body);
    // Two ticks in: the work has started and has three more to run.
    utils::run_ticks(&mut app, 4);
    // Sent somewhere else mid-cast: the order goes, and the work with it.
    utils::select(&mut app, necromancer);
    utils::push_command(
        &mut app,
        PlayerCommand::Move {
            target: utils::pos(12, 12),
            flush: true,
        },
    );
    utils::run_ticks(&mut app, utils::APPLY + 8);

    assert_eq!(count_of(&app, "skeleton"), 0, "nothing was raised");
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "and the energy was never spent"
    );
    assert_eq!(
        app.world()
            .resource::<EntityIndex>()
            .remains_entries()
            .len(),
        1,
        "the body it was working on still lies there"
    );
}

//
// ─── Paying for a cast ──────────────────────────────────────────────────────
//

#[test]
fn cast_paid_at_start_pays_on_its_first_tick_of_casting() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_committed", body);
    // The command applies on the third tick (utils::APPLY = 3), which is the
    // first tick of casting: paid then, four ticks before it lands.
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(count_of(&app, "skeleton"), 0, "nothing has landed yet");
    assert_eq!(
        energy_as_u32(&app, necromancer),
        60,
        "100 - 40, paid already"
    );
    // On its cooldown from the same tick, which the cooldown pass after the
    // orders has already counted down once: 10 - 1.
    assert_eq!(
        cooldown_of(&app, necromancer, "raise_committed"),
        Cooldown::Cooling { remaining: 9 }
    );

    // The fifth tick of casting lands it: 3 + 5 - 1 = 7 ticks in all.
    utils::run_ticks(&mut app, 4);
    assert_eq!(count_of(&app, "skeleton"), 2);
    assert_eq!(energy_as_u32(&app, necromancer), 60, "paid once, not again");
}

#[test]
fn cast_paid_at_start_is_announced_as_it_lands() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);
    utils::record_announcements(&mut app);

    utils::cast_on(&mut app, necromancer, "raise_committed", body);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(skill_casts(&app).is_empty(), "paid on tick 3, not yet cast");
    utils::run_ticks(&mut app, 4);

    assert_eq!(
        skill_casts(&app),
        vec![SimulationEvent::SkillCast {
            caster: necromancer,
            target: SkillTarget::Position(utils::pos(6, 5)),
            skill: utils::skill_id(&app, "raise_committed"),
        }],
        "announced once, on tick 7 as it lands"
    );
}

#[test]
fn cast_paid_at_start_cut_short_keeps_what_it_paid() {
    let mut app = app();
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_committed", body);
    utils::run_ticks(&mut app, 4);
    // Sent somewhere else on its second tick of casting.
    move_away(&mut app, necromancer);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(count_of(&app, "skeleton"), 0, "nothing was raised");
    assert_eq!(
        energy_as_u32(&app, necromancer),
        60,
        "100 - 40, never given back"
    );
    // Paid on tick 3, its cooldown counted down that tick and every tick
    // since, to tick 7: 10 - (7 - 3 + 1) = 5.
    assert_eq!(
        cooldown_of(&app, necromancer, "raise_committed"),
        Cooldown::Cooling { remaining: 5 }
    );
}

#[test]
fn refundable_cast_cut_short_gives_everything_back() {
    let mut app = app();
    utils::grant_gold(&mut app, 50);
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_refundable", body);
    utils::run_ticks(&mut app, 4);
    assert_eq!(utils::gold(app.world()), 40, "50 - 10, paid at the start");
    move_away(&mut app, necromancer);
    utils::run_ticks(&mut app, utils::APPLY);

    assert_eq!(count_of(&app, "skeleton"), 0);
    assert_eq!(energy_as_u32(&app, necromancer), 100, "the energy is back");
    assert_eq!(utils::gold(app.world()), 50, "and the gold, to who paid it");
    assert_eq!(
        cooldown_of(&app, necromancer, "raise_refundable"),
        Cooldown::Ready,
        "and the cooldown is reset"
    );
}

#[test]
fn refundable_cast_that_cannot_land_at_its_point_gives_everything_back() {
    let mut app = app();
    utils::grant_gold(&mut app, 50);
    let necromancer = caster(&mut app, 5, 5);
    let body = fallen(&mut app, 6, 5);
    utils::record_announcements(&mut app);
    // Room for exactly the two skeletons beside the body when casting starts.
    utils::set_all_cells_statically_occupied(app.world_mut(), true);
    utils::free_cell(&mut app, CellPos::new(7, 5));
    utils::free_cell(&mut app, CellPos::new(7, 6));

    utils::cast_on(&mut app, necromancer, "raise_refundable", body);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        energy_as_u32(&app, necromancer),
        60,
        "judged and paid at the start"
    );
    // One of the two cells is taken before the point: the raise no longer fits.
    utils::hold_cell(&mut app, CellPos::new(7, 6));
    utils::run_ticks(&mut app, 4);

    assert_eq!(count_of(&app, "skeleton"), 0, "half a raise is not offered");
    assert_eq!(
        energy_as_u32(&app, necromancer),
        100,
        "a cast that fizzles gives it back"
    );
    assert_eq!(utils::gold(app.world()), 50);
    assert_eq!(
        cooldown_of(&app, necromancer, "raise_refundable"),
        Cooldown::Ready,
        "and its cooldown"
    );
    assert!(skill_casts(&app).is_empty(), "and nothing was cast");
}

#[test]
fn refundable_cast_of_caster_disabled_mid_cast_gives_everything_back() {
    let mut app = app();
    utils::grant_gold(&mut app, 50);
    let (necromancer_entity, necromancer) = utils::create_owned(&mut app, "necromancer", 5, 5, 0);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_refundable", body);
    utils::run_ticks(&mut app, 4);
    utils::disable_for(&mut app, necromancer_entity, 20);
    utils::run_ticks(&mut app, 2);

    assert_eq!(count_of(&app, "skeleton"), 0);
    assert_eq!(energy_as_u32(&app, necromancer), 100, "the energy is back");
    assert_eq!(utils::gold(app.world()), 50, "and the gold");
    assert_eq!(
        cooldown_of(&app, necromancer, "raise_refundable"),
        Cooldown::Ready
    );
}

#[test]
fn caster_relocated_out_of_reach_mid_cast_breaks_cast() {
    let mut app = app();
    utils::grant_gold(&mut app, 50);
    let (necromancer_entity, necromancer) = utils::create_owned(&mut app, "necromancer", 5, 5, 0);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_refundable", body);
    utils::run_ticks(&mut app, 4);
    // Six cells from the body, two past the raise's reach of four: a cast
    // under way does not walk back.
    spawn::relocate(
        app.world_mut(),
        &[(necromancer_entity, CellPos::new(12, 5))],
    );
    utils::run_ticks(&mut app, 2);

    assert_eq!(count_of(&app, "skeleton"), 0);
    assert_eq!(energy_as_u32(&app, necromancer), 100, "refunded");
    assert!(orders_of(&app, necromancer).is_empty());
    assert_eq!(
        entity_def::position(app.world(), necromancer_entity),
        utils::pos(12, 5)
    );
}

#[test]
fn refunded_health_cost_holds_under_maximum() {
    let mut app = app();
    let (necromancer_entity, necromancer) = utils::create_owned(&mut app, "necromancer", 5, 5, 0);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "blood_rite", body);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        utils::health_as_u32(&app, necromancer_entity),
        30,
        "40 - 10"
    );
    pools::restore(
        app.world_mut(),
        necromancer_entity,
        PoolId::HEALTH,
        utils::fixed("5"),
    );
    move_away(&mut app, necromancer);
    utils::run_ticks(&mut app, utils::APPLY);

    // Mended to 35 while it cast, the 10 paid back is held at its 40, not 45.
    assert_eq!(utils::health_as_u32(&app, necromancer_entity), 40);
}

#[test]
fn caster_dying_mid_cast_gets_nothing_back() {
    let mut app = app();
    utils::grant_gold(&mut app, 50);
    let (necromancer_entity, necromancer) = utils::create_owned(&mut app, "necromancer", 5, 5, 0);
    let body = fallen(&mut app, 6, 5);

    utils::cast_on(&mut app, necromancer, "raise_refundable", body);
    utils::run_ticks(&mut app, 4);
    spawn::destroy_entity(app.world_mut(), necromancer_entity);
    utils::run_ticks(&mut app, 1);

    assert_eq!(
        utils::gold(app.world()),
        40,
        "50 - 10: a caster that died is owed nothing"
    );
}

#[test]
fn cast_without_reach_breaks_when_its_aim_is_gone() {
    let mut app = app();
    let acolyte = summoner(&mut app, 5, 5);
    let necromancer = caster(&mut app, 11, 5);
    let body = fallen(&mut app, 12, 5);

    // The acolyte starts working on tick 3 and would land on tick 3 + 5 - 1
    // = 7; the necromancer's instant raise spends the body on tick 4.
    utils::cast_on(&mut app, acolyte, "raise_from_afar", body);
    utils::run_ticks(&mut app, 1);
    raise(&mut app, necromancer, body);
    utils::run_ticks(&mut app, 4);

    assert_eq!(
        count_of(&app, "skeleton"),
        2,
        "the necromancer's two, and none of the acolyte's"
    );
    assert!(
        orders_of(&app, acolyte).is_empty(),
        "the acolyte's cast broke the tick its body was gone"
    );
    assert_eq!(energy_as_u32(&app, acolyte), 100, "having paid nothing");
}

#[test]
fn target_leaving_reach_breaks_cast_under_way() {
    let mut app = app();
    let (necromancer_entity, necromancer) = utils::create_owned(&mut app, "necromancer", 5, 5, 0);
    let (runner_entity, runner) = utils::create_owned(&mut app, "runner", 8, 5, 1);

    utils::cast_on(&mut app, necromancer, "slow_bolt", runner);
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(
        energy_as_u32(&app, necromancer),
        60,
        "paid as casting starts"
    );
    // The runner walks off at half a cell a tick: out of the bolt's four cells
    // well before its tenth tick of casting.
    utils::order_move(&mut app, runner_entity, 20, 5);
    utils::run_ticks(&mut app, 12);

    assert_eq!(
        utils::health_as_u32(&app, runner_entity),
        30,
        "the bolt never landed"
    );
    assert_eq!(
        energy_as_u32(&app, necromancer),
        60,
        "and what it paid stays spent"
    );
    assert!(
        orders_of(&app, necromancer).is_empty(),
        "the cast broke rather than chasing its target"
    );
    assert_eq!(
        entity_def::position(app.world(), necromancer_entity),
        utils::pos(5, 5),
        "the caster stood where it cast"
    );
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// App with two players and the raising roster: `raise_dead`, reaching four
/// cells, on a `necromancer` that can walk; `raise_where_it_stands`, reaching
/// nowhere, on a rooted `acolyte`; both cost 40 energy and make two
/// `skeleton`s from a body. Plus a `soldier` that leaves a `corpse` and two
/// summons that can never be set down — a `colossus` wider than the map and a
/// `warden` costing supply nobody provides.
fn app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        registry.register(
            EntityTypeDef::new("corpse")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_tags(["remains"])
                .with_stat(EntityStatId::LIFETIME, FixedU64::from_num(600)),
        );
        registry.register(
            EntityTypeDef::new("skeleton")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20)),
        );
        registry.register(
            EntityTypeDef::new("soldier")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(30))
                .with_dying(2, utils::leaves("corpse")),
        );
        registry.register(
            EntityTypeDef::new("colossus")
                .with_location(utils::GROUND, CellSize::new(40, 40), Solidity::Solid)
                .with_pool(Pool::health(20)),
        );
        let blight = registry.register_field(
            "blight",
            FieldDef::new(
                FieldLayer::Passable(utils::GROUND.into()),
                FieldDecay::Never,
                FieldVision::Dark,
                Detection::Blind,
            ),
        );
        registry.register(
            EntityTypeDef::new("ghoul")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20))
                .with_field_placement([FieldPlacement::Forbids { field: blight }]),
        );
        registry.register(
            EntityTypeDef::new("warden")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20))
                .with_stat(EntityStatId::SUPPLY_COST, FixedU64::from_num(2)),
        );
        let raises = |registry: &mut ContentRegistry, name: &str, summoned: &str, reach| {
            let entity_type = registry
                .type_id(summoned)
                .expect("the summoned type is registered");
            registry.register_skill(
                name,
                SkillDef {
                    cooldown: 10,
                    caster: SkillCaster::Entity {
                        costs: vec![Cost::Energy(FixedU64::from_num(40))],
                        casting: Casting::Instant,
                        effect: EntityCastEffect::Summon {
                            at: SummonAt::Fallen {
                                kinds: Kinds::Any,
                                reach,
                            },
                            entity_type,
                            count: Quantity::Constant(2),
                            landing: Landing::Anywhere,
                            within: Vec::new(),
                            arrives: SummonArrival::Standing,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        let raise_dead = raises(
            &mut registry,
            "raise_dead",
            "skeleton",
            Reach::Within(Quantity::Constant(4)),
        );
        let raise_colossus = raises(
            &mut registry,
            "raise_colossus",
            "colossus",
            Reach::Within(Quantity::Constant(4)),
        );
        let raise_wardens = raises(
            &mut registry,
            "raise_wardens",
            "warden",
            Reach::Within(Quantity::Constant(4)),
        );
        let raise_ghouls = raises(
            &mut registry,
            "raise_ghouls",
            "ghoul",
            Reach::Within(Quantity::Constant(4)),
        );
        // A cast that brings its own caster down.
        let self_immolate = registry.register_skill(
            "self_immolate",
            SkillDef {
                cooldown: 10,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Damage {
                        amount: FixedU64::from_num(1000),
                        on: Recipient::Caster,
                    },
                },
                requires: Vec::new(),
            },
        );
        // A raise the caster works at: the skeletons rise on the fifth tick and
        // it stands through three more before it takes another order. Its
        // cooldown is shorter than that hold, so nothing but the cast's own
        // stage stops it landing again while the caster stands.
        let raise_slowly = {
            let entity_type = registry
                .type_id("skeleton")
                .expect("the summoned type is registered");
            registry.register_skill(
                "raise_slowly",
                SkillDef {
                    cooldown: 2,
                    caster: SkillCaster::Entity {
                        costs: vec![Cost::Energy(FixedU64::from_num(40))],
                        casting: Casting::Delayed {
                            point: Quantity::Constant(5),
                            period: Quantity::Constant(8),
                            pays: PaymentPoint::AtLanding,
                        },
                        effect: EntityCastEffect::Summon {
                            at: SummonAt::Fallen {
                                kinds: Kinds::Any,
                                reach: Reach::Within(Quantity::Constant(4)),
                            },
                            entity_type,
                            count: Quantity::Constant(2),
                            landing: Landing::Anywhere,
                            within: Vec::new(),
                            arrives: SummonArrival::Standing,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        // A raise worked over a stat: what the caster reads for its point is
        // `ritual_time`, which a buff can move either way under it mid-cast.
        // Content may not author a point outside its period, so the two buffs
        // are what put it there.
        let ritual_time = registry.register_entity_stat("ritual_time", FixedU64::ONE);
        let raise_by_stat = {
            let entity_type = registry
                .type_id("skeleton")
                .expect("the summoned type is registered");
            registry.register_skill(
                "raise_by_stat",
                SkillDef {
                    cooldown: 10,
                    caster: SkillCaster::Entity {
                        costs: vec![Cost::Energy(FixedU64::from_num(40))],
                        casting: Casting::Delayed {
                            point: Quantity::Stat(ritual_time),
                            period: Quantity::Constant(12),
                            pays: PaymentPoint::AtLanding,
                        },
                        effect: EntityCastEffect::Summon {
                            at: SummonAt::Fallen {
                                kinds: Kinds::Any,
                                reach: Reach::Within(Quantity::Constant(4)),
                            },
                            entity_type,
                            count: Quantity::Constant(2),
                            landing: Landing::Anywhere,
                            within: Vec::new(),
                            arrives: SummonArrival::Standing,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        registry.register_player_buff(
            "drawn_out_rites",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                    stat: ritual_time,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(10),
                }])],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register_player_buff(
            "trimmed_rites",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                    stat: ritual_time,
                    op: ModifierOp::FlatAdd,
                    magnitude: utils::signed_fixed("-0.75"),
                }])],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register_player_buff(
            "quickened_rites",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                    stat: ritual_time,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(-8),
                }])],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        // A raise that calls up as many as the caster's `host_size` says.
        let host_size = registry.register_entity_stat("host_size", FixedU64::ONE);
        let raise_host = {
            let entity_type = registry
                .type_id("skeleton")
                .expect("the summoned type is registered");
            registry.register_skill(
                "raise_host",
                SkillDef {
                    cooldown: 10,
                    caster: SkillCaster::Entity {
                        costs: Vec::new(),
                        casting: Casting::Instant,
                        effect: EntityCastEffect::Summon {
                            at: SummonAt::Fallen {
                                kinds: Kinds::Any,
                                reach: Reach::Within(Quantity::Constant(4)),
                            },
                            entity_type,
                            count: Quantity::Stat(host_size),
                            landing: Landing::Anywhere,
                            within: Vec::new(),
                            arrives: SummonArrival::Standing,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        // Raises paid when their casting starts: one that keeps what it paid
        // when cut short, and one that gives it all back, gold included.
        registry.register_resource("gold");
        let paid_at_start = |registry: &mut ContentRegistry, name: &str, refund, costs| {
            let entity_type = registry
                .type_id("skeleton")
                .expect("the summoned type is registered");
            registry.register_skill(
                name,
                SkillDef {
                    cooldown: 10,
                    caster: SkillCaster::Entity {
                        costs,
                        casting: Casting::Delayed {
                            point: Quantity::Constant(5),
                            period: Quantity::Constant(8),
                            pays: PaymentPoint::AtStart(refund),
                        },
                        effect: EntityCastEffect::Summon {
                            at: SummonAt::Fallen {
                                kinds: Kinds::Any,
                                reach: Reach::Within(Quantity::Constant(4)),
                            },
                            entity_type,
                            count: Quantity::Constant(2),
                            landing: Landing::Anywhere,
                            within: Vec::new(),
                            arrives: SummonArrival::Standing,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        let raise_committed = paid_at_start(
            &mut registry,
            "raise_committed",
            Refund::None,
            vec![Cost::Energy(FixedU64::from_num(40))],
        );
        let raise_refundable = paid_at_start(
            &mut registry,
            "raise_refundable",
            Refund::Full,
            vec![
                Cost::Energy(FixedU64::from_num(40)),
                Cost::Resources(price::from([("gold", 10)])),
            ],
        );
        let blood_rite = paid_at_start(
            &mut registry,
            "blood_rite",
            Refund::Full,
            vec![Cost::Health(FixedU64::from_num(10))],
        );
        // A bolt worked over ten ticks at an enemy within four cells, paid when
        // its casting starts and kept when cut short.
        let slow_bolt = registry.register_skill(
            "slow_bolt",
            SkillDef {
                cooldown: 10,
                caster: SkillCaster::Entity {
                    costs: vec![Cost::Energy(FixedU64::from_num(40))],
                    casting: Casting::Delayed {
                        point: Quantity::Constant(10),
                        period: Quantity::Constant(10),
                        pays: PaymentPoint::AtStart(Refund::None),
                    },
                    effect: EntityCastEffect::Damage {
                        amount: FixedU64::from_num(5),
                        on: Recipient::Standing {
                            whom: Whom {
                                side: Affiliation::Enemy,
                                kinds: Kinds::Any,
                            },
                            reach: Reach::Within(Quantity::Constant(4)),
                        },
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(
            utils::walker("runner", utils::GROUND)
                .with_pool(Pool::health(30))
                .with_sight_range(20),
        );
        // A raise worked over five ticks from wherever the caster stands, paid
        // when it lands.
        let raise_from_afar = {
            let entity_type = registry
                .type_id("skeleton")
                .expect("the summoned type is registered");
            registry.register_skill(
                "raise_from_afar",
                SkillDef {
                    cooldown: 10,
                    caster: SkillCaster::Entity {
                        costs: vec![Cost::Energy(FixedU64::from_num(40))],
                        casting: Casting::Delayed {
                            point: Quantity::Constant(5),
                            period: Quantity::Constant(8),
                            pays: PaymentPoint::AtLanding,
                        },
                        effect: EntityCastEffect::Summon {
                            at: SummonAt::Fallen {
                                kinds: Kinds::Any,
                                reach: Reach::Wherever,
                            },
                            entity_type,
                            count: Quantity::Constant(2),
                            landing: Landing::Anywhere,
                            within: Vec::new(),
                            arrives: SummonArrival::Standing,
                        },
                    },
                    requires: Vec::new(),
                },
            )
        };
        // The same raise, cast from wherever the caster stands.
        let raise_where_it_stands = raises(
            &mut registry,
            "raise_where_it_stands",
            "skeleton",
            Reach::Wherever,
        );
        registry.register(
            utils::walker("necromancer", utils::GROUND)
                .with_pool(Pool::health(40))
                .with_sight_range(20)
                .with_pool(Pool::energy(100))
                .with_stat(ritual_time, FixedU64::from_num(10))
                .with_stat(host_size, FixedU64::lit("3.5"))
                .with_skills([
                    raise_committed,
                    raise_refundable,
                    slow_bolt,
                    raise_host,
                    raise_dead,
                    raise_colossus,
                    raise_wardens,
                    raise_ghouls,
                    blood_rite,
                    raise_slowly,
                    raise_by_stat,
                    self_immolate,
                ]),
        );
        // A mound rises over ten ticks of its own work.
        registry.register(
            EntityTypeDef::new("mound")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_rise(
                    MOUND_RISE_TIME,
                    utils::rise_initial_pools(&[PoolId::HEALTH]),
                ),
        );
        let mound = registry.type_id("mound").expect("just registered");
        // A mound summoned rising beside the caster, and one set down
        // standing on an aimed cell the caster's side must see.
        let raise_mound = registry.register_skill(
            "raise_mound",
            summon_skill(
                mound,
                SummonAt::Caster,
                Landing::Anywhere,
                Vec::new(),
                SummonArrival::Rising,
                Vec::new(),
            ),
        );
        let sight_mound = registry.register_skill(
            "sight_mound",
            summon_skill(
                mound,
                SummonAt::Cell {
                    reach: Reach::Wherever,
                },
                Landing::Sighted,
                Vec::new(),
                SummonArrival::Standing,
                Vec::new(),
            ),
        );
        // A lantern lights its owner's glow two cells about itself; a mound
        // summoned rising inside its owner's glow, held to it until risen,
        // for 30 energy.
        let glow = registry.register_field(
            "glow",
            FieldDef::new(
                FieldLayer::Passable(utils::GROUND.into()),
                FieldDecay::Instant,
                FieldVision::Dark,
                Detection::Blind,
            ),
        );
        registry.register(
            EntityTypeDef::new("lantern")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_field_sources([FieldSourceDef::new(
                    glow,
                    2,
                    FieldGrowth::Instant,
                    Emission::Nothing,
                    Emission::Nothing,
                )]),
        );
        // A spire rises over a seam of gold, which it covers.
        registry.register(
            EntityTypeDef::new("seam")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("spire")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_rise(
                    MOUND_RISE_TIME,
                    utils::rise_initial_pools(&[PoolId::HEALTH]),
                )
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_overbuilds("seam"),
        );
        let spire = registry.type_id("spire").expect("just registered");
        let raise_spire = registry.register_skill(
            "raise_spire",
            summon_skill(
                spire,
                SummonAt::Cell {
                    reach: Reach::Wherever,
                },
                Landing::Anywhere,
                Vec::new(),
                SummonArrival::Rising,
                Vec::new(),
            ),
        );
        // A well rises over a spring, which holds no ground of its own.
        registry.register(
            EntityTypeDef::new("spring")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Passable)
                .with_resource_source("gold", DepletionPolicy::Destroy),
        );
        registry.register(
            EntityTypeDef::new("well")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(20))
                .with_dying(1, [])
                .with_rise(
                    MOUND_RISE_TIME,
                    utils::rise_initial_pools(&[PoolId::HEALTH]),
                )
                .with_resource_source("gold", DepletionPolicy::Destroy)
                .with_overbuilds("spring"),
        );
        let well = registry.type_id("well").expect("just registered");
        let raise_well = registry.register_skill(
            "raise_well",
            summon_skill(
                well,
                SummonAt::Cell {
                    reach: Reach::Wherever,
                },
                Landing::Anywhere,
                Vec::new(),
                SummonArrival::Rising,
                Vec::new(),
            ),
        );
        let glow_mound = registry.register_skill(
            "glow_mound",
            summon_skill(
                mound,
                SummonAt::Cell {
                    reach: Reach::Wherever,
                },
                Landing::Anywhere,
                vec![FieldPlacement::Requires {
                    field: glow,
                    of: Affiliation::Own,
                    coverage: FieldCoverage::Every,
                }],
                SummonArrival::Rising,
                vec![Cost::Energy(FixedU64::from_num(30))],
            ),
        );
        registry.register(
            EntityTypeDef::new("acolyte")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(40))
                .with_sight_range(20)
                .with_pool(Pool::energy(100))
                .with_skills([
                    raise_where_it_stands,
                    raise_from_afar,
                    raise_mound,
                    sight_mound,
                    glow_mound,
                    raise_spire,
                    raise_well,
                ]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    utils::start_game(&mut app);
    app
}

/// Sends `unit` of player 0 off toward the far corner, replacing what it was
/// doing.
fn move_away(app: &mut App, unit: SimulationId) {
    utils::select(app, unit);
    utils::push_command(
        app,
        PlayerCommand::Move {
            target: utils::pos(12, 12),
            flush: true,
        },
    );
}

/// Where `caster`'s cooldown for the skill named `skill` stands.
fn cooldown_of(app: &App, caster: SimulationId, skill: &str) -> Cooldown {
    let skill = utils::skill_id(app, skill);
    let caster = app
        .world()
        .resource::<EntityIndex>()
        .alive(caster)
        .expect("the caster is alive");
    app.world()
        .entity(caster)
        .get::<SkillsComponent>()
        .expect("a caster has its skills")
        .cooldown(skill)
}

/// Puts a player buff on everything player 0 owns, standing and to come.
fn apply_buff(app: &mut App, name: &str) {
    let buff = app
        .world()
        .resource::<ContentRegistry>()
        .player_buff(name)
        .expect("the buff is registered");
    app.world_mut().resource_mut::<PlayerBuffs>().apply(
        PlayerId::from(0u8),
        buff,
        StackRule::Ignore,
        Term::Forever,
    );
}

/// A necromancer of player 0 at `(x, y)`.
fn caster(app: &mut App, x: u32, y: u32) -> SimulationId {
    utils::create_owned(app, "necromancer", x, y, 0).1
}

/// An acolyte of player 0 at `(x, y)` — a caster with no reach of its own.
fn summoner(app: &mut App, x: u32, y: u32) -> SimulationId {
    utils::create_owned(app, "acolyte", x, y, 0).1
}

/// Kills a soldier of player 1 at `(x, y)` and returns the body it leaves.
fn fallen(app: &mut App, x: u32, y: u32) -> SimulationId {
    let (soldier, _) = utils::create_owned(app, "soldier", x, y, 1);
    damage::apply(
        app.world_mut(),
        SimulationId(u32::MAX),
        soldier,
        FixedU64::from_num(1000),
        Slain::Remains,
    );
    utils::run_ticks(app, 4);
    app.world()
        .resource::<EntityIndex>()
        .remains_entries()
        .last()
        .expect("the soldier left a body")
        .0
}

/// Ticks a `mound` rises over.
const MOUND_RISE_TIME: u32 = 10;

/// Commands `caster` to raise `body` with `raise_dead`.
fn raise(app: &mut App, caster: SimulationId, body: SimulationId) {
    utils::cast_on(app, caster, "raise_dead", body);
}

/// The orders `unit` holds.
fn orders_of(app: &App, unit: SimulationId) -> Vec<Order> {
    let entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(unit)
        .expect("the caster is standing");
    entity_def::orders(app.world(), entity).to_vec()
}

/// How many entities of `type_name` stand.
fn count_of(app: &App, type_name: &str) -> usize {
    owners_of(app, type_name).count()
}

/// The owner of every standing entity of `type_name`.
fn owners(app: &App, type_name: &str) -> impl Iterator<Item = Option<u8>> {
    owners_of(app, type_name)
}

fn owners_of(app: &App, type_name: &str) -> impl Iterator<Item = Option<u8>> {
    let world = app.world();
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .filter(move |&(_, entity)| {
            world
                .entity(entity)
                .get::<EntityInfoComponent>()
                .is_some_and(|info| info.type_name() == type_name)
        })
        .map(move |(_, entity)| entity_def::owner(world, entity))
}

/// Every cast announced since announcements were recorded.
fn skill_casts(app: &App) -> Vec<SimulationEvent> {
    app.world()
        .resource::<utils::Announced>()
        .0
        .iter()
        .filter(|event| matches!(event, SimulationEvent::SkillCast { .. }))
        .cloned()
        .collect()
}

/// The caster's current energy as a whole number.
fn energy_as_u32(app: &App, caster: SimulationId) -> u32 {
    let entity = app
        .world()
        .resource::<EntityIndex>()
        .alive(caster)
        .expect("the caster is standing");
    utils::energy_as_u32(app, entity)
}

/// A skill summoning one `entity_type` at `at`, landing as `landing` asks
/// inside the field rules `within`, arriving as `arrives` says, for `costs`.
fn summon_skill(
    entity_type: EntityTypeId,
    at: SummonAt,
    landing: Landing,
    within: Vec<FieldPlacement>,
    arrives: SummonArrival,
    costs: Vec<Cost>,
) -> SkillDef {
    SkillDef {
        cooldown: 10,
        caster: SkillCaster::Entity {
            costs,
            casting: Casting::Instant,
            effect: EntityCastEffect::Summon {
                at,
                entity_type,
                count: Quantity::Constant(1),
                landing,
                within,
                arrives,
            },
        },
        requires: Vec::new(),
    }
}
