//! Wear: what work costs the machines that do it.
//!
//! # Wear is oracle state, and that decides where it lives
//!
//! Every other "player kit" number in this game — credits, upgrade marks,
//! optics, disposition — is live-only, kept outside the replay hash on
//! purpose. Wear cannot be. A worn crew digs slower, a seized one stops, and
//! how long a crew dug is exactly what decides where the hole ends up. So
//! this ledger sits inside [`crate::mining::Mining`] beside the fuel tank,
//! which is the struct `Rebuilt` carries: replay re-runs the same ticks and
//! re-derives the same wear, and the two sides cannot disagree about how
//! much ground got cut.
//!
//! That is the same argument the fuel loop made, and it is why the two read
//! alike here: **a seized tick is a tick nobody works**, exactly as a dry
//! tick is.
//!
//! # Ticks worked, not seconds elapsed
//!
//! Wear accrues per *tick of work*, never per second of wall time. A machine
//! parked in the garage ages not at all, a machine that dug all night is
//! ruined, and a session recorded on a slow machine wears identically on a
//! fast one — the same reason the journal counts ticks in the first place.
//!
//! # The worst machine sets the pace
//!
//! A crew works as a crew. The duty cycle comes off the *worst* condition in
//! it rather than an average, because an average is something a player has
//! to be told and a worst is something they can see: the roster names the
//! bad machine, and mending that one machine visibly speeds the whole dig.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::mining::MachineRef;

const MAGIC: &[u8; 4] = b"VXWR";
const VERSION: u32 = 1;

/// Ticks of work a machine takes to cross each threshold. At the drones'
/// eight ticks a second these are roughly nine, eighteen and twenty-seven
/// minutes of *actual digging* — long enough that a first excavation never
/// meets wear, short enough that a career does.
pub const WORN_AT: u32 = 4_400;
pub const FAILING_AT: u32 = 8_800;
pub const SEIZED_AT: u32 = 13_200;

/// What one repair costs, in printed spare parts.
pub const PARTS_PER_REPAIR: u64 = 2;

/// The good a repair is paid in. Printed at the fabricator, which is why
/// this round waited for the workshop.
pub const SPARE_PART: &str = "engine:spare_part";

/// How worn a machine is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Condition {
    Fresh,
    Worn,
    Failing,
    Seized,
}

impl Condition {
    /// What the ticks say.
    pub fn of(ticks: u32) -> Condition {
        if ticks >= SEIZED_AT {
            Condition::Seized
        } else if ticks >= FAILING_AT {
            Condition::Failing
        } else if ticks >= WORN_AT {
            Condition::Worn
        } else {
            Condition::Fresh
        }
    }

    /// What the roster calls it.
    pub fn name(self) -> &'static str {
        match self {
            Condition::Fresh => "FRESH",
            Condition::Worn => "WORN",
            Condition::Failing => "FAILING",
            Condition::Seized => "SEIZED",
        }
    }

    /// One tick in every `n` is lost to this condition; `None` never skips,
    /// `Some(1)` never works.
    ///
    /// Deterministic in the tick counter, never in wall time or a roll —
    /// this is arithmetic the replay oracle re-runs.
    pub fn skip_every(self) -> Option<u32> {
        match self {
            Condition::Fresh => None,
            Condition::Worn => Some(5),
            Condition::Failing => Some(2),
            Condition::Seized => Some(1),
        }
    }
}

/// Every machine's condition, and the counter the duty cycle runs on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Wear {
    /// Ticks worked, per machine. Keyed by kind and index — the same shape
    /// as [`MachineRef`], which is how a roster row finds its own row here.
    worked: BTreeMap<(u8, u32), u32>,
    /// Ticks this ledger has been asked to run. The duty cycle's phase, so
    /// skipping is spread evenly rather than bunched.
    spins: u64,
}

