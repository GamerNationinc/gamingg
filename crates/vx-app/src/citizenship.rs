//! Citizenship of the Ruined City: ten thousand credits, once, for ever.
//!
//! What the money buys is a gate. The city's modern trace is stamped with its
//! four gateways shut — see [`vx_world::fort::Fort::sealed`] — and paying
//! writes air into them. That makes enrolling an order on the journal like
//! everything else that moves a block, and it makes the gate an *edit* rather
//! than a change to worldgen, for the reason `masonry.rs` gives: the ground
//! stays pure in the seed, and the decision lives in a file of its own.
//!
//! No ledger of which gates have been opened. [`open_the_gates`] is
//! idempotent and costs a few hundred block reads, so it is simply run when
//! you pay and once a dispatch window after that from the one-copy network
//! tick. A gate whose chunk was not resident when you paid opens within a
//! window of your arriving at it.

use std::io::{Read, Write};
use std::path::Path;

use vx_world::town::TownSite;
use vx_world::world::World;

const MAGIC: &[u8; 4] = b"VXCZ";
const VERSION: u32 = 1;

/// What citizenship costs. Ridiculous on purpose: the city is a place you
/// earn your way into.
pub const PRICE: u64 = 10_000;

/// Write air into every gate block of the city's modern trace whose chunk is
/// resident. Returns how many blocks it opened; zero the second time.
pub fn open_the_gates(world: &mut World, city: &TownSite) -> usize {
    let mut opened = 0;
    for at in vx_world::fort::fort_for(city).gate_cells() {
        if world.is_loaded(at.chunk()) && world.block(at) != vx_core::BlockId::AIR {
            world.set_block(at, vx_core::BlockId::AIR);
            opened += 1;
        }
    }
    opened
}

/// Is this column inside the city's core — the ground citizenship covers?
pub fn inside(city: &TownSite, x: i32, z: i32) -> bool {
    (x - city.centre.0).abs() <= city.core_half && (z - city.centre.1).abs() <= city.core_half
}

pub fn save(enrolled: bool, directory: &Path) -> std::io::Result<()> {
    let mut file = std::fs::File::create(directory.join("citizenship.dat"))?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&[u8::from(enrolled)])
}

/// Whether the player is a citizen. A missing or damaged file means no: the
/// worst that costs is the price of the gate, which is a lot, so a damaged
/// file is logged rather than silently read as a refusal.
pub fn load(directory: &Path) -> bool {
    match read(&directory.join("citizenship.dat")) {
        Ok(enrolled) => enrolled,
        Err(error) => {
            log::warn!("ignoring damaged citizenship file: {error}");
            false
        }
    }
}

fn read(path: &Path) -> std::io::Result<bool> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
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
    let mut enrolled = [0u8; 1];
    file.read_exact(&mut enrolled)?;
    Ok(enrolled[0] != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("vx-citizenship-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn citizenship_survives_a_save_and_a_missing_file_means_no() {
        let directory = scratch("roundtrip");
        assert!(!load(&directory));
        save(true, &directory).unwrap();
        assert!(load(&directory));
        save(false, &directory).unwrap();
        assert!(!load(&directory));
        std::fs::write(directory.join("citizenship.dat"), b"garbage").unwrap();
        assert!(!load(&directory));
    }

    /// Paying opens exactly the gate and nothing else, once.
    #[test]
    fn opening_the_gates_writes_air_where_the_gate_stood_and_nowhere_else() {
        let mut world = World::new(2024);
        let city = world.city();
        let centre = vx_core::BlockPos::new(city.centre.0, city.ground, city.centre.1).chunk();
        world.load_around(centre, 4);
        let cells = vx_world::fort::fort_for(&city).gate_cells();
        assert!(!cells.is_empty());
        for at in &cells {
            assert_ne!(world.block(*at), vx_core::BlockId::AIR, "the gate was open at {at:?}");
        }
        let before = vx_world::world_hash(&world);

        let opened = open_the_gates(&mut world, &city);
        assert_eq!(opened, cells.len(), "not every gate block was opened");
        for at in &cells {
            assert_eq!(world.block(*at), vx_core::BlockId::AIR, "still shut at {at:?}");
        }
        assert_ne!(vx_world::world_hash(&world), before, "opening the gate moved no ground");
        assert_eq!(open_the_gates(&mut world, &city), 0, "the gate opened twice");

        // Ground that is not resident is left alone, not invented.
        let mut empty = World::new(2024);
        assert_eq!(open_the_gates(&mut empty, &city), 0);
        assert_eq!(empty.loaded_chunks().count(), 0, "opening a gate loaded a chunk");
    }

    #[test]
    fn the_core_is_the_safe_ground() {
        let city = World::new(2024).city();
        let (cx, cz) = city.centre;
        assert!(inside(&city, cx, cz));
        assert!(inside(&city, cx + city.core_half, cz - city.core_half));
        assert!(!inside(&city, cx + city.core_half + 1, cz));
    }
}
