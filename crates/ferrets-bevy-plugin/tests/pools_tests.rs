//! Pools: the rates a pool carries unsaid, and what moving a maximum does to the pool under it.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    cost::Cost,
    entity_buffs::{EntityBuffDef, EntityBuffId, Lasting},
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry, RevertCarry, ViaInterrupted,
    },
    player_buffs::{PlayerBuffDef, PlayerBuffId},
    pool::{Pool, PoolInitial},
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    requirement::{Bound, Requirement, Threshold},
    stack_rule::StackRule,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::PlayerCommand,
    components::pools::{self, Spending},
    entity_def, game_loop,
    session::{GameSession, player_slot::PlayerSlot, player_type::PlayerType},
};

//
// ─── Carried rates ──────────────────────────────────────────────────────────
//

#[test]
fn drain_reaches_type_that_never_declared_it() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The keep names no drain; its health pool carries one at zero, so the
    // buff's 2 a tick lands: 100 − 2 = 98.
    let burning = buff_id(&app, "burning");
    utils::apply_buff(app.world_mut(), keep, burning);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 98);
    assert_eq!(
        entity_def::effective_stat(app.world(), keep, EntityStatId::HEALTH_DRAIN),
        Some(FixedU64::from_num(2))
    );
}

#[test]
fn energy_regen_reaches_type_that_names_none() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        pools::spend(
            app.world_mut(),
            keep,
            PoolId::ENERGY,
            FixedU64::from_num(10)
        ),
        Spending::Paid
    );

    // 40 − 10 = 30, nothing regenerated unsaid; then 1 a tick from the buff.
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 30);
    let bright = buff_id(&app, "bright");
    utils::apply_buff(app.world_mut(), keep, bright);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 31);
}

//
// ─── Shifts ─────────────────────────────────────────────────────────────────
//

#[test]
fn share_keeps_share_as_maximum_comes_and_goes() {
    let mut app = app();
    let keep = wounded_keep(&mut app, "50");

    // 50 of 100 is a half: a half of the doubled 200 is 100, and back to 50.
    let doubled = buff_id(&app, "doubled");
    utils::apply_buff(app.world_mut(), keep, doubled);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 100);
    game_loop::buffs::remove_entity_buff(app.world_mut(), keep, doubled);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 50);
}

#[test]
fn difference_moves_pool_by_same_amount() {
    let mut app = app();
    let keep = wounded_keep(&mut app, "50");

    // +50 to the maximum is +50 to the pool: 50 → 100 of 150, and back.
    let braced = buff_id(&app, "braced");
    utils::apply_buff(app.world_mut(), keep, braced);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 100);
    assert_eq!(
        entity_def::effective_stat(app.world(), keep, EntityStatId::MAX_HEALTH),
        Some(FixedU64::from_num(150))
    );
    game_loop::buffs::remove_entity_buff(app.world_mut(), keep, braced);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 50);
}

#[test]
fn clamp_keeps_pool_under_new_maximum() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Full at 100, the maximum halved by clamp: held at the new 50; lifted
    // again, the pool stays at 50 of 100.
    let halved = buff_id(&app, "halved");
    utils::apply_buff(app.world_mut(), keep, halved);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 50);
    game_loop::buffs::remove_entity_buff(app.world_mut(), keep, halved);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 50);
}

#[test]
fn lowered_maximum_never_empties_live_pool() {
    let mut app = app();
    let keep = wounded_keep(&mut app, "99");

    // 1 of 100 moved by −99 is nothing, but a live pool is held at the least
    // live amount instead.
    let crushed = buff_id(&app, "crushed");
    utils::apply_buff(app.world_mut(), keep, crushed);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health(&app, keep), FixedU64::DELTA);
}

#[test]
fn difference_steps_before_clamp_arriving_together() {
    let mut app = app();
    let keep = wounded_keep(&mut app, "60");

    // 40 of 100. Difference steps before clamp: (100 + 50) = 150, 40 + 50 =
    // 90; then the clamp: 150 × 0.5 = 75, the 90 held under it at 75. Clamp
    // first would give 40 under 50, then 40 + 25 = 65.
    let braced = buff_id(&app, "braced");
    let halved = buff_id(&app, "halved");
    utils::apply_buff(app.world_mut(), keep, halved);
    utils::apply_buff(app.world_mut(), keep, braced);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::effective_stat(app.world(), keep, EntityStatId::MAX_HEALTH),
        Some(FixedU64::from_num(75))
    );
    assert_eq!(utils::health_as_u32(&app, keep), 75);
}

#[test]
fn stack_change_is_part_leaving_and_arriving() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // One stack of +10 (difference): 100 → 110. The second stack leaves the
    // +10 (110 → 100) and brings +20 (100 → 120).
    let layered = buff_id(&app, "layered");
    utils::apply_buff(app.world_mut(), keep, layered);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 110);
    utils::apply_buff(app.world_mut(), keep, layered);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 120);
}

#[test]
fn player_buff_moves_owned_pools_by_its_shift_and_spares_rival() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    let (rival, _) = utils::create_owned(&mut app, "keep", 9, 9, 1);
    utils::run_ticks(&mut app, 1);

    // Player 0's drill: +10 health, difference — 100 → 110 of 110 on its own
    // keep, nothing on the rival's.
    let drill = player_buff_id(&app, "drill");
    game_loop::buffs::apply_player_buff(app.world_mut(), 0, drill);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 110);
    assert_eq!(utils::health_as_u32(&app, rival), 100);
}

#[test]
fn simultaneous_arrivals_step_share_before_difference() {
    let mut app = app();
    let keep = wounded_keep(&mut app, "50");

    // Both in one tick. Share first: 50 × 200 / 100 = 100 of 200; then
    // difference: 100 + (300 − 200) = 200 of 300. `swell` stepping first, as
    // its earlier registration once had it, gives 150 then 225.
    let doubled = buff_id(&app, "doubled");
    let swell = buff_id(&app, "swell");
    utils::apply_buff(app.world_mut(), keep, swell);
    utils::apply_buff(app.world_mut(), keep, doubled);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 200);
}

#[test]
fn simultaneous_arrivals_of_one_shift_step_once() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Both clamp, in one tick: one step to 100 × (1 + 1 − 0.5) = 150, the 100
    // held kept — not the cut clipping it to 50 before the raise.
    let halved = buff_id(&app, "halved");
    let grown = buff_id(&app, "grown");
    utils::apply_buff(app.world_mut(), keep, halved);
    utils::apply_buff(app.world_mut(), keep, grown);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::effective_stat(app.world(), keep, EntityStatId::MAX_HEALTH),
        Some(FixedU64::from_num(150))
    );
    assert_eq!(utils::health_as_u32(&app, keep), 100);
}

#[test]
fn equal_parts_leave_one_by_one() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let plated = buff_id(&app, "plated");
    let drill = player_buff_id(&app, "drill");
    utils::apply_buff(app.world_mut(), keep, plated);
    game_loop::buffs::apply_player_buff(app.world_mut(), 0, drill);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 120);

    // Two equal parts, +10 by difference each; one leaves: 120 − 10 = 110.
    game_loop::buffs::remove_entity_buff(app.world_mut(), keep, plated);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, keep), 110);
}

#[test]
fn energy_pool_follows_its_maximum() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // Full at 40, the energy maximum doubled with its share kept: 80 of 80.
    let charged = buff_id(&app, "charged");
    utils::apply_buff(app.world_mut(), keep, charged);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 80);
}

//
// ─── Changes of form ────────────────────────────────────────────────────────
//

