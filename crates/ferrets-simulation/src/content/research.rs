//! The research vocabulary: handles, the timed technologies content declares,
//! and the catalogue of what an entity type can research.

use serde::{Deserialize, Serialize};

use super::player_buffs::PlayerBuffId;
use crate::resources::Cost;

/// A handle to a registered research, assigned in registration order.
///
/// Content declares researches by name and the registry mints their ids, so
/// identical content registered in the same order resolves to identical ids on
/// every peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResearchId(u16);

impl ResearchId {
    /// Creates a research id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more researches registered than ResearchId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A content-defined research: a timed, paid acquisition a player completes
/// once, unlocking whatever names it in a requirement list and applying its
/// buff, when it carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchDef {
    /// Price to research, paid when the research is commanded. Empty means free.
    pub cost: Cost,
    /// Ticks a researcher works to complete the research.
    pub research_time: u32,
    /// The player buff applied to the researching player on completion. `None`
    /// means the research is purely an unlock.
    pub buff: Option<PlayerBuffId>,
    /// Requirements for starting the research — each entry names an entity
    /// type, a tag, or another research (see
    /// [`requirements::met`](crate::requirements::met)).
    pub requires: Vec<String>,
}

impl ResearchDef {
    /// Creates a new `ResearchDef` with the given data.
    ///
    /// Panics if `research_time` is `0` or a requirement entry is empty.
    pub fn new(
        cost: Cost,
        research_time: u32,
        buff: Option<PlayerBuffId>,
        requires: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        assert!(research_time > 0, "research_time must be greater than 0");
        let requires: Vec<String> = requires.into_iter().map(Into::into).collect();
        assert!(
            requires.iter().all(|name| !name.is_empty()),
            "requirement names must not be empty"
        );

        Self {
            cost,
            research_time,
            buff,
            requires,
        }
    }
}

/// Content-defined research catalogue: which researches this entity can host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearcherDef {
    researches: Vec<ResearchId>,
}

impl ResearcherDef {
    /// Creates a new `ResearcherDef` with the given data.
    ///
    /// Panics if `researches` is empty.
    pub fn new(researches: impl IntoIterator<Item = ResearchId>) -> Self {
        let researches: Vec<ResearchId> = researches.into_iter().collect();
        assert!(!researches.is_empty(), "researches must not be empty");

        Self { researches }
    }

    /// Returns `true` if the research can be hosted here.
    pub fn can_research(&self, research: ResearchId) -> bool {
        self.researches.contains(&research)
    }

    /// Returns the researches that can be hosted.
    pub fn researches(&self) -> impl Iterator<Item = ResearchId> + '_ {
        self.researches.iter().copied()
    }
}
