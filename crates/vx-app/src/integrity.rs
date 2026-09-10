//! Integrity: what *accidents* cost the machines that have them.
//!
//! # Why this is not part of `wear.rs`
//!
//! [`crate::wear`] models a machine grinding down: gradual, predictable, paid
//! for in ticks of honest work, and mended for two spare parts. It is the cost
//! of using a thing. This is the other half — the cost of a thing going wrong —
//! and the two are separate because they end differently. A worn machine
//! stops; a broken one is *gone*, and the fleet is one machine smaller for the
//! rest of the save.
//!
//! Until this module existed there was no second half at all. `garage.rs` had
//! `grant` and no opposite, so every machine a player ever bought was
//! permanent, and the roadmap's one-line summary of the gap was exact: **a
//! machine cannot collide with anything, so it cannot crash.** A fleet you
//! cannot lose is not capital, it is a number that only goes up.
//!
//! # It is oracle state, for the same reason wear is
//!
//! A destroyed drone stops cutting, and how long a crew dug is exactly what
//! decides where the hole ends up — so integrity has to be re-derived by
//! replay rather than trusted from a file, and it lives inside
//! [`crate::mining::Mining`] beside the tank and the wear ledger for that
//! reason.
//!
//! **Nothing here needs an order of its own.** Every way a machine can be
//! damaged is already a consequence of something on the wire: a `Pilot`
//! command that flew it into a hillside, a `Fire` that put a slug through it,
//! an `Advance` during which a tree came down on it or a seized machine was
//! asked to work anyway. The damage is arithmetic over orders that are already
//! recorded, which is why this round adds no journal tag and no `VERSION`
//! bump. If that ever stops being true it is a bump, loudly — the same note
//! `MachineTag` carries about the kestrel.
//!
//! # One identity, two ledgers
//!
//! The key is [`crate::wear::key`] itself rather than a lookalike, so a
//! machine cannot be one thing to the wear ledger and another to this one.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::mining::MachineRef;
use crate::wear::{key, PARTS_PER_REPAIR, SPARE_PART};

const MAGIC: &[u8; 4] = b"VXIT";
const VERSION: u32 = 1;

/// What a machine can absorb before it is scrap.
///
/// A round number on purpose: every figure below is quoted as a fraction of
/// it, so the balance reads as "three slugs" or "a four-block dive" rather
/// than as arithmetic.
pub const HULL: u32 = 100;

/// Damage per block of rock a hand-flown machine drives into.
///
/// Quadratic in the depth rather than linear, so the difference between
/// brushing a ridge and flying into a mountain is a difference in kind. One
/// block is a scratch; four is most of a hull; six is instant scrap.
pub const IMPACT_PER_BLOCK: u32 = 6;

/// What one slug does to a machine.
///
/// Three hits and a fresh machine is gone. Deliberately fewer than a body
/// takes: a drone is a thin metal box that cannot flinch, and making it
/// tougher than the player would read as wrong.
pub const SLUG_HIT: u32 = 34;

/// What a *seized* machine takes for each tick it is asked to work anyway.
///
/// The one slow way to lose a machine, and the only one that is entirely the
/// player's fault. Before this, `Condition::Seized` was terminal but safe —
/// it stopped the crew and waited, for ever, for two spare parts. Now
/// ignoring it eventually costs the machine, which is what makes the repair
/// bench a decision instead of a chore you can always postpone.
pub const SEIZED_TICK: u32 = 1;

/// Parts to patch a damaged machine back to a whole hull.
///
/// Twice a wear repair: bending metal back is dearer than an oil change, and
/// the gap is the point.
pub const PARTS_PER_PATCH: u64 = PARTS_PER_REPAIR * 2;

/// How badly knocked about a machine is.
///
/// Deliberately the same shape as [`crate::wear::Condition`] — four states,
/// ordered worst-last, with a `name` for the roster — so the handheld can put
/// the two side by side without either one needing a special case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Damage {
    Sound,
    Dented,
    Buckled,
    Wrecked,
}

