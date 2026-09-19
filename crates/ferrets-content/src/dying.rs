//! Content-defined dying-phase property structs: how long the phase lasts, and
//! what the entity leaves standing where it fell.

use std::collections::BTreeSet;

/// A kind of death, as content names it.
///
/// One arm per cause the simulation can announce, carrying none of what a cause
/// carries with it, so a type can name the deaths it leaves something behind
/// for without naming what took it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeathKind {
    /// Brought down by damage.
    Killed,
    /// A timed life that ran out.
    Expired,
    /// Nothing sustained it any longer.
    Decayed,
    /// A broodling whose breeder died.
    Orphaned,
    /// A broodling left with no berth to sit in.
    Unseated,
    /// Went down inside the carrier holding it, which is off the map: a death
    /// there leaves nothing, whatever names it.
    CarriedDown,
    /// Spent on the construction site it founded.
    Consumed,
    /// Called off before it was finished.
    Cancelled,
    /// A resource source that ran out.
    Depleted,
    /// A resource source taken off the map by what was raised over it.
    Overbuilt,
}

/// Which deaths hand a bequest on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeftBy {
    /// The engine's own rule: something is left when a death ends the
    /// entity's life on the map, and not when the game takes it off the board
    /// or when it goes down somewhere it had already been taken off.
    Ordinary,
    /// Exactly the deaths content named, and no other.
    Named(Vec<DeathKind>),
}

impl LeftBy {
    /// Whether a death of the given kind hands the bequest on.
    pub fn left_by(&self, kind: DeathKind) -> bool {
        match self {
            LeftBy::Ordinary => match kind {
                DeathKind::Killed
                | DeathKind::Expired
                | DeathKind::Decayed
                | DeathKind::Orphaned
                | DeathKind::Unseated => true,
                DeathKind::CarriedDown
                | DeathKind::Consumed
                | DeathKind::Cancelled
                | DeathKind::Depleted
                | DeathKind::Overbuilt => false,
            },
            LeftBy::Named(kinds) => kinds.contains(&kind),
        }
    }
}

/// One thing a death hands on: what is left standing, how many of it, and which
/// deaths leave it.
///
/// What is left decides for itself what it then is: a type wearing the
/// [`REMAINS`](crate::tags::REMAINS) tag lies there as a body does, and
/// anything else stands up as an ordinary entity of whoever owned the
/// deceased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bequest {
    /// The entity type left standing.
    entity_type: String,
    /// How many of it.
    count: u32,
    /// The deaths that leave it.
    causes: LeftBy,
}

impl Bequest {
    /// Creates a new `Bequest` with the given data.
    ///
    /// Panics if `entity_type` is empty, if `count` is zero, if a named list is
    /// empty — a bequest no death hands on is content that does nothing — or if
    /// it names a kind twice.
    pub fn new(entity_type: impl Into<String>, count: u32, causes: LeftBy) -> Self {
        let entity_type = entity_type.into();
        assert!(!entity_type.is_empty(), "entity_type must not be empty");
        assert!(count > 0, "a bequest of '{entity_type}' leaves none of it");
        if let LeftBy::Named(kinds) = &causes {
            assert!(
                !kinds.is_empty(),
                "a bequest of '{entity_type}' is left by no death at all"
            );
            let mut named: BTreeSet<DeathKind> = BTreeSet::new();
            for &kind in kinds {
                assert!(
                    named.insert(kind),
                    "a bequest of '{entity_type}' names {kind:?} twice"
                );
            }
        }

        Self {
            entity_type,
            count,
            causes,
        }
    }

    /// The entity type left standing.
    #[inline]
    pub fn entity_type(&self) -> &str {
        &self.entity_type
    }

    /// How many of it.
    #[inline]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The deaths that leave it.
    #[inline]
    pub fn causes(&self) -> &LeftBy {
        &self.causes
    }

    /// Whether a death of the given kind leaves it.
    #[inline]
    pub fn left_by(&self, kind: DeathKind) -> bool {
        self.causes.left_by(kind)
    }
}

/// Content-defined configuration of an entity's dying phase: how long it lasts
/// and what it hands on.
///
/// A destroyed entity waits out the ticks this states before it is removed, or
/// goes the tick it dies when it states none. Destruction is independent of health — depleted resource sources,
/// cancelled constructions, and scripted removals all destroy entities that may
/// never take damage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DyingDef {
    /// Ticks the entity waits between dying and leaving the world, or `None`
    /// for one that goes at once — what remains do, since a body has already
    /// spent its whole existence lying there.
    dying_time: Option<u32>,
    /// What the death hands on, in the order content named it. Empty means the
    /// entity leaves nothing behind whatever takes it.
    leaves: Vec<Bequest>,
}

impl DyingDef {
    /// Creates a new `DyingDef` with the given data.
    ///
    /// Panics if `dying_time` is `Some(0)` — a wait of no ticks is no wait,
    /// which is what `None` says — if neither a wait nor a bequest is stated,
    /// or if two bequests name the same type.
    pub fn new(dying_time: Option<u32>, leaves: impl IntoIterator<Item = Bequest>) -> Self {
        assert!(
            dying_time != Some(0),
            "a dying time of no ticks is no dying time at all"
        );
        let leaves: Vec<Bequest> = leaves.into_iter().collect();
        assert!(
            dying_time.is_some() || !leaves.is_empty(),
            "a death that waits for nothing and leaves nothing is no dying at all"
        );
        let mut named: BTreeSet<&str> = BTreeSet::new();
        for bequest in &leaves {
            assert!(
                named.insert(bequest.entity_type()),
                "a death leaves '{}' twice",
                bequest.entity_type()
            );
        }

        Self { dying_time, leaves }
    }

    /// Returns how long the entity waits before leaving the world, or `None`
    /// for one that goes at once.
    #[inline]
    pub fn dying_time(&self) -> Option<u32> {
        self.dying_time
    }

    /// Returns what the death hands on.
    #[inline]
    pub fn leaves(&self) -> &[Bequest] {
        &self.leaves
    }
}
