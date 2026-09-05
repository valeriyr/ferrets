//! Acts an entity performs once when it comes to stand: they wait for a site
//! to finish, act the first standing tick, never repeat, and survive a change
//! of form.

mod utils;

use bevy::prelude::*;
use ferrets_content::{
    build::BuilderAttendance,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{FieldAction, FieldDecay, FieldDef, FieldGrowth, FieldId, FieldSourceDef, FieldVision},
    location::Solidity,
    morph::{MorphCancel, MorphPlacement, MorphTime, MorphTransition},
    registry::ContentRegistry,
    skills::{EntityCastEffect, EntityCastTarget, SkillCaster, SkillDef},
    stand::StandingAct,
    work::WorkPresence,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_simulation::{
    command::{PlayerCommand, SkillCasterRef, SkillTarget},
    session::GameSession,
};

//
// ─── When the act happens ─────────────────────────────────────────────────────
//

#[test]
fn standing_act_removes_only_unsustained_coverage_within_its_radius() {
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
        "beyond the radius it lingers"
    );
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
            position: utils::pos(24, 22),
            flush: true,
        },
    );
    // Once the command lands the site is raised at once and takes four ticks
    // of work to finish; going up, it acts on nothing.
    utils::run_ticks(&mut app, utils::APPLY);
    assert_eq!(utils::count_of_type(app.world_mut(), "cleanser"), 1);
    assert!(
        utils::covered_by(&app, blight, 24, 20, 0),
        "still under construction"
    );

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

    // A patch spewed within its radius afterwards is left alone.
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
    let spew = app
        .world()
        .resource::<ContentRegistry>()
        .skill("spew")
        .unwrap();
    utils::push_command(
        app,
        PlayerCommand::UseSkill {
            skill: spew,
            caster: SkillCasterRef::Entity(overlord),
            target: Some(SkillTarget::Position(utils::pos(x, y))),
        },
    );
    utils::run_ticks(app, utils::APPLY);
}

/// The handle of the one field the fixture registers.
fn blight(app: &App) -> FieldId {
    app.world()
        .resource::<ContentRegistry>()
        .field("blight")
        .unwrap()
}

fn mover(name: &str) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(utils::GROUND, CellSize::ONE, Solidity::Solid)
        .with_movement(
            FixedU64::from_num(0.5),
            FixedU64::from_num(0.5),
            FixedU64::ONE,
            FixedU64::from_num(360),
            FixedU64::from_num(360),
        )
        .with_health(20)
        .with_dying(1, None)
}

fn building(name: &str, side: u32, build_time: u32) -> EntityTypeDef {
    EntityTypeDef::new(name)
        .with_location(utils::GROUND, CellSize::new(side, side), Solidity::Solid)
        .with_health(100)
        .with_dying(1, None)
        .with_build_time(build_time)
}

/// App with two rival players over a blight field that never recedes on its
/// own, so what a standing act clears and leaves is exactly what the tests
/// see. A hive spreads blight; a cleanser and a purifier — two forms of one
/// thing — each clear it as they come to stand; an overlord spews patches of
/// it; a drone raises cleansers.
fn stand_app() -> App {
    let mut app = utils::make_app(utils::human_slots(2));
    {
        let mut registry = app.world_mut().resource_mut::<ContentRegistry>();
        let blight = registry.register_field(
            "blight",
            FieldDef::new(utils::GROUND, FieldDecay::Never, FieldVision::Dark),
        );

        registry.register(
            building("hive", 2, 4).with_field_sources([FieldSourceDef::new(
                blight,
                4,
                FieldGrowth::Instant,
                None,
            )]),
        );

        let clears_blight = StandingAct::Field {
            field: blight,
            radius: 3,
            action: FieldAction::Clear,
        };
        let refit = |into: &str| {
            MorphTransition::new(
                into,
                None,
                MorphTime::Constant(1),
                MorphPlacement::Revalidate,
                MorphCancel::Forfeit,
                Vec::new(),
                Vec::<String>::new(),
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

        let spew = registry.register_skill(
            "spew",
            SkillDef {
                cooldown: 1,
                caster: SkillCaster::Entity {
                    costs: Vec::new(),
                    target: EntityCastTarget::Position,
                    effect: EntityCastEffect::Field {
                        field: blight,
                        radius: 1,
                        action: FieldAction::Cover,
                    },
                },
                requires: Vec::new(),
            },
        );
        registry.register(mover("overlord").with_skills([spew]));
        // Sight enough to see where it builds: the build command is gated on
        // the fog.
        registry.register(
            mover("drone")
                .with_sight_range(4)
                .with_stat(EntityStatId::BUILD_RANGE, FixedU64::ONE)
                .with_builder(["cleanser"], BuilderAttendance::Crew(WorkPresence::Present)),
        );
    }
    app.world_mut().resource::<ContentRegistry>().validate();
    app.world_mut().resource_mut::<GameSession>().start();
    app
}
