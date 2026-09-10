//! The air side, on disk: your fliers, and every sector you have already swept.
//!
//! # Why this file exists
//!
//! [`crate::pile`] saved the base and stopped there, which was the whole of
//! what stage 50 had noticed. The `Fleet` it took the base off also carries
//! the fliers themselves and **`surveys` — everything the scanner has ever
//! learned** — and neither of those was written anywhere.
//!
//! That is worse than it sounds, because a sweep is not free. The flier burns
//! HHO out of the base pile to fly it ([`crate::fuel`]), so a survey is bought
//! and paid for. Saving threw the receipt away: you came back to a fleet that
//! had never scanned anything, re-flew ground you had already covered, burned
//! the fuel a second time, and had no way of knowing you were doing it. A
//! half-finished sweep — the state a player who quit mid-scan is actually in —
//! was lost in the same breath.
//!
//! # Its own file, not a version bump on `pile.dat`
//!
//! One concern per file, and a hard practical reason on top: `pile.rs`'s
//! loader returns "nothing here" on a version it does not know, so bumping it
//! would silently **erase the pile** of anybody opening a v1 save with a new
//! build. A new file cannot do that to an old one.
//!
//! # What is not in it
//!
//! `Fleet::suspended` — what a flier was doing before the player took its
//! stick. A reload hands the stick back by definition, so the flier arrives
//! idle over the base and is re-tasked like any other. Saving a suspended
//! order would resume an errand the player had already interrupted.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::{Fleet, FleetSnapshot, Flier, FlierState, Sector, Stockpile, SurveySnapshot};
use vx_core::BlockPos;

const MAGIC: &[u8; 4] = b"VXFL";
const VERSION: u32 = 1;

/// Caps, so a damaged or hand-edited file cannot ask for an enormous
/// allocation and cannot assert a fleet the game could never have produced.
const MAX_FLIERS: u32 = 4_096;
const MAX_SURVEYS: u32 = 65_536;
/// A sector is 64 blocks square, so 4,096 columns is a full one and a little
/// slack is generous.
const MAX_COLUMNS: u32 = 8_192;
const MAX_NAME: u32 = 64;
const MAX_ROWS: u32 = 4_096;
const MAX_CAPACITY: u64 = 1_000_000;
/// The scanner reaches tens of blocks down, not thousands.
const MAX_SCAN_DEPTH: i32 = 4_096;

/// Write the fliers, the scanner's reach and every survey to `fleet.dat`.
pub fn save(fleet: &Fleet, directory: &Path) -> std::io::Result<()> {
    let air = fleet.snapshot();
    let mut file = crate::keeping::begin(directory, "fleet.dat")?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&air.scan_depth.to_le_bytes())?;
    write_option_u32(&mut file, air.controlled.map(|index| index as u32))?;

    file.write_all(&(air.fliers.len() as u32).to_le_bytes())?;
    for flier in &air.fliers {
        write_pos(&mut file, flier.position)?;
        write_pos(&mut file, flier.previous_position)?;
        write_state(&mut file, flier.state)?;
        write_pile(&mut file, &flier.cargo)?;
        file.write_all(&flier.capacity.to_le_bytes())?;
    }

    // Sorted by `Fleet::snapshot`, so the same fleet writes the same bytes
    // twice however the maps happen to hash today.
    file.write_all(&(air.surveys.len() as u32).to_le_bytes())?;
    for survey in &air.surveys {
        file.write_all(&survey.sector.x.to_le_bytes())?;
        file.write_all(&survey.sector.z.to_le_bytes())?;
        file.write_all(&[u8::from(survey.complete)])?;
        file.write_all(&(survey.covered.len() as u32).to_le_bytes())?;
        for (x, z) in &survey.covered {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
        }
        file.write_all(&(survey.hits.len() as u32).to_le_bytes())?;
        for ((x, z), (depth, hover)) in &survey.hits {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&depth.to_le_bytes())?;
            file.write_all(&hover.to_le_bytes())?;
        }
    }
    file.commit()
}

