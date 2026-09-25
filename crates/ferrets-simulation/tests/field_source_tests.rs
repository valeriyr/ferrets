//! The growth state a field source carries across a form change.

use ferrets_content::{
    detection::Detection,
    field::{
        Emission, FieldDecay, FieldDef, FieldGrowth, FieldId, FieldLayer, FieldSourceDef,
        FieldVision,
    },
    registry::ContentRegistry,
};
use ferrets_simulation::components::field_source::{
    Emitted, FieldSourceState, FieldSourcesComponent,
};

//
// ─── Carrying reach across a form change ──────────────────────────────────────
//

#[test]
fn instant_source_spans_its_radius_whatever_it_took_over() {
    let field = creep();
    let instant = FieldSourceDef::new(
        field,
        6,
        FieldGrowth::Instant,
        Emission::Nothing,
        Emission::Full,
    );

    assert_eq!(
        FieldSourceState::carried(&instant, 2),
        FieldSourceState::full(&instant)
    );
}

#[test]
fn gradual_source_carries_reach_and_restarts_its_cycle() {
    let field = creep();
    let gradual = FieldSourceDef::new(
        field,
        6,
        gradual_growth(4, 1),
        Emission::Nothing,
        Emission::Full,
    );

    let carried = FieldSourceState::carried(&gradual, 3);

    assert_eq!(carried.reach, 3);
    assert_eq!(carried.countdown, 4);
}

#[test]
fn gradual_source_never_carries_reach_past_its_radius() {
    let field = creep();
    let gradual = FieldSourceDef::new(
        field,
        6,
        gradual_growth(4, 1),
        Emission::Nothing,
        Emission::Full,
    );

    assert_eq!(FieldSourceState::carried(&gradual, 10).reach, 6);
}

#[test]
fn sources_without_predecessor_start_fresh() {
    let field = creep();
    let defs = [
        FieldSourceDef::new(
            field,
            6,
            gradual_growth(4, 1),
            Emission::Nothing,
            Emission::Full,
        ),
        FieldSourceDef::new(
            field,
            3,
            gradual_growth(2, 1),
            Emission::Nothing,
            Emission::Full,
        ),
    ];
    let previous = FieldSourcesComponent(vec![FieldSourceState {
        reach: 5,
        countdown: 0,
        last_emitted: Emitted::Reach(5),
    }]);

    let carried = FieldSourcesComponent::carried(&defs, &previous);

    assert_eq!(carried.0[0].reach, 5);
    assert_eq!(carried.0[1], FieldSourceState::seeded(&defs[1]));
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A gradual growth of `cycle` ticks per cell from `initial_radius`.
fn gradual_growth(cycle: u32, initial_radius: u32) -> FieldGrowth {
    FieldGrowth::Gradual {
        cycle,
        initial_radius,
    }
}

/// The handle of one registered field over a ground layer.
fn creep() -> FieldId {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground");
    registry.register_field(
        "creep",
        FieldDef::new(
            FieldLayer::Passable(ground.into()),
            FieldDecay::Never,
            FieldVision::Dark,
            Detection::Blind,
        ),
    )
}
