//! The terms that last moved each of an entity's pool maxima, with how each
//! moves its pool, and how a pool's value moves as those parts come and go.

use bevy_ecs::prelude::*;
use ferrets_content::{
    entity_buffs::EntityBuffId,
    entity_effect::EntityEffect,
    entity_modifiers::EntityModifiers,
    entity_stats::EntityStatId,
    pool_def::PoolId,
    pool_shift::PoolShift,
    registry::ContentRegistry,
    stats::{self, ModifierOp},
};
use ferrets_math::{FixedI64, FixedU64};

/// One effect's terms on a pool's maximum as last folded, with the shift they
/// move the pool by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolShiftTerms {
    /// What the terms coming or going do to the pool.
    pub pool_shift: PoolShift,
    /// The terms on the maximum, as the fold reads them.
    pub terms: Vec<(ModifierOp, FixedI64)>,
}

/// The shift terms each pool maximum of an entity was last folded from.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct PoolShiftsComponent(Vec<(PoolId, Vec<PoolShiftTerms>)>);

impl PoolShiftsComponent {
    /// The shift terms `pool`'s maximum was last folded from.
    pub fn parts(&self, pool: PoolId) -> &[PoolShiftTerms] {
        self.0
            .iter()
            .find(|(of, _)| *of == pool)
            .map_or(&[][..], |(_, parts)| parts.as_slice())
    }

    /// Remembers `parts` as what `pool`'s maximum was last folded from.
    pub fn remember(&mut self, pool: PoolId, parts: Vec<PoolShiftTerms>) {
        match self.0.iter_mut().find(|(of, _)| *of == pool) {
            Some((_, remembered)) => *remembered = parts,
            None => self.0.push((pool, parts)),
        }
    }
}

/// `current` moved step by step from the parts `before` to the parts `now`:
/// what left, one step for each shift in the reverse of [`STEP_ORDER`], then
/// what arrived, one for each shift in it — every part of that shift going
/// or coming at once — moving it by that shift from the maximum the previous
/// step left. Clamp parts move nothing on the way: whatever of them left or
/// arrived, the value is held once under the final maximum. Equal parts count
/// one by one: two standing where one stood before is one arrival.
pub(crate) fn stepped(
    pool: PoolId,
    mut current: FixedU64,
    base: FixedU64,
    floor: FixedU64,
    before: &[PoolShiftTerms],
    now: &[PoolShiftTerms],
) -> FixedU64 {
    let maximum_of = |parts: &[PoolShiftTerms]| {
        stats::fold(
            base,
            parts.iter().flat_map(|part| part.terms.iter().copied()),
        )
        .max(floor)
    };
    // What left is what stood before and is not matched one for one by what
    // stands now; what arrived is the rest of now.
    let mut left: Vec<PoolShiftTerms> = before.to_vec();
    let mut arrived: Vec<PoolShiftTerms> = Vec::new();
    for part in now {
        match left.iter().position(|kept| kept == part) {
            Some(index) => {
                left.remove(index);
            }
            None => arrived.push(part.clone()),
        }
    }
    let mut standing: Vec<PoolShiftTerms> = before.to_vec();
    let mut clamped = false;
    for pool_shift in STEP_ORDER.into_iter().rev() {
        let leaving: Vec<&PoolShiftTerms> = left
            .iter()
            .filter(|part| part.pool_shift == pool_shift)
            .collect();
        if leaving.is_empty() {
            continue;
        }
        let old = maximum_of(&standing);
        for part in leaving {
            let index = standing
                .iter()
                .position(|kept| kept == part)
                .expect("a part that left stood before");
            standing.remove(index);
        }
        match pool_shift {
            PoolShift::Share | PoolShift::Difference => {
                current = shifted(pool, pool_shift, current, old, maximum_of(&standing));
            }
            PoolShift::Clamp => clamped = true,
        }
    }
    for pool_shift in STEP_ORDER {
        let coming: Vec<&PoolShiftTerms> = arrived
            .iter()
            .filter(|part| part.pool_shift == pool_shift)
            .collect();
        if coming.is_empty() {
            continue;
        }
        let old = maximum_of(&standing);
        standing.extend(coming.into_iter().cloned());
        match pool_shift {
            PoolShift::Share | PoolShift::Difference => {
                current = shifted(pool, pool_shift, current, old, maximum_of(&standing));
            }
            PoolShift::Clamp => clamped = true,
        }
    }
    if clamped {
        let maximum = maximum_of(&standing);
        current = shifted(pool, PoolShift::Clamp, current, maximum, maximum);
    }
    current
}

