//! The garage: the machines you actually own.
//!
//! # Why this exists at all
//!
//! Until now machines were free. A flier arrived the moment the world opened
//! and a whole crew of ground drones appeared, unpaid for, on every dispatch —
//! while the shop sold two capped upgrade lines and nothing else. So there was
//! nothing to mine *for*, nothing to trade *for*, and the economy had no sink
//! to drain into.
//!
//! This is the sink. Ore becomes credits, credits become machines, machines
//! mine more ore. That is the loop, and everything else in the game hangs off
//! whether it exists.
//!
//! # Its own file
//!
//! `garage.dat` rather than a field on `wallet.dat`, following the rule
//! [`crate::clock`] states outright: one concern per file, no migrations.
//! Folding it in would force a version bump on a loader that rejects unknown
//! versions outright, and would silently wipe a player's credits the first time
//! they ran a new build against an old save.
//!
//! Machines are **name-keyed**, like skills and upgrade lines, so the shelf
//! grows by adding a row rather than a variant.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::wallet::Wallet;

const MAGIC: &[u8; 4] = b"VXGR";
const VERSION: u32 = 1;

/// A ground drone: cuts rock and hauls it to the mine mouth.
pub const DRONE: &str = "drone";
/// A flier: sweeps sectors for ore and ferries piles home.
pub const FLIER: &str = "flier";
/// The kestrel: a palm-sized scout that rides the pack. One per person —
/// the fleet is for swarms, the pack is for one — which `buy` enforces.
pub const KESTREL: &str = "kestrel";

/// A watch box for your own roof: the same kit the sheriff has, at the same
/// price the sheriff paid. Symmetric tech, made purchasable.
pub const WATCHBOX: &str = "watch box";

/// The fabricator: raw stock in, anything out. Bought once, then placed
/// wherever you want it — and moved by breaking it, like the chest.
pub const PRINTER: &str = "fabricator";

/// Every machine the shop sells, in the order it lists them.
/// The electrolyser: the machine that turns a lake into fuel.
pub const ELECTROLYSER: &str = "electrolyser";

pub const KINDS: [&str; 6] = [DRONE, FLIER, KESTREL, WATCHBOX, PRINTER, ELECTROLYSER];

/// What the first one of each kind costs.
///
/// A first drone should be a few good ore runs rather than a grind — you can
/// hand-mine and sell your way to one inside the opening session. A flier costs
/// more because you are given one for nothing to start with; buying a second is
/// a real expansion rather than a first step. The kestrel sits between: it
/// earns nothing, but what it sees is worth a drone.
const FIRST_COST: [u64; KINDS.len()] = [250, 400, 300, 800, 500, 450];

/// How much dearer each machine is than the one before, in eighths.
///
/// Twelve eighths is one and a half. Steep enough that a fourth drone is a
/// decision, gentle enough that the second is soon.
const RISE_EIGHTHS: u64 = 12;

/// What the next machine of a kind costs, given how many you already own.
pub fn cost(kind: &str, owned: u32) -> u64 {
    let Some(index) = KINDS.iter().position(|known| *known == kind) else {
        return u64::MAX;
    };
    let mut price = FIRST_COST[index];
    // Saturating, so an absurd fleet cannot wrap the price back to affordable.
    for _ in 0..owned.min(24) {
        price = price.saturating_mul(RISE_EIGHTHS) / 8;
    }
    price
}

/// A machine's name as the shop should print it.
pub fn display_name(kind: &str) -> String {
    kind.to_uppercase()
}

/// What the player owns.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Garage {
    owned: BTreeMap<String, u32>,
}

impl Garage {
    pub fn new() -> Self {
        Garage::default()
    }

    /// How many of a kind are owned. Never bought is none.
    pub fn owned(&self, kind: &str) -> u32 {
        self.owned.get(kind).copied().unwrap_or(0)
    }

    /// Put one on the books without paying — for the starter machine and for
    /// the `--drones` override, not for the shop.
    pub fn grant(&mut self, kind: &str, count: u32) {
        let entry = self.owned.entry(kind.to_string()).or_insert(0);
        *entry = entry.saturating_add(count);
    }

    /// Buy one, spending from `wallet`.
    ///
    /// Atomic in the way that matters: a refused purchase changes neither the
    /// credits nor the garage. Spend first, add second — the same shape as
    /// [`crate::shop::buy`].
    pub fn buy(&mut self, wallet: &mut Wallet, kind: &str) -> bool {
        if !KINDS.contains(&kind) {
            return false;
        }
        // One kestrel and one watch box per person, by design rather than
        // by price curve: the pack holds one scout, the house has one roof.
        if matches!(kind, KESTREL | WATCHBOX | PRINTER | ELECTROLYSER) && self.owned(kind) > 0 {
            return false;
        }
        if !wallet.spend(cost(kind, self.owned(kind))) {
            return false;
        }
        self.grant(kind, 1);
        true
    }

    /// Fit a module — a coil, a hardened link. One per fleet, not a rising
    /// curve: a module is a thing you own, not a machine you accumulate, so
    /// the catalogue and the price live with whatever feature defines them
    /// and this only records that you have it.
    pub fn buy_module(&mut self, wallet: &mut Wallet, name: &str, cost: u64) -> bool {
        if self.owned(name) > 0 {
            return false;
        }
        if !wallet.spend(cost) {
            return false;
        }
        self.grant(name, 1);
        true
    }

