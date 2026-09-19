//! The docks a building offers annexes: what stands in each, what an annex
//! does with no primary, and who may take one.
//!
//! Nothing here is ordered. The bond is re-derived from what stands where every
//! tick, so a primary that is built, lands, lifts off or dies changes it by
//! standing or ceasing to.

use bevy_ecs::{prelude::*, world::World};
use ferrets_content::{
    annex::{AloneConduct, AnnexClaim, AnnexDef, AnnexLife, AnnexWork, DockDef},
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    stats::{EntityModifier, ModifierOp},
};
use ferrets_geometry::cell_pos::CellPos;
use ferrets_math::FixedI64;

use crate::{
    components::{
        annex::{AnnexComponent, Docking, DocksComponent},
        build::UnderConstructionComponent,
        dying::DyingComponent,
        entity_info::EntityInfoComponent,
        location::LocationComponent,
    },
    entity_def,
    entity_index::EntityIndex,
    events::DeathCause,
    session::{GameSession, player_id::PlayerId},
    simulation_id::SimulationId,
    spawn,
};

/// The primary `annex` stands with.
///
/// [`Docking::Alone`] for an entity that is no annex at all, which is what an
/// annex with no primary amounts to.
pub(crate) fn primary_of(world: &World, annex: Entity) -> Docking {
    world
        .entity(annex)
        .get::<AnnexComponent>()
        .map(|annex| annex.docked_to)
        .unwrap_or(Docking::Alone)
}

/// Whether an annex of `type_name` stands docked with `primary` — standing
/// meaning alive and no longer under construction, as a requirement reads it.
pub fn has_standing_annex(world: &World, primary: Entity, type_name: &str) -> bool {
    let Some(docked) = world.entity(primary).get::<DocksComponent>() else {
        return false;
    };
    let index = world.resource::<EntityIndex>();
    docked.annexes.values().any(|&id| {
        index.alive(id).is_some_and(|annex| {
            entity_def::of(world, annex).name == type_name
                && !world.entity(annex).contains::<UnderConstructionComponent>()
        })
    })
}

/// Whether `entity` is an annex standing with no primary, on terms that stop
/// it working.
pub fn idles_alone(world: &World, entity: Entity) -> bool {
    match primary_of(world, entity) {
        Docking::Primary(_) => false,
        Docking::Alone => match entity_def::annex_def(world, entity).map(|annex| annex.alone()) {
            Some(AloneConduct::Standing {
                work: AnnexWork::Idles,
                ..
            }) => true,
            Some(
                AloneConduct::Standing {
                    work: AnnexWork::Works,
                    ..
                }
                | AloneConduct::Razed,
            )
            | None => false,
        },
    }
}

/// Where the annex of `dock` stands, in map cells.
fn dock_anchor(world: &World, primary: Entity, dock: &DockDef) -> CellPos {
    let origin = entity_def::footprint_rect(world, primary).origin;
    CellPos::new(origin.x + dock.at().x, origin.y + dock.at().y)
}

/// The anchor of the first dock of `primary` that takes `type_name`, or `None`
/// when it offers no such dock.
pub fn dock_anchor_for(world: &World, primary: Entity, annex: &EntityTypeDef) -> Option<CellPos> {
    entity_def::of(world, primary)
        .docks
        .iter()
        .find(|dock| dock.accepts().admits(annex))
        .map(|dock| dock_anchor(world, primary, dock))
}

/// Whether a site of `def` raised by `builder` may stand at `anchor`: an annex
/// stands in a dock of its builder that takes it, and nowhere else. Anything
/// that is no annex stands where it is put.
pub fn allows_placement(
    world: &World,
    builder: Entity,
    def: &EntityTypeDef,
    anchor: CellPos,
) -> bool {
    def.annex.is_none() || has_dock_at(world, builder, def, anchor)
}

