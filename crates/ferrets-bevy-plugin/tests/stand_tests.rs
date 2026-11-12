//! Acts an entity performs once when it comes to stand: they wait for a site
//! to finish, act the first standing tick, never repeat, and survive a change
//! of form.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    area::Area,
    build::BuilderAttendance,
    detection::Detection,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{Emission, FieldAction, FieldDecay, FieldDef, FieldGrowth, FieldId, FieldSourceDef},
    location::Solidity,
    morph::{
        MorphCancel, MorphCourse, MorphInterrupted, MorphPlacement, MorphReason, MorphTransition,
        PoolCarry,
    },
    player_buffs::PlayerBuffDef,
    pool::Pool,
    pool_def::PoolId,
    pool_shift::PoolShift,
    quantity::Quantity,
    registry::ContentRegistry,
    skills::{Casting, EntityCastEffect, Reach, SkillCaster, SkillDef, Spot},
    stack_rule::StackRule,
    stand::StandingAct,
    stats::{EntityModifier, ModifierOp},
    terrains::Terrains,
    work::{CrewLimit, WorkPresence},
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::{FixedI64, FixedU64};
use ferrets_simulation::{command::PlayerCommand, game_loop::buffs};

//
// ─── When the act happens ─────────────────────────────────────────────────────
//

#[test]
fn standing_act_removes_only_unsustained_coverage_within_its_area() {
    let mut app = stand_app();
    let blight = blight(&app);
    let hive = utils::place(&mut app, "hive", 4, 4, 0);
    utils::run_ticks(&mut app, 1);
    assert!(utils::covered_by(&app, blight, 9, 4, 0));

    // Alive, the hive sustains what the cleanser reaches.
    utils::create_owned(&mut app, "cleanser", 12, 4, 1);
    utils::run_ticks(&mut app, 1);
    assert!(
        utils::covered_by(&app, blight, 9, 4, 0),
        "sustained blight stays"
    );

    // A second cleanser acts on what the dead hive left behind.
    utils::deplete(&mut app, hive);
    utils::create_owned(&mut app, "cleanser", 11, 4, 1);
    utils::run_ticks(&mut app, 1);
    assert!(
        !utils::covered_by(&app, blight, 9, 4, 0),
        "orphaned blight in reach is cleared"
    );
    assert!(
        utils::covered_by(&app, blight, 5, 4, 0),
        "beyond the area it lingers"
    );
}

