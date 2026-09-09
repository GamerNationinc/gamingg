//! The dispatch you left running, on disk.
//!
//! # Why this file exists
//!
//! A crew did not survive a save.
//!
//! `Mining::operation` is a private field holding the whole excavation — the
//! job board with its claims, every drone with its cargo and its grudges, and
//! the mine-mouth pile the flier ferries from — and it appeared in **no save
//! file anywhere**. You could earn the credits, buy the drones, mark a body,
//! set them cutting, save, quit, and come back to a half-dug hole with nothing
//! working in it and no way to resume. The ground survived, because ground
//! lives in the region file. The work did not.
//!
//! That is the third round running that the same shape of thing has turned up
//! in the same place — the base pile in stage 50, the player themselves in 51,
//! the crew here — and it is the worst of the three, because a drone is the
//! one part of this game you are *meant* to walk away from.
//!
//! # It resumes; it does not run on
//!
//! A restored crew picks up where it stopped. It does **not** advance for the
//! time the game was shut. Crediting elapsed wall-clock would put real time
//! into a simulation whose whole correctness argument is that it is a function
//! of the tick — `--replay` could never agree with a session that mined while
//! nobody was watching, and the world hash would stop meaning anything.
//!
//! # Why it is not a method on `Mining`
//!
//! Same line [`crate::pile`] and [`crate::wear`] hold: `vx-agent` knows
//! nothing about save directories and must not start, and `Mining` owns the
//! simulation rather than the filing. `vx-agent` grew plain snapshot types for
//! this round; the bytes are written here.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::{
    BoardSnapshot, DroneId, DroneSnapshot, DroneState, Job, JobId, JobKind, OperationSnapshot,
    Stockpile, VoxelAabb,
};
use vx_core::BlockPos;
use vx_world::World;

use crate::mining::Mining;

const MAGIC: &[u8; 4] = b"VXDG";
const VERSION: u32 = 1;

/// Caps, so a damaged file cannot ask for an enormous allocation and cannot
/// assert a crew or a board the game could never have produced.
///
/// These are integrity bounds as much as they are damage control: a
/// hand-edited `dig.dat` claiming a million drones or a drone with a cargo
/// capacity of `u64::MAX` is refused at the door rather than loaded and
/// believed. See [`crate::pile`] for the same argument about names.
const MAX_DRONES: u32 = 4_096;
const MAX_JOBS: u32 = 1_000_000;
const MAX_NAME: u32 = 64;
const MAX_ROWS: u32 = 4_096;
/// No machine in this game carries more than a few hundred blocks; a thousand
/// times that is comfortably past anything reachable and nowhere near a wrap.
const MAX_CAPACITY: u64 = 1_000_000;

/// Write the running dispatch to `dig.dat`.
///
/// Nothing running writes a present-flag of zero rather than leaving the file
/// behind, so a dispatch that was cancelled stays cancelled instead of being
/// resurrected by a stale file from two saves ago.
pub fn save(mining: &Mining, directory: &Path) -> std::io::Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(directory.join("dig.dat"))?);
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    let Some(dig) = mining.operation_snapshot() else {
        return file.write_all(&[0u8]).and_then(|()| file.flush());
    };
    file.write_all(&[1u8])?;
    write_pos(&mut file, dig.home)?;
    file.write_all(&dig.fields_built.to_le_bytes())?;
    write_option_u32(&mut file, dig.controlled.map(|index| index as u32))?;
    write_pile(&mut file, &dig.stockpile)?;

    file.write_all(&(dig.board.entries.len() as u32).to_le_bytes())?;
    file.write_all(&dig.board.next_id.to_le_bytes())?;
    for (job, claimed_by) in &dig.board.entries {
        file.write_all(&job.id.0.to_le_bytes())?;
        file.write_all(&[match job.kind {
            JobKind::Access => 0u8,
            JobKind::Extract => 1,
        }])?;
        write_pos(&mut file, job.region.min)?;
        write_pos(&mut file, job.region.max)?;
        file.write_all(&job.priority.to_le_bytes())?;
        write_option_u32(&mut file, claimed_by.map(|drone| drone.0))?;
    }

    file.write_all(&(dig.drones.len() as u32).to_le_bytes())?;
    for drone in &dig.drones {
        file.write_all(&drone.id.0.to_le_bytes())?;
        write_pos(&mut file, drone.position)?;
        write_pos(&mut file, drone.previous_position)?;
        write_state(&mut file, drone.state)?;
        write_pile(&mut file, &drone.cargo)?;
        file.write_all(&drone.capacity.to_le_bytes())?;
        file.write_all(&drone.grade.to_le_bytes())?;
        write_option_u64(&mut file, drone.job.map(|job| job.0))?;
        file.write_all(&(drone.denied.len() as u32).to_le_bytes())?;
        for pos in &drone.denied {
            write_pos(&mut file, *pos)?;
        }
        write_option_u64(&mut file, drone.denied_job.map(|job| job.0))?;
    }
    file.flush()
}

/// Read it back and put the crew to work, tolerating absence and damage.
///
/// Needs the world because restoring **re-pins the dispatch's ground** — see
/// [`Mining::restore_operation`], where the argument for that lives.
pub fn load(mining: &mut Mining, world: &mut World, directory: &Path) {
    let path = directory.join("dig.dat");
    match read(&path) {
        Ok(Some(dig)) => mining.restore_operation(world, dig),
        Ok(None) => {}
        Err(error) => log::warn!("ignoring damaged dig at {}: {error}", path.display()),
    }
}