/// Read it back, tolerating absence and damage.
///
/// The base is **not** touched: it is [`crate::pile`]'s concern and has been
/// since stage 50, and a save written before this round has a pile file and no
/// fleet file. So this restores the air side around whatever base is already
/// there rather than replacing the fleet wholesale.
pub fn load(fleet: &mut Fleet, directory: &Path) {
    let path = directory.join("fleet.dat");
    match read(&path) {
        Ok(Some(air)) => {
            let base = fleet.base.take();
            let orphan = fleet.orphan_take();
            *fleet = Fleet::restore(FleetSnapshot { base, orphan, ..air });
        }
        Ok(None) => {}
        Err(error) => log::warn!("ignoring damaged fleet at {}: {error}", path.display()),
    }
}

fn read(path: &Path) -> std::io::Result<Option<FleetSnapshot>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a fleet file"));
    }
    if read_u32(&mut file)? != VERSION {
        return Ok(None);
    }

    let scan_depth = read_i32(&mut file)?;
    if !(0..=MAX_SCAN_DEPTH).contains(&scan_depth) {
        return Err(std::io::Error::other("implausible scanner reach"));
    }
    let controlled = read_option_u32(&mut file)?.map(|index| index as usize);

    let count = read_u32(&mut file)?;
    if count > MAX_FLIERS {
        return Err(std::io::Error::other("implausible fleet size"));
    }
    let mut fliers = Vec::with_capacity(count.min(64) as usize);
    for _ in 0..count {
        let position = read_pos(&mut file)?;
        let previous_position = read_pos(&mut file)?;
        let state = read_state(&mut file)?;
        let cargo = read_pile(&mut file)?;
        let capacity = read_u64(&mut file)?;
        if capacity > MAX_CAPACITY {
            return Err(std::io::Error::other("implausible flier capacity"));
        }
        fliers.push(Flier {
            position,
            previous_position,
            state,
            cargo,
            capacity,
        });
    }

    let survey_count = read_u32(&mut file)?;
    if survey_count > MAX_SURVEYS {
        return Err(std::io::Error::other("implausible survey count"));
    }
    let mut surveys = Vec::with_capacity(survey_count.min(256) as usize);
    for _ in 0..survey_count {
        let sector = Sector {
            x: read_i32(&mut file)?,
            z: read_i32(&mut file)?,
        };
        let mut complete = [0u8; 1];
        file.read_exact(&mut complete)?;
        let covered_count = read_u32(&mut file)?;
        if covered_count > MAX_COLUMNS {
            return Err(std::io::Error::other("implausible covered column count"));
        }
        let mut covered = Vec::with_capacity(covered_count.min(4096) as usize);
        for _ in 0..covered_count {
            covered.push((read_i32(&mut file)?, read_i32(&mut file)?));
        }
        let hit_count = read_u32(&mut file)?;
        if hit_count > MAX_COLUMNS {
            return Err(std::io::Error::other("implausible hit count"));
        }
        let mut hits = Vec::with_capacity(hit_count.min(4096) as usize);
        for _ in 0..hit_count {
            let at = (read_i32(&mut file)?, read_i32(&mut file)?);
            hits.push((at, (read_i32(&mut file)?, read_i32(&mut file)?)));
        }
        surveys.push(SurveySnapshot {
            sector,
            covered,
            hits,
            complete: complete[0] != 0,
        });
    }

    // A stick held by a flier that is not there would panic the first time
    // anything looked for it.
    if controlled.is_some_and(|index| index >= fliers.len()) {
        return Err(std::io::Error::other("the stick is held by nobody"));
    }

    Ok(Some(FleetSnapshot {
        fliers,
        base: None,
        scan_depth,
        surveys,
        controlled,
        // The pile's concern, not the air side's — `pile.dat` carries both the
        // declared base and whatever a broken container left in holding, and
        // `load` puts this back rather than overwriting it.
        orphan: Stockpile::new(),
    }))
}

