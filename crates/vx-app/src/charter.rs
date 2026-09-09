//! Towns somebody founded.
//!
//! # The third route into office, and the gap it had to cross
//!
//! The civic note named three ways to hold a town's chair: win it at the
//! ballot box, **found a town and hold its offices by default**, or take one.
//! Stage 40 shipped the first. This module is the second.
//!
//! Every civic system — the roster, the offices, the market, the ballot, the
//! permits, the schedule, the wall — takes a [`TownSite`] and keys on its
//! centre or its seed. A town that is not on the lattice slots into all of
//! them unchanged. The one thing that could not see such a town was worldgen,
//! which is pure in `(seed, cell)` and must stay so. The project's idiom for
//! that is the one every round since 36 has used: **a stored ledger whose
//! effect on the ground is a journaled edit**. Stands, wells, the fluid and
//! fire are all that shape, and so is a charter.
//!
//! # Derived, not authored
//!
//! A charter stores a [`TownSite`] and the tick it was filed. The site is
//! built by [`site_at`] exactly the way the lattice builds its own — ground
//! from the natural height clamped the same way, the hometown's core width,
//! a seed hashed off the world's and the centre — so everything downstream
//! falls out of that seed: three settlers and their trades, the market's
//! opening books, the fort's trace. A founded town is named from the same
//! book every other town is named from, and nothing past this module can
//! tell which kind of town it is looking at. That is the point.
//!
//! # What is stored
//!
//! Only what cannot be derived: the sites founded, when, and how many
//! charters have been printed and not yet filed. The ground itself is the
//! journal's business — `Command::Found` raises it on both sides of the
//! oracle — and this ledger is the bookkeeping that lives beside it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use vx_world::town::{self, Speciality, TownName, TownSite};
use vx_world::World;

const MAGIC: &[u8; 4] = b"VXCH";
const VERSION: u32 = 1;

/// Salted well clear of the lattice's own per-town stream, so a founded
/// town's people are not quietly the people of whatever cell it sits in.
const FOUNDED_SALT: u64 = 0xc4a7_7e5e_d000_0001;

/// The founder's due, in the same points the ballot counts trade in.
///
/// A settler owes the founder the roof over their head, so the first polls
/// do not throw out a mayor who has not yet traded with anybody. Sized to
/// carry a seat against the incumbency bonus with nothing else on the
/// sheet, and no more: trade still buys the rest, and a founder who never
/// opens the shop is a founder the town can tire of.
pub const FOUNDER: i64 = 50;

/// One town somebody founded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Charter {
    pub site: TownSite,
    /// The tick it was filed on.
    pub founded: u64,
}

/// Why a plot cannot be chartered, in the words the terminal says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Another town is within a lattice cell; its name.
    Close(String),
    Steep,
    Wet,
}

impl Refusal {
    pub fn line(&self) -> String {
        match self {
            Refusal::Close(name) => format!("TOO CLOSE TO {name}"),
            Refusal::Steep => "TOO STEEP - THE PLOT CANNOT BE LEVELLED".to_string(),
            Refusal::Wet => "TOO WET - FOUND ON DRY GROUND".to_string(),
        }
    }
}

/// Every town founded, and the charters waiting to be filed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Charters {
    towns: BTreeMap<(i32, i32), Charter>,
    /// Charters printed and not yet used.
    pub unfiled: u32,
}

