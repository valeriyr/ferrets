//! The domain vocabulary: the one layer a body is in and the terrains it may
//! be over there.

use ferrets_pathfinder::{layer_id::LayerId, terrain_mask::TerrainMask};

use crate::terrains::Terrains;

/// A handle to a registered domain, assigned in registration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DomainId(u16);

impl DomainId {
    /// Creates the id the domain registered `index`-th resolves to.
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    /// Creates a domain id for the given registration index.
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("more domains registered than DomainId can hold"))
    }

    /// The registration index this id refers to.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainDef {
    /// The layer its bodies are in.
    layer: LayerId,
    /// The terrains its bodies may be over.
    terrains: Terrains,
}

impl DomainDef {
    /// Creates a new `DomainDef` with the given data.
    ///
    /// Panics if the domain is over no terrain at all.
    pub fn new(layer: LayerId, terrains: Terrains) -> Self {
        match terrains {
            Terrains::Only(terrains) => assert!(
                terrains != TerrainMask::EMPTY,
                "a domain over no terrain holds nothing; declare it over \"any\""
            ),
            Terrains::Any => {}
        }
        Self { layer, terrains }
    }

    /// The layer its bodies are in.
    #[inline]
    pub fn layer(&self) -> LayerId {
        self.layer
    }

    /// The terrains its bodies may be over.
    #[inline]
    pub fn terrains(&self) -> Terrains {
        self.terrains
    }
}
