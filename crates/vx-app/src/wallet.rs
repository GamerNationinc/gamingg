//! Credits and bought upgrades: the money half of the player file pair.
//!
//! Same discipline as [`crate::skills`]: upgrades are **name-keyed entries**,
//! not enum variants, so the shop's shelf grows by adding rows, and the file
//! (`wallet.dat`, its own small file beside `player.dat` — one concern per
//! file, no migrations) tolerates absence and damage. A corrupt wallet is an
//! empty wallet, logged, never a failed world.
//!
//! Upgrade *effects* compose multiplicatively on top of skill effects: the
//! grind and the wallet both work for the player, and a fresh wallet (level
//! 0 everywhere) is the exact identity.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"VXWA";
const VERSION: u32 = 1;

/// The drill-power upgrade line.
pub const DRILL: &str = "drill";
/// The cargo-capacity upgrade line, applied to every hold in the fleet.
pub const CARGO: &str = "cargo";
/// The kestrel's cell line: each mark shortens the recharge after a flight.
pub const CELL: &str = "cell";
/// The pack line: how much you can carry before the weight tells on you.
pub const PACK: &str = "pack";
/// The fabricator's rollers: every print finishes sooner. The one line the
/// counter does not stock — the workshop improves itself, from the inside.
pub const PRESS: &str = "press";
/// The suit lamp's reflector: a longer, stronger throw underground.
pub const LAMP: &str = "lamp";
/// Lead in the suit: what makes a uranium face somewhere you can work
/// rather than somewhere you can visit.
pub const SHIELD: &str = "shield";

/// Every line, in the order panels list them.
pub const LINES: [&str; 7] = [DRILL, CARGO, CELL, PACK, PRESS, LAMP, SHIELD];

/// What each line does, for the panels and the terminal's `kit`.
pub fn describes(line: &str) -> &'static str {
    match line {
        DRILL => "THE DRILL BITES HARDER",
        CARGO => "EVERY HOLD IN THE FLEET CARRIES MORE",
        CELL => "THE KESTREL RECHARGES SOONER",
        PACK => "YOU CARRY MORE BEFORE IT SLOWS YOU",
        PRESS => "THE FABRICATOR PRINTS FASTER",
        LAMP => "THE LAMP THROWS FURTHER",
        SHIELD => "THE SUIT KEEPS MORE OF THE DOSE OUT",
        _ => "",
    }
}

/// Levels each upgrade line can reach.
pub const MAX_UPGRADE: u32 = 5;

/// Credits and owned upgrade levels.
#[derive(Debug, Default)]
pub struct Wallet {
    credits: u64,
    upgrades: BTreeMap<String, u32>,
}

impl Wallet {
    pub fn new() -> Self {
        Wallet::default()
    }

    pub fn credits(&self) -> u64 {
        self.credits
    }

    pub fn earn(&mut self, amount: u64) {
        self.credits = self.credits.saturating_add(amount);
    }

    /// Spend exactly `amount`, or refuse and change nothing.
    #[must_use]
    pub fn spend(&mut self, amount: u64) -> bool {
        if self.credits < amount {
            return false;
        }
        self.credits -= amount;
        true
    }

    /// The owned level of an upgrade line. Unbought is level 0.
    pub fn upgrade(&self, name: &str) -> u32 {
        self.upgrades.get(name).copied().unwrap_or(0)
    }

    /// Raise an upgrade line one level, capped, returning the new level.
    pub fn raise(&mut self, name: &str) -> u32 {
        let entry = self.upgrades.entry(name.to_string()).or_insert(0);
        *entry = (*entry + 1).min(MAX_UPGRADE);
        *entry
    }