/// Whether `primary` offers a dock that takes `annex` with it standing at
/// `anchor`.
fn has_dock_at(world: &World, primary: Entity, annex: &EntityTypeDef, anchor: CellPos) -> bool {
    entity_def::of(world, primary)
        .docks
        .iter()
        .any(|dock| dock.accepts().admits(annex) && dock_anchor(world, primary, dock) == anchor)
}

/// One dock on offer this tick.
struct Offer {
    /// The primary offering it.
    primary: Entity,
    /// The offering primary's id, which orders the offers and records the bond.
    primary_id: SimulationId,
    /// Which of the primary type's declared docks this is.
    dock_index: usize,
    /// The map cell an annex stands on to hold the dock.
    anchor: CellPos,
}

/// Re-derives every annex's primary, applies what an annex with none does, and
/// hands over the annexes a claim gives away.
pub fn advance(world: &mut World) {
    let offers = docks_on_offer(world);
    let annexes = standing_annexes(world);

    // Every bond is derived from one snapshot of the offers before any of them
    // is applied: settling hands annexes over, which moves the owners a later
    // claim is judged against.
    let mut docked: Vec<(Entity, Docking)> = Vec::new();
    for &annex in &annexes {
        let found = primary_among(world, annex, &offers);
        docked.push((annex, found));
    }
    for &(annex, found) in &docked {
        settle(world, annex, found);
    }
    rebuild_docks(world, &docked, &offers);
    tend_sites(world, &offers);
    for &(annex, found) in &docked {
        match found {
            Docking::Alone => stand_alone(world, annex),
            Docking::Primary(_) => {}
        }
    }
}

/// Sets a half-raised annex working while a primary offers the dock it stands
/// on, and answers for one whose dock has gone by its type's `alone` terms.
///
/// An annex is raised by the building it belongs to, so the work follows that
/// building rather than an order: a primary that lifts off mid-build leaves the
/// site standing, and one that lands back over it takes the work up again
/// without being told to. What the order does is found the site; what the dock
/// does is work it. A type that cannot stand without a primary loses its site
/// the way it would lose the finished building.
fn tend_sites(world: &mut World, offers: &[Offer]) {
    let mut sites: Vec<(SimulationId, Entity)> = world
        .query_filtered::<(Entity, &EntityInfoComponent), (
            With<AnnexComponent>,
            With<UnderConstructionComponent>,
            Without<DyingComponent>,
        )>()
        .iter(world)
        .map(|(site, info)| (info.id(), site))
        .collect();
    sites.sort_unstable_by_key(|&(id, _)| id);

    for (_, site) in sites {
        let anchor = entity_def::footprint_rect(world, site).origin;
        let annex = entity_def::of(world, site);
        let owner = entity_def::owner(world, site);
        // Only the owner's own primary works it: a rival's landing beside a
        // half-raised annex does not finish it for them.
        let holders: Vec<(SimulationId, Entity)> = offers
            .iter()
            .filter(|offer| offer.anchor == anchor)
            .filter(|offer| {
                entity_def::of(world, offer.primary).docks[offer.dock_index]
                    .accepts()
                    .admits(annex)
            })
            .filter(|offer| entity_def::owner(world, offer.primary) == owner)
            .map(|offer| (offer.primary_id, offer.primary))
            .collect();
        // A primary whose form is changing still stands on the ground its
        // dock covers, so the dock has not gone — but it works nothing, as a
        // job changing form takes no newcomer.
        let tender = holders
            .iter()
            .find(|&&(_, primary)| !entity_def::changing(world, primary))
            .map(|&(primary_id, _)| primary_id);

        match (holders.is_empty(), terms(world, site).alone()) {
            // Nothing offers the dock it stands on, and its type cannot stand
            // without a primary: the site answers to the same terms its
            // finished form would, and goes the same way, with nothing
            // refunded — the conduct belongs to the type, not to the stage
            // the building happens to have reached.
            (true, AloneConduct::Razed) => {
                spawn::despawn_entity(world, site, DeathCause::Decayed);
            }
            (true, AloneConduct::Standing { .. }) | (false, _) => {
                world
                    .entity_mut(site)
                    .get_mut::<UnderConstructionComponent>()
                    .expect("the sites were gathered by the marker they carry")
                    .tend(tender);
            }
        }
    }
}