#[test]
fn transition_declared_share_keeps_share() {
    let mut app = app();
    let (egg, _) = utils::create_owned(&mut app, "egg", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, egg, "25");

    // 25 of 50 as an egg is a half; the husk keeps it: 40 of 80.
    utils::order_morph(&mut app, egg, "husk");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), egg), "husk");
    assert_eq!(utils::health_as_u32(&app, egg), 40);
}

#[test]
fn transition_declared_share_carries_exact_points() {
    let mut app = app();
    let (egg, _) = utils::create_owned(&mut app, "egg", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, egg, "20");

    // 30 of 50 into the husk's 80: 30 × 80 / 50 = 48 exactly, where the share
    // taken first, 30 / 50 = 0.6, has no exact binary value.
    utils::order_morph(&mut app, egg, "husk");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), egg), "husk");
    assert_eq!(utils::health_as_u32(&app, egg), 48);
}

#[test]
fn transition_declared_difference_moves_pool_by_same_amount() {
    let mut app = app();
    let (egg, _) = utils::create_owned(&mut app, "egg", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, egg, "25");

    // 25 of 50 into the grub's 80: 25 + (80 − 50) = 55.
    utils::order_morph(&mut app, egg, "grub");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), egg), "grub");
    assert_eq!(utils::health_as_u32(&app, egg), 55);
}

#[test]
fn transition_declared_clamp_keeps_points() {
    let mut app = app();
    let (egg, _) = utils::create_owned(&mut app, "egg", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, egg, "25");

    // 25 of 50 into the pupa's 80: the 25 stay, under a maximum above them.
    utils::order_morph(&mut app, egg, "pupa");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), egg), "pupa");
    assert_eq!(utils::health_as_u32(&app, egg), 25);
}

#[test]
fn transition_declared_full_lands_full() {
    let mut app = app();
    let (egg, _) = utils::create_owned(&mut app, "egg", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, egg, "30");

    // 20 of 50 as an egg; the hatch is declared full: 80 of 80.
    utils::order_morph(&mut app, egg, "hatchling");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), egg), "hatchling");
    assert_eq!(utils::health_as_u32(&app, egg), 80);
}

#[test]
fn landing_carries_each_named_pool_by_its_own_carry() {
    let mut app = app();
    let (flask, _) = utils::create_owned(&mut app, "flask", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, flask, "25");
    pools::drain(app.world_mut(), flask, PoolId::ENERGY, utils::fixed("30"));

    // Health by share, 25 of 50 onto 80: 40. Energy full: 20 of 20.
    utils::order_morph(&mut app, flask, "vessel");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), flask), "vessel");
    assert_eq!(utils::health_as_u32(&app, flask), 40);
    assert_eq!(utils::energy_as_u32(&app, flask), 20);
}

#[test]
fn pool_landing_names_not_keeps_its_value_under_new_maximum() {
    let mut app = app();
    let (flask, _) = utils::create_owned(&mut app, "flask", 5, 5, 0);
    let (other, _) = utils::create_owned(&mut app, "flask", 7, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), other, PoolId::ENERGY, utils::fixed("30"));

    // Energy is not named: the full 40 is held under the jar's 30, and 10 of
    // 40 stays 10.
    utils::order_morph(&mut app, flask, "jar");
    utils::order_morph(&mut app, other, "jar");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(utils::energy_as_u32(&app, flask), 30);
    assert_eq!(utils::energy_as_u32(&app, other), 10);
}

#[test]
fn pool_new_form_gains_starts_full_and_pool_it_lacks_goes() {
    let mut app = app();
    let (seed, _) = utils::create_owned(&mut app, "seed", 5, 5, 0);
    let (flask, _) = utils::create_owned(&mut app, "flask", 7, 5, 0);
    utils::run_ticks(&mut app, 1);

    utils::order_morph(&mut app, seed, "flask");
    utils::order_morph(&mut app, flask, "lamp");
    utils::run_ticks(&mut app, HATCHES);
    // The seed had no energy: the flask's 40 starts full. The lamp has none:
    // the flask's goes.
    assert_eq!(utils::energy_as_u32(&app, seed), 40);
    assert!(!entity_def::has_pool(app.world(), flask, PoolId::ENERGY));
}

#[test]
fn pool_starts_at_its_declared_initial() {
    let mut app = app();
    let (wick, _) = utils::create_owned(&mut app, "wick", 5, 5, 0);
    let (taper, _) = utils::create_owned(&mut app, "taper", 7, 5, 0);
    utils::run_ticks(&mut app, 1);
    // A quarter of 40 is 10; the taper's 15 stands as declared.
    assert_eq!(utils::energy_as_u32(&app, wick), 10);
    assert_eq!(utils::energy_as_u32(&app, taper), 15);
    // Health says nothing: full.
    assert_eq!(utils::health_as_u32(&app, wick), 50);
}

#[test]
fn pool_new_form_gains_starts_at_its_initial() {
    let mut app = app();
    let (seed, _) = utils::create_owned(&mut app, "seed", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    // The seed holds no energy: the wick's is gained, not carried.
    assert!(!entity_def::has_pool(app.world(), seed, PoolId::ENERGY));
    utils::order_morph(&mut app, seed, "wick");
    utils::run_ticks(&mut app, HATCHES);
    // Gained, it starts at a quarter of 40: 10.
    assert_eq!(entity_def::type_name(app.world(), seed), "wick");
    assert_eq!(utils::energy_as_u32(&app, seed), 10);
}

#[test]
fn share_initial_reads_maximum_its_passives_leave() {
    let mut app = app();
    let (lantern, _) = utils::create_owned(&mut app, "dim_lantern", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    // Half of 50 + 50 from `glow`: 50 of 100, not half of the bare 50.
    assert_eq!(utils::health_as_u32(&app, lantern), 50);
}

#[test]
fn kept_pool_carried_as_initial_starts_at_new_forms_initial() {
    let mut app = app();
    app.world_mut().resource_mut::<ContentRegistry>().register(
        EntityTypeDef::new("carafe")
            .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
            .with_pool(Pool::health(50))
            .with_pool(Pool::energy(40))
            .with_morphs([MorphTransition::new(
                "wick",
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(2),
                MorphPlacement::Reserve,
                MorphCancel::Committed,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [(PoolId::ENERGY, PoolCarry::Initial)],
            )]),
    );
    let (carafe, _) = utils::create_owned(&mut app, "carafe", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), carafe, PoolId::ENERGY, utils::fixed("35"));
    utils::order_morph(&mut app, carafe, "wick");
    utils::run_ticks(&mut app, HATCHES);
    // 40 − 35 = 5 as a carafe; the wick starts it at its own quarter of 40: 10.
    assert_eq!(entity_def::type_name(app.world(), carafe), "wick");
    assert_eq!(utils::energy_as_u32(&app, carafe), 10);
}

#[test]
fn initial_carry_enters_interim_and_lands_at_each_forms_initial() {
    let mut app = carry_initial_app();
    let (sprout, _) = utils::create_owned(&mut app, "sprout", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    // 10 as a sprout, 5 after a drain; the pod starts it at half of 40, 20.
    pools::drain(app.world_mut(), sprout, PoolId::ENERGY, utils::fixed("5"));
    utils::order_morph(&mut app, sprout, "bloom");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), sprout), "pod");
    assert_eq!(utils::energy_as_u32(&app, sprout), 20);
    // The bloom starts it at a quarter of 60, 15.
    utils::run_ticks(&mut app, 11);
    assert_eq!(entity_def::type_name(app.world(), sprout), "bloom");
    assert_eq!(utils::energy_as_u32(&app, sprout), 15);
}