#[test]
fn standing_act_reads_its_area_from_stat() {
    let mut app = stand_app();
    let blight = blight(&app);
    let (_, overlord) = utils::create_owned(&mut app, "overlord", 20, 20, 0);
    utils::cast_at(&mut app, overlord, "spew", 10, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    utils::cast_at(&mut app, overlord, "spew", 13, 10);
    utils::run_ticks(&mut app, utils::APPLY);
    assert!(utils::covered_by(&app, blight, 14, 10, 0));

    let wide = app
        .world()
        .resource::<ContentRegistry>()
        .player_buff("wide_scrubbing")
        .expect("the buff is registered");
    buffs::apply_player_buff(app.world_mut(), 1, wide);
    utils::create_owned(&mut app, "scrubber", 10, 10, 1);
    utils::run_ticks(&mut app, 1);

    // The scrub reach stat holds 2 + 1 = 3 under its owner's buff: (13, 10)
    // is cleared, (14, 10) is not.
    assert!(!utils::covered_by(&app, blight, 13, 10, 0));
    assert!(utils::covered_by(&app, blight, 14, 10, 0));
}

#[test]
fn standing_act_waits_for_site_to_finish() {
    let mut app = stand_app();
    let blight = blight(&app);
    spew_at(&mut app, 24, 20);
    // Already within reach of the site, so it is raised on the first tick.
    let (_, drone) = utils::create_owned(&mut app, "drone", 23, 22, 0);

    utils::push_command(
        &mut app,
        PlayerCommand::BuildEntity {
            builder: drone,
            type_name: "cleanser".into(),
            anchor: utils::cell(24, 22),
            flush: true,
        },
    );
    // Once the command lands the site is raised at once and takes four ticks
    // of work to finish; rising, it acts on nothing.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::count_of_type(app.world_mut(), "cleanser"), 1);
    assert!(utils::covered_by(&app, blight, 24, 20, 0), "still rising");

    // The last tick of work completes the site, and the act follows on the
    // first tick it stands finished.
    utils::run_ticks(&mut app, 5);
    assert!(
        !utils::covered_by(&app, blight, 24, 20, 0),
        "cleared as it finished"
    );
}

#[test]
fn map_placed_entity_acts_on_its_first_tick() {
    let mut app = stand_app();
    let blight = blight(&app);
    spew_at(&mut app, 24, 20);
    assert!(utils::covered_by(&app, blight, 24, 20, 0));

    utils::place(&mut app, "cleanser", 26, 20, 1);
    utils::run_ticks(&mut app, 1);
    assert!(!utils::covered_by(&app, blight, 24, 20, 0));
}

//
// ─── Once, and only once ──────────────────────────────────────────────────────
//

#[test]
fn standing_act_is_performed_once() {
    let mut app = stand_app();
    let blight = blight(&app);
    // The cleanser has stood for a while: its act is behind it.
    utils::create_owned(&mut app, "cleanser", 26, 20, 1);
    utils::run_ticks(&mut app, 2);

    // A patch spewed within its area afterwards is left alone.
    spew_at(&mut app, 24, 20);
    utils::run_ticks(&mut app, 2);
    assert!(utils::covered_by(&app, blight, 24, 20, 0));
}

#[test]
fn form_change_keeps_standing_act_performed() {
    let mut app = stand_app();
    let blight = blight(&app);
    // The local player's own, so the change can be ordered.
    let (_, cleanser) = utils::create_owned(&mut app, "cleanser", 26, 20, 0);
    utils::run_ticks(&mut app, 2);
    spew_at(&mut app, 24, 20);
    assert!(utils::covered_by(&app, blight, 24, 20, 0));

    // Changed into its other form and back, it stands with its act behind it
    // both times: the patch is left alone.
    for into in ["purifier", "cleanser"] {
        utils::select(&mut app, cleanser);
        utils::push_command(
            &mut app,
            PlayerCommand::Morph {
                type_name: into.into(),
                flush: true,
            },
        );
        utils::run_ticks(&mut app, utils::APPLY + 2);
        assert_eq!(utils::count_of_type(app.world_mut(), into), 1);
        assert!(
            utils::covered_by(&app, blight, 24, 20, 0),
            "no second act as {into}"
        );
    }
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// Player 0's overlord spews a patch of blight on the cell and the ticks apply.
fn spew_at(app: &mut App, x: u32, y: u32) {
    let (_, overlord) = utils::create_owned(app, "overlord", 20, 20, 0);
    utils::cast_at(app, overlord, "spew", x, y);
    utils::run_ticks(app, utils::APPLY);
}

/// The handle of the one field the fixture registers.
fn blight(app: &App) -> FieldId {
    app.world()
        .resource::<ContentRegistry>()
        .field("blight")
        .unwrap()
}

fn building(name: &str, side: u32, rise_time: u32) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(
            utils::GROUND_DOMAIN,
            CellSize::new(side, side),
            Solidity::Solid,
            utils::GROUND,
        )
        .with_pool(Pool::health(100))
        .with_dying(1, [])
        .with_rise(rise_time, utils::rise_initial_pools(&[PoolId::HEALTH]))
}

/// App with two rival players over a blight field that never recedes on its
/// own, so what a standing act clears and leaves is exactly what the tests
/// see. A hive spreads blight; a cleanser and a purifier — two forms of one
/// thing — each clear it as they come to stand, and a scrubber as far as a
/// stat of its reads; an overlord spews patches of it; a drone raises
/// cleansers.
fn stand_app() -> App {
    let mut app = utils::make_app(utils::human_slots(2));
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let blight = registry.register_field(
            "blight",
            FieldDef::new(
                Terrains::Only(utils::LAND.into()),
                utils::GROUND.into(),
                FieldDecay::Never,
                None,
                None,
                Detection::Blind,
            ),
        );

        registry.register(
            building("hive", 2, 4).with_field_sources([FieldSourceDef::new(
                blight,
                Area::Around(Quantity::from(4)),
                FieldGrowth::Instant,
                Emission::Nothing,
                Emission::Full,
            )]),
        );

        let clears_blight = StandingAct::Field {
            field: blight,
            area: Area::Around(Quantity::from(3)),
            action: FieldAction::Clear,
        };
        let refit = |into: &str| {
            MorphTransition::new(
                into,
                MorphCourse::direct(MorphInterrupted::Reverts),
                Quantity::from(1),
                MorphPlacement::Revalidate,
                MorphCancel::Forfeit,
                MorphReason::Change,
                Vec::new(),
                Vec::new(),
                [(PoolId::HEALTH, PoolCarry::Shift(PoolShift::Share))],
            )
        };
        registry.register(
            building("cleanser", 1, 4)
                .with_standing_acts([clears_blight])
                .with_morphs([refit("purifier")]),
        );
        registry.register(
            building("purifier", 1, 4)
                .with_standing_acts([clears_blight])
                .with_morphs([refit("cleanser")]),
        );

        let scrub_reach = registry.register_entity_stat("scrub_reach", FixedU64::ZERO);
        registry.register_player_buff(
            "wide_scrubbing",
            PlayerBuffDef {
                player_modifiers: Vec::new(),
                entity_modifiers: vec![EntityModifiers::Stats(vec![EntityModifier {
                    stat: scrub_reach,
                    op: ModifierOp::FlatAdd,
                    magnitude: FixedI64::from_num(1),
                }])],
                duration: None,
                stack_rule: StackRule::Ignore,
            },
        );
        registry.register(
            building("scrubber", 1, 4)
                .with_stat(scrub_reach, FixedU64::from_num(2))
                .with_standing_acts([StandingAct::Field {
                    field: blight,
                    area: Area::Around(Quantity::Stat(scrub_reach)),
                    action: FieldAction::Clear,
                }]),
        );

        let spew = registry.register_skill(
            "spew",
            SkillDef {
                cooldown: 1,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    casting: Casting::Instant,
                    effect: EntityCastEffect::Field {
                        at: Spot::Ground {
                            reach: Reach::Wherever,
                        },
                        field: blight,
                        area: Area::Around(Quantity::from(1)),
                        action: FieldAction::Cover,
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(utils::walker("overlord", utils::GROUND_DOMAIN).with_skills([spew]));
        // Sight enough to see where it builds: the build command is gated on
        // the fog.
        registry.register(
            utils::walker("drone", utils::GROUND_DOMAIN)
                .with_sight_range(4)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(
                    ["cleanser"],
                    BuilderAttendance::Crew(WorkPresence::Present {
                        crew: CrewLimit::ONE,
                    }),
                ),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    utils::start_game(&mut app);
    app
}