    /// Is this module fitted?
    pub fn fitted(&self, name: &str) -> bool {
        self.owned(name) > 0
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "garage.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.owned.len() as u32).to_le_bytes())?;
        for (kind, count) in &self.owned {
            file.write_all(&(kind.len() as u32).to_le_bytes())?;
            file.write_all(kind.as_bytes())?;
            file.write_all(&count.to_le_bytes())?;
        }
        file.commit()
    }

    /// Read it back, tolerating absence and damage.
    ///
    /// A damaged garage is an *empty* one, logged, never a failed world. That
    /// is a harsh loss — it is the player's fleet — but the alternative is
    /// refusing to open a world at all, and the starter flier means an empty
    /// garage is still playable.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("garage.dat");
        match read_garage(&path) {
            Ok(Some(garage)) => *self = garage,
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting with an empty garage", path.display());
            }
        }
    }
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_garage(path: &Path) -> std::io::Result<Option<Garage>> {
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
    if read_u32(&mut file)? != VERSION {
        return Err(std::io::Error::other("unknown version"));
    }

    let mut garage = Garage::new();
    let kinds = read_u32(&mut file)?;
    for _ in 0..kinds {
        let length = read_u32(&mut file)? as usize;
        if length > 64 {
            return Err(std::io::Error::other("implausible machine name"));
        }
        let mut bytes = vec![0u8; length];
        file.read_exact(&mut bytes)?;
        let kind =
            String::from_utf8(bytes).map_err(|_| std::io::Error::other("name is not text"))?;
        let count = read_u32(&mut file)?;
        garage.owned.insert(kind, count);
    }
    Ok(Some(garage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_garage_owns_nothing() {
        let garage = Garage::new();
        assert_eq!(garage.owned(DRONE), 0);
        assert_eq!(garage.owned(FLIER), 0);
        assert_eq!(garage.owned("nonsense"), 0);
    }

    #[test]
    fn each_machine_costs_more_than_the_one_before() {
        let mut last = 0;
        for owned in 0..6 {
            let price = cost(DRONE, owned);
            assert!(price > last, "a {owned}th drone cost {price}, no more than {last}");
            last = price;
        }
        // The first one is reachable in an opening session rather than a grind.
        assert_eq!(cost(DRONE, 0), 250);
        // A flier costs more, because you are given one for nothing.
        assert!(cost(FLIER, 0) > cost(DRONE, 0));
        // An unknown machine is unaffordable rather than free.
        assert_eq!(cost("nonsense", 0), u64::MAX);
    }

    #[test]
    fn a_huge_fleet_cannot_wrap_the_price_back_to_affordable() {
        // Saturating arithmetic: the twenty-fifth drone must not cost nothing.
        assert!(cost(DRONE, 30) >= cost(DRONE, 10));
    }

    #[test]
    fn buying_spends_exactly_the_price_and_adds_exactly_one() {
        let mut wallet = Wallet::new();
        wallet.earn(1_000);
        let mut garage = Garage::new();

        let price = cost(DRONE, 0);
        assert!(garage.buy(&mut wallet, DRONE));
        assert_eq!(wallet.credits(), 1_000 - price);
        assert_eq!(garage.owned(DRONE), 1);
        assert_eq!(garage.owned(FLIER), 0, "the wrong machine arrived");

        // The next one costs more.
        let second = cost(DRONE, 1);
        assert!(second > price);
        assert!(garage.buy(&mut wallet, DRONE));
        assert_eq!(wallet.credits(), 1_000 - price - second);
        assert_eq!(garage.owned(DRONE), 2);
    }

    #[test]
    fn a_refused_purchase_changes_nothing_at_all() {
        let mut broke = Wallet::new();
        broke.earn(cost(DRONE, 0) - 1);
        let mut garage = Garage::new();

        assert!(!garage.buy(&mut broke, DRONE));
        assert_eq!(broke.credits(), cost(DRONE, 0) - 1, "credits moved on a refusal");
        assert_eq!(garage.owned(DRONE), 0, "a refused machine arrived anyway");

        // And an unknown kind cannot be bought at any price.
        let mut rich = Wallet::new();
        rich.earn(u64::MAX / 2);
        assert!(!garage.buy(&mut rich, "nonsense"));
        assert_eq!(rich.credits(), u64::MAX / 2);
    }

    #[test]
    fn granting_bypasses_the_till() {
        // What the starter flier and the `--drones` override use.
        let mut garage = Garage::new();
        garage.grant(DRONE, 3);
        assert_eq!(garage.owned(DRONE), 3);
        garage.grant(DRONE, 1);
        assert_eq!(garage.owned(DRONE), 4);
    }

    #[test]
    fn the_garage_round_trips_and_tolerates_damage() {
        let directory = std::env::temp_dir().join(format!("vx-garage-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut garage = Garage::new();
        garage.grant(DRONE, 4);
        garage.grant(FLIER, 2);
        garage.save(&directory).unwrap();

        let mut read = Garage::new();
        read.load(&directory);
        assert_eq!(read, garage, "the garage did not survive the trip");
        assert_eq!(read.owned(DRONE), 4);

        std::fs::write(directory.join("garage.dat"), b"NOT A GARAGE").unwrap();
        let mut damaged = Garage::new();
        damaged.load(&directory);
        assert_eq!(damaged.owned(DRONE), 0, "a damaged garage invented machines");

        std::fs::remove_dir_all(&directory).ok();
        let mut missing = Garage::new();
        missing.load(&directory);
        assert_eq!(missing.owned(DRONE), 0);
    }
}
