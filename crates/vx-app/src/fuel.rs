//! What the fleet burns, and what happens when it runs out.
//!
//! # Machines stop being perpetual
//!
//! Until this round a drone dug forever for nothing. That made every other
//! cost in the game a one-off: buy the machine, and it works free until the
//! sun burns out. A fuel is the first *running* cost, and a running cost is
//! what turns a stockpile into a supply line — the difference between "can I
//! afford this" and "can I keep this going".
//!
//! # Oxyhydrogen
//!
//! The fuel is water taken apart and handed back: two parts hydrogen to one
//! of oxygen, split by electrodes at [`crate::electrolysis`] and burned back
//! into water in the machines. Which means the loop closes on something the
//! world already has an ocean of — the cost was never the water, it is the
//! electrodes, the time, and being somewhere with a lake.
//!
//! # Why the tank lives with the fleet and burns inside `advance`
//!
//! Fuel decides how much ground gets dug, and ground is what the world hash
//! covers — so a tank that behaved differently on replay would be a
//! divergence with the fleet's name on it. The tank therefore lives on
//! `Mining`, which replay carries, and it is drawn and burned inside
//! `Mining::advance`, which is the very call `Command::Advance` replays. No
//! new order is needed for fuelling: both sides run the same code over the
//! same pile for the same number of ticks, and arrive at the same tank.

use std::io::{Read, Write};
use std::path::Path;

/// The good a tank is filled from.
pub const CELL: &str = "engine:hho_cell";

/// The other good a tank is filled from: raw well gas, compressed at the
/// wellhead.
///
/// # Why a second fuel exists at all
///
/// Oxyhydrogen has one problem as the only fuel in the game: it needs a
/// lake. That made every long-range dig a haulage question with exactly one
/// answer — go back to the water. Well gas is the answer for somebody who
/// would rather sink a hole where they already are. It is worse fuel, and
/// deliberately: a canister of it buys about three fifths of what a canister
/// of oxyhydrogen does, because it is what came out of the ground rather
/// than something anybody made.
pub const GAS: &str = "engine:gas_cell";

/// How many machine-ticks one canister is worth.
///
/// Five minutes of one machine at the journal's eight ticks a second. A crew
/// of four gets through a cell in a minute and a quarter, which is the number
/// that decides whether a dig is a trip to the lake or an afternoon.
pub const TICKS_PER_CELL: u32 = 2_400;

/// What a canister of well gas is worth, in the same machine-ticks. Three
/// fifths of oxyhydrogen: cheaper to come by, and it shows.
pub const TICKS_PER_GAS: u32 = 1_440;

/// Below this many machine-ticks the readout starts warning.
const LOW: u32 = TICKS_PER_CELL / 2;

const MAGIC: &[u8; 4] = b"VXFU";
const VERSION: u32 = 1;

/// The fleet's tank, in machine-ticks rather than canisters.
///
/// Machine-ticks because a crew of six burns six times as fast as a lone
/// drone, and counting whole cells would make that unrepresentable without
/// fractions — which are exactly what a determinism argument does not want.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tank {
    /// Machine-ticks left in the tank.
    pub charge: u32,
    /// Did the fleet want fuel this tick and find none?
    pub dry: bool,
}

impl Tank {
    /// Burn one tick's worth for `machines`, drawing on `pile` as needed.
    ///
    /// Returns whether the fleet may work this tick. Drawing happens here
    /// rather than at a refuelling station because the pile *is* the fleet's
    /// store: the flier ferries into it, the shop sells out of it, and a
    /// machine standing next to a canister it is not allowed to open would
    /// be a rule with nothing behind it.
    pub fn burn(&mut self, machines: u32, pile: &mut vx_agent::Stockpile) -> bool {
        if machines == 0 {
            self.dry = false;
            return true;
        }
        while self.charge < machines {
            // Oxyhydrogen first, always. It is the better fuel and the one a
            // player went to the trouble of making, so burning the well gas
            // while good cells sit in the pile would be the machine
            // second-guessing them. Gas is what it falls back on.
            if pile.take(CELL, 1) > 0 {
                self.charge += TICKS_PER_CELL;
            } else if pile.take(GAS, 1) > 0 {
                self.charge += TICKS_PER_GAS;
            } else {
                self.dry = true;
                return false;
            }
        }
        self.charge -= machines;
        self.dry = false;
        true
    }

    /// Whole canisters' worth still in the tank.
    pub fn cells(&self) -> u32 {
        self.charge / TICKS_PER_CELL
    }

    /// What the HUD says, or nothing when there is no fleet to fuel.
    pub fn readout(&self, machines: u32, spare_cells: u64) -> Option<String> {
        if machines == 0 && self.charge == 0 && spare_cells == 0 {
            return None;
        }
        if self.dry {
            return Some("FLEET DRY - NO FUEL".to_string());
        }
        let held = self.cells() as u64 + spare_cells;
        if self.charge < LOW && held == 0 {
            return Some("HHO LOW".to_string());
        }
        Some(format!("HHO {held}"))
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "fuel.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&self.charge.to_le_bytes())?;
        file.commit()
    }