fn read(path: &Path) -> std::io::Result<Option<OperationSnapshot>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a dig file"));
    }
    if read_u32(&mut file)? != VERSION {
        return Ok(None);
    }
    let mut present = [0u8; 1];
    file.read_exact(&mut present)?;
    if present[0] == 0 {
        return Ok(None);
    }

    let home = read_pos(&mut file)?;
    let fields_built = read_u64(&mut file)?;
    let controlled = read_option_u32(&mut file)?.map(|index| index as usize);
    let stockpile = read_pile(&mut file)?;

    let jobs = read_u32(&mut file)?;
    if jobs > MAX_JOBS {
        return Err(std::io::Error::other("implausible job count"));
    }
    let next_id = read_u64(&mut file)?;
    let mut entries = Vec::with_capacity(jobs.min(1024) as usize);
    for _ in 0..jobs {
        let id = JobId(read_u64(&mut file)?);
        let mut kind = [0u8; 1];
        file.read_exact(&mut kind)?;
        let kind = match kind[0] {
            0 => JobKind::Access,
            1 => JobKind::Extract,
            _ => return Err(std::io::Error::other("unknown job kind")),
        };
        let region = VoxelAabb::new(read_pos(&mut file)?, read_pos(&mut file)?);
        let priority = read_i32(&mut file)?;
        let claimed_by = read_option_u32(&mut file)?.map(DroneId);
        entries.push((
            Job {
                id,
                kind,
                region,
                priority,
            },
            claimed_by,
        ));
    }

    let crew = read_u32(&mut file)?;
    if crew > MAX_DRONES {
        return Err(std::io::Error::other("implausible crew size"));
    }
    let mut drones = Vec::with_capacity(crew.min(64) as usize);
    for _ in 0..crew {
        let id = DroneId(read_u32(&mut file)?);
        let position = read_pos(&mut file)?;
        let previous_position = read_pos(&mut file)?;
        let state = read_state(&mut file)?;
        let cargo = read_pile(&mut file)?;
        let capacity = read_u64(&mut file)?;
        if capacity > MAX_CAPACITY {
            return Err(std::io::Error::other("implausible drone capacity"));
        }
        let grade = read_i32(&mut file)?;
        let job = read_option_u64(&mut file)?.map(JobId);
        let denied_count = read_u32(&mut file)?;
        if denied_count > MAX_JOBS {
            return Err(std::io::Error::other("implausible refusal list"));
        }
        let mut denied = Vec::with_capacity(denied_count.min(1024) as usize);
        for _ in 0..denied_count {
            denied.push(read_pos(&mut file)?);
        }
        let denied_job = read_option_u64(&mut file)?.map(JobId);
        drones.push(DroneSnapshot {
            id,
            position,
            previous_position,
            state,
            cargo,
            capacity,
            grade,
            job,
            denied,
            denied_job,
        });
    }

    // A wheel held by a drone that is not there would panic the tick loop the
    // first time it looked. Refused rather than silently cleared, because a
    // file that disagrees with itself is a file to distrust whole.
    if controlled.is_some_and(|index| index >= drones.len()) {
        return Err(std::io::Error::other("the wheel is held by nobody"));
    }

    Ok(Some(OperationSnapshot {
        board: BoardSnapshot { entries, next_id },
        stockpile,
        home,
        drones,
        fields_built,
        controlled,
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

fn write_option_u64(file: &mut impl Write, value: Option<u64>) -> std::io::Result<()> {
    match value {
        Some(value) => {
            file.write_all(&[1u8])?;
            file.write_all(&value.to_le_bytes())
        }
        None => file.write_all(&[0u8]),
    }
}

/// A drone's state, with the job id its variants carry.
fn write_state(file: &mut impl Write, state: DroneState) -> std::io::Result<()> {
    match state {
        DroneState::Idle => file.write_all(&[0u8]),
        DroneState::Travelling(job) => {
            file.write_all(&[1u8])?;
            file.write_all(&job.0.to_le_bytes())
        }
        DroneState::Digging(job) => {
            file.write_all(&[2u8])?;
            file.write_all(&job.0.to_le_bytes())
        }
        DroneState::Hauling => file.write_all(&[3u8]),
        DroneState::Stuck => file.write_all(&[4u8]),
        DroneState::Manual => file.write_all(&[5u8]),
    }
}

/// Stockpile rows come off a `BTreeMap`, so they are already sorted: the same
/// crew writes the same bytes twice.
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

fn read_state(file: &mut impl Read) -> std::io::Result<DroneState> {
    let mut tag = [0u8; 1];
    file.read_exact(&mut tag)?;
    Ok(match tag[0] {
        0 => DroneState::Idle,
        1 => DroneState::Travelling(JobId(read_u64(file)?)),
        2 => DroneState::Digging(JobId(read_u64(file)?)),
        3 => DroneState::Hauling,
        4 => DroneState::Stuck,
        5 => DroneState::Manual,
        _ => return Err(std::io::Error::other("unknown drone state")),
    })
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

fn read_option_u64(file: &mut impl Read) -> std::io::Result<Option<u64>> {
    let mut present = [0u8; 1];
    file.read_exact(&mut present)?;
    match present[0] {
        0 => Ok(None),
        1 => Ok(Some(read_u64(file)?)),
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
