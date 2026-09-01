//! Whose buildings keep a player in a last-standing game.

use serde::{Deserialize, Serialize};

/// Whose buildings keep a player in a last-standing game.
///
/// Either way, units an eliminated player still owns are not taken from them:
/// standing orders keep running and stances keep engaging, but no one can
/// command them any more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EliminationScope {
    /// A player is out of the game once they hold no standing building of
    /// their own, even while their allies fight on.
    Player,
    /// A player is out of the game only once their whole side holds no
    /// standing building: a player who lost every building of their own
    /// keeps playing their remaining units while an ally still holds one,
    /// and the side is eliminated as one when its last building falls.
    Side,
}
