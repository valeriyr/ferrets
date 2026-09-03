//! Per-player tallies of what a game has done so far.
//!
//! Only *historical* quantities live here — things that happened and cannot
//! un-happen. What a player owns right now, army value and population among it,
//! is answerable from the world itself instead.
//!
//! Everything is broken down by entity type and resource kind; a game that
//! wants a score reads these and weighs them however it likes.
//!
//! These are deterministic simulation state: reproduced exactly on replay, and
//! not folded into the checksum.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use ferrets_math::FixedU64;

use ferrets_content::entity_type_def::EntityTypeId;

use crate::session::player_slot::PlayerId;

/// What one player has done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerTally {
    /// Entities that finished, by what they were — trained, or completed on a
    /// construction site. Never merely placed at game start, a founded site not
    /// before its completion, and a change of form not at all.
    produced: BTreeMap<EntityTypeId, u32>,
    /// Entities of this player's brought down by fire — an enemy's, an ally's,
    /// or their own — by what they were.
    lost: BTreeMap<EntityTypeId, u32>,
    /// Enemy entities this player destroyed, by what the victim was.
    killed: BTreeMap<EntityTypeId, u32>,
    /// Resources banked by carriers, by kind — not every way a stockpile grows.
    gathered: BTreeMap<String, u32>,
    /// Resources charged for anything, by kind.
    spent: BTreeMap<String, u32>,
    /// Resources given back when an order fell through, by kind; [`Self::spent`]
    /// is not rewritten.
    refunded: BTreeMap<String, u32>,
    /// Health removed from enemies, before any overkill is discarded.
    damage_dealt: FixedU64,
    /// Health removed from this player's own entities.
    damage_taken: FixedU64,
    /// Research topics finished.
    research_completed: u32,
    /// Skills that went off.
    skills_cast: u32,
}

impl PlayerTally {
    /// How many of `entity_type` this player finished.
    pub fn produced(&self, entity_type: EntityTypeId) -> u32 {
        self.produced.get(&entity_type).copied().unwrap_or(0)
    }

    /// How many of `entity_type` this player lost to fire.
    pub fn lost(&self, entity_type: EntityTypeId) -> u32 {
        self.lost.get(&entity_type).copied().unwrap_or(0)
    }

    /// How many enemy `entity_type` this player destroyed.
    pub fn killed(&self, entity_type: EntityTypeId) -> u32 {
        self.killed.get(&entity_type).copied().unwrap_or(0)
    }

    /// How much of `kind` this player's carriers banked.
    pub fn gathered(&self, kind: &str) -> u32 {
        self.gathered.get(kind).copied().unwrap_or(0)
    }

    /// How much of `kind` this player was charged.
    pub fn spent(&self, kind: &str) -> u32 {
        self.spent.get(kind).copied().unwrap_or(0)
    }

    /// How much of `kind` this player was paid back for orders that fell through.
    pub fn refunded(&self, kind: &str) -> u32 {
        self.refunded.get(kind).copied().unwrap_or(0)
    }

    /// Health this player removed from enemies.
    pub fn damage_dealt(&self) -> FixedU64 {
        self.damage_dealt
    }

    /// Health enemies removed from this player.
    pub fn damage_taken(&self) -> FixedU64 {
        self.damage_taken
    }

    /// Research topics this player finished.
    pub fn research_completed(&self) -> u32 {
        self.research_completed
    }

    /// Skills this player's entities cast.
    pub fn skills_cast(&self) -> u32 {
        self.skills_cast
    }

    /// Every type this player finished, with its count, in handle order.
    pub fn produced_types(&self) -> impl Iterator<Item = (EntityTypeId, u32)> {
        self.produced.iter().map(|(&id, &count)| (id, count))
    }

    /// Every type this player lost, with its count, in handle order.
    pub fn lost_types(&self) -> impl Iterator<Item = (EntityTypeId, u32)> {
        self.lost.iter().map(|(&id, &count)| (id, count))
    }

    /// Every type this player destroyed, with its count, in handle order.
    pub fn killed_types(&self) -> impl Iterator<Item = (EntityTypeId, u32)> {
        self.killed.iter().map(|(&id, &count)| (id, count))
    }

    /// Every resource kind this player banked, with the total, in name order.
    pub fn gathered_kinds(&self) -> impl Iterator<Item = (&str, u32)> {
        self.gathered
            .iter()
            .map(|(kind, &amount)| (kind.as_str(), amount))
    }