/// The site a charter at `centre` would found, built the way the lattice
/// builds its own.
///
/// Pure in its arguments, which is what lets the live game and the replay
/// raise the same town from the same order.
pub fn site_at(
    world_seed: u64,
    centre: (i32, i32),
    name: TownName,
    natural: &impl Fn(i32, i32) -> i32,
) -> TownSite {
    let ground = natural(centre.0, centre.1);
    TownSite {
        centre,
        ground: ground.clamp(vx_world::SEA_LEVEL + town::MIN_DRY + 1, 140),
        core_half: town::HOME_CORE_HALF,
        // A founded town is a depot until somebody gives it a reason to be
        // anything else.
        speciality: Speciality::Depot,
        name,
        seed: vx_world::seed::finalise(
            world_seed
                ^ FOUNDED_SALT
                ^ (centre.0 as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (centre.1 as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f),
        ),
    }
}

/// Could a town be founded here?
///
/// The lattice's own rules, asked of the ground the player stands on: a
/// full cell clear of every town — the lattice's and the founded ones —
/// dry, and flat enough to level. Refusals come back in words.
pub fn may_found(world: &World, at: (i32, i32), name: TownName) -> Result<TownSite, Refusal> {
    // Nearest first, so the refusal names the town that is actually in the
    // way rather than one further off.
    if let Some(neighbour) = world.towns_near(at, town::CELL).first() {
        return Err(Refusal::Close(neighbour.name.to_string()));
    }
    let generator = world.generator();
    let natural = |x: i32, z: i32| generator.natural_height_at(x, z);
    if natural(at.0, at.1) <= vx_world::SEA_LEVEL + town::MIN_DRY {
        return Err(Refusal::Wet);
    }
    if !town::buildable(&natural, at, town::HOME_CORE_HALF) {
        return Err(Refusal::Steep);
    }
    Ok(site_at(world.seed(), at, name, &natural))
}

impl Charters {
    /// The charter for a town, if it was founded rather than drawn.
    pub fn get(&self, town: (i32, i32)) -> Option<&Charter> {
        self.towns.get(&town)
    }

    /// Was this town founded by the player?
    pub fn founded(&self, town: (i32, i32)) -> bool {
        self.towns.contains_key(&town)
    }

    /// A charter came off the printer.
    pub fn print(&mut self) {
        self.unfiled += 1;
    }

    /// File a charter: the town is founded. Consumes one unfiled charter;
    /// returns whether there was one to consume.
    pub fn file(&mut self, site: TownSite, tick: u64) -> bool {
        if self.unfiled == 0 {
            return false;
        }
        self.unfiled -= 1;
        self.towns
            .entry(site.centre)
            .or_insert(Charter { site, founded: tick });
        true
    }

    /// Tell a world about every town in the ledger, for a load: the ground
    /// is already in the region files, so nothing is raised — the world just
    /// has to answer for them.
    pub fn tell(&self, world: &mut World) {
        for charter in self.towns.values() {
            world.found(charter.site);
        }
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "charters.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&self.unfiled.to_le_bytes())?;
        file.write_all(&(self.towns.len() as u32).to_le_bytes())?;
        for charter in self.towns.values() {
            let site = charter.site;
            let (head, tail) = site.name.indices();
            file.write_all(&site.centre.0.to_le_bytes())?;
            file.write_all(&site.centre.1.to_le_bytes())?;
            file.write_all(&site.ground.to_le_bytes())?;
            file.write_all(&site.core_half.to_le_bytes())?;
            file.write_all(&[speciality_byte(site.speciality), head, tail, 0])?;
            file.write_all(&site.seed.to_le_bytes())?;
            file.write_all(&charter.founded.to_le_bytes())?;
        }
        file.commit()
    }

    /// A missing or unreadable file is a frontier nobody has founded
    /// anything on, not an error.
    pub fn load(&mut self, directory: &Path) {
        let Ok(mut file) = std::fs::File::open(directory.join("charters.dat")) else {
            return;
        };
        let mut magic = [0u8; 4];
        let mut word = [0u8; 4];
        if file.read_exact(&mut magic).is_err() || &magic != MAGIC {
            return;
        }
        if file.read_exact(&mut word).is_err() || u32::from_le_bytes(word) != VERSION {
            return;
        }
        if file.read_exact(&mut word).is_err() {
            return;
        }
        let unfiled = u32::from_le_bytes(word);
        if file.read_exact(&mut word).is_err() {
            return;
        }
        let count = u32::from_le_bytes(word);
        let mut towns = BTreeMap::new();
        for _ in 0..count {
            let mut ints = [[0u8; 4]; 4];
            let mut tags = [0u8; 4];
            let mut seed = [0u8; 8];
            let mut founded = [0u8; 8];
            for int in &mut ints {
                if file.read_exact(int).is_err() {
                    return;
                }
            }
            if file.read_exact(&mut tags).is_err()
                || file.read_exact(&mut seed).is_err()
                || file.read_exact(&mut founded).is_err()
            {
                return;
            }
            let site = TownSite {
                centre: (i32::from_le_bytes(ints[0]), i32::from_le_bytes(ints[1])),
                ground: i32::from_le_bytes(ints[2]),
                core_half: i32::from_le_bytes(ints[3]),
                speciality: speciality_from(tags[0]),
                name: TownName::from_indices(tags[1], tags[2]),
                seed: u64::from_le_bytes(seed),
            };
            towns.insert(
                site.centre,
                Charter {
                    site,
                    founded: u64::from_le_bytes(founded),
                },
            );
        }
        self.unfiled = unfiled;
        self.towns = towns;
    }
}

fn speciality_byte(speciality: Speciality) -> u8 {
    match speciality {
        Speciality::Depot => 0,
        Speciality::Mine => 1,
        Speciality::Refinery => 2,
    }
}

fn speciality_from(byte: u8) -> Speciality {
    match byte {
        1 => Speciality::Mine,
        2 => Speciality::Refinery,
        _ => Speciality::Depot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;

    fn name() -> TownName {
        TownName::from_words("iron", "reach").unwrap()
    }

    /// The lattice's rules, asked of arbitrary ground: too close to the
    /// hometown, and fine a cell out on flat dry ground. Pure in
    /// `(world, at)` — asking twice is the same answer.
    #[test]
    fn founding_is_refused_beside_a_town_and_allowed_a_cell_away() {
        let world = World::new(2024);
        match may_found(&world, (60, 40), name()) {
            Err(Refusal::Close(who)) => assert_eq!(who, "STONEHAVEN"),
            other => panic!("founding beside the hometown was {other:?}"),
        }
        // Walk east a cell at a time until the lattice has left a gap that
        // is dry and flat; there is always one within a few cells.
        let mut found = None;
        for step in 1..12 {
            let at = (step * town::CELL + 60, 40);
            if let Ok(site) = may_found(&world, at, name()) {
                found = Some((at, site));
                break;
            }
        }
        let (at, site) = found.expect("no chartable ground within twelve cells");
        assert_eq!(site.centre, at);
        assert_eq!(site.name.to_string(), "IRONREACH");
        assert_eq!(site.speciality, Speciality::Depot);
        assert_eq!(site.core_half, town::HOME_CORE_HALF);
        assert_eq!(may_found(&world, at, name()), Ok(site), "may_found is not pure");
        // And its seed is its own: not the hometown's, not a lattice cell's.
        assert_ne!(site.seed, 0);
        assert_ne!(site.seed, world.seed());
    }

    /// Steep and wet ground are refused in words.
    #[test]
    fn a_cliff_and_a_shore_are_refused() {
        let world = World::new(2024);
        let generator = world.generator();
        let natural = |x: i32, z: i32| generator.natural_height_at(x, z);
        let mut steep = None;
        let mut wet = None;
        for cell_x in 1..40 {
            for cell_z in -6..6 {
                let at = (cell_x * town::CELL + 100, cell_z * town::CELL + 100);
                if !world.towns_near(at, town::CELL).is_empty() {
                    continue;
                }
                if natural(at.0, at.1) <= vx_world::SEA_LEVEL + town::MIN_DRY {
                    wet.get_or_insert(at);
                } else if !town::buildable(&natural, at, town::HOME_CORE_HALF) {
                    steep.get_or_insert(at);
                }
            }
        }
        if let Some(at) = wet {
            assert_eq!(may_found(&world, at, name()), Err(Refusal::Wet));
        }
        if let Some(at) = steep {
            assert_eq!(may_found(&world, at, name()), Err(Refusal::Steep));
        }
        assert!(wet.is_some() || steep.is_some(), "the whole country is flat and dry");
        assert_eq!(Refusal::Steep.line(), "TOO STEEP - THE PLOT CANNOT BE LEVELLED");
    }

    /// A charter is consumed by filing, and filing without one is refused.
    #[test]
    fn a_charter_is_consumed_and_founding_without_one_is_refused() {
        let world = World::new(2024);
        let site = site_at(world.seed(), (town::CELL * 2, 0), name(), &|_, _| 90);
        let mut charters = Charters::default();
        assert!(!charters.file(site, 10), "founded with no charter");
        charters.print();
        assert_eq!(charters.unfiled, 1);
        assert!(charters.file(site, 10));
        assert_eq!(charters.unfiled, 0);
        assert!(charters.founded(site.centre));
        assert_eq!(charters.get(site.centre).map(|c| c.founded), Some(10));
        // Filing the same town twice keeps the first charter's date.
        charters.print();
        assert!(charters.file(site, 99));
        assert_eq!(charters.get(site.centre).map(|c| c.founded), Some(10));
    }

    /// The ledger survives a save, and a loaded charter town is in the
    /// world's answer before a chunk of it is touched.
    #[test]
    fn charters_survive_a_save_and_tell_the_world() {
        let directory = std::env::temp_dir().join(format!("vx-charters-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let world = World::new(2024);
        let site = site_at(world.seed(), (town::CELL * 3 + 7, -town::CELL - 9), name(), &|_, _| 88);
        let mut charters = Charters::default();
        charters.print();
        charters.print();
        charters.file(site, 4_200);
        charters.save(&directory).unwrap();

        let mut read_back = Charters::default();
        read_back.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(read_back, charters);
        assert_eq!(read_back.unfiled, 1);

        let mut fresh = World::new(2024);
        assert!(fresh.towns_near(site.centre, 10).is_empty());
        read_back.tell(&mut fresh);
        assert_eq!(fresh.towns_near(site.centre, 10).first(), Some(&site));
        assert!(fresh.chunk(ChunkPos::new(0, 0)).is_none(), "telling loaded a chunk");

        // An empty directory loads clean.
        let mut none = Charters::default();
        none.load(&std::env::temp_dir().join("vx-charters-nowhere"));
        assert!(!none.founded((0, 0)) && none.unfiled == 0);
    }
}
