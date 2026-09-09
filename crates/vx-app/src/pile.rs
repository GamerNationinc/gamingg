//! The fleet's base pile, on disk.
//!
//! # Why this file exists
//!
//! Everything else a player owns survived a save. The wallet did
//! (`wallet.dat`), the town's books did — including a price your own selling
//! moved (`economy.dat`) — and the chest in the house did, contents and all
//! (`homestead.dat`). The one pile the shop actually sells out of did not.
//!
//! `Fleet` has no save of any kind anywhere in `vx-agent`, and
//! `App::save_world` names the tank, the wear ledger and the wells and stops.
//! So `Mining::default()` came back on load with `base: None`, and a player who
//! mined, saved and reloaded found the goods gone — *and* the base undeclared,
//! which meant the very next block they cut evaporated too, exactly the silent
//! loss stage 48 closed, reappearing across a save boundary. The container
//! block was still standing in the region file the whole time; the pile lived
//! in the struct.
//!
//! # Why it is not a method on `Fleet`
//!
//! `vx-agent` knows nothing about save directories and should not start now —
//! it is the simulation side of the networking seam. The same line
//! [`crate::wear`] holds by living here while keying on `MachineRef`.
//!
//! # Why it matters more than the goods
//!
//! The pile is what the fuel tank burns out of: `Mining::fuelled` draws HHO
//! from `base.stockpile`, and a tick the fleet cannot pay for is a tick nobody
//! works. So a pile that forgets is a fleet that stops, and a fleet that stops
//! cuts different ground — the same argument [`crate::fuel`] and
//! [`crate::wear`] make for living inside the replayed simulation rather than
//! beside it.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::Fleet;
use vx_core::BlockPos;

const MAGIC: &[u8; 4] = b"VXBP";
const VERSION: u32 = 2;

/// Longest good name accepted, so a damaged file cannot ask for a huge buffer.
const MAX_NAME: u32 = 64;

/// And a cap on how many kinds a pile can hold: more than the registry has,
/// and far short of an allocation a damaged file could weaponise.
const MAX_ROWS: u32 = 4_096;

/// Write the base and its pile to `pile.dat`.
///
/// A fleet with no base writes a present-flag of zero — "there is no pile" is
/// a fact worth recording, not an absence to be inferred from a missing file.
///
/// Version 2 adds the **orphaned** goods: what a broken container was holding,
/// waiting for a new one. Breaking your own container used to destroy
/// everything in it, so there was nothing to write; since stage 52 the goods
/// are kept and they have to survive a save like everything else. A version-1
/// file simply has none, which is exactly true of a save written before the
/// goods were kept.
pub fn save(fleet: &Fleet, directory: &Path) -> std::io::Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(directory.join("pile.dat"))?);
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    match &fleet.base {
        Some(base) => {
            file.write_all(&[1u8])?;
            file.write_all(&base.position.x.to_le_bytes())?;
            file.write_all(&base.position.y.to_le_bytes())?;
            file.write_all(&base.position.z.to_le_bytes())?;
            let rows: Vec<(&str, u64)> = base.stockpile.entries().collect();
            file.write_all(&(rows.len() as u32).to_le_bytes())?;
            for (name, count) in rows {
                file.write_all(&(name.len() as u32).to_le_bytes())?;
                file.write_all(name.as_bytes())?;
                file.write_all(&count.to_le_bytes())?;
            }
        }
        None => file.write_all(&[0u8])?,
    }
    write_rows(&mut file, fleet.orphaned())?;
    file.flush()
}

/// A stockpile's rows. Off a `BTreeMap`, so already sorted and stable.
fn write_rows(file: &mut impl Write, pile: &vx_agent::Stockpile) -> std::io::Result<()> {
    let rows: Vec<(&str, u64)> = pile.entries().collect();
    file.write_all(&(rows.len() as u32).to_le_bytes())?;
    for (name, count) in rows {
        file.write_all(&(name.len() as u32).to_le_bytes())?;
        file.write_all(name.as_bytes())?;
        file.write_all(&count.to_le_bytes())?;
    }
    Ok(())
}

/// Read it back, tolerating absence and damage.
///
/// No file means a fleet that never declared a base — a fresh world, or a save
/// written before this round — and the fleet is left exactly as it was. A
/// damaged file is logged and ignored rather than taking the world down, the
/// same bargain every other loader here makes.
pub fn load(fleet: &mut Fleet, directory: &Path) {
    let path = directory.join("pile.dat");
    match read(&path) {
        Ok(Some(stored)) => {
            if let Some((at, goods)) = stored.base {
                fleet.set_base(at);
                if let Some(base) = fleet.base.as_mut() {
                    for (name, count) in goods {
                        base.stockpile.add(name, count);
                    }
                }
            }
            // Goods with no container go back into holding. If the loop above
            // just declared a base, `set_base` has already drained whatever
            // was orphaned before, so these land in the orphan slot and wait
            // for the *next* container — which is what the file says happened.
            fleet.orphan_goods(stored.orphan);
        }
        Ok(None) => {}
        Err(error) => log::warn!("ignoring damaged pile at {}: {error}", path.display()),
    }
}

