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
    BoardSnapshot, DroneId, DroneSnapshot, DroneState, HeapPlan, HeapShape, Job, JobId, JobKind,
    OperationSnapshot, Stockpile, VoxelAabb,
};
use vx_core::BlockPos;
use vx_world::World;

use crate::mining::Mining;

const MAGIC: &[u8; 4] = b"VXDG";
/// Version 2 adds the marked-but-undispatched area — the corners you picked
/// by eye before sending anybody at them, which a save used to bin.
///
/// A version-1 file still loads: the reader takes the dispatch and defaults
/// the mark. Refusing an old file would mean this stage — whose whole point is
/// that nothing is lost — losing somebody's running crew on the way past.
/// Version 3 adds the spoil heap: the plan the crew is stacking and how far up
/// it has got. A running heap is the same concern as a running dispatch — work
/// you left the crew doing — so it goes in the same file rather than a new one.
///
/// Both older versions still load, on the same bargain: an absent heap is no
/// heap, which is exactly true of a save written before the crew could build.
const VERSION: u32 = 3;
const VERSION_WITHOUT_THE_MARK: u32 = 1;
const VERSION_WITHOUT_THE_HEAP: u32 = 2;

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
    let mut file = crate::keeping::begin(directory, "dig.dat")?;
    write_dig(mining, &mut file)?;
    file.commit()
}

fn write_dig(mining: &Mining, file: &mut impl Write) -> std::io::Result<()> {
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    // The mark first, so it is written whether or not anybody is digging —
    // the two are independent, and the commonest case for a mark to matter is
    // exactly the case where there is no dispatch yet.
    let (corners, chosen) = mining.marked();
    file.write_all(&(corners.len().min(2) as u8).to_le_bytes())?;
    for corner in corners.iter().take(2) {
        write_pos(file, *corner)?;
    }
    file.write_all(&(chosen as u32).to_le_bytes())?;
    let Some(dig) = mining.operation_snapshot() else {
        return file.write_all(&[0u8]);
    };
    file.write_all(&[1u8])?;
    write_pos(file, dig.home)?;
    file.write_all(&dig.fields_built.to_le_bytes())?;
    write_option_u32(file, dig.controlled.map(|index| index as u32))?;
    write_pile(file, &dig.stockpile)?;

    file.write_all(&(dig.board.entries.len() as u32).to_le_bytes())?;
    file.write_all(&dig.board.next_id.to_le_bytes())?;
    for (job, claimed_by) in &dig.board.entries {
        file.write_all(&job.id.0.to_le_bytes())?;
        file.write_all(&[match job.kind {
            JobKind::Access => 0u8,
            JobKind::Extract => 1,
            JobKind::Stack => 2,
        }])?;
        write_pos(file, job.region.min)?;
        write_pos(file, job.region.max)?;
        file.write_all(&job.priority.to_le_bytes())?;
        write_option_u32(file, claimed_by.map(|drone| drone.0))?;
    }

    file.write_all(&(dig.drones.len() as u32).to_le_bytes())?;
    for drone in &dig.drones {
        file.write_all(&drone.id.0.to_le_bytes())?;
        write_pos(file, drone.position)?;
        write_pos(file, drone.previous_position)?;
        write_state(file, drone.state)?;
        write_pile(file, &drone.cargo)?;
        file.write_all(&drone.capacity.to_le_bytes())?;
        file.write_all(&drone.grade.to_le_bytes())?;
        write_option_u64(file, drone.job.map(|job| job.0))?;
        file.write_all(&(drone.denied.len() as u32).to_le_bytes())?;
        for pos in &drone.denied {
            write_pos(file, *pos)?;
        }
        write_option_u64(file, drone.denied_job.map(|job| job.0))?;
    }

    // The heap, last, so a version-2 reader that stops here still gets a
    // whole dispatch — the same tolerance `VERSION_WITHOUT_THE_MARK` bought.
    write_heap(file, dig.heap.as_ref(), dig.stacked)?;
    Ok(())
}

/// The spoil heap the crew is stacking, and how far up it has got.
///
/// The cells are **not** written: they are a pure function of the footprint
/// and the shape, so writing them would be caching what a loader can
/// recompute — the same argument `micro`'s masks make about the journal. What
/// has to be written is the shape and the footprint, because they are what the
/// *player chose*, and the count, because progress is not derivable from
/// anything else.
fn write_heap(
    file: &mut impl Write,
    heap: Option<&HeapPlan>,
    stacked: u64,
) -> std::io::Result<()> {
    match heap {
        Some(plan) => {
            file.write_all(&[1u8])?;
            file.write_all(&[match plan.shape {
                HeapShape::Pyramid => 0u8,
                HeapShape::Spiral => 1,
                HeapShape::Shaft => 2,
            }])?;
            write_pos(file, plan.footprint.min)?;
            write_pos(file, plan.footprint.max)?;
            file.write_all(&stacked.to_le_bytes())
        }
        None => file.write_all(&[0u8]),
    }
}

