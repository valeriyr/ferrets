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
