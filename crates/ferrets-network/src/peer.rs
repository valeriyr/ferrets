//! Peer identity.

/// An opaque transport-native peer handle — *not* a dense slot index (that is
/// [`PlayerId`](ferrets_simulation::session::player_slot::PlayerId)). Concrete
/// transports put their own identity here, and the
/// [`Roster`](crate::roster::Roster) maps it to a slot, so the values may be sparse.
///
/// `u64` so a transport can carry a 64-bit native id directly — notably a Steam
/// `CSteamID` — without keeping its own mapping table. (Wider/non-numeric ids,
/// e.g. a WebRTC UUID, are mapped down to a `u64` by that transport.)
pub type PeerId = u64;

/// The session host's peer id: the node that opens the lobby takes this handle
/// and assigns every joiner an ascending one, so it is the single fixed point
/// every node agrees on for which peer is the host. The one definition of that
/// fact — everything that asks "is this the host" resolves back to here.
pub const HOST_PEER: PeerId = 0;
