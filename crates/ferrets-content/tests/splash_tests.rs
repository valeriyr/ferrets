//! A blast's bands: a core and the rings around it, and the declarations a
//! splash refuses as saying nothing.

use ferrets_content::{
    area::Area,
    entity_stats::EntityStatId,
    quantity::Quantity,
    splash::{SplashDef, SplashShape},
};
use ferrets_math::FixedU64;

mod utils;

//
// ─── Rings ────────────────────────────────────────────────────────────────────
//

#[test]
fn rings_keep_their_widths_and_fractions_outward() {
    let splash = blast(
        Area::Around(Quantity::from(1)),
        vec![
            (Quantity::from(1), utils::fixed("0.25")),
            (Quantity::Constant(utils::fixed("1.5")), utils::fixed("0.1")),
        ],
    );

    assert_eq!(
        splash.core(),
        (Area::Around(Quantity::from(1)), utils::fixed("0.5"))
    );
    assert_eq!(
        splash.rings().collect::<Vec<_>>(),
        vec![
            (Quantity::from(1), utils::fixed("0.25")),
            (Quantity::Constant(utils::fixed("1.5")), utils::fixed("0.1")),
        ]
    );
}

#[test]
#[should_panic(
    expected = "a splash ring of constant width 0 reaches no further than the band inside it"
)]
fn ring_of_constant_zero_width_around_core_is_refused() {
    blast(
        Area::Around(Quantity::from(1)),
        vec![(Quantity::from(0), utils::fixed("0.25"))],
    );
}

#[test]
#[should_panic(
    expected = "a splash ring of constant width 0 reaches no further than the band inside it"
)]
fn ring_of_constant_zero_width_around_ring_is_refused() {
    blast(
        Area::Inside,
        vec![
            (Quantity::from(0), utils::fixed("0.25")),
            (Quantity::from(0), utils::fixed("0.1")),
        ],
    );
}

#[test]
fn ring_of_zero_width_around_inside_core_is_touching_ring() {
    let splash = blast(
        Area::Inside,
        vec![(Quantity::from(0), utils::fixed("0.25"))],
    );

    assert_eq!(
        splash.rings().collect::<Vec<_>>(),
        vec![(Quantity::from(0), utils::fixed("0.25"))]
    );
}

#[test]
fn ring_width_read_from_stat_is_never_judged_at_load() {
    let width = Quantity::Stat(EntityStatId::ATTACK_RANGE);
    let splash = blast(
        Area::Around(Quantity::from(1)),
        vec![(width, utils::fixed("0.25"))],
    );

    assert_eq!(
        splash.rings().collect::<Vec<_>>(),
        vec![(width, utils::fixed("0.25"))]
    );
}

//
// ─── Fractions ────────────────────────────────────────────────────────────────
//

#[test]
fn outermost_ring_at_zero_fraction_is_kept() {
    let splash = blast(
        Area::Around(Quantity::from(1)),
        vec![(Quantity::from(1), FixedU64::ZERO)],
    );

    assert_eq!(
        splash.rings().collect::<Vec<_>>(),
        vec![(Quantity::from(1), FixedU64::ZERO)]
    );
}

#[test]
fn core_alone_at_zero_fraction_is_kept() {
    let splash = SplashDef::new(
        SplashShape::Circular,
        (Area::Around(Quantity::from(1)), FixedU64::ZERO),
        Vec::new(),
        utils::GROUND,
        true,
    );

    assert_eq!(
        splash.core(),
        (Area::Around(Quantity::from(1)), FixedU64::ZERO)
    );
}

#[test]
fn core_at_zero_fraction_inside_rings_is_kept() {
    let splash = SplashDef::new(
        SplashShape::Circular,
        (Area::Around(Quantity::from(1)), FixedU64::ZERO),
        vec![(Quantity::from(1), utils::fixed("0.5"))],
        utils::GROUND,
        true,
    );

    assert_eq!(
        splash.core(),
        (Area::Around(Quantity::from(1)), FixedU64::ZERO)
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A circular ground blast whose core reaches `core` at half the damage,
/// ringed by `rings`.
fn blast(core: Area, rings: Vec<(Quantity, FixedU64)>) -> SplashDef {
    SplashDef::new(
        SplashShape::Circular,
        (core, utils::fixed("0.5")),
        rings,
        utils::GROUND,
        true,
    )
}