/// Every dock a standing primary offers this tick.
fn docks_on_offer(world: &mut World) -> Vec<Offer> {
    let mut primaries: Vec<(Entity, SimulationId)> = world
        .query_filtered::<(Entity, &EntityInfoComponent), (
            With<DocksComponent>,
            With<LocationComponent>,
            Without<DyingComponent>,
            Without<UnderConstructionComponent>,
        )>()
        .iter(world)
        .map(|(entity, info)| (entity, info.id()))
        .collect();
    // Ordered by id, so no reader of the offers can rest on query order —
    // not the pick of a primary, nor the dock an annex is recorded in.
    primaries.sort_unstable_by_key(|&(_, id)| id);

    let mut offers = Vec::new();
    for (primary, primary_id) in primaries {
        if !entity_def::stands_on_grid(world, primary) {
            continue;
        }
        for (dock_index, dock) in entity_def::of(world, primary).docks.iter().enumerate() {
            offers.push(Offer {
                primary,
                primary_id,
                dock_index,
                anchor: dock_anchor(world, primary, dock),
            });
        }
    }
    offers
}

/// Every standing annex this tick, in id order.
fn standing_annexes(world: &mut World) -> Vec<Entity> {
    let mut annexes: Vec<(SimulationId, Entity)> = world
        .query_filtered::<(Entity, &EntityInfoComponent), (
            With<AnnexComponent>,
            With<LocationComponent>,
            Without<DyingComponent>,
            Without<UnderConstructionComponent>,
        )>()
        .iter(world)
        .map(|(entity, info)| (info.id(), entity))
        .collect();
    // Handed-over annexes are settled in id order, so the outcome of two
    // claims landing in one tick never rests on query order.
    annexes.sort_unstable_by_key(|&(id, _)| id);
    annexes
        .into_iter()
        .filter(|&(_, annex)| entity_def::stands_on_grid(world, annex))
        .map(|(_, annex)| annex)
        .collect()
}

/// The primary whose dock `annex` stands in, among the offers its claim
/// admits. Ties go to the lowest id.
fn primary_among(world: &World, annex: Entity, offers: &[Offer]) -> Docking {
    let anchor = entity_def::footprint_rect(world, annex).origin;
    let annex_def = entity_def::of(world, annex);
    let claim = terms(world, annex).claim();
    let owner = entity_def::owner(world, annex);

    offers
        .iter()
        .filter(|offer| offer.anchor == anchor)
        .filter(|offer| {
            entity_def::of(world, offer.primary).docks[offer.dock_index]
                .accepts()
                .admits(annex_def)
        })
        .filter(|offer| claim_admits(world, claim, owner, entity_def::owner(world, offer.primary)))
        .min_by_key(|offer| offer.primary_id)
        .map(|offer| Docking::Primary(offer.primary_id))
        .unwrap_or(Docking::Alone)
}

/// Whether an annex owned by `owner` on `claim` terms docks with a primary
/// owned by `primary`.
fn claim_admits(
    world: &World,
    claim: AnnexClaim,
    owner: Option<PlayerId>,
    primary: Option<PlayerId>,
) -> bool {
    match claim {
        AnnexClaim::Bound => owner.is_some() && owner == primary,
        AnnexClaim::Allied => match (owner, primary) {
            (Some(owner), Some(primary)) => {
                world.resource::<GameSession>().are_allied(owner, primary)
            }
            _ => false,
        },
        AnnexClaim::Seized => primary.is_some(),
    }
}