/// What the file holds: the declared base and its goods, if any, and whatever
/// is waiting for a container.
struct Stored {
    base: Option<(BlockPos, Vec<(String, u64)>)>,
    orphan: Vec<(String, u64)>,
}

fn read(path: &Path) -> std::io::Result<Option<Stored>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a pile file"));
    }
    let version = read_u32(&mut file)?;
    if version == 0 || version > VERSION {
        return Ok(None);
    }
    let mut present = [0u8; 1];
    file.read_exact(&mut present)?;
    let base = if present[0] == 0 {
        None
    } else {
        let at = BlockPos::new(
            read_i32(&mut file)?,
            read_i32(&mut file)?,
            read_i32(&mut file)?,
        );
        Some((at, read_rows(&mut file)?))
    };
    // Version 1 stops here and has no orphaned goods, which is the truth: a
    // save from before stage 52 was written by a build that destroyed them.
    let orphan = if version >= 2 {
        read_rows(&mut file)?
    } else {
        Vec::new()
    };
    Ok(Some(Stored { base, orphan }))
}

fn read_rows(file: &mut impl Read) -> std::io::Result<Vec<(String, u64)>> {
    let rows = read_u32(file)?;
    if rows > MAX_ROWS {
        return Err(std::io::Error::other("implausible pile manifest"));
    }
    let mut goods = Vec::with_capacity(rows.min(1024) as usize);
    for _ in 0..rows {
        let length = read_u32(file)?;
        if length > MAX_NAME {
            return Err(std::io::Error::other("implausible good name"));
        }
        let mut name = vec![0u8; length as usize];
        file.read_exact(&mut name)?;
        let name = String::from_utf8(name)
            .map_err(|_| std::io::Error::other("a good's name is not text"))?;
        let mut count = [0u8; 8];
        file.read_exact(&mut count)?;
        goods.push((name, u64::from_le_bytes(count)));
    }
    Ok(goods)
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_i32(file: &mut impl Read) -> std::io::Result<i32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(i32::from_le_bytes(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("vx-pile-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn a_pile_round_trips_through_disk() {
        let directory = scratch("round");
        let mut fleet = Fleet::new();
        fleet.set_base(BlockPos::new(-16, 73, 8));
        if let Some(base) = fleet.base.as_mut() {
            base.stockpile.add("engine:copper_ore", 37);
            base.stockpile.add("engine:stone", 4);
            base.stockpile.add("engine:hho_cell", 2);
        }
        save(&fleet, &directory).unwrap();

        let mut read_back = Fleet::new();
        load(&mut read_back, &directory);
        std::fs::remove_dir_all(&directory).ok();

        let base = read_back.base.expect("no base came back");
        assert_eq!(base.position, BlockPos::new(-16, 73, 8));
        assert_eq!(base.stockpile.count("engine:copper_ore"), 37);
        assert_eq!(base.stockpile.count("engine:stone"), 4);
        assert_eq!(base.stockpile.count("engine:hho_cell"), 2);
        assert_eq!(base.stockpile.total(), 43);
    }

    /// A fleet that never declared a base says so, rather than the reader
    /// having to guess from a missing file.
    #[test]
    fn a_fleet_with_no_base_writes_that_it_has_none() {
        let directory = scratch("nobase");
        let fleet = Fleet::new();
        save(&fleet, &directory).unwrap();

        let mut read_back = Fleet::new();
        read_back.set_base(BlockPos::new(1, 2, 3));
        load(&mut read_back, &directory);
        std::fs::remove_dir_all(&directory).ok();
        // Loading a "no base" file leaves the fleet alone rather than
        // clearing it: `load` restores what was saved, and a save that had
        // nothing to restore restores nothing.
        assert!(read_back.base.is_some());
    }

    #[test]
    fn a_missing_or_damaged_pile_is_no_pile_at_all() {
        let directory = scratch("damaged");
        let mut fleet = Fleet::new();
        load(&mut fleet, &directory);
        assert!(fleet.base.is_none(), "a missing file invented a base");

        std::fs::write(directory.join("pile.dat"), b"NOPE and then some").unwrap();
        let mut fleet = Fleet::new();
        load(&mut fleet, &directory);
        std::fs::remove_dir_all(&directory).ok();
        assert!(fleet.base.is_none(), "a damaged file invented a base");
    }

    /// The rows come off a `BTreeMap`, so the bytes are the same twice — a
    /// save that churned would make every quit a fresh write.
    #[test]
    fn the_same_pile_writes_the_same_bytes() {
        let directory = scratch("stable");
        let mut fleet = Fleet::new();
        fleet.set_base(BlockPos::new(0, 70, 0));
        if let Some(base) = fleet.base.as_mut() {
            base.stockpile.add("engine:stone", 1);
            base.stockpile.add("engine:copper_ore", 2);
            base.stockpile.add("engine:log", 3);
        }
        save(&fleet, &directory).unwrap();
        let first = std::fs::read(directory.join("pile.dat")).unwrap();
        save(&fleet, &directory).unwrap();
        let second = std::fs::read(directory.join("pile.dat")).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(first, second);
    }
}