#[test]
fn initial_carry_reverts_to_origins_initial() {
    let mut app = carry_initial_app();
    let (sprout, _) = utils::create_owned(&mut app, "sprout", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::order_morph(&mut app, sprout, "bloom");
    utils::run_ticks(&mut app, 1);
    // 20 in the pod, 2 after a drain; called off, the sprout starts it at 10.
    pools::drain(app.world_mut(), sprout, PoolId::ENERGY, utils::fixed("18"));
    utils::soft_cancel_orders(app.world_mut(), sprout);
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), sprout), "sprout");
    assert_eq!(utils::energy_as_u32(&app, sprout), 10);
}

#[test]
fn initial_carry_starts_pool_interim_lacked_at_initial() {
    let mut app = carry_initial_app();
    // Through a husk with no energy, landing on a fruit: a quarter of 60, 15.
    let (sprout, _) = utils::create_owned(&mut app, "sprout", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), sprout, PoolId::ENERGY, utils::fixed("5"));
    utils::order_morph(&mut app, sprout, "fruit");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), sprout), "husk");
    utils::run_ticks(&mut app, 11);
    assert_eq!(entity_def::type_name(app.world(), sprout), "fruit");
    assert_eq!(utils::energy_as_u32(&app, sprout), 15);

    // Called off in the husk, back to the sprout's own 10.
    let (other, _) = utils::create_owned(&mut app, "sprout", 8, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), other, PoolId::ENERGY, utils::fixed("5"));
    utils::order_morph(&mut app, other, "fruit");
    utils::run_ticks(&mut app, 1);
    utils::soft_cancel_orders(app.world_mut(), other);
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), other), "sprout");
    assert_eq!(utils::energy_as_u32(&app, other), 10);
}

#[test]
fn amount_initial_stands_whatever_its_passives_raise() {
    let mut app = app();
    let (lamp, _) = utils::create_owned(&mut app, "low_lamp", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    // 30 of 50 + 20 from `wings`: 30 as declared, not 30 + 20.
    assert_eq!(utils::health_as_u32(&app, lamp), 30);
}

#[test]
fn amount_initial_held_under_maximum_its_passives_lower() {
    let mut app = app();
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let dimmed = registry.register_entity_buff("dimmed", passive_buff("-20", PoolShift::Clamp));
        registry.register(
            EntityTypeDef::new("squat_lamp")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::builtin(
                    PoolId::HEALTH,
                    FixedU64::from_num(50),
                    FixedU64::ZERO,
                    FixedU64::ZERO,
                    PoolInitial::Amount(utils::fixed("45")),
                ))
                .with_passives([dimmed]),
        );
    }
    let (lamp, _) = utils::create_owned(&mut app, "squat_lamp", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    // 45 held under 50 − 20 = 30.
    assert_eq!(utils::health_as_u32(&app, lamp), 30);
}

#[test]
fn interim_declared_full_enters_full() {
    let mut app = app();
    let (larva, _) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, larva, "25");

    // 25 of 50 as a larva; the cocoon is entered full: 100 of 100.
    utils::order_morph(&mut app, larva, "moth");
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), larva), "cocoon");
    assert_eq!(utils::health_as_u32(&app, larva), 100);
}

#[test]
fn revert_declared_restore_gives_back_origin_value() {
    let mut app = app();
    let (larva, _) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, larva, "30");
    utils::order_morph(&mut app, larva, "moth");
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, larva, "50");

    // 20 of 50 at the start; the cocoon's 50 of 100 is not read: back to 20.
    utils::soft_cancel_orders(app.world_mut(), larva);
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), larva), "larva");
    assert_eq!(utils::health_as_u32(&app, larva), 20);
}

#[test]
fn revert_declared_share_carries_from_interim() {
    let mut app = app();
    let (larva, _) = utils::create_owned(&mut app, "larva", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, larva, "25");
    // 25 of 50 entered by share: 25 × 100 / 50 = 50 of 100.
    utils::order_morph(&mut app, larva, "beetle");
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, larva), 50);
    utils::wound(&mut app, larva, "30");

    // 20 of 100 carried back by share: 20 × 50 / 100 = 10.
    utils::soft_cancel_orders(app.world_mut(), larva);
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), larva), "larva");
    assert_eq!(utils::health_as_u32(&app, larva), 10);
}

#[test]
fn buff_interim_cannot_carry_leaves_origin_before_record() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let charged = buff_id(&app, "charged");
    utils::apply_buff(app.world_mut(), caterpillar, charged);
    utils::run_ticks(&mut app, 1);
    pools::drain(
        app.world_mut(),
        caterpillar,
        PoolId::ENERGY,
        utils::fixed("20"),
    );

    // 60 of 80 under `charged`; the chrysalis cannot carry it, so it leaves
    // the caterpillar first, by its share: 60 × 40 / 80 = 30 of 40 recorded,
    // and given back on the cancel.
    utils::order_morph(&mut app, caterpillar, "skipper");
    utils::run_ticks(&mut app, 1);
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::type_name(app.world(), caterpillar),
        "caterpillar"
    );
    assert_eq!(utils::energy_as_u32(&app, caterpillar), 30);
}

#[test]
fn pool_interim_lacks_takes_shifts_of_its_absence() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(
        app.world_mut(),
        caterpillar,
        PoolId::ENERGY,
        utils::fixed("30"),
    );
    utils::order_morph(&mut app, caterpillar, "skipper");
    utils::run_ticks(&mut app, 1);

    // `attuned` arrives while the chrysalis holds no energy; the 10 of 40
    // recorded takes its difference on the return: 10 + 40 = 50 of 80.
    let attuned = player_buff_id(&app, "attuned");
    game_loop::buffs::apply_player_buff(app.world_mut(), 0, attuned);
    utils::run_ticks(&mut app, 1);
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::type_name(app.world(), caterpillar),
        "caterpillar"
    );
    assert_eq!(utils::energy_as_u32(&app, caterpillar), 50);
}

#[test]
fn landing_carries_pool_interim_lacked_from_origin_maximum() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(
        app.world_mut(),
        caterpillar,
        PoolId::ENERGY,
        utils::fixed("30"),
    );

    // 10 of the caterpillar's 40, carried by share onto the emperor's 100:
    // 10 × 100 / 40 = 25.
    utils::order_morph(&mut app, caterpillar, "emperor");
    utils::run_ticks(&mut app, 11);
    assert_eq!(entity_def::type_name(app.world(), caterpillar), "emperor");
    assert_eq!(utils::energy_as_u32(&app, caterpillar), 25);
}

#[test]
fn landing_carries_pool_interim_lacked_from_origin_maximum_now() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(
        app.world_mut(),
        caterpillar,
        PoolId::ENERGY,
        utils::fixed("20"),
    );
    utils::order_morph(&mut app, caterpillar, "emperor");
    utils::run_ticks(&mut app, 1);

    // `attuned` arrives in the chrysalis: the 20 of 40 recorded takes its
    // difference, 60 under the caterpillar's maximum now, 80; carried by
    // share onto the emperor's 100 + 40: 60 × 140 / 80 = 105.
    let attuned = player_buff_id(&app, "attuned");
    game_loop::buffs::apply_player_buff(app.world_mut(), 0, attuned);
    utils::run_ticks(&mut app, 10);
    assert_eq!(entity_def::type_name(app.world(), caterpillar), "emperor");
    assert_eq!(utils::energy_as_u32(&app, caterpillar), 105);
}