fn write_pos(file: &mut impl Write, pos: BlockPos) -> std::io::Result<()> {
    file.write_all(&pos.x.to_le_bytes())?;
    file.write_all(&pos.y.to_le_bytes())?;
    file.write_all(&pos.z.to_le_bytes())
}

fn write_option_u32(file: &mut impl Write, value: Option<u32>) -> std::io::Result<()> {
    match value {
        Some(value) => {
            file.write_all(&[1u8])?;
            file.write_all(&value.to_le_bytes())
        }
        None => file.write_all(&[0u8]),
    }
}

fn write_state(file: &mut impl Write, state: FlierState) -> std::io::Result<()> {
    match state {
        FlierState::Idle => file.write_all(&[0u8]),
        FlierState::Scanning { sector, waypoint } => {
            file.write_all(&[1u8])?;
            file.write_all(&sector.x.to_le_bytes())?;
            file.write_all(&sector.z.to_le_bytes())?;
            file.write_all(&(waypoint as u64).to_le_bytes())
        }
        FlierState::ToPickup { mine } => {
            file.write_all(&[2u8])?;
            file.write_all(&(mine as u64).to_le_bytes())
        }
        FlierState::ToBase => file.write_all(&[3u8]),
        FlierState::Manual => file.write_all(&[4u8]),
        // Stage 57. A tombstone has to survive a save or a reload would
        // resurrect a machine you watched go into a hillside.
        FlierState::Lost => file.write_all(&[5u8]),
    }
}

fn read_state(file: &mut impl Read) -> std::io::Result<FlierState> {
    let mut tag = [0u8; 1];
    file.read_exact(&mut tag)?;
    Ok(match tag[0] {
        0 => FlierState::Idle,
        1 => FlierState::Scanning {
            sector: Sector {
                x: read_i32(file)?,
                z: read_i32(file)?,
            },
            waypoint: read_u64(file)? as usize,
        },
        2 => FlierState::ToPickup {
            mine: read_u64(file)? as usize,
        },
        3 => FlierState::ToBase,
        4 => FlierState::Manual,
        5 => FlierState::Lost,
        _ => return Err(std::io::Error::other("unknown flier state")),
    })
}

fn write_pile(file: &mut impl Write, pile: &Stockpile) -> std::io::Result<()> {
    let rows: Vec<(&str, u64)> = pile.entries().collect();
    file.write_all(&(rows.len() as u32).to_le_bytes())?;
    for (name, count) in rows {
        file.write_all(&(name.len() as u32).to_le_bytes())?;
        file.write_all(name.as_bytes())?;
        file.write_all(&count.to_le_bytes())?;
    }
    Ok(())
}

fn read_pile(file: &mut impl Read) -> std::io::Result<Stockpile> {
    let rows = read_u32(file)?;
    if rows > MAX_ROWS {
        return Err(std::io::Error::other("implausible cargo manifest"));
    }
    let mut pile = Stockpile::new();
    for _ in 0..rows {
        let length = read_u32(file)?;
        if length > MAX_NAME {
            return Err(std::io::Error::other("implausible good name"));
        }
        let mut name = vec![0u8; length as usize];
        file.read_exact(&mut name)?;
        let name =
            String::from_utf8(name).map_err(|_| std::io::Error::other("a name is not text"))?;
        pile.add(name, read_u64(file)?);
    }
    Ok(pile)
}

fn read_pos(file: &mut impl Read) -> std::io::Result<BlockPos> {
    Ok(BlockPos::new(
        read_i32(file)?,
        read_i32(file)?,
        read_i32(file)?,
    ))
}

fn read_option_u32(file: &mut impl Read) -> std::io::Result<Option<u32>> {
    let mut present = [0u8; 1];
    file.read_exact(&mut present)?;
    match present[0] {
        0 => Ok(None),
        1 => Ok(Some(read_u32(file)?)),
        _ => Err(std::io::Error::other("not a present-flag")),
    }
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

fn read_u64(file: &mut impl Read) -> std::io::Result<u64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}