    /// Write the wallet beside the world save.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "wallet.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&self.credits.to_le_bytes())?;
        file.write_all(&(self.upgrades.len() as u32).to_le_bytes())?;
        for (name, level) in &self.upgrades {
            let bytes = name.as_bytes();
            file.write_all(&(bytes.len() as u32).to_le_bytes())?;
            file.write_all(bytes)?;
            file.write_all(&level.to_le_bytes())?;
        }
        file.commit()
    }

    /// Load the wallet, tolerating absence and damage.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("wallet.dat");
        match read_wallet(&path) {
            Ok(Some((credits, upgrades))) => {
                log::info!("loaded wallet: {credits} credits, {} upgrades", upgrades.len());
                self.credits = credits;
                self.upgrades = upgrades;
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting fresh", path.display());
                self.credits = 0;
                self.upgrades.clear();
            }
        }
    }
}

type WalletData = (u64, BTreeMap<String, u32>);

fn read_wallet(path: &Path) -> std::io::Result<Option<WalletData>> {
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
    let mut credits = [0u8; 8];
    file.read_exact(&mut credits)?;
    file.read_exact(&mut word)?;
    let count = u32::from_le_bytes(word);

    let mut upgrades = BTreeMap::new();
    for _ in 0..count {
        file.read_exact(&mut word)?;
        let length = u32::from_le_bytes(word) as usize;
        if length > 256 {
            return Err(std::io::Error::other("upgrade name implausibly long"));
        }
        let mut name = vec![0u8; length];
        file.read_exact(&mut name)?;
        file.read_exact(&mut word)?;
        upgrades.insert(
            String::from_utf8(name).map_err(std::io::Error::other)?,
            u32::from_le_bytes(word),
        );
    }
    Ok(Some((u64::from_le_bytes(credits), upgrades)))
}

/// How much faster the drill chews per owned level: +25% each.
pub fn drill_multiplier(level: u32) -> f32 {
    1.0 + 0.25 * level as f32
}

/// A hold's capacity with the cargo line applied: +50% of base per level.
pub fn boosted_capacity(base: u64, level: u32) -> u64 {
    base + base * level as u64 / 2
}

/// What you can carry with the pack line applied: +30% of base per mark.
///
/// Safe against the replay oracle for a reason worth writing down: the
/// player's load reaches the journal as a *byte recorded in the
/// `MoveCommand`*, not as something replay re-derives. An upgrade that
/// changed how much you can carry would otherwise quietly change how fast
/// a replayed session walked.
pub fn pack_capacity(base: u64, level: u32) -> u64 {
    base + base * 3 * level as u64 / 10
}

/// How much of a print's time the rollers save: -15% per mark, compounding
/// no faster than that. Print *timing* is live-only — the journal records
/// the order, and its replay arm moves the pile in one go — so this can
/// change freely without the oracle noticing.
pub fn press_multiplier(level: u32) -> f32 {
    1.0 / (1.0 + 0.15 * level as f32)
}

/// The beam with the reflector fitted: +12% reach and +10% strength per
/// mark. A shader uniform and nothing else, so it never reaches the hash.
pub fn boosted_beam(strength: f32, reach: f32, level: u32) -> (f32, f32) {
    let level = level.min(MAX_UPGRADE) as f32;
    (strength * (1.0 + 0.10 * level), reach * (1.0 + 0.12 * level))
}