impl Damage {
    /// What the damage taken says.
    pub fn of(taken: u32) -> Damage {
        if taken >= HULL {
            Damage::Wrecked
        } else if taken * 2 >= HULL {
            Damage::Buckled
        } else if taken > 0 {
            Damage::Dented
        } else {
            Damage::Sound
        }
    }

    /// What the roster calls it.
    pub fn name(self) -> &'static str {
        match self {
            Damage::Sound => "SOUND",
            Damage::Dented => "DENTED",
            Damage::Buckled => "BUCKLED",
            Damage::Wrecked => "WRECKED",
        }
    }
}

/// Damage taken, per machine.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Integrity {
    /// Keyed by [`crate::wear::key`], so a machine is the same machine here
    /// as it is in the wear ledger.
    taken: BTreeMap<(u8, u32), u32>,
}

impl Integrity {
    /// Damage this machine has taken.
    pub fn taken(&self, machine: MachineRef) -> u32 {
        key(machine)
            .and_then(|key| self.taken.get(&key).copied())
            .unwrap_or(0)
    }

    /// Hull left, in the same units.
    pub fn left(&self, machine: MachineRef) -> u32 {
        HULL.saturating_sub(self.taken(machine))
    }

    /// How this machine is holding up.
    pub fn damage(&self, machine: MachineRef) -> Damage {
        Damage::of(self.taken(machine))
    }

    /// Hurt a machine, and say whether **this** blow is what finished it.
    ///
    /// Returning "did it die *now*" rather than "is it dead" is what stops one
    /// wreck being reported — and salvaged, and mourned — once per tick for
    /// the rest of the session: the caller acts on the edge, not the level.
    pub fn hurt(&mut self, machine: MachineRef, amount: u32) -> bool {
        let Some(key) = key(machine) else {
            // The kestrel is absent from this ledger for the same reason it is
            // absent from wear's: it runs on its own budget. It cannot be
            // wrecked, and saying so here is cheaper than saying it at every
            // call site.
            return false;
        };
        if amount == 0 {
            return false;
        }
        let taken = self.taken.entry(key).or_insert(0);
        let was_whole = *taken < HULL;
        *taken = taken.saturating_add(amount).min(HULL);
        was_whole && *taken >= HULL
    }

    /// Beat the dents out: back to a whole hull, for parts off the pile.
    ///
    /// Refuses a wreck. A machine that is scrap is not repaired, it is walked
    /// out to and stripped — see [`crate::wrecks`] — and letting the bench
    /// undo a loss would take the loss back out of the game.
    ///
    /// Takes the parts here rather than at the call site, exactly as
    /// [`crate::wear::Wear::repair`] does and for the same reason: the live
    /// game and the replay arm run one function and cannot drift.
    pub fn patch(&mut self, machine: MachineRef, pile: &mut vx_agent::Stockpile) -> bool {
        let Some(key) = key(machine) else {
            return false;
        };
        let taken = self.taken.get(&key).copied().unwrap_or(0);
        if taken == 0 || taken >= HULL {
            return false;
        }
        if pile.count(SPARE_PART) < PARTS_PER_PATCH {
            return false;
        }
        pile.take(SPARE_PART, PARTS_PER_PATCH);
        self.taken.remove(&key);
        true
    }

    /// Forget a machine entirely.
    ///
    /// Called when a wreck is salvaged and the hulk stops existing. Without it
    /// the ledger would grow one dead row per machine lost, for ever, and —
    /// worse — a *later* machine that happened to land on the same index would
    /// inherit a wreck's damage. Indices are never reused today (see
    /// [`crate::mining::MachineRef`] and the tombstone rule in
    /// `vx_agent::DroneState::Lost`), so this is belt to that braces.
    pub fn forget(&mut self, machine: MachineRef) {
        if let Some(key) = key(machine) {
            self.taken.remove(&key);
        }
    }