#[test]
fn landing_carries_pool_interim_lacked_under_origin_passives() {
    let mut app = app();
    let (glowworm, _) = utils::create_owned(&mut app, "glowworm", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(
        app.world_mut(),
        glowworm,
        PoolId::ENERGY,
        utils::fixed("30"),
    );

    // 30 of 60 under `spark`, which belongs to the glowworm's form: it does
    // not leave while the chrysalis is worn, so the 30 stands under 60 and
    // is carried by share onto the emperor's 100: 30 × 100 / 60 = 50.
    utils::order_morph(&mut app, glowworm, "emperor");
    utils::run_ticks(&mut app, 11);
    assert_eq!(entity_def::type_name(app.world(), glowworm), "emperor");
    assert_eq!(utils::energy_as_u32(&app, glowworm), 50);
}

#[test]
fn buff_interim_cannot_carry_leaves_origin_before_its_pools_are_recorded() {
    let mut app = app();
    let (silkworm, _) = utils::create_owned(&mut app, "silkworm", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let weighed = buff_id(&app, "weighed");
    utils::apply_buff(app.world_mut(), silkworm, weighed);
    utils::run_ticks(&mut app, 1);
    // 100 − 50 = 50 of 50, then wounded to 40.
    utils::wound(&mut app, silkworm, "10");

    // The chrysalis has no armor: `weighed` leaves the silkworm first, by
    // difference, 40 + 50 = 90 of 100, and 90 is recorded. Entered by share:
    // 90 × 200 / 100 = 180.
    utils::order_morph(&mut app, silkworm, "butterfly");
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, silkworm), 180);

    // Restored: the 90 recorded, under the silkworm's 100.
    utils::soft_cancel_orders(app.world_mut(), silkworm);
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), silkworm), "silkworm");
    assert_eq!(utils::health_as_u32(&app, silkworm), 90);
}

#[test]
fn revert_keeps_origin_passive_out_of_its_missed_shifts() {
    let mut app = app();
    let (glowworm, _) = utils::create_owned(&mut app, "glowworm", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(
        app.world_mut(),
        glowworm,
        PoolId::ENERGY,
        utils::fixed("30"),
    );

    // 30 of 60 under `spark`; back as a glowworm, `spark` is fitted again and
    // is the glowworm's form's own, not a buff arriving: 30 of 60.
    utils::order_morph(&mut app, glowworm, "emperor");
    utils::run_ticks(&mut app, 1);
    utils::soft_cancel_orders(app.world_mut(), glowworm);
    utils::run_ticks(&mut app, 1);
    assert_eq!(entity_def::type_name(app.world(), glowworm), "glowworm");
    assert_eq!(utils::energy_as_u32(&app, glowworm), 30);
}

#[test]
fn landing_fills_pool_interim_lacked_when_named_full() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(
        app.world_mut(),
        caterpillar,
        PoolId::ENERGY,
        utils::fixed("30"),
    );

    // 10 of 40 recorded; the regent's landing names energy full: 100 of 100.
    utils::order_morph(&mut app, caterpillar, "regent");
    utils::run_ticks(&mut app, 11);
    assert_eq!(entity_def::type_name(app.world(), caterpillar), "regent");
    assert_eq!(utils::energy_as_u32(&app, caterpillar), 100);
}

#[test]
fn revert_by_restore_holds_under_origin_maximum_now() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let braced = buff_id(&app, "braced");
    utils::apply_buff(app.world_mut(), caterpillar, braced);
    utils::run_ticks(&mut app, 1);
    // 50 + 50 (`hide`) + 50 (`braced`) = 150 of 150, recorded at the start.
    utils::order_morph(&mut app, caterpillar, "butterfly");
    utils::run_ticks(&mut app, 1);
    game_loop::buffs::remove_entity_buff(app.world_mut(), caterpillar, braced);
    utils::run_ticks(&mut app, 1);

    // Restored: the 150 recorded, held under the caterpillar's 100 now.
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::type_name(app.world(), caterpillar),
        "caterpillar"
    );
    assert_eq!(utils::health_as_u32(&app, caterpillar), 100);
}

#[test]
fn revert_restoring_pool_interim_lacked_holds_under_origin_maximum_now() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    // 40 of 40 energy recorded at the start; the chrysalis holds none.
    utils::order_morph(&mut app, caterpillar, "butterfly");
    utils::run_ticks(&mut app, 1);
    let dimmed = player_buff_id(&app, "dimmed");
    game_loop::buffs::apply_player_buff(app.world_mut(), 0, dimmed);
    utils::run_ticks(&mut app, 1);

    // Restored: the 40 recorded, held under the caterpillar's 40 − 20 = 20
    // now.
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::type_name(app.world(), caterpillar),
        "caterpillar"
    );
    assert_eq!(utils::energy_as_u32(&app, caterpillar), 20);
}

#[test]
fn lowered_maximum_may_empty_energy() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), keep, PoolId::ENERGY, utils::fixed("30"));

    // 10 of 40, cut by 30 by difference: 10 − 30 held at 0 — only a live
    // health pool is kept above empty.
    let sapping = buff_id(&app, "sapping");
    utils::apply_buff(app.world_mut(), keep, sapping);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 0);
}

#[test]
fn share_from_zero_maximum_returns_nothing() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), keep, PoolId::ENERGY, utils::fixed("10"));

    // 30 of 40, the maximum taken to 40 × (1 − 1) = 0: 30 × 0 / 40 = 0.
    let silenced = buff_id(&app, "silenced");
    utils::apply_buff(app.world_mut(), keep, silenced);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 0);

    // Back to 40 from a zero maximum: no share to keep, 0 of 40.
    game_loop::buffs::remove_entity_buff(app.world_mut(), keep, silenced);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 0);
}

#[test]
fn clamp_from_zero_maximum_returns_nothing() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), keep, PoolId::ENERGY, utils::fixed("10"));

    // 30 of 40 held under a maximum of 0: min(30, 0) = 0; back to 40, the
    // value stays where it is: 0 of 40.
    let hushed = buff_id(&app, "hushed");
    utils::apply_buff(app.world_mut(), keep, hushed);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 0);
    game_loop::buffs::remove_entity_buff(app.world_mut(), keep, hushed);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 0);
}

#[test]
fn difference_from_zero_maximum_returns_whole_new_maximum() {
    let mut app = app();
    let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    pools::drain(app.world_mut(), keep, PoolId::ENERGY, utils::fixed("10"));

    // 30 of 40, the maximum down by 40: 30 − 40 held at 0; back up by 40:
    // 0 + 40 = 40 of 40.
    let muted = buff_id(&app, "muted");
    utils::apply_buff(app.world_mut(), keep, muted);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 0);
    game_loop::buffs::remove_entity_buff(app.world_mut(), keep, muted);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::energy_as_u32(&app, keep), 40);
}

#[test]
fn entity_spawns_full_under_clamped_passive_raising_maximum() {
    let mut app = app();
    let (lantern, _) = utils::create_owned(&mut app, "lantern", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // 50 + 50 from `glow`, started again full after the passive is fitted:
    // 100 of 100.
    assert_eq!(
        entity_def::effective_stat(app.world(), lantern, EntityStatId::MAX_HEALTH),
        Some(FixedU64::from_num(100))
    );
    assert_eq!(utils::health_as_u32(&app, lantern), 100);
}

#[test]
fn revert_by_restore_lands_under_origin_passive_maximum() {
    let mut app = app();
    let caterpillar = spun_caterpillar(&mut app, "butterfly");

    // 60 of 100 at the start, put back under the caterpillar's 50 + 50.
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::type_name(app.world(), caterpillar),
        "caterpillar"
    );
    assert_eq!(utils::health_as_u32(&app, caterpillar), 60);
}

#[test]
fn revert_by_share_carries_onto_origin_passive_maximum() {
    let mut app = app();
    let caterpillar = spun_caterpillar(&mut app, "hawkmoth");

    // 40 of 200 by share onto 100: 40 × 100 / 200 = 20.
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, caterpillar), 20);
}

