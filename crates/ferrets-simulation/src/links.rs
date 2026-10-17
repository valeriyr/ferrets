//! Links: the running sets of gateways that link with one another, and the
//! passengers inside each.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::{entity::Entity, resource::Resource, world::World};
use ferrets_content::{
    entity_type_def::EntityTypeDef,
    gateway::Joins,
    network::NetworkId,
    registry::ContentRegistry,
    transport::PassengerFate,
    tunnel::{Passage, TunnelId},
};

use crate::{
    bore_id::BoreId,
    components::tunnel::TunnelEndComponent,
    entity_def,
    entity_index::EntityIndex,
    networks::{self, NetworkScope},
    session::{GameSession, player_id::PlayerId},
    simulation_id::SimulationId,
};

/// Which link a body is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkKey {
    /// One running network: the network and the scope its gateways link in.
    Network {
        /// The network.
        network: NetworkId,
        /// Whose gateways link.
        scope: NetworkScope,
    },
    /// One held tunnel: the tunnel and the bore its ends open.
    Tunnel {
        /// The tunnel.
        tunnel: TunnelId,
        /// The bore its ends open.
        bore: BoreId,
    },
}

/// Which running link a gateway is one of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    /// It is one of this link's gateways.
    Joined(LinkKey),
    /// It is one of no link: an ownerless gateway of a network whose
    /// `boarding` scopes its links by owner.
    Apart,
}

/// Every passenger inside a link, by the link it is inside.
#[derive(Resource, Debug, Default)]
pub struct LinkPassengers {
    /// The passengers inside each link anyone is inside.
    passengers: BTreeMap<LinkKey, BTreeSet<SimulationId>>,
}

impl LinkPassengers {
    /// The passengers in `key`, in id order.
    pub fn passengers(&self, key: LinkKey) -> impl Iterator<Item = SimulationId> + '_ {
        self.passengers.get(&key).into_iter().flatten().copied()
    }

    /// Puts `passenger` in `key`.
    ///
    /// Panics if it is there already.
    pub fn insert(&mut self, key: LinkKey, passenger: SimulationId) {
        assert!(
            self.passengers.entry(key).or_default().insert(passenger),
            "{passenger:?} is in {key:?} already"
        );
    }

    /// Takes `passenger` out of `key`.
    ///
    /// Panics if it is not there.
    pub fn remove(&mut self, key: LinkKey, passenger: SimulationId) {
        let passengers = self
            .passengers
            .get_mut(&key)
            .unwrap_or_else(|| panic!("{key:?} holds nobody"));
        assert!(
            passengers.remove(&passenger),
            "{passenger:?} is not in {key:?}"
        );
        if passengers.is_empty() {
            self.passengers.remove(&key);
        }
    }

    /// Takes every passenger out of `key`.
    pub fn take(&mut self, key: LinkKey) -> BTreeSet<SimulationId> {
        self.passengers.remove(&key).unwrap_or_default()
    }
}

/// The link `gateway` is one of, whether or not the link holds.
///
/// Panics if `gateway` is no gateway.
pub fn membership_of(world: &World, gateway: Entity) -> Membership {
    membership_from(
        world.resource::<ContentRegistry>(),
        world.resource::<GameSession>(),
        entity_def::of(world, gateway),
        entity_def::owner(world, gateway),
        world.entity(gateway).get::<TunnelEndComponent>(),
    )
}

/// The link a gateway of type `def`, owned by `owner`, with the tunnel end
/// `end` when it is one, is one of.
///
/// Panics if `def` is no gateway, or names a tunnel and `end` is `None`.
pub fn membership_from(
    registry: &ContentRegistry,
    session: &GameSession,
    def: &EntityTypeDef,
    owner: Option<PlayerId>,
    end: Option<&TunnelEndComponent>,
) -> Membership {
    let joins = def
        .gateway()
        .expect("only a gateway is one of a link")
        .joins();
    match joins {
        Joins::Network(network) => match networks::scope_of(registry, session, network, owner) {
            Some(scope) => Membership::Joined(LinkKey::Network { network, scope }),
            None => Membership::Apart,
        },
        Joins::Tunnel(tunnel) => Membership::Joined(LinkKey::Tunnel {
            tunnel,
            bore: end.expect("a tunnel end keeps its binding").bore,
        }),
    }
}

/// The passengers inside the link `gateway` is one of, in id order; none for
/// a gateway of no link.
///
/// Panics if `gateway` is no gateway.
pub fn passengers_of(world: &World, gateway: Entity) -> impl Iterator<Item = SimulationId> + '_ {
    let key = match membership_of(world, gateway) {
        Membership::Joined(key) => Some(key),
        Membership::Apart => None,
    };
    key.into_iter()
        .flat_map(|key| world.resource::<LinkPassengers>().passengers(key))
}

/// Whether a gateway other than `lost` keeps the link `key`: one alive and
/// not dying whose link it is, standing, a site or carried.
pub fn kept_by_others(world: &World, key: LinkKey, lost: Entity) -> bool {
    world
        .resource::<EntityIndex>()
        .alive_entries()
        .into_iter()
        .any(|(_, entity)| {
            entity != lost
                && entity_def::of(world, entity).gateway().is_some()
                && !entity_def::dying(world, entity)
                && membership_of(world, entity) == Membership::Joined(key)
        })
}

/// What happens to the passengers held when the link `gateway` is one of loses
/// its last gateway, or `None` for a link that holds nobody.
pub fn fate_of(world: &World, gateway: Entity) -> Option<PassengerFate> {
    let registry = world.resource::<ContentRegistry>();
    let joins = entity_def::of(world, gateway)
        .gateway()
        .expect("only a gateway is one of a link")
        .joins();
    match joins {
        Joins::Network(network) => Some(registry.network_def(network).fate()),
        Joins::Tunnel(tunnel) => match registry.tunnel_def(tunnel).passage() {
            Passage::Held { fate, .. } => Some(fate),
            Passage::Crosses => None,
        },
    }
}