/// The stable key for a machine. The kestrel is deliberately absent: it
/// runs on a cell and a cooldown, which is its own budget, and charging it
/// wear as well would be charging twice for the same wing — the same line
/// the fuel loop drew.
///
/// Shared with [`crate::integrity`], which keys its own ledger the same way.
/// Two ledgers about the same machines have to agree about what a machine
/// *is*, and the cheap way to guarantee that is one function rather than two
/// that look alike.
pub(crate) fn key(machine: MachineRef) -> Option<(u8, u32)> {
    match machine {
        MachineRef::Digger(index) => Some((0, index as u32)),
        MachineRef::Flier(index) => Some((1, index as u32)),
        MachineRef::Kestrel => None,
    }
}

impl Wear {
    /// Ticks this machine has worked.
    pub fn ticks(&self, machine: MachineRef) -> u32 {
        key(machine)
            .and_then(|key| self.worked.get(&key).copied())
            .unwrap_or(0)
    }

    /// How this machine is holding up.
    pub fn condition(&self, machine: MachineRef) -> Condition {
        Condition::of(self.ticks(machine))
    }

    /// The worst condition among `diggers` diggers and `fliers` fliers.
    pub fn worst(&self, diggers: usize, fliers: usize) -> Condition {
        let diggers = (0..diggers).map(|index| self.condition(MachineRef::Digger(index)));
        let fliers = (0..fliers).map(|index| self.condition(MachineRef::Flier(index)));
        diggers.chain(fliers).max().unwrap_or(Condition::Fresh)
    }

    /// One tick of work for a crew of this shape: charges every machine a
    /// tick and answers whether the crew actually turns this tick.
    ///
    /// Called from inside `Mining::advance`, beside the fuel burn, so the
    /// answer is part of what replay re-derives.
    pub fn tick(&mut self, diggers: usize, fliers: usize) -> bool {
        let worst = self.worst(diggers, fliers);
        let spin = self.spins;
        self.spins = self.spins.wrapping_add(1);

        let turns = match worst.skip_every() {
            None => true,
            // The phase is the counter, so a worn crew loses an evenly
            // spaced tick rather than stuttering in bursts.
            Some(every) => !spin.is_multiple_of(u64::from(every)),
        };
        if !turns {
            return false;
        }
        // Only work wears a machine. A stalled tick costs nothing, which is
        // what stops a seized crew from wearing itself further into a hole
        // it can never climb out of.
        for index in 0..diggers {
            *self.worked.entry((0, index as u32)).or_insert(0) += 1;
        }
        for index in 0..fliers {
            *self.worked.entry((1, index as u32)).or_insert(0) += 1;
        }
        true
    }

    /// Mend one machine: back to nothing worked.
    ///
    /// Takes the parts off the pile here rather than at the call site so
    /// that the live game and the journal's replay arm run one function and
    /// cannot drift. Returns whether it happened.
    pub fn repair(&mut self, machine: MachineRef, pile: &mut vx_agent::Stockpile) -> bool {
        let Some(key) = key(machine) else {
            return false;
        };
        if self.worked.get(&key).copied().unwrap_or(0) == 0 {
            return false;
        }
        if pile.count(SPARE_PART) < PARTS_PER_REPAIR {
            return false;
        }
        pile.take(SPARE_PART, PARTS_PER_REPAIR);
        self.worked.remove(&key);
        true
    }

    /// The worst machine in the crew, for the HUD's warning line.
    pub fn complaint(&self, diggers: usize, fliers: usize) -> Option<String> {
        match self.worst(diggers, fliers) {
            Condition::Fresh => None,
            Condition::Worn => None,
            Condition::Failing => Some("A MACHINE IS FAILING - REPAIR IT".to_string()),
            Condition::Seized => Some("A MACHINE HAS SEIZED - THE CREW IS STOPPED".to_string()),
        }
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "wear.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.worked.len() as u32).to_le_bytes())?;
        for ((kind, index), ticks) in &self.worked {
            file.write_all(&[*kind])?;
            file.write_all(&index.to_le_bytes())?;
            file.write_all(&ticks.to_le_bytes())?;
        }
        file.write_all(&self.spins.to_le_bytes())?;
        file.commit()
    }