#[test]
fn revert_naming_no_pool_keeps_values_under_origin_maximum() {
    let mut app = app();
    let caterpillar = spun_caterpillar(&mut app, "skipper");

    // Health kept: 40 under 100. Energy, which the chrysalis lacked: the 10
    // of 40 it held at the start.
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(utils::health_as_u32(&app, caterpillar), 40);
    assert_eq!(utils::energy_as_u32(&app, caterpillar), 10);
}

#[test]
fn landing_carries_under_destination_passive_maximum() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    utils::wound(&mut app, caterpillar, "40");

    // 60 of 100 by share onto the monarch's 80 + 20: 60 × 100 / 100 = 60.
    utils::order_morph(&mut app, caterpillar, "monarch");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), caterpillar), "monarch");
    assert_eq!(utils::health_as_u32(&app, caterpillar), 60);
}

#[test]
fn buff_interim_cannot_carry_ends_and_stays_ended_after_cancel() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let charged = buff_id(&app, "charged");
    utils::apply_buff(app.world_mut(), caterpillar, charged);
    utils::order_morph(&mut app, caterpillar, "skipper");
    utils::run_ticks(&mut app, 1);
    assert!(!entity_def::bears(app.world(), caterpillar, charged));

    // Back as a caterpillar, the energy maximum is its own 40, not 40 × 2.
    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert_eq!(
        entity_def::type_name(app.world(), caterpillar),
        "caterpillar"
    );
    assert!(!entity_def::bears(app.world(), caterpillar, charged));
    assert_eq!(
        entity_def::effective_stat(app.world(), caterpillar, EntityStatId::MAX_ENERGY),
        Some(FixedU64::from_num(40))
    );
}

#[test]
fn buff_destination_cannot_carry_ends_on_landing() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let charged = buff_id(&app, "charged");
    utils::apply_buff(app.world_mut(), caterpillar, charged);

    utils::order_morph(&mut app, caterpillar, "monarch");
    utils::run_ticks(&mut app, HATCHES);
    assert_eq!(entity_def::type_name(app.world(), caterpillar), "monarch");
    assert!(!entity_def::bears(app.world(), caterpillar, charged));
}

#[test]
fn buff_that_fits_rides_through_change() {
    let mut app = app();
    let (caterpillar, _) = utils::create_owned(&mut app, "caterpillar", 5, 5, 0);
    utils::run_ticks(&mut app, 1);
    let doubled = buff_id(&app, "doubled");
    utils::apply_buff(app.world_mut(), caterpillar, doubled);
    utils::order_morph(&mut app, caterpillar, "skipper");
    utils::run_ticks(&mut app, 1);

    utils::soft_cancel_orders(app.world_mut(), caterpillar);
    utils::run_ticks(&mut app, 1);
    assert!(entity_def::bears(app.world(), caterpillar, doubled));
}

//
// ─── Expected results ───────────────────────────────────────────────────────
//

#[test]
fn stepwise_fold_moves_pools_as_expected() {
    use Act::{Apply, ApplyPlayer, Heal, Remove};
    // (row, wound at the start (`None` unwounded), then per tick: what changes, health, maximum)
    let rows: Vec<(&str, Option<&str>, Vec<Tick>)> = vec![
        // 50 × 200 / 100 = 100 of 200, and back: 100 × 100 / 200 = 50.
        (
            "share round trip",
            Some("50"),
            vec![
                (vec![Apply("doubled")], 100, 200),
                (vec![Remove("doubled")], 50, 100),
            ],
        ),
        // 50 + 50 = 100 of 150, and back: 100 − 50 = 50.
        (
            "difference round trip",
            Some("50"),
            vec![
                (vec![Apply("braced")], 100, 150),
                (vec![Remove("braced")], 50, 100),
            ],
        ),
        // Full, cut to 50 by clamp: min(100, 50) = 50; the cut lifted, the
        // value stays: 50 of 100.
        (
            "clamp cut is lasting",
            None,
            vec![
                (vec![Apply("halved")], 50, 50),
                (vec![Remove("halved")], 50, 100),
            ],
        ),
        // A clamp raise leaves the value: 50 of 200, then 50 of 100.
        (
            "clamp raise leaves value",
            Some("50"),
            vec![
                (vec![Apply("grown")], 50, 200),
                (vec![Remove("grown")], 50, 100),
            ],
        ),
        // Together: share 50 × 200 / 100 = 100, difference + 100 = 200 of
        // 300. Leaving together, mirrored: difference 200 − 100 = 100 of 200,
        // share 100 × 100 / 200 = 50 of 100.
        (
            "share and difference come and go together",
            Some("50"),
            vec![
                (vec![Apply("doubled"), Apply("swell")], 200, 300),
                (vec![Remove("doubled"), Remove("swell")], 50, 100),
            ],
        ),
        // Departures first: 100 − 50 = 50 of 100; then 50 × 200 / 100 = 100.
        (
            "difference leaves as share arrives",
            Some("50"),
            vec![
                (vec![Apply("braced")], 100, 150),
                (vec![Remove("braced"), Apply("doubled")], 100, 200),
            ],
        ),
        // Share first: 200 of 200; clamp last: min(200, 300) = 200 of 300.
        (
            "share before clamp",
            None,
            vec![(vec![Apply("grown"), Apply("doubled")], 200, 300)],
        ),
        // One difference stack: 110 of 110; the second: the old part leaves,
        // 110 − 10 = 100, the new arrives, 100 + 20 = 120 of 120.
        (
            "difference stacks",
            None,
            vec![
                (vec![Apply("layered")], 110, 110),
                (vec![Apply("layered")], 120, 120),
            ],
        ),
        // A clamp stack: 100 of 110; healed full, 110 of 110; the second
        // stack settles in one step: min(110, 120) = 110 of 120.
        (
            "clamp stack settles once",
            None,
            vec![
                (vec![Apply("rung")], 100, 110),
                (vec![Heal("10")], 110, 110),
                (vec![Apply("rung")], 110, 120),
            ],
        ),
        // An entity buff and a player buff, both difference, in one step:
        // 100 + 60 = 160 of 160.
        (
            "entity and player difference together",
            None,
            vec![(vec![Apply("braced"), ApplyPlayer("drill")], 160, 160)],
        ),
    ];
    for (row, wound, ticks) in rows {
        let mut app = app();
        let (keep, _) = utils::create_owned(&mut app, "keep", 5, 5, 0);
        utils::run_ticks(&mut app, 1);
        if let Some(wound) = wound {
            utils::wound(&mut app, keep, wound);
        }
        for (step, (acts, health, maximum)) in ticks.into_iter().enumerate() {
            for act in acts {
                match act {
                    Apply(name) => {
                        let id = buff_id(&app, name);
                        utils::apply_buff(app.world_mut(), keep, id);
                    }
                    Remove(name) => {
                        let id = buff_id(&app, name);
                        game_loop::buffs::remove_entity_buff(app.world_mut(), keep, id);
                    }
                    ApplyPlayer(name) => {
                        let id = player_buff_id(&app, name);
                        game_loop::buffs::apply_player_buff(app.world_mut(), 0, id);
                    }
                    Heal(amount) => {
                        pools::restore(app.world_mut(), keep, PoolId::HEALTH, utils::fixed(amount))
                    }
                }
            }
            utils::run_ticks(&mut app, 1);
            assert_eq!(
                (
                    utils::health_as_u32(&app, keep),
                    entity_def::effective_stat(app.world(), keep, EntityStatId::MAX_HEALTH)
                        .map(|maximum| maximum.to_num::<u32>()),
                ),
                (health, Some(maximum)),
                "{row}, step {step}"
            );
        }
    }
}

