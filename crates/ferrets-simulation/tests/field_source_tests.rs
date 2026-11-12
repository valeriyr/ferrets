//! The growth state a field source starts in and carries across a form change.

use ferrets_content::{
    area::Area,
    detection::Detection,
    field::{Emission, FieldDecay, FieldDef, FieldGrowth, FieldId, FieldSourceDef},
    layer::LayerLevel,
    quantity::Quantity,
    registry::ContentRegistry,
    terrains::Terrains,
};
use ferrets_simulation::components::field_source::{
    Emitted, FieldReach, FieldSourceState, FieldSourcesComponent, Growth,
};

//
// ─── Taking over growth across a form change ──────────────────────────────────
//

#[test]
fn instant_source_spans_its_area_whatever_it_took_over() {
    let (field, _) = fields();
    let instant = FieldSourceDef::new(
        field,
        Area::Around(Quantity::from(6)),
        FieldGrowth::Instant,
        Emission::Nothing,
        Emission::Full,
    );

    let taken = FieldSourceState::taking_over(&instant, &growing(field, 2, 1));

    assert_eq!(taken.growth, Growth::Unread(FieldReach::Full));
}

#[test]
fn gradual_source_carries_growth_and_restarts_its_cycle() {
    let (field, _) = fields();
    let gradual = FieldSourceDef::new(
        field,
        Area::Around(Quantity::from(6)),
        gradual_growth(4, 1),
        Emission::Nothing,
        Emission::Full,
    );

    let taken = FieldSourceState::taking_over(&gradual, &growing(field, 3, 1));

    assert_eq!(
        taken.growth,
        Growth::Growing {
            reach: 3,
            countdown: 4
        }
    );
}

#[test]
fn source_taking_over_keeps_what_predecessor_put_on_grid() {
    let (field, _) = fields();
    let instant = FieldSourceDef::new(
        field,
        Area::Around(Quantity::from(6)),
        FieldGrowth::Instant,
        Emission::Nothing,
        Emission::Full,
    );
    let gradual = gradual_source(field);

    for def in [instant, gradual] {
        let taken = FieldSourceState::taking_over(&def, &growing(field, 3, 1));
        assert_eq!(taken.last_emitted, Emitted::Reach(3));
    }
}

#[test]
fn gradual_source_takes_over_start_not_yet_read() {
    let (field, _) = fields();
    let gradual = FieldSourceDef::new(
        field,
        Area::Around(Quantity::from(6)),
        gradual_growth(4, 1),
        Emission::Nothing,
        Emission::Full,
    );
    let previous = FieldSourceState::new(&gradual, FieldReach::Full);

    assert_eq!(
        FieldSourceState::taking_over(&gradual, &previous).growth,
        Growth::Unread(FieldReach::Full)
    );
}

#[test]
fn sources_without_predecessor_start_at_their_growth_start() {
    let (field, _) = fields();
    let defs = [
        FieldSourceDef::new(
            field,
            Area::Around(Quantity::from(6)),
            gradual_growth(4, 1),
            Emission::Nothing,
            Emission::Full,
        ),
        FieldSourceDef::new(
            field,
            Area::Around(Quantity::from(3)),
            gradual_growth(2, 1),
            Emission::Nothing,
            Emission::Full,
        ),
    ];
    let previous = FieldSourcesComponent(vec![growing(field, 5, 1)]);

    let taken = FieldSourcesComponent::taking_over(&defs, &previous);

    assert_eq!(
        taken.0[0].growth,
        Growth::Growing {
            reach: 5,
            countdown: 4
        }
    );
    assert_eq!(
        taken.0[1],
        FieldSourceState::new(&defs[1], FieldReach::Initial)
    );
}

#[test]
fn sources_take_over_growth_of_same_field_whatever_order_fields_come_in() {
    let (creep, power) = fields();
    let defs = [gradual_source(power), gradual_source(creep)];
    let previous = FieldSourcesComponent(vec![growing(creep, 8, 1), growing(power, 3, 1)]);

    let taken = FieldSourcesComponent::taking_over(&defs, &previous);

    assert_eq!(
        taken.0[0].growth,
        Growth::Growing {
            reach: 3,
            countdown: 4
        }
    );
    assert_eq!(
        taken.0[1].growth,
        Growth::Growing {
            reach: 8,
            countdown: 4
        }
    );
}