/// Records the primary `annex` now stands with, and hands the annex over when
/// the primary it stands with is another player's and the claim allows it.
///
/// The claim is judged every tick a bond stands, not only on the tick it
/// forms: a primary that is itself a seized annex changes hands without its
/// own dock changing, and what stands in that dock follows it.
fn settle(world: &mut World, annex: Entity, found: Docking) {
    if primary_of(world, annex) != found {
        world
            .entity_mut(annex)
            .get_mut::<AnnexComponent>()
            .expect("a standing annex carries the component it was settled from")
            .docked_to = found;
    }
    let Docking::Primary(primary_id) = found else {
        return;
    };
    let primary = world
        .resource::<EntityIndex>()
        .alive(primary_id)
        .expect("a bond was settled from an offer, and only a living primary offers a dock");
    let claim = terms(world, annex).claim();
    match claim {
        AnnexClaim::Bound | AnnexClaim::Allied => {}
        AnnexClaim::Seized => {
            let taker = entity_def::owner(world, primary)
                .expect("a seized annex docks only with a primary that has an owner");
            if entity_def::owner(world, annex) != Some(taker) {
                spawn::change_owner(world, annex, taker, primary_id);
            }
        }
    }
}

/// Applies what standing with no primary does to `annex` this tick.
///
/// A fading annex is left to the stat pipeline: its drain is a modifier like a
/// field's (see [`modifiers`]), and the health flow answers for the death.
fn stand_alone(world: &mut World, annex: Entity) {
    let alone = terms(world, annex).alone();
    match alone {
        AloneConduct::Razed => spawn::despawn_entity(world, annex, DeathCause::Decayed),
        AloneConduct::Standing { .. } => {}
    }
}

/// The modifiers `entity` carries for standing with no primary: a fading annex
/// drains health for as long as it stands alone, the way a structure outside
/// the field that sustains it does.
pub fn modifiers(world: &World, entity: Entity) -> Vec<EntityModifier> {
    let Some(annex) = entity_def::annex_def(world, entity) else {
        return Vec::new();
    };
    match (primary_of(world, entity), annex.alone()) {
        (
            Docking::Alone,
            AloneConduct::Standing {
                life: AnnexLife::Fades { per_tick },
                ..
            },
        ) => vec![EntityModifier {
            stat: EntityStatId::HEALTH_DRAIN,
            op: ModifierOp::FlatAdd,
            magnitude: FixedI64::from_num(per_tick),
        }],
        (Docking::Alone, AloneConduct::Standing { .. } | AloneConduct::Razed)
        | (Docking::Primary(_), _) => Vec::new(),
    }
}

/// Rewrites what stands in each primary's docks from the bonds just settled.
fn rebuild_docks(world: &mut World, docked: &[(Entity, Docking)], offers: &[Offer]) {
    // Emptied and refilled in place: the component travels with the form, so
    // rewriting what stands in the docks costs nothing but the map.
    let holders: Vec<Entity> = world
        .query_filtered::<Entity, With<DocksComponent>>()
        .iter(world)
        .collect();
    for holder in holders {
        world
            .entity_mut(holder)
            .get_mut::<DocksComponent>()
            .expect("the holders were gathered by the component they carry")
            .annexes
            .clear();
    }

    for &(annex, found) in docked {
        let Docking::Primary(primary_id) = found else {
            continue;
        };
        let offer = offers
            .iter()
            .find(|offer| {
                offer.primary_id == primary_id
                    && offer.anchor == entity_def::footprint_rect(world, annex).origin
            })
            .expect("a bond was settled from an offer, and nothing has moved since");
        let annex_id = entity_def::simulation_id(world, annex);
        world
            .entity_mut(offer.primary)
            .get_mut::<DocksComponent>()
            .expect("a primary that offered a dock carries the component")
            .annexes
            .insert(offer.dock_index, annex_id);
    }
}

/// The annex terms of a standing annex.
fn terms(world: &World, annex: Entity) -> AnnexDef {
    entity_def::annex_def(world, annex)
        .expect("an annex component implies the annex properties it was fitted from")
}