/// The kestrel's full-cell recharge with the cell line applied: a straight
/// walk from the stock cost down to the best, one fifth per mark.
pub fn recharge_ticks(level: u32) -> u32 {
    let stock = vx_agent::kestrel::COOLDOWN;
    let best = vx_agent::kestrel::COOLDOWN_BEST;
    let step = (stock - best) / MAX_UPGRADE;
    stock - step * level.min(MAX_UPGRADE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_wallet_is_the_exact_identity_on_every_line() {
        // The whole promise of the upgrade system in one assertion: level
        // zero changes nothing at all, on every line, including the ones
        // this round added.
        assert_eq!(pack_capacity(400, 0), 400);
        assert_eq!(press_multiplier(0), 1.0);
        assert_eq!(boosted_beam(1.5, 16.0, 0), (1.5, 16.0));
        assert_eq!(drill_multiplier(0), 1.0);
        assert_eq!(boosted_capacity(400, 0), 400);
        assert_eq!(recharge_ticks(0), vx_agent::kestrel::COOLDOWN);
    }

    #[test]
    fn every_new_line_improves_monotonically_and_caps() {
        let mut carry = 0;
        let mut time = f32::MAX;
        let mut throw = 0.0;
        for level in 0..=MAX_UPGRADE {
            let next_carry = pack_capacity(400, level);
            assert!(next_carry >= carry, "the pack shrank at {level}");
            carry = next_carry;

            let next_time = press_multiplier(level);
            assert!(next_time <= time, "the press slowed at {level}");
            time = next_time;

            let (_, reach) = boosted_beam(1.5, 16.0, level);
            assert!(reach >= throw, "the lamp dimmed at {level}");
            throw = reach;
        }
        // Past the cap nothing further is owed: a save that somehow holds a
        // sixth mark gets the fifth mark's effect, never a runaway.
        assert_eq!(
            boosted_beam(1.5, 16.0, MAX_UPGRADE + 3),
            boosted_beam(1.5, 16.0, MAX_UPGRADE)
        );
    }

    #[test]
    fn every_line_is_listed_and_described() {
        // A line the panels cannot name is a line the player cannot find.
        for line in LINES {
            assert!(!describes(line).is_empty(), "{line} has no description");
            for character in describes(line).chars() {
                assert!(vx_render::font::knows(character), "undrawable in {line}");
            }
        }
    }

    #[test]
    fn earning_and_spending_are_exact_and_overdrafts_refused() {
        let mut wallet = Wallet::new();
        wallet.earn(100);
        assert!(wallet.spend(60));
        assert_eq!(wallet.credits(), 40);
        assert!(!wallet.spend(41), "overdraft accepted");
        assert_eq!(wallet.credits(), 40, "a refused spend changed the balance");
        assert!(wallet.spend(40));
        assert_eq!(wallet.credits(), 0);
    }

    #[test]
    fn upgrades_start_at_zero_and_cap() {
        let mut wallet = Wallet::new();
        assert_eq!(wallet.upgrade(DRILL), 0);
        for expected in 1..=MAX_UPGRADE {
            assert_eq!(wallet.raise(DRILL), expected);
        }
        assert_eq!(wallet.raise(DRILL), MAX_UPGRADE, "the cap did not hold");
        assert_eq!(wallet.upgrade(CARGO), 0, "lines are independent");
    }

    #[test]
    fn level_zero_is_the_identity_and_effects_grow() {
        assert_eq!(drill_multiplier(0), 1.0);
        assert_eq!(boosted_capacity(64, 0), 64);
        for level in 1..=MAX_UPGRADE {
            assert!(drill_multiplier(level) > drill_multiplier(level - 1));
            assert!(boosted_capacity(64, level) > boosted_capacity(64, level - 1));
        }
    }

    #[test]
    fn the_wallet_round_trips_through_disk() {
        let directory = std::env::temp_dir().join(format!("vx-wallet-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut wallet = Wallet::new();
        wallet.earn(1234);
        wallet.raise(DRILL);
        wallet.raise(CARGO);
        wallet.raise(CARGO);
        wallet.save(&directory).unwrap();

        let mut reloaded = Wallet::new();
        reloaded.load(&directory);
        assert_eq!(reloaded.credits(), 1234);
        assert_eq!(reloaded.upgrade(DRILL), 1);
        assert_eq!(reloaded.upgrade(CARGO), 2);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_corrupt_wallet_is_a_fresh_wallet_not_a_crash() {
        let directory =
            std::env::temp_dir().join(format!("vx-wallet-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("wallet.dat"), b"NOPE this is not a wallet").unwrap();

        let mut wallet = Wallet::new();
        wallet.earn(999); // pre-existing state must be cleared by a bad load
        wallet.load(&directory);
        assert_eq!(wallet.credits(), 0);
        assert_eq!(wallet.upgrade(DRILL), 0);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_missing_wallet_keeps_current_state() {
        let directory =
            std::env::temp_dir().join(format!("vx-wallet-none-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut wallet = Wallet::new();
        wallet.earn(5);
        wallet.load(&directory);
        assert_eq!(wallet.credits(), 5, "absence should not reset anything");

        std::fs::remove_dir_all(&directory).ok();
    }
}