    /// Every resource kind this player was charged, with the total, in name order.
    pub fn spent_kinds(&self) -> impl Iterator<Item = (&str, u32)> {
        self.spent
            .iter()
            .map(|(kind, &amount)| (kind.as_str(), amount))
    }

    /// Every resource kind this player was paid back, with the total, in name
    /// order.
    pub fn refunded_kinds(&self) -> impl Iterator<Item = (&str, u32)> {
        self.refunded
            .iter()
            .map(|(kind, &amount)| (kind.as_str(), amount))
    }
}

/// Tallies for all players in the session, indexed by [`PlayerId`].
#[derive(Resource, Debug)]
pub struct Statistics(Vec<PlayerTally>);

impl Statistics {
    /// Creates an empty tally for each player.
    pub fn new(player_count: usize) -> Self {
        Self(vec![PlayerTally::default(); player_count])
    }

    /// One player's tally.
    pub fn player(&self, player: PlayerId) -> &PlayerTally {
        &self.0[player as usize]
    }

    /// Every player's tally, in slot order.
    pub fn players(&self) -> impl Iterator<Item = (PlayerId, &PlayerTally)> {
        self.0
            .iter()
            .enumerate()
            .map(|(slot, tally)| (slot as PlayerId, tally))
    }

    /// Counts one finished entity of `entity_type` for `player`.
    pub(crate) fn record_produced(&mut self, player: PlayerId, entity_type: EntityTypeId) {
        bump(&mut self.0[player as usize].produced, entity_type);
    }

    /// Counts one of `player`'s entities of `entity_type` lost to fire.
    pub(crate) fn record_lost(&mut self, player: PlayerId, entity_type: EntityTypeId) {
        bump(&mut self.0[player as usize].lost, entity_type);
    }

    /// Credits `player` with destroying one enemy entity of `entity_type`.
    pub(crate) fn record_killed(&mut self, player: PlayerId, entity_type: EntityTypeId) {
        bump(&mut self.0[player as usize].killed, entity_type);
    }

    /// Adds a banked load to `player`'s gathered total for `kind`.
    pub(crate) fn record_gathered(&mut self, player: PlayerId, kind: &str, amount: u32) {
        add_amount(&mut self.0[player as usize].gathered, kind, amount);
    }

    /// Adds a charge to `player`'s spent total for `kind`.
    pub(crate) fn record_spent(&mut self, player: PlayerId, kind: &str, amount: u32) {
        add_amount(&mut self.0[player as usize].spent, kind, amount);
    }

    /// Adds a give-back to `player`'s refunded total for `kind`.
    pub(crate) fn record_refunded(&mut self, player: PlayerId, kind: &str, amount: u32) {
        add_amount(&mut self.0[player as usize].refunded, kind, amount);
    }

    /// Adds health `player` removed from an enemy.
    pub(crate) fn record_damage_dealt(&mut self, player: PlayerId, amount: FixedU64) {
        let dealt = &mut self.0[player as usize].damage_dealt;
        *dealt = dealt.saturating_add(amount);
    }

    /// Adds health removed from one of `player`'s entities.
    pub(crate) fn record_damage_taken(&mut self, player: PlayerId, amount: FixedU64) {
        let taken = &mut self.0[player as usize].damage_taken;
        *taken = taken.saturating_add(amount);
    }

    /// Counts one finished research topic for `player`.
    pub(crate) fn record_research(&mut self, player: PlayerId) {
        let tally = &mut self.0[player as usize];
        tally.research_completed = tally.research_completed.saturating_add(1);
    }

    /// Counts one cast for `player`.
    pub(crate) fn record_skill_cast(&mut self, player: PlayerId) {
        let tally = &mut self.0[player as usize];
        tally.skills_cast = tally.skills_cast.saturating_add(1);
    }
}

/// Adds one to a per-type counter.
fn bump(counts: &mut BTreeMap<EntityTypeId, u32>, entity_type: EntityTypeId) {
    let count = counts.entry(entity_type).or_insert(0);
    *count = count.saturating_add(1);
}

/// Adds `amount` to a per-kind total.
fn add_amount(totals: &mut BTreeMap<String, u32>, kind: &str, amount: u32) {
    match totals.get_mut(kind) {
        Some(total) => *total = total.saturating_add(amount),
        None => {
            totals.insert(kind.to_string(), amount);
        }
    }
}
