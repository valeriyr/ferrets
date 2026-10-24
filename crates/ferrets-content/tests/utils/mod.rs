#![allow(dead_code)]

use ferrets_content::{
    attack::{AttackDef, Delivery, Slain, Weapon},
    domain::{DomainDef, DomainId},
    dying::{Bequest, LeftBy},
    entity_type_def::EntityTypeDef,
    location::Solidity,
    pool_def::PoolId,
    registry::ContentRegistry,
    rise::RisePool,
    terrains::Terrains,
};
use ferrets_geometry::cell_size::CellSize;
use ferrets_math::FixedU64;
use ferrets_nav_grid::{layer_id::LayerId, layer_level::LayerLevel, layer_mask::LayerMask};

/// The navigation layers these tests register, in the order a registry mints
/// them: the first layer declared is `1`, and each next one takes the next bit.
pub const GROUND: LayerId = LayerId::new(1);
pub const WATER: LayerId = LayerId::new(2);
pub const AIR: LayerId = LayerId::new(4);

/// The domain [`ground_registry`] declares: the ground layer over every
/// terrain. The first domain registered.
pub const GROUND_DOMAIN: DomainId = DomainId::new(0);
/// A domain in [`WATER`] and one in [`AIR`], as a registry mints the second
/// and third domains declared.
pub const WATER_DOMAIN: DomainId = DomainId::new(1);
pub const AIR_DOMAIN: DomainId = DomainId::new(2);

/// A fresh registry that already knows the "ground" navigation layer, the
/// "land" terrain and the [`GROUND_DOMAIN`] over it.
pub fn ground_registry() -> ContentRegistry {
    let mut registry = ContentRegistry::default();
    let ground = registry.register_layer("ground", LayerLevel::At(1));
    registry.register_terrain("land");
    assert_eq!(
        registry.register_domain("ground", DomainDef::new(ground, Terrains::Any, None)),
        GROUND_DOMAIN
    );
    registry
}

/// A one-cell solid entity in `domain`, occupying its layer, with nothing else
/// declared.
pub fn standing(name: &str, domain: DomainId) -> EntityTypeDef {
    sized(name, domain, CellSize::ONE)
}

/// One bequest of `entity_type`, handed on by any death that ends a life.
pub fn leaves(entity_type: &str) -> Vec<Bequest> {
    vec![Bequest::new(entity_type, 1, LeftBy::Ordinary)]
}

/// A solid entity in `domain`, occupying its layer, whose footprint spans
/// `size`.
pub fn sized(name: &str, domain: DomainId, size: CellSize) -> EntityTypeDef {
    EntityTypeDef::new(name).with_location(domain, size, Solidity::Solid, domain_layer(domain))
}

/// The layer of a domain these tests declare.
///
/// Panics for any other domain.
fn domain_layer(domain: DomainId) -> LayerId {
    match domain {
        GROUND_DOMAIN => GROUND,
        WATER_DOMAIN => WATER,
        AIR_DOMAIN => AIR,
        other => panic!("the helpers declare no domain {other:?}"),
    }
}

/// A weapon reaching `targets` that lands its hit where it stands, aimed from the
/// body — the plainest one there is, for tests about anything but the weapon.
pub fn weapon(targets: impl Into<LayerMask>) -> AttackDef {
    AttackDef::new(Weapon::new(
        targets,
        Delivery::Instant,
        None,
        Slain::Remains,
    ))
}

/// A fixed-point value parsed from decimal digits.
pub fn fixed(text: &str) -> FixedU64 {
    text.parse()
        .unwrap_or_else(|_| panic!("'{text}' is a value"))
}

/// Each of `pools` held at its own initial while rising.
pub fn rise_initial_pools(pools: &[PoolId]) -> Vec<(PoolId, RisePool)> {
    pools
        .iter()
        .map(|pool| (*pool, RisePool::Initial))
        .collect()
}