    /// Read it back, tolerating absence and damage — a lost ledger is a
    /// fleet that has never worked, which is generous and harmless.
    pub fn load(&mut self, directory: &Path) {
        match read(&directory.join("wear.dat")) {
            Ok(Some(wear)) => *self = wear,
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged wear ledger: {error}"),
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Wear>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("bad magic"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Err(std::io::Error::other("unknown version"));
    }
    file.read_exact(&mut word)?;
    let rows = u32::from_le_bytes(word);
    let mut worked = BTreeMap::new();
    for _ in 0..rows {
        let mut kind = [0u8; 1];
        file.read_exact(&mut kind)?;
        file.read_exact(&mut word)?;
        let index = u32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        worked.insert((kind[0], index), u32::from_le_bytes(word));
    }
    let mut spins = [0u8; 8];
    file.read_exact(&mut spins)?;
    Ok(Some(Wear {
        worked,
        spins: u64::from_le_bytes(spins),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wear_comes_from_work_and_nothing_else() {
        let mut wear = Wear::default();
        // A crew that never runs never ages.
        assert_eq!(wear.condition(MachineRef::Digger(0)), Condition::Fresh);
        for _ in 0..10_000 {
            wear.tick(0, 0);
        }
        assert_eq!(wear.ticks(MachineRef::Digger(0)), 0, "an idle fleet wore out");

        // A working one does.
        for _ in 0..WORN_AT {
            wear.tick(2, 1);
        }
        assert_eq!(wear.condition(MachineRef::Digger(1)), Condition::Worn);
        assert_eq!(wear.condition(MachineRef::Flier(0)), Condition::Worn);
        // A machine that was not in the crew is untouched.
        assert_eq!(wear.condition(MachineRef::Digger(5)), Condition::Fresh);
    }

    #[test]
    fn the_crew_slows_then_stops_as_it_wears() {
        // The whole mechanic in one measurement: how many of a thousand
        // ticks actually turn, at each condition.
        let turning = |start: u32| {
            let mut wear = Wear::default();
            wear.worked.insert((0, 0), start);
            let mut turned = 0;
            for _ in 0..1_000 {
                // Re-pin the wear so the condition under test does not drift
                // as the crew works.
                wear.worked.insert((0, 0), start);
                if wear.tick(1, 0) {
                    turned += 1;
                }
            }
            turned
        };
        let fresh = turning(0);
        let worn = turning(WORN_AT);
        let failing = turning(FAILING_AT);
        let seized = turning(SEIZED_AT);

        assert_eq!(fresh, 1_000, "a fresh crew lost a tick");
        assert!(worn < fresh, "wear cost nothing");
        assert!(failing < worn, "failing was no worse than worn");
        assert_eq!(seized, 0, "a seized crew kept digging");
    }

    #[test]
    fn a_seized_crew_does_not_wear_itself_further() {
        let mut wear = Wear::default();
        wear.worked.insert((0, 0), SEIZED_AT);
        for _ in 0..500 {
            wear.tick(1, 0);
        }
        assert_eq!(
            wear.ticks(MachineRef::Digger(0)),
            SEIZED_AT,
            "a stopped machine kept ageing"
        );
    }

    #[test]
    fn repair_costs_parts_and_refuses_without_them() {
        let mut wear = Wear::default();
        // Work it until it seizes rather than for a fixed count: a worn
        // crew skips ticks, so it takes *more* calls than SEIZED_AT to
        // accrue SEIZED_AT ticks of actual work. That is the mechanic, not
        // an accident — see the test below.
        while wear.condition(MachineRef::Digger(0)) != Condition::Seized {
            wear.tick(1, 0);
        }

        let mut pile = vx_agent::Stockpile::new();
        assert!(
            !wear.repair(MachineRef::Digger(0), &mut pile),
            "mended a machine out of thin air"
        );
        pile.add(SPARE_PART, PARTS_PER_REPAIR);
        assert!(wear.repair(MachineRef::Digger(0), &mut pile));
        assert_eq!(wear.condition(MachineRef::Digger(0)), Condition::Fresh);
        assert_eq!(pile.count(SPARE_PART), 0, "the parts were not spent");

        // Nothing to mend is a refusal, not a free part sink.
        pile.add(SPARE_PART, PARTS_PER_REPAIR);
        assert!(!wear.repair(MachineRef::Digger(0), &mut pile));
        assert_eq!(pile.count(SPARE_PART), PARTS_PER_REPAIR);
    }

    #[test]
    fn a_worn_machine_wears_slower_because_it_works_less() {
        // A pleasing consequence of charging wear only on ticks that turn:
        // machines decay towards seizing rather than falling off a cliff,
        // and the last stretch of a machine's life is the longest.
        let calls_to_reach = |target: Condition| {
            let mut wear = Wear::default();
            let mut calls = 0u32;
            while wear.condition(MachineRef::Digger(0)) != target {
                wear.tick(1, 0);
                calls += 1;
                assert!(calls < 200_000, "never reached {target:?}");
            }
            calls
        };
        let to_worn = calls_to_reach(Condition::Worn);
        let to_failing = calls_to_reach(Condition::Failing);
        let to_seized = calls_to_reach(Condition::Seized);

        // Every threshold is the same distance in *worked* ticks...
        assert_eq!(to_worn, WORN_AT);
        // ...but each later stretch takes more calls than the one before,
        // because fewer of those calls are working ticks.
        assert!(to_failing - to_worn > WORN_AT);
        assert!(to_seized - to_failing > to_failing - to_worn);
    }

    #[test]
    fn the_worst_machine_sets_the_pace() {
        let mut wear = Wear::default();
        wear.worked.insert((0, 0), 0);
        wear.worked.insert((0, 1), SEIZED_AT);
        assert_eq!(wear.worst(2, 0), Condition::Seized);
        // And mending it hands the crew back.
        let mut pile = vx_agent::Stockpile::new();
        pile.add(SPARE_PART, PARTS_PER_REPAIR);
        assert!(wear.repair(MachineRef::Digger(1), &mut pile));
        assert_eq!(wear.worst(2, 0), Condition::Fresh);
    }

    #[test]
    fn two_identical_runs_wear_identically() {
        // The oracle's requirement, asserted directly: wear is a pure
        // function of the ticks worked, so two runs of the same shape end in
        // exactly the same state — bytes included.
        let run = || {
            let mut wear = Wear::default();
            for step in 0..5_000 {
                wear.tick(3, 1);
                if step == 2_000 {
                    let mut pile = vx_agent::Stockpile::new();
                    pile.add(SPARE_PART, PARTS_PER_REPAIR);
                    wear.repair(MachineRef::Digger(1), &mut pile);
                }
            }
            wear
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn the_ledger_round_trips_through_disk() {
        let directory = std::env::temp_dir().join(format!("vx-wear-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut wear = Wear::default();
        for _ in 0..FAILING_AT {
            wear.tick(2, 1);
        }
        wear.save(&directory).unwrap();
        let mut read_back = Wear::default();
        read_back.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(wear, read_back);
    }

    #[test]
    fn a_missing_or_damaged_ledger_is_a_fresh_fleet() {
        let directory = std::env::temp_dir().join(format!("vx-wear-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut wear = Wear::default();
        wear.load(&directory);
        assert_eq!(wear, Wear::default());

        std::fs::write(directory.join("wear.dat"), b"junk").unwrap();
        let mut wear = Wear::default();
        wear.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(wear, Wear::default(), "a damaged ledger was not ignored");
    }

    #[test]
    fn every_condition_is_drawable_and_ordered() {
        let ladder = [
            Condition::Fresh,
            Condition::Worn,
            Condition::Failing,
            Condition::Seized,
        ];
        for pair in ladder.windows(2) {
            assert!(pair[0] < pair[1], "the ladder is out of order");
        }
        for step in ladder {
            for character in step.name().chars() {
                assert!(vx_render::font::knows(character), "undrawable {character:?}");
            }
        }
    }
}
