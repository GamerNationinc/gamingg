//! What the operation has hauled home.
//!
//! Keyed by **namespaced block name**, never by [`vx_core::BlockId`], for the
//! same reason the save format is: ids are handed out in registration order, so
//! installing a mod shifts every one of them. A stockpile keyed on numbers
//! would quietly turn into a pile of something else.

use std::collections::BTreeMap;

use vx_core::{BlockId, BlockRegistry};

/// Counts of blocks recovered, by name.
///
/// A `BTreeMap` rather than a `HashMap` so listing the pile comes out in a
/// stable order — a readout that reshuffled itself every frame would be
/// unreadable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stockpile {
    counts: BTreeMap<String, u64>,
}

impl Stockpile {
    pub fn new() -> Self {
        Stockpile::default()
    }

    /// Saturating on purpose.
    ///
    /// A pile has no ceiling of its own — a long enough dig keeps adding —
    /// and a wrap here would turn a hoard into nothing, or nothing into a
    /// hoard. Saturating is the honest end of the range: at `u64::MAX` blocks
    /// the arithmetic simply stops moving, which is a bug nobody can reach
    /// rather than a bug that pays out.
    pub fn add(&mut self, name: impl Into<String>, amount: u64) {
        if amount == 0 {
            return;
        }
        let slot = self.counts.entry(name.into()).or_insert(0);
        *slot = slot.saturating_add(amount);
    }

    /// Record a block by id, resolving its name through `registry`.
    ///
    /// Returns false for an id the registry does not know, which is the only
    /// case where a haul could go unrecorded — worth reporting rather than
    /// silently dropping, since it would show up later as a conservation
    /// mismatch with no explanation.
    pub fn add_block(&mut self, registry: &BlockRegistry, block: BlockId, amount: u64) -> bool {
        match registry.get(block) {
            Some(def) => {
                self.add(def.name.clone(), amount);
                true
            }
            None => false,
        }
    }

    /// Remove up to `amount`, returning how much was actually taken.
    pub fn take(&mut self, name: &str, amount: u64) -> u64 {
        let Some(held) = self.counts.get_mut(name) else {
            return 0;
        };
        let taken = amount.min(*held);
        *held -= taken;
        if *held == 0 {
            self.counts.remove(name);
        }
        taken
    }

    /// Empty the pile, handing out every row.
    ///
    /// Used where goods *move* rather than being consumed — a broken container
    /// tipping into the fleet's holding, say — so the caller cannot forget to
    /// clear what it took and double the goods by accident.
    pub fn drain(&mut self) -> impl Iterator<Item = (String, u64)> {
        std::mem::take(&mut self.counts).into_iter()
    }

    pub fn count(&self, name: &str) -> u64 {
        self.counts.get(name).copied().unwrap_or(0)
    }

    /// Everything held, across all kinds. The conservation checks compare this
    /// against blocks actually removed from the world.
    /// Saturating, like [`Stockpile::add`]: a total that wrapped would report
    /// a hoard as nearly empty, which every conservation check in the game
    /// would then agree with.
    pub fn total(&self) -> u64 {
        self.counts
            .values()
            .fold(0u64, |sum, held| sum.saturating_add(*held))
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// Kinds held, in name order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, u64)> {
        self.counts.iter().map(|(name, count)| (name.as_str(), *count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockDef, BlockRegistry};

    fn registry() -> BlockRegistry {
        let mut registry = BlockRegistry::new();
        registry.register(BlockDef::uniform("engine:stone", 0)).unwrap();
        registry.register(BlockDef::uniform("engine:copper_ore", 7)).unwrap();
        registry
    }

    #[test]
    fn adding_accumulates_per_kind() {
        let mut pile = Stockpile::new();
        pile.add("engine:stone", 4);
        pile.add("engine:copper_ore", 3);
        pile.add("engine:stone", 6);

        assert_eq!(pile.count("engine:stone"), 10);
        assert_eq!(pile.count("engine:copper_ore"), 3);
        assert_eq!(pile.total(), 13);
    }

    #[test]
    fn an_unknown_kind_counts_as_nothing_rather_than_failing() {
        let pile = Stockpile::new();
        assert_eq!(pile.count("engine:nothing"), 0);
        assert!(pile.is_empty());
    }

    #[test]
    fn taking_more_than_is_held_takes_what_there_is() {
        let mut pile = Stockpile::new();
        pile.add("engine:stone", 5);

        assert_eq!(pile.take("engine:stone", 8), 5);
        assert_eq!(pile.count("engine:stone"), 0);
        assert!(pile.is_empty(), "an emptied kind should stop being listed");
        assert_eq!(pile.take("engine:stone", 1), 0);
    }

    #[test]
    fn blocks_are_recorded_by_name_not_by_id() {
        // The whole point. Two registries with the same block at different ids
        // must produce the same pile, or a mod install rewrites history.
        let plain = registry();
        let mut shifted = BlockRegistry::new();
        for extra in 0..5 {
            shifted
                .register(BlockDef::uniform(format!("mod:filler{extra}"), 0))
                .unwrap();
        }
        shifted.register(BlockDef::uniform("engine:stone", 0)).unwrap();
        shifted.register(BlockDef::uniform("engine:copper_ore", 7)).unwrap();

        let ore_plain = plain.id_of("engine:copper_ore").unwrap();
        let ore_shifted = shifted.id_of("engine:copper_ore").unwrap();
        assert_ne!(ore_plain, ore_shifted, "the ids should differ for this to mean anything");

        let mut a = Stockpile::new();
        let mut b = Stockpile::new();
        assert!(a.add_block(&plain, ore_plain, 9));
        assert!(b.add_block(&shifted, ore_shifted, 9));

        assert_eq!(a, b);
        assert_eq!(a.count("engine:copper_ore"), 9);
    }

    #[test]
    fn an_unregistered_block_is_reported_rather_than_dropped() {
        let registry = registry();
        let mut pile = Stockpile::new();
        assert!(!pile.add_block(&registry, BlockId(9_999), 1));
        assert_eq!(pile.total(), 0);
    }

    #[test]
    fn adding_nothing_does_not_create_an_entry() {
        let mut pile = Stockpile::new();
        pile.add("engine:stone", 0);
        assert!(pile.is_empty());
    }

    #[test]
    fn entries_come_out_in_a_stable_order() {
        let mut pile = Stockpile::new();
        pile.add("engine:stone", 1);
        pile.add("engine:copper_ore", 1);
        pile.add("engine:dirt", 1);

        let names: Vec<&str> = pile.entries().map(|(name, _)| name).collect();
        assert_eq!(names, ["engine:copper_ore", "engine:dirt", "engine:stone"]);
    }
}