    /// The worst machine in the crew, for the HUD's warning line.
    pub fn complaint(&self, diggers: usize, fliers: usize) -> Option<String> {
        let worst = (0..diggers)
            .map(|index| self.damage(MachineRef::Digger(index)))
            .chain((0..fliers).map(|index| self.damage(MachineRef::Flier(index))))
            .max()
            .unwrap_or(Damage::Sound);
        match worst {
            Damage::Sound | Damage::Dented => None,
            Damage::Buckled => Some("A MACHINE IS BUCKLED - PATCH IT".to_string()),
            Damage::Wrecked => Some("A MACHINE IS DOWN - GO AND GET IT".to_string()),
        }
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "integrity.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.taken.len() as u32).to_le_bytes())?;
        for ((kind, index), taken) in &self.taken {
            file.write_all(&[*kind])?;
            file.write_all(&index.to_le_bytes())?;
            file.write_all(&taken.to_le_bytes())?;
        }
        file.commit()
    }

    /// Read it back, tolerating absence and damage — a lost ledger is a fleet
    /// that has never been knocked about, which is generous and harmless.
    pub fn load(&mut self, directory: &Path) {
        match read(&directory.join("integrity.dat")) {
            Ok(Some(integrity)) => *self = integrity,
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged integrity ledger: {error}"),
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Integrity>> {
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
    let mut taken = BTreeMap::new();
    for _ in 0..rows {
        let mut kind = [0u8; 1];
        file.read_exact(&mut kind)?;
        file.read_exact(&mut word)?;
        let index = u32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        taken.insert((kind[0], index), u32::from_le_bytes(word));
    }
    Ok(Some(Integrity { taken }))
}

/// What a dive into rock costs, in hull.
///
/// `depth` is how far *below* the column's safe altitude the machine ended up:
/// zero for a clean pass, one for clipping the top of a ridge, six or more for
/// flying straight into a hillside. Quadratic, and saturating, so the far end
/// is "scrap" rather than an overflow.
pub fn impact(depth: i32) -> u32 {
    let depth = depth.max(0) as u32;
    IMPACT_PER_BLOCK
        .saturating_mul(depth)
        .saturating_mul(depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_machine_is_sound_and_a_hundred_points_is_scrap() {
        let mut integrity = Integrity::default();
        assert_eq!(integrity.damage(MachineRef::Digger(0)), Damage::Sound);
        assert_eq!(integrity.left(MachineRef::Digger(0)), HULL);

        assert!(!integrity.hurt(MachineRef::Digger(0), SLUG_HIT));
        assert_eq!(integrity.damage(MachineRef::Digger(0)), Damage::Dented);
        assert!(!integrity.hurt(MachineRef::Digger(0), SLUG_HIT));
        assert_eq!(integrity.damage(MachineRef::Digger(0)), Damage::Buckled);
        assert!(
            integrity.hurt(MachineRef::Digger(0), SLUG_HIT),
            "the third slug did not finish it"
        );
        assert!(integrity.damage(MachineRef::Digger(0)) == Damage::Wrecked);
        assert_eq!(integrity.left(MachineRef::Digger(0)), 0);
    }

    /// **A wreck is reported once.**
    ///
    /// `hurt` answers "did this blow finish it", not "is it finished". Get
    /// that backwards and a downed machine files a fresh wreck, a fresh map
    /// pin and a fresh line in the terminal on every tick something touches
    /// it, for the rest of the session.
    #[test]
    fn only_the_blow_that_kills_reports_a_kill() {
        let mut integrity = Integrity::default();
        assert!(integrity.hurt(MachineRef::Flier(0), HULL));
        for _ in 0..50 {
            assert!(
                !integrity.hurt(MachineRef::Flier(0), SLUG_HIT),
                "a wreck died twice"
            );
        }
    }

    /// Damage is per machine, and a neighbour is untouched. The whole
    /// tombstone design rests on indices meaning what they say.
    #[test]
    fn machines_are_hurt_one_at_a_time() {
        let mut integrity = Integrity::default();
        integrity.hurt(MachineRef::Digger(1), HULL);
        assert!(integrity.damage(MachineRef::Digger(1)) == Damage::Wrecked);
        assert_eq!(integrity.damage(MachineRef::Digger(0)), Damage::Sound);
        assert_eq!(integrity.damage(MachineRef::Digger(2)), Damage::Sound);
        assert_eq!(integrity.damage(MachineRef::Flier(1)), Damage::Sound);
    }

    /// The kestrel takes no wear, and it takes no damage either — one
    /// sentence, said in one place.
    #[test]
    fn the_kestrel_is_absent_from_this_ledger_too() {
        let mut integrity = Integrity::default();
        assert!(!integrity.hurt(MachineRef::Kestrel, HULL * 10));
        assert_ne!(integrity.damage(MachineRef::Kestrel), Damage::Wrecked);
    }

    #[test]
    fn a_dive_costs_more_than_a_scrape() {
        assert_eq!(impact(0), 0);
        assert_eq!(impact(-3), 0, "flying above the rock hurt something");
        assert!(impact(1) < HULL / 4, "a scrape wrote a machine off");
        assert!(impact(4) >= HULL / 2, "a four-block dive barely marked it");
        assert!(impact(6) >= HULL, "flying into a hillside was survivable");
    }

    #[test]
    fn a_patch_costs_parts_and_refuses_a_wreck() {
        let mut integrity = Integrity::default();
        let mut pile = vx_agent::Stockpile::new();
        pile.add(SPARE_PART, 10);

        // Nothing wrong with it: nothing to pay for.
        assert!(!integrity.patch(MachineRef::Digger(0), &mut pile));
        assert_eq!(pile.count(SPARE_PART), 10);

        integrity.hurt(MachineRef::Digger(0), SLUG_HIT);
        assert!(integrity.patch(MachineRef::Digger(0), &mut pile));
        assert_eq!(pile.count(SPARE_PART), 10 - PARTS_PER_PATCH);
        assert_eq!(integrity.damage(MachineRef::Digger(0)), Damage::Sound);

        // Scrap is not patched. It is walked out to.
        integrity.hurt(MachineRef::Digger(0), HULL);
        assert!(!integrity.patch(MachineRef::Digger(0), &mut pile));
        assert!(integrity.damage(MachineRef::Digger(0)) == Damage::Wrecked);
    }

    #[test]
    fn a_patch_you_cannot_pay_for_does_not_happen() {
        let mut integrity = Integrity::default();
        let mut pile = vx_agent::Stockpile::new();
        pile.add(SPARE_PART, PARTS_PER_PATCH - 1);
        integrity.hurt(MachineRef::Digger(0), SLUG_HIT);
        assert!(!integrity.patch(MachineRef::Digger(0), &mut pile));
        assert_eq!(integrity.damage(MachineRef::Digger(0)), Damage::Dented);
        assert_eq!(pile.count(SPARE_PART), PARTS_PER_PATCH - 1);
    }

    #[test]
    fn the_ledger_round_trips() {
        let directory = std::env::temp_dir().join(format!(
            "vx-integrity-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();

        let mut integrity = Integrity::default();
        integrity.hurt(MachineRef::Digger(0), 17);
        integrity.hurt(MachineRef::Flier(2), HULL);
        integrity.save(&directory).unwrap();

        let mut back = Integrity::default();
        back.load(&directory);
        assert_eq!(back, integrity);
        assert_eq!(back.taken(MachineRef::Digger(0)), 17);
        assert!(back.damage(MachineRef::Flier(2)) == Damage::Wrecked);

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A ledger that is not there is a fleet nothing has happened to, not a
    /// failed world. The rule every sidecar in this game follows.
    #[test]
    fn an_absent_ledger_is_a_whole_fleet() {
        let directory = std::env::temp_dir().join(format!(
            "vx-integrity-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();

        let mut integrity = Integrity::default();
        integrity.hurt(MachineRef::Digger(0), 40);
        integrity.load(&directory);
        assert_eq!(
            integrity.taken(MachineRef::Digger(0)),
            40,
            "an absent file overwrote a live ledger"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A corrupt ledger warns and resets rather than failing the world.
    #[test]
    fn a_torn_ledger_resets_rather_than_failing() {
        let directory = std::env::temp_dir().join(format!(
            "vx-integrity-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("integrity.dat"), b"junk not a ledger").unwrap();

        let mut integrity = Integrity::default();
        integrity.load(&directory);
        assert_eq!(integrity, Integrity::default());

        std::fs::remove_dir_all(&directory).ok();
    }
}