/// `value`, held under the parts `before`, stepped to the parts `now` as the
/// fold steps a pool it holds, with the maximum `now` gives — both folded
/// from `base` and held at `floor`.
pub(crate) fn replayed(
    pool: PoolId,
    value: FixedU64,
    base: FixedU64,
    floor: FixedU64,
    before: &[PoolShiftTerms],
    now: &[PoolShiftTerms],
) -> (FixedU64, FixedU64) {
    let maximum =
        stats::fold(base, now.iter().flat_map(|part| part.terms.iter().copied())).max(floor);
    (stepped(pool, value, base, floor, before, now), maximum)
}

/// `current` under a maximum moved from `old` to `new` by `pool_shift`, held under
/// `new`. A live health pool is never emptied by it.
pub(crate) fn shifted(
    pool: PoolId,
    pool_shift: PoolShift,
    current: FixedU64,
    old: FixedU64,
    new: FixedU64,
) -> FixedU64 {
    let moved = match pool_shift {
        PoolShift::Share => {
            if old == FixedU64::ZERO {
                FixedU64::ZERO
            } else {
                current.saturating_mul(new) / old
            }
        }
        PoolShift::Difference => {
            if new >= old {
                current.saturating_add(new - old)
            } else {
                current.saturating_sub(old - new)
            }
        }
        PoolShift::Clamp => current,
    }
    .min(new);
    match pool {
        PoolId::HEALTH if current > FixedU64::ZERO && moved == FixedU64::ZERO => FixedU64::DELTA,
        _ => moved,
    }
}

/// The parts of `entity_modifiers` that move `maximum_stat`.
pub(crate) fn parts_of<'a>(
    entity_modifiers: impl IntoIterator<Item = &'a EntityModifiers>,
    maximum_stat: EntityStatId,
) -> Vec<PoolShiftTerms> {
    entity_modifiers
        .into_iter()
        .filter_map(|modifiers| match modifiers {
            EntityModifiers::Stats(modifiers) => {
                assert!(
                    modifiers
                        .iter()
                        .all(|modifier| modifier.stat != maximum_stat),
                    "a set of stats laid over an entity names no pool maximum"
                );
                None
            }
            EntityModifiers::PoolMaximums {
                modifiers,
                pool_shift,
            } => {
                let terms: Vec<(ModifierOp, FixedI64)> = modifiers
                    .iter()
                    .filter(|modifier| modifier.stat == maximum_stat)
                    .map(|modifier| (modifier.op, modifier.magnitude))
                    .collect();
                (!terms.is_empty()).then_some(PoolShiftTerms {
                    pool_shift: *pool_shift,
                    terms,
                })
            }
        })
        .collect()
}

/// The parts the buffs `passives` lay on `maximum_stat`, one stack each.
pub(crate) fn passive_terms(
    registry: &ContentRegistry,
    passives: impl IntoIterator<Item = EntityBuffId>,
    maximum_stat: EntityStatId,
) -> Vec<PoolShiftTerms> {
    let entity_modifiers: Vec<EntityModifiers> = passives
        .into_iter()
        .flat_map(|id| registry.entity_buff_def(id).effects.iter())
        .filter_map(|effect| match effect {
            EntityEffect::Modifiers(modifiers) => Some(modifiers.clone()),
            EntityEffect::Disable | EntityEffect::Conceal => None,
        })
        .collect();
    parts_of(&entity_modifiers, maximum_stat)
}

/// The parts of `candidates` found in `parts`, matched one for one.
pub(crate) fn common(
    candidates: &[PoolShiftTerms],
    parts: &[PoolShiftTerms],
) -> Vec<PoolShiftTerms> {
    let mut unmatched: Vec<&PoolShiftTerms> = parts.iter().collect();
    candidates
        .iter()
        .filter(
            |candidate| match unmatched.iter().position(|part| part == candidate) {
                Some(index) => {
                    unmatched.remove(index);
                    true
                }
                None => false,
            },
        )
        .cloned()
        .collect()
}

/// `parts` with each of `leaving` taken out one for one where it stands, and
/// `arriving` added.
pub(crate) fn swapped(
    parts: &[PoolShiftTerms],
    leaving: &[PoolShiftTerms],
    arriving: &[PoolShiftTerms],
) -> Vec<PoolShiftTerms> {
    let mut swapped = parts.to_vec();
    for part in leaving {
        let index = swapped
            .iter()
            .position(|kept| kept == part)
            .expect("swapped is given leaving parts that stand in the parts");
        swapped.remove(index);
    }
    swapped.extend(arriving.iter().cloned());
    swapped
}

/// The shifts in the order one fold steps what arrived; what left steps in
/// the reverse order, so parts that come together and go together give the
/// value back.
const STEP_ORDER: [PoolShift; 3] = [PoolShift::Share, PoolShift::Difference, PoolShift::Clamp];
