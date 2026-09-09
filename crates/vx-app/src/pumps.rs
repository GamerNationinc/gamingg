//! Which pumps are running.
//!
//! # A switch is not a thing in flight
//!
//! A pump is printed at the fabricator, carried out, placed, and switched on
//! by hand. The block itself lives in the region file like every other block,
//! so the pump survives a reload — but until stage 54 the *switch* did not.
//! `Active::pumps` was a bare `Vec<BlockPos>` initialised to empty on every
//! boot, and the census filed it under "things mid-flight" beside slugs in the
//! air and water still settling.
//!
//! That was the wrong bucket, and it is worth naming why, because the bucket
//! is how the bug survived three consecutive rounds of persistence work. A
//! slug in the air is mid-flight: the journal re-derives it from tick zero and
//! saving one would land it twice. A running pump is not in flight. It is a
//! standing decision, the same kind of thing as which optic you left the dial
//! on — and the game already saves that. So you would come back to a pump
//! sitting exactly where you built it, doing nothing, with no indication that
//! anything was wrong beyond the water having stopped.
//!
//! # Positions, not state
//!
//! What is written is the set of switched-on positions and nothing else. How
//! far through its stroke a pump is (`pump_step`) genuinely *is* mid-flight
//! and stays out; a pump that resumes at the top of its cycle lifts the same
//! water a fraction of a second later, and the journal re-derives the lift
//! either way.
//!
//! A position whose block is no longer a pump — mined out while the file sat
//! on disk, or a save carried across a worldgen change — is dropped on load
//! rather than kept as a ghost that pumps nothing.

use std::io::{Read, Write};
use std::path::Path;

use vx_core::BlockPos;

const MAGIC: &[u8; 4] = b"VXPU";
const VERSION: u32 = 1;

/// A world could hold a lot of pumps; it could not hold this many, so a count
/// past it is a corrupt file rather than an ambitious waterworks.
const MAX_PUMPS: u32 = 100_000;

/// Write down which pumps are switched on.
pub fn save(running: &[BlockPos], directory: &Path) -> std::io::Result<()> {
    let mut file = crate::keeping::begin(directory, "pumps.dat")?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&(running.len() as u32).to_le_bytes())?;
    for at in running {
        file.write_all(&at.x.to_le_bytes())?;
        file.write_all(&at.y.to_le_bytes())?;
        file.write_all(&at.z.to_le_bytes())?;
    }
    file.commit()
}

/// Read it back, keeping only the positions that are still pumps.
///
/// Tolerant like every other loader here: absent is none running, damaged is a
/// warning and none running. A world where the pumps came back off is a world
/// you switch them on again in; a world that refuses to load is not.
pub fn load(world: &vx_world::World, directory: &Path) -> Vec<BlockPos> {
    let path = directory.join("pumps.dat");
    let stored = match read(&path) {
        Ok(Some(stored)) => stored,
        Ok(None) => return Vec::new(),
        Err(error) => {
            log::warn!("ignoring damaged pumps at {}: {error}", path.display());
            return Vec::new();
        }
    };
    let Some(pump) = world.registry().id_of("engine:pump") else {
        return Vec::new();
    };
    stored
        .into_iter()
        .filter(|at| world.block(*at) == pump)
        .collect()
}

fn read(path: &Path) -> std::io::Result<Option<Vec<BlockPos>>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a pumps file"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    file.read_exact(&mut word)?;
    let count = u32::from_le_bytes(word);
    if count > MAX_PUMPS {
        return Err(std::io::Error::other("implausible pump count"));
    }
    let mut running = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut at = [0i32; 3];
        for value in &mut at {
            file.read_exact(&mut word)?;
            *value = i32::from_le_bytes(word);
        }
        running.push(BlockPos::new(at[0], at[1], at[2]));
    }
    Ok(Some(running))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("gamingg-pumps-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    fn world_with_a_pump_at(at: BlockPos) -> vx_world::World {
        let mut world = vx_world::World::new(3);
        world.load_around(ChunkPos::new(0, 0), 2);
        let pump = world.registry().id_of("engine:pump").expect("pump");
        world.set_block(at, pump);
        world
    }

    /// The bug, as a test: a pump you switched on is still on when you come
    /// back to it.
    #[test]
    fn a_running_pump_is_still_running_after_a_reload() {
        let directory = scratch("round-trip");
        let at = BlockPos::new(2, 64, -3);
        let world = world_with_a_pump_at(at);

        save(&[at], &directory).expect("save");
        assert_eq!(load(&world, &directory), vec![at]);
    }

    /// And one you switched off stays off, which is the same promise.
    #[test]
    fn a_pump_nobody_switched_on_comes_back_off() {
        let directory = scratch("empty");
        let world = world_with_a_pump_at(BlockPos::new(2, 64, -3));
        save(&[], &directory).expect("save");
        assert!(load(&world, &directory).is_empty());
    }

    /// A position that is no longer a pump is not a pump. Mine one out while
    /// its position sits in the file and it does not come back as a ghost
    /// lifting water out of thin air.
    #[test]
    fn a_position_that_is_no_longer_a_pump_is_dropped() {
        let directory = scratch("ghost");
        let at = BlockPos::new(2, 64, -3);
        let mut world = world_with_a_pump_at(at);
        save(&[at], &directory).expect("save");

        let air = world.registry().id_of("engine:air").expect("air");
        world.set_block(at, air);
        assert!(
            load(&world, &directory).is_empty(),
            "a mined-out pump came back running"
        );
    }

    #[test]
    fn a_missing_or_damaged_file_is_no_pumps_running() {
        let directory = scratch("damaged");
        let world = world_with_a_pump_at(BlockPos::new(2, 64, -3));
        assert!(load(&world, &directory).is_empty(), "absence invented a pump");

        std::fs::write(directory.join("pumps.dat"), b"not a pumps file").expect("write");
        assert!(load(&world, &directory).is_empty(), "damage invented a pump");

        let mut future = MAGIC.to_vec();
        future.extend_from_slice(&99u32.to_le_bytes());
        std::fs::write(directory.join("pumps.dat"), future).expect("write");
        assert!(load(&world, &directory).is_empty());
    }
}