    /// Load the tank, tolerating absence and damage.
    ///
    /// Persisted, and it has to be: replay re-derives the tank from tick zero,
    /// so a session that reloaded with an empty tank would burn out at a
    /// different tick than its own journal says it did — and the ground would
    /// disagree by exactly the digging that difference bought.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("fuel.dat");
        match read_tank(&path) {
            Ok(Some(charge)) => self.charge = charge,
            Ok(None) => {}
            Err(error) => log::warn!("unreadable {}: {error}", path.display()),
        }
    }
}

fn read_tank(path: &Path) -> std::io::Result<Option<u32>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a fuel file"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    file.read_exact(&mut word)?;
    Ok(Some(u32::from_le_bytes(word)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pile(cells: u64) -> vx_agent::Stockpile {
        let mut pile = vx_agent::Stockpile::new();
        if cells > 0 {
            pile.add(CELL, cells);
        }
        pile
    }

    #[test]
    fn a_crew_burns_faster_than_a_single_machine() {
        let mut alone = Tank::default();
        let mut crew = Tank::default();
        let mut one_pile = pile(1);
        let mut crew_pile = pile(1);
        for _ in 0..100 {
            assert!(alone.burn(1, &mut one_pile));
            assert!(crew.burn(4, &mut crew_pile));
        }
        assert_eq!(
            TICKS_PER_CELL - alone.charge,
            (TICKS_PER_CELL - crew.charge) / 4,
            "four machines did not burn four times the fuel"
        );
    }

    #[test]
    fn an_empty_pile_stops_the_fleet_and_says_so() {
        let mut tank = Tank::default();
        let mut empty = pile(0);
        assert!(!tank.burn(2, &mut empty), "a dry fleet kept working");
        assert!(tank.dry);
        assert_eq!(tank.readout(2, 0).as_deref(), Some("FLEET DRY - NO FUEL"));

        // And it picks straight back up when a canister arrives.
        let mut stocked = pile(1);
        assert!(tank.burn(2, &mut stocked));
        assert!(!tank.dry);
        assert_eq!(stocked.count(CELL), 0, "the canister was not drawn");
    }

    #[test]
    fn one_canister_runs_one_machine_for_its_whole_charge() {
        let mut tank = Tank::default();
        let mut stock = pile(1);
        for tick in 0..TICKS_PER_CELL {
            assert!(tank.burn(1, &mut stock), "ran dry early at tick {tick}");
        }
        assert!(!tank.burn(1, &mut stock), "a canister outlasted its charge");
    }

    #[test]
    fn nothing_burns_when_nothing_is_working() {
        let mut tank = Tank::default();
        let mut stock = pile(1);
        for _ in 0..500 {
            assert!(tank.burn(0, &mut stock));
        }
        assert_eq!(tank.charge, 0, "an idle fleet drew fuel it never used");
        assert_eq!(stock.count(CELL), 1);
    }

    #[test]
    fn the_tank_survives_a_save() {
        let directory = std::env::temp_dir().join(format!("vx-fuel-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut tank = Tank::default();
        let mut stock = pile(2);
        for _ in 0..900 {
            tank.burn(3, &mut stock);
        }
        tank.save(&directory).unwrap();

        let mut loaded = Tank::default();
        loaded.load(&directory);
        assert_eq!(loaded.charge, tank.charge);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn every_readout_is_drawable() {
        let mut tank = Tank::default();
        for machines in [0u32, 1, 5] {
            for spare in [0u64, 3] {
                if let Some(line) = tank.readout(machines, spare) {
                    for character in line.chars() {
                        assert!(vx_render::font::knows(character), "undrawable {character:?}");
                    }
                }
            }
        }
        tank.dry = true;
        for character in tank.readout(1, 0).unwrap().chars() {
            assert!(vx_render::font::knows(character));
        }
    }

    #[test]
    fn well_gas_fuels_the_fleet_when_the_good_stuff_runs_out() {
        // The fallback order is the whole design: a player who has both
        // should burn what they made, and a player who has only what came
        // out of a hole should still get to dig.
        let mut pile = vx_agent::Stockpile::default();
        pile.add(CELL, 1);
        pile.add(GAS, 1);

        let mut tank = Tank::default();
        assert!(tank.burn(1, &mut pile));
        assert_eq!(pile.count(CELL), 0, "the tank did not reach for the HHO first");
        assert_eq!(pile.count(GAS), 1, "the gas went first");

        // Run the oxyhydrogen out, then the gas has to carry it.
        tank.charge = 0;
        assert!(tank.burn(1, &mut pile));
        assert_eq!(pile.count(GAS), 0);
        assert_eq!(tank.charge, TICKS_PER_GAS - 1);

        // And gas is honestly worse — pinned at build time, because the
        // fallback order above only makes sense while it is true.
        const { assert!(TICKS_PER_GAS < TICKS_PER_CELL) };

        // Nothing left: dry, and it says so.
        tank.charge = 0;
        assert!(!tank.burn(1, &mut pile));
        assert!(tank.dry);
    }
}