#[test]
fn source_of_field_old_form_lacked_starts_at_its_growth_start() {
    let (creep, power) = fields();
    let defs = [gradual_source(power), gradual_source(creep)];
    let previous = FieldSourcesComponent(vec![growing(creep, 8, 1)]);

    let taken = FieldSourcesComponent::taking_over(&defs, &previous);

    assert_eq!(
        taken.0[0],
        FieldSourceState::new(&defs[0], FieldReach::Initial)
    );
    assert_eq!(
        taken.0[1].growth,
        Growth::Growing {
            reach: 8,
            countdown: 4
        }
    );
}

//
// ─── A source's growth and its start over ─────────────────────────────────────
//

#[test]
fn read_growth_gives_its_reach_and_unread_none() {
    assert_eq!(
        Growth::Growing {
            reach: 5,
            countdown: 2
        }
        .reach(),
        Some(5)
    );
    assert_eq!(Growth::Spanning { reach: 3 }.reach(), Some(3));
    assert_eq!(Growth::Unread(FieldReach::Initial).reach(), None);
}

#[test]
fn growth_given_new_reach_keeps_its_arm_and_countdown() {
    assert_eq!(
        Growth::Growing {
            reach: 5,
            countdown: 2
        }
        .with_reach(3),
        Growth::Growing {
            reach: 3,
            countdown: 2
        }
    );
    assert_eq!(
        Growth::Spanning { reach: 5 }.with_reach(3),
        Growth::Spanning { reach: 3 }
    );
}

#[test]
fn source_reaches_its_read_reach_or_patch_it_put_down() {
    let (field, _) = fields();
    let unread = |last_emitted| FieldSourceState {
        field,
        growth: Growth::Unread(FieldReach::Initial),
        last_emitted,
    };

    assert_eq!(growing(field, 3, 1).reach(), Some(3));
    assert_eq!(unread(Emitted::Reach(2)).reach(), Some(2));
    assert_eq!(unread(Emitted::Nothing).reach(), None);
}

#[test]
#[should_panic(expected = "only a read growth has its reach replaced")]
fn unread_growth_has_no_reach_to_replace() {
    Growth::Unread(FieldReach::Full).with_reach(3);
}

#[test]
fn sources_started_over_wait_unread_at_their_growth_start() {
    let (creep, power) = fields();
    let mut sources = FieldSourcesComponent(vec![growing(creep, 5, 2), growing(power, 3, 1)]);

    sources.start_over();

    for state in &sources.0 {
        assert_eq!(state.growth, Growth::Unread(FieldReach::Initial));
        assert_eq!(state.last_emitted, Emitted::Nothing);
    }
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

/// A gradual growth of `cycle` ticks per cell from `start_cells` around
/// the footprint.
fn gradual_growth(cycle: u32, start_cells: u32) -> FieldGrowth {
    FieldGrowth::Gradual {
        cycle,
        starts_at: Area::Around(Quantity::from(start_cells)),
    }
}

/// A gradual source of `field` reaching six cells out from its footprint,
/// growing a cell every four ticks from one cell out.
fn gradual_source(field: FieldId) -> FieldSourceDef {
    FieldSourceDef::new(
        field,
        Area::Around(Quantity::from(6)),
        gradual_growth(4, 1),
        Emission::Nothing,
        Emission::Full,
    )
}

/// A source of `field` grown `reach` out, `countdown` ticks from its next
/// cell, last emitting that reach.
fn growing(field: FieldId, reach: u32, countdown: u32) -> FieldSourceState {
    FieldSourceState {
        field,
        growth: Growth::Growing { reach, countdown },
        last_emitted: Emitted::Reach(reach),
    }
}

/// The handles of two registered fields, creep and power, lying on every
/// terrain of the one ground layer.
fn fields() -> (FieldId, FieldId) {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground", LayerLevel::At(1), &[]);
    let mut register = |name| {
        registry.register_field(
            name,
            FieldDef::new(
                Terrains::Any,
                ground.into(),
                FieldDecay::Never,
                None,
                None,
                Detection::Blind,
            ),
        )
    };
    (register("creep"), register("power"))
}