/// Read a heap back, re-planning its cells from the footprint and shape.
///
/// Returns `Ok(None)` for a file written before the crew could build.
fn read_heap(
    file: &mut impl Read,
    world: &World,
) -> std::io::Result<(Option<HeapPlan>, u64)> {
    let mut present = [0u8; 1];
    if file.read_exact(&mut present).is_err() {
        // A version-3 file that stops here is damaged, but a dispatch read in
        // full is still worth keeping: no heap rather than no crew.
        return Ok((None, 0));
    }
    if present[0] == 0 {
        return Ok((None, 0));
    }
    let mut shape = [0u8; 1];
    file.read_exact(&mut shape)?;
    let shape = match shape[0] {
        0 => HeapShape::Pyramid,
        1 => HeapShape::Spiral,
        2 => HeapShape::Shaft,
        _ => return Err(std::io::Error::other("unknown heap shape")),
    };
    let footprint = VoxelAabb::new(read_pos(file)?, read_pos(file)?);
    let stacked = read_u64(file)?;
    Ok((vx_agent::heap::plan(world, footprint, shape), stacked))
}

/// Read it back and put the crew to work, tolerating absence and damage.
///
/// Needs the world because restoring **re-pins the dispatch's ground** — see
/// [`Mining::restore_operation`], where the argument for that lives.
pub fn load(mining: &mut Mining, world: &mut World, directory: &Path) {
    let path = directory.join("dig.dat");
    match read(&path, world) {
        Ok(Some(stored)) => {
            // The mark first: `restore_operation` takes the ground, and
            // `Mining::mark` refuses to run once a dispatch exists — which is
            // the same rule the live game plays by, so putting them back in
            // the other order would silently drop the mark.
            mining.restore_mark(world, &stored.corners, stored.chosen);
            if let Some(dig) = stored.dig {
                mining.restore_operation(world, dig);
            }
        }
        Ok(None) => {}
        Err(error) => log::warn!("ignoring damaged dig at {}: {error}", path.display()),
    }
}

/// What `dig.dat` holds: an area you marked, and a crew you sent, either of
/// which can be present without the other.
struct StoredDig {
    corners: Vec<BlockPos>,
    chosen: usize,
    dig: Option<OperationSnapshot>,
}

fn read(path: &Path, world: &World) -> std::io::Result<Option<StoredDig>> {
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
    let version = read_u32(&mut file)?;
    // A version-1 file has no mark in it, and that is fine: it loads with an
    // empty one. Refusing it would mean this round, whose whole point is that
    // nothing is lost, losing somebody's running crew on the way past.
    let (corners, chosen) = match version {
        VERSION | VERSION_WITHOUT_THE_HEAP => read_mark(&mut file)?,
        VERSION_WITHOUT_THE_MARK => (Vec::new(), 0),
        _ => return Ok(None),
    };
    let mut present = [0u8; 1];
    file.read_exact(&mut present)?;
    if present[0] == 0 {
        return Ok(Some(StoredDig {
            corners,
            chosen,
            dig: None,
        }));
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
            2 => JobKind::Stack,
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

    // The heap, if this file is new enough to have one. An older save simply
    // has no heap, which is exactly true of the build that wrote it.
    let (heap, stacked) = if version >= VERSION {
        read_heap(&mut file, world)?
    } else {
        (None, 0)
    };

    // A wheel held by a drone that is not there would panic the tick loop the
    // first time it looked. Refused rather than silently cleared, because a
    // file that disagrees with itself is a file to distrust whole.
    if controlled.is_some_and(|index| index >= drones.len()) {
        return Err(std::io::Error::other("the wheel is held by nobody"));
    }

    Ok(Some(StoredDig {
        corners,
        chosen,
        dig: Some(OperationSnapshot {
            board: BoardSnapshot { entries, next_id },
            stockpile,
            home,
            drones,
            fields_built,
            heap,
            stacked,
            controlled,
        }),
    }))
}

/// The marked corners and the chosen method. Two corners at most, because
/// two corners is what an area is.
fn read_mark(file: &mut impl Read) -> std::io::Result<(Vec<BlockPos>, usize)> {
    let mut count = [0u8; 1];
    file.read_exact(&mut count)?;
    if count[0] > 2 {
        return Err(std::io::Error::other("an area with more than two corners"));
    }
    let mut corners = Vec::with_capacity(count[0] as usize);
    for _ in 0..count[0] {
        corners.push(read_pos(file)?);
    }
    let chosen = read_u32(file)? as usize;
    Ok((corners, chosen))
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
        DroneState::Stacking(job) => {
            file.write_all(&[6u8])?;
            file.write_all(&job.0.to_le_bytes())
        }
        // Stage 57. Seven, because `Stacking` took six in stage 56.
        DroneState::Lost => file.write_all(&[7u8]),
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
        6 => DroneState::Stacking(JobId(read_u64(file)?)),
        7 => DroneState::Lost,
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