//
// ─── Writes ─────────────────────────────────────────────────────────────────
//

#[test]
fn restore_holds_under_effective_maximum() {
    let mut app = app();
    let keep = wounded_keep(&mut app, "30");
    let braced = buff_id(&app, "braced");
    utils::apply_buff(app.world_mut(), keep, braced);
    utils::run_ticks(&mut app, 1);

    // 70 of 100, braced to 120 of 150; 120 + 50 = 170 is held at the braced
    // 150, not the declared 100.
    pools::restore(app.world_mut(), keep, PoolId::HEALTH, utils::fixed("50"));
    assert_eq!(utils::health_as_u32(&app, keep), 150);
}

#[test]
fn refunded_cost_holds_under_maximum() {
    let mut app = app();
    let (egg, egg_id) = utils::create_owned(&mut app, "egg", 5, 5, 0);
    utils::run_ticks(&mut app, 1);

    // The shell costs 10 of the egg's 50; mended back to 50 while it changes,
    // the egg cancels, and the 10 paid back is held at its 50, not 60.
    utils::order_morph(&mut app, egg, "shell");
    utils::run_ticks(&mut app, 2);
    assert_eq!(utils::health_as_u32(&app, egg), 40);
    pools::restore(app.world_mut(), egg, PoolId::HEALTH, utils::fixed("10"));
    utils::push_command(&mut app, PlayerCommand::CancelMorph { entity: egg_id });
    utils::run_ticks(&mut app, utils::APPLY + 1);
    assert_eq!(entity_def::type_name(app.world(), egg), "egg");
    assert_eq!(utils::health_as_u32(&app, egg), 50);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Ticks from a morph order to its landing: the declared 2.
const HATCHES: u32 = 2;

/// Two players. Types: `keep` (100 health, 40 energy, no rates named), `egg`
/// (50 health) that becomes a `hatchling` (80 health, declared full) or a
/// `husk` (80 health, declared share), a `grub` (80 health, declared
/// difference) or a `pupa` (80 health, declared clamp) in two ticks, or a
/// `shell` (80 health)
/// in ten for 10 health, refunded if canceled. A `flask` (50 health, 40
/// energy) becomes a `vessel` (80, 20; health share, energy full), a `jar`
/// (80, 30; health share) or a `lamp` (50 health; health share); a `seed`
/// (50 health) becomes a flask (health share). A `larva` (50 health) passes
/// through a `cocoon` (100 health) in ten ticks into a `moth` (entering
/// full, reverting by restore) or a `beetle` (entering and reverting by
/// share), both 80 health and landing by share. A `caterpillar` (50 health
/// raised to 100 by its passive `hide`, `max_health +50` by difference; 40
/// energy) passes through a `chrysalis` (200 health, no energy, no passive),
/// entered and landed by share, into a `butterfly` (reverting by restore), a
/// `hawkmoth` (by share) or a `skipper` (naming no pool), each 80 health;
/// or becomes a `monarch` directly by share (80 health raised to 100 by its
/// passive `wings`, `+20` by difference), or through the chrysalis an
/// `emperor` (80 health, 100 energy), every pool landing by share, or a
/// `regent` (80 health, 100 energy) landing health by share and energy full. A
/// `silkworm` (100 health, armor 0) passes through the chrysalis into a
/// butterfly, reverting by restore; `weighed` (`max_health −50` by
/// difference beside `armor +1`) is a buff the chrysalis cannot carry. A
/// `glowworm` (50 health; 40 energy raised to 60 by its passive `spark`,
/// `max_energy +20` by difference) becomes an emperor the same way. A `lantern` (50 health) bears the
/// passive `glow`, `max_health +50` by clamp. Entity buffs, each
/// forever: `swell` (`max_health +100%`, difference, registered first),
/// `sapping` (`max_energy −30`, difference), `grown` (`max_health +100%`, clamp), `plated` (`max_health +10`,
/// difference), `burning` (`health_drain +2`), `bright` (`energy_regen +1`),
/// `doubled` (`max_health +100%`, share), `braced` (`max_health +50`,
/// difference), `halved` (`max_health −50%`, clamp), `crushed`
/// (`max_health −99`, difference), `layered` (`max_health +10`, difference,
/// stacking to five), `charged` (`max_energy +100%`, share). Player buff:
/// `drill` (`max_health +10` over every owned unit, difference), `attuned`
/// (`max_energy +40` over every owned unit, difference). Entity buff `rung`
/// (`max_health +10`, clamp, stacking to five).
fn app() -> App {
    let mut app = utils::make_app(vec![
        PlayerSlot::occupied(0, PlayerType::Human, None, None),
        PlayerSlot::occupied(1, PlayerType::Human, None, None),
    ]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        for (name, stat, op, magnitude, pool_shift) in [
            (
                "swell",
                EntityStatId::MAX_HEALTH,
                ModifierOp::PercentAdd,
                "1",
                Some(PoolShift::Difference),
            ),
            (
                "grown",
                EntityStatId::MAX_HEALTH,
                ModifierOp::PercentAdd,
                "1",
                Some(PoolShift::Clamp),
            ),
            (
                "sapping",
                EntityStatId::MAX_ENERGY,
                ModifierOp::FlatAdd,
                "-30",
                Some(PoolShift::Difference),
            ),
            (
                "silenced",
                EntityStatId::MAX_ENERGY,
                ModifierOp::PercentAdd,
                "-1",
                Some(PoolShift::Share),
            ),
            (
                "hushed",
                EntityStatId::MAX_ENERGY,
                ModifierOp::PercentAdd,
                "-1",
                Some(PoolShift::Clamp),
            ),
            (
                "muted",
                EntityStatId::MAX_ENERGY,
                ModifierOp::PercentAdd,
                "-1",
                Some(PoolShift::Difference),
            ),
            (
                "plated",
                EntityStatId::MAX_HEALTH,
                ModifierOp::FlatAdd,
                "10",
                Some(PoolShift::Difference),
            ),
            (
                "burning",
                EntityStatId::HEALTH_DRAIN,
                ModifierOp::FlatAdd,
                "2",
                None,
            ),
            (
                "bright",
                EntityStatId::ENERGY_REGEN,
                ModifierOp::FlatAdd,
                "1",
                None,
            ),
            (
                "doubled",
                EntityStatId::MAX_HEALTH,
                ModifierOp::PercentAdd,
                "1",
                Some(PoolShift::Share),
            ),
            (
                "braced",
                EntityStatId::MAX_HEALTH,
                ModifierOp::FlatAdd,
                "50",
                Some(PoolShift::Difference),
            ),
            (
                "halved",
                EntityStatId::MAX_HEALTH,
                ModifierOp::PercentAdd,
                "-0.5",
                Some(PoolShift::Clamp),
            ),
            (
                "crushed",
                EntityStatId::MAX_HEALTH,
                ModifierOp::FlatAdd,
                "-99",
                Some(PoolShift::Difference),
            ),
            (
                "charged",
                EntityStatId::MAX_ENERGY,
                ModifierOp::PercentAdd,
                "1",
                Some(PoolShift::Share),
            ),
        ] {
            registry.register_entity_buff(
                name,
                buff(stat, op, magnitude, pool_shift, StackRule::Ignore),
            );
        }
        registry.register_entity_buff(
            "layered",
            buff(
                EntityStatId::MAX_HEALTH,
                ModifierOp::FlatAdd,
                "10",
                Some(PoolShift::Difference),
                StackRule::StackToCap(5),
            ),
        );
        registry.register_entity_buff(
            "rung",
            buff(
                EntityStatId::MAX_HEALTH,
                ModifierOp::FlatAdd,
                "10",
                Some(PoolShift::Clamp),
                StackRule::StackToCap(5),
            ),
        );
        registry.register_player_buff(
            "attuned",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::PoolMaximums {
                    modifiers: vec![utils::flat(EntityStatId::MAX_ENERGY, "40")],
                    pool_shift: PoolShift::Difference,
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register_player_buff(
            "dimmed",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::PoolMaximums {
                    modifiers: vec![utils::flat(EntityStatId::MAX_ENERGY, "-20")],
                    pool_shift: PoolShift::Clamp,
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register_player_buff(
            "drill",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::PoolMaximums {
                    modifiers: vec![utils::flat(EntityStatId::MAX_HEALTH, "10")],
                    pool_shift: PoolShift::Difference,
                }],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register(
            EntityTypeDef::new("keep")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(100))
                .with_pool(Pool::energy(40))
                .with_dying(2, []),
        );
        let hatch = |into: &str, pools: PoolCarry| {
            MorphTransition::new(
                into,
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(2),
                MorphPlacement::Reserve,
                MorphCancel::Committed,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [(PoolId::HEALTH, pools)],
            )
        };
        registry.register(
            EntityTypeDef::new("egg")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(50))
                .with_dying(2, [])
                .with_morphs([
                    hatch("hatchling", PoolCarry::Full),
                    hatch("husk", PoolCarry::Shift(PoolShift::Share)),
                    hatch("grub", PoolCarry::Shift(PoolShift::Difference)),
                    hatch("pupa", PoolCarry::Shift(PoolShift::Clamp)),
                    MorphTransition::new(
                        "shell",
                        MorphCourse::direct(MorphInterrupted::Reverts),
                        Quantity::Constant(10),
                        MorphPlacement::Reserve,
                        MorphCancel::Refundable,
                        MorphReason::Change,
                        vec![Cost::Health(FixedU64::from_num(10))],
                        Vec::new(),
                        [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                ]),
        );
        for name in ["hatchling", "husk", "grub", "pupa", "shell"] {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                    .with_pool(Pool::health(80))
                    .with_dying(2, []),
            );
        }
        let pour = |into: &str, carries: &[(PoolId, PoolCarry)]| {
            MorphTransition::new(
                into,
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::Constant(2),
                MorphPlacement::Reserve,
                MorphCancel::Committed,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                carries.iter().copied(),
            )
        };
        let vessel = |name: &str, health: u32, energy: Option<u32>| {
            let def = EntityTypeDef::new(name)
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(health))
                .with_dying(2, []);
            match energy {
                Some(energy) => def.with_pool(Pool::energy(energy)),
                None => def,
            }
        };
        registry.register(vessel("flask", 50, Some(40)).with_morphs([
            pour(
                "vessel",
                &[
                    (PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share)),
                    (PoolId::ENERGY, PoolCarry::Full),
                ],
            ),
            pour(
                "jar",
                &[(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            ),
            pour(
                "lamp",
                &[(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            ),
        ]));
        registry.register(vessel("vessel", 80, Some(20)));
        registry.register(vessel("jar", 80, Some(30)));
        registry.register(vessel("lamp", 50, None));
        let pupate = |into: &str, enter: PoolCarry, revert: RevertCarry| {
            MorphTransition::new(
                into,
                MorphCourse::via(
                    "cocoon",
                    [(PoolId::HEALTH, enter)],
                    ViaInterrupted::reverts([(PoolId::HEALTH, revert)]),
                ),
                Quantity::Constant(10),
                MorphPlacement::Reserve,
                MorphCancel::Forfeit,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            )
        };
        registry.register(vessel("larva", 50, None).with_morphs([
            pupate("moth", PoolCarry::Full, RevertCarry::Restore),
            pupate(
                "beetle",
                PoolCarry::Shift(PoolShift::Share),
                RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
            ),
        ]));
        let hide = registry.register_entity_buff("hide", passive_buff("50", PoolShift::Difference));
        let wings =
            registry.register_entity_buff("wings", passive_buff("20", PoolShift::Difference));
        let glow = registry.register_entity_buff("glow", passive_buff("50", PoolShift::Clamp));
        let spin = |into: &str, revert: &[(PoolId, RevertCarry)], land: &[(PoolId, PoolCarry)]| {
            MorphTransition::new(
                into,
                MorphCourse::via(
                    "chrysalis",
                    [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ViaInterrupted::reverts(revert.iter().copied()),
                ),
                Quantity::Constant(10),
                MorphPlacement::Reserve,
                MorphCancel::Forfeit,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                land.iter().copied(),
            )
        };
        let health_share = [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))];
        registry.register(
            vessel("caterpillar", 50, Some(40))
                .with_passives([hide])
                .with_morphs([
                    spin(
                        "butterfly",
                        &[
                            (PoolId::HEALTH, RevertCarry::Restore),
                            (PoolId::ENERGY, RevertCarry::Restore),
                        ],
                        &health_share,
                    ),
                    spin(
                        "hawkmoth",
                        &[(
                            PoolId::HEALTH,
                            RevertCarry::Carry(PoolCarry::Shift(PoolShift::Share)),
                        )],
                        &health_share,
                    ),
                    spin("skipper", &[], &health_share),
                    spin(
                        "emperor",
                        &[],
                        &[
                            (PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share)),
                            (PoolId::ENERGY, PoolCarry::Shift(PoolShift::Share)),
                        ],
                    ),
                    spin(
                        "regent",
                        &[],
                        &[
                            (PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share)),
                            (PoolId::ENERGY, PoolCarry::Full),
                        ],
                    ),
                    pour(
                        "monarch",
                        &[(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
                    ),
                ]),
        );
        registry.register(vessel("chrysalis", 200, None));
        registry.register_entity_buff(
            "weighed",
            EntityBuffDef {
                effects: vec![
                    EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                        modifiers: vec![utils::flat(EntityStatId::MAX_HEALTH, "-50")],
                        pool_shift: PoolShift::Difference,
                    }),
                    EntityEffect::Modifiers(EntityModifiers::Stats(vec![utils::flat(
                        EntityStatId::ARMOR,
                        "1",
                    )])),
                ],
                lasting: Lasting::Forever,
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register(
            vessel("silkworm", 100, None)
                .with_stat(EntityStatId::ARMOR, FixedU64::ZERO)
                .with_morphs([spin(
                    "butterfly",
                    &[(PoolId::HEALTH, RevertCarry::Restore)],
                    &health_share,
                )]),
        );
        registry.register(vessel("lantern", 50, None).with_passives([glow]));
        // A lantern starting at half its health, its maximum raised by `glow`.
        registry.register(
            EntityTypeDef::new("dim_lantern")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::builtin(
                    PoolId::HEALTH,
                    FixedU64::from_num(50),
                    FixedU64::ZERO,
                    FixedU64::ZERO,
                    PoolInitial::Share(utils::fixed("0.5")),
                ))
                .with_passives([glow]),
        );
        // A lamp starting at 30 of its 50 health, its maximum raised by
        // `wings` and the raise's difference given to the pool.
        registry.register(
            EntityTypeDef::new("low_lamp")
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::builtin(
                    PoolId::HEALTH,
                    FixedU64::from_num(50),
                    FixedU64::ZERO,
                    FixedU64::ZERO,
                    PoolInitial::Amount(utils::fixed("30")),
                ))
                .with_passives([wings]),
        );
        // A wick starting at a quarter of its 40 energy, and a taper at 15.
        for (name, initial) in [
            ("wick", PoolInitial::Share(utils::fixed("0.25"))),
            ("taper", PoolInitial::Amount(utils::fixed("15"))),
        ] {
            registry.register(
                EntityTypeDef::new(name)
                    .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                    .with_pool(Pool::health(50))
                    .with_pool(Pool::builtin(
                        PoolId::ENERGY,
                        FixedU64::from_num(40),
                        FixedU64::ZERO,
                        FixedU64::ZERO,
                        initial,
                    ))
                    .with_dying(2, []),
            );
        }
        for name in ["butterfly", "hawkmoth", "skipper"] {
            registry.register(vessel(name, 80, None));
        }
        registry.register(vessel("monarch", 80, None).with_passives([wings]));
        registry.register(vessel("emperor", 80, Some(100)));
        registry.register(vessel("regent", 80, Some(100)));
        let spark = registry.register_entity_buff(
            "spark",
            EntityBuffDef {
                effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
                    modifiers: vec![utils::flat(EntityStatId::MAX_ENERGY, "20")],
                    pool_shift: PoolShift::Difference,
                })],
                lasting: Lasting::While(Requirement::Health(Bound::Amount(Threshold::AtLeast(
                    FixedU64::ONE,
                )))),
                stack_rule: StackRule::Ignore,
                interrupted_by: Vec::new(),
            },
        );
        registry.register(
            vessel("glowworm", 50, Some(40))
                .with_passives([spark])
                .with_morphs([spin(
                    "emperor",
                    &[],
                    &[
                        (PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share)),
                        (PoolId::ENERGY, PoolCarry::Shift(PoolShift::Share)),
                    ],
                )]),
        );
        registry.register(vessel("cocoon", 100, None));
        registry.register(vessel("moth", 80, None));
        registry.register(vessel("beetle", 80, None));
        registry.register(vessel("seed", 50, None).with_morphs([
            pour(
                "flask",
                &[(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            ),
            pour(
                "wick",
                &[(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            ),
        ]));
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}

/// A forever buff of one modifier.
fn buff(
    stat: EntityStatId,
    op: ModifierOp,
    magnitude: &str,
    pool_shift: Option<PoolShift>,
    stack_rule: StackRule,
) -> EntityBuffDef {
    let modifiers = vec![EntityModifier {
        stat,
        op,
        magnitude: utils::signed_fixed(magnitude),
    }];
    let modifiers = match pool_shift {
        Some(pool_shift) => EntityModifiers::PoolMaximums {
            modifiers,
            pool_shift,
        },
        None => EntityModifiers::Stats(modifiers),
    };
    EntityBuffDef {
        effects: vec![EntityEffect::Modifiers(modifiers)],
        lasting: Lasting::Forever,
        stack_rule,
        interrupted_by: Vec::new(),
    }
}

/// Player 0's keep, a tick old, with `amount` of its 100 health taken.
fn wounded_keep(app: &mut App, amount: &str) -> Entity {
    let (keep, _) = utils::create_owned(app, "keep", 5, 5, 0);
    utils::run_ticks(app, 1);
    utils::wound(app, keep, amount);
    keep
}

/// The id of the fixture's entity buff `name`.
fn buff_id(app: &App, name: &str) -> EntityBuffId {
    app.world()
        .resource::<ContentRegistry>()
        .entity_buff(name)
        .expect("the fixture registers the buff")
}

/// A passive held while its bearer lives: `max_health +magnitude`, moving
/// the pool by `pool_shift`.
fn passive_buff(magnitude: &str, pool_shift: PoolShift) -> EntityBuffDef {
    EntityBuffDef {
        effects: vec![EntityEffect::Modifiers(EntityModifiers::PoolMaximums {
            modifiers: vec![utils::flat(EntityStatId::MAX_HEALTH, magnitude)],
            pool_shift,
        })],
        lasting: Lasting::While(Requirement::Health(Bound::Amount(Threshold::AtLeast(
            FixedU64::ONE,
        )))),
        stack_rule: StackRule::Ignore,
        interrupted_by: Vec::new(),
    }
}

/// Player 0's caterpillar in its chrysalis on the way to `into`: 60 of 100
/// health and 10 of 40 energy at the start, entered by share at
/// 60 × 200 / 100 = 120 of 200, then wounded to 40 of 200.
fn spun_caterpillar(app: &mut App, into: &str) -> Entity {
    let (caterpillar, _) = utils::create_owned(app, "caterpillar", 5, 5, 0);
    utils::run_ticks(app, 1);
    utils::wound(app, caterpillar, "40");
    pools::drain(
        app.world_mut(),
        caterpillar,
        PoolId::ENERGY,
        utils::fixed("30"),
    );
    utils::order_morph(app, caterpillar, into);
    utils::run_ticks(app, 1);
    assert_eq!(entity_def::type_name(app.world(), caterpillar), "chrysalis");
    assert_eq!(utils::health_as_u32(app, caterpillar), 120);
    utils::wound(app, caterpillar, "80");
    caterpillar
}

/// One tick of a row of the expected-results table: what changes before it,
/// then the health and the maximum after it.
type Tick = (Vec<Act>, u32, u32);

/// One change a row of the expected-results table makes before a tick.
enum Act {
    /// The fixture's entity buff, applied to the keep.
    Apply(&'static str),
    /// The fixture's entity buff, taken off the keep.
    Remove(&'static str),
    /// The fixture's player buff, applied to player 0.
    ApplyPlayer(&'static str),
    /// Health restored to the keep, a decimal string.
    Heal(&'static str),
}

/// The id of the fixture's player buff `name`.
fn player_buff_id(app: &App, name: &str) -> PlayerBuffId {
    app.world()
        .resource::<ContentRegistry>()
        .player_buff(name)
        .expect("the fixture registers the buff")
}

/// App whose `sprout` (energy 40 starting at 10) changes into a `bloom`
/// through a `pod` (energy 40 starting at half) or into a `fruit` through a
/// `husk` with no energy, both destinations at a quarter of 60; every carry
/// of energy, the reverts included, starts it at the form's initial.
fn carry_initial_app() -> App {
    let mut app = utils::make_app(vec![PlayerSlot::occupied(0, PlayerType::Human, None, None)]);
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let energy = |maximum: u32, initial: PoolInitial| {
            Pool::builtin(
                PoolId::ENERGY,
                FixedU64::from_num(maximum),
                FixedU64::ZERO,
                FixedU64::ZERO,
                initial,
            )
        };
        let vessel = |name: &str| {
            EntityTypeDef::new(name)
                .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
                .with_pool(Pool::health(50))
        };
        registry
            .register(vessel("pod").with_pool(energy(40, PoolInitial::Share(utils::fixed("0.5")))));
        registry.register(vessel("husk"));
        for name in ["bloom", "fruit"] {
            registry.register(
                vessel(name).with_pool(energy(60, PoolInitial::Share(utils::fixed("0.25")))),
            );
        }
        let via = |into: &str, interim: &str, enter: Vec<(PoolId, PoolCarry)>| {
            MorphTransition::new(
                into,
                MorphCourse::via(
                    interim,
                    enter,
                    ViaInterrupted::reverts([(
                        PoolId::ENERGY,
                        RevertCarry::Carry(PoolCarry::Initial),
                    )]),
                ),
                Quantity::Constant(10),
                MorphPlacement::Reserve,
                MorphCancel::Forfeit,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [(PoolId::ENERGY, PoolCarry::Initial)],
            )
        };
        registry.register(
            vessel("sprout")
                .with_pool(energy(40, PoolInitial::Amount(utils::fixed("10"))))
                .with_morphs([
                    via("bloom", "pod", vec![(PoolId::ENERGY, PoolCarry::Initial)]),
                    via("fruit", "husk", Vec::new()),
                ]),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
