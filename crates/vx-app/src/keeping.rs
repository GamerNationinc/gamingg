//! Keeping: how this game writes a save down, and how it proves it.
//!
//! # Why there is a module for this at all
//!
//! Thirty-four separate ledgers is the right shape — one concern per file is
//! what lets a subsystem change its format without a version bump rippling
//! through everything else — but it leaves two questions nobody was asking.
//!
//! **Is each file written safely?** Until stage 54, no. Every one of them
//! opened its destination with `File::create`, which *truncates first*, and
//! wrote into it directly. A save interrupted halfway — a crash, a kill, a
//! battery — left whichever file was open short. And because every loader in
//! this game is deliberately tolerant, a short file is not an error: it is a
//! subsystem quietly resetting to its defaults. `Wallet::load` answers a
//! truncated `wallet.dat` by setting your credits to zero and logging a
//! warning nobody will read.
//!
//! **Is the set of them written coherently?** Also no. The thirty-four writes
//! run one after another and any prefix of them can land, so an interrupted
//! save could leave your wallet from *after* a sale beside your ore pile from
//! *before* it — and nothing on the read side ever compared the two. That is a
//! duplication glitch you reach by pulling the plug, which is the same class
//! of thing stage 52 was asked to close.
//!
//! So: [`write`] makes each file atomic, and [`Manifest`] makes the set
//! atomic. `vx_world::save` has done the first of those for region files since
//! it was written; this is that pattern brought to the other thirty-four.
//!
//! # And one place the two boot paths agree
//!
//! [`restore_the_fleet`] exists because the live game and the headless
//! [`crate::session`] had each grown their own copy of "put the fleet back
//! together", and they disagreed. The session's was right and the live one
//! dropped the goods a broken container was holding — see that function's
//! note. Two copies of a restore is one copy too many.

use std::io::Write;
use std::path::Path;

use crate::mining::Mining;

/// A save file being written: a temporary beside its destination, which
/// becomes the destination on [`Writing::commit`] and evaporates otherwise.
///
/// Writes go through it exactly as they went through a `BufWriter` before —
/// it implements [`Write`], so every existing `write_all` and every helper
/// taking `&mut impl Write` is untouched. What changes is the two ends: where
/// the bytes land on the way in, and the rename that publishes them.
pub struct Writing {
    file: Option<std::io::BufWriter<std::fs::File>>,
    temporary: std::path::PathBuf,
    target: std::path::PathBuf,
}

impl Write for Writing {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self.file.as_mut() {
            Some(file) => file.write(bytes),
            None => Err(std::io::Error::other("writing after commit")),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for Writing {
    /// A save that was never committed leaves nothing behind.
    ///
    /// Without this a writer that returned early on an error would strand a
    /// `.writing` file in the save directory for ever — invisible to every
    /// loader, since nothing globs, but so is a slow leak.
    fn drop(&mut self) {
        if self.file.take().is_some() {
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
}

impl Writing {
    /// Publish it: flush, push it to the device, and rename it into place.
    ///
    /// The `sync_all` is the part that is easy to leave out and pointless to
    /// leave out. A `flush` only moves bytes from this process into the page
    /// cache; the whole scenario this guards against — the power going, the
    /// handheld being killed — is one where the page cache does not survive
    /// either. Rename after sync is what makes "the file is there" and "the
    /// file's contents are there" the same statement.
    pub fn commit(mut self) -> std::io::Result<()> {
        let Some(mut file) = self.file.take() else {
            return Err(std::io::Error::other("committed twice"));
        };
        file.flush()?;
        file.into_inner()
            .map_err(|error| std::io::Error::other(format!("{error}")))?
            .sync_all()?;
        std::fs::rename(&self.temporary, &self.target)
    }
}

/// Begin writing a save file so that it ends up either wholly there or wholly
/// not there.
///
/// Lifted from `vx_world::save::write_atomically`, which has protected the
/// region files since they existed. Nothing here is novel; it is the same
/// discipline finally applied to the other thirty-four ledgers.
pub fn begin(directory: &Path, name: &str) -> std::io::Result<Writing> {
    std::fs::create_dir_all(directory)?;
    let target = directory.join(name);
    // Beside the target, never in the system temp: a rename across
    // filesystems is not atomic and is not even the same syscall.
    let temporary = directory.join(format!("{name}.writing"));
    let file = std::io::BufWriter::new(std::fs::File::create(&temporary)?);
    Ok(Writing {
        file: Some(file),
        temporary,
        target,
    })
}

/// Every sidecar file the live game persists, and the module that owns it.
///
/// # Why a table rather than a comment
///
/// `App::save_world` writes these one after another, and `App::resumed` reads
/// them back seven hundred lines away, and until stage 54 the only thing
/// connecting the two was a table in the module docs that nothing checked. It
/// had **six wrong filenames in it** — `map.dat` for `explored.dat`,
/// `bank.dat` for `vaults.dat`, `ballot.dat` for `elections.dat`,
/// `charter.dat` for `charters.dat`, `succession.dat` for `stands.dat`,
/// `electrolysis.dat` for `electrolyser.dat` — because a comment that is never
/// executed is a comment that drifts.
///
/// So this list is code, the manifest is written from what a save **actually
/// produced**, and a test compares the two. A subsystem that stops saving is a
/// red test rather than a discovery three stages later. The second column is
/// the module that owns the file, which is the other half of the same problem:
/// the loads bind to locals named `rads`, `sightings`, `bath`, `eyes`, `press`,
/// `rack`, `cabinet`, `holes` and `vaults`, and matching those to their files
/// was, until now, a job for whoever was reading.
pub const FILES: [(&str, &str); 36] = [
    ("log.dat", "journal"),
    ("explored.dat", "map"),
    ("player.dat", "skills"),
    ("wallet.dat", "wallet"),
    ("clock.dat", "clock"),
    ("economy.dat", "economy"),
    ("garage.dat", "garage"),
    ("postings.dat", "beacon"),
    ("homestead.dat", "homestead"),
    ("permits.dat", "permits"),
    ("whereabouts.dat", "whereabouts"),
    ("pile.dat", "pile"),
    ("fleet.dat", "fleet"),
    ("dig.dat", "dig"),
    ("fuel.dat", "fuel"),
    ("wear.dat", "wear"),
    ("wells.dat", "well"),
    ("marks.dat", "scout"),
    ("dose.dat", "dose"),
    ("drillmod.dat", "drillmod"),
    ("garrisons.dat", "garrison"),
    ("pumps.dat", "pumps"),
    ("arsenal.dat", "arsenal"),
    ("intrusion.dat", "intrusion"),
    ("printer.dat", "printer"),
    ("optics.dat", "optics"),
    ("electrolyser.dat", "electrolysis"),
    ("arcade.dat", "arcade"),
    ("health.dat", "health"),
    ("stands.dat", "succession"),
    ("warrants.dat", "warrant"),
    ("elections.dat", "ballot"),
    ("charters.dat", "charter"),
    ("reputation.dat", "reputation"),
    ("vaults.dat", "bank"),
    ("friends.dat", "disposition"),
];

/// How often a fallback save is taken while the game is autosaving itself.
///
/// The snapshot copies the ledgers and links the ground, which is cheap but
/// not free on a world with thousands of regions. Every save takes one on
/// demand — a manual `F5`, the terminal, closing the window — and an autosave
/// in between shares whatever rollback point is standing. So the worst case
/// is losing back to here, and the common case is losing nothing.
pub const SNAPSHOT_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

/// How long the game goes without saving itself, when something has changed.
pub const AUTOSAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(120);

/// What one save actually did.
///
/// Returned rather than logged, because a save that failed is the one thing
/// in this game the player most needs told and used to be the one thing they
/// were never told.
#[derive(Debug, Clone, Default)]
pub struct Kept {
    pub generation: u64,
    pub chunks: usize,
    pub took: std::time::Duration,
    /// What did not go. Empty is the only good answer.
    pub failed: Vec<&'static str>,
    /// There is no save directory at all: this session cannot write anything,
    /// and never could.
    pub impossible: bool,
}

impl Kept {
    pub fn went_well(&self) -> bool {
        !self.impossible && self.failed.is_empty()
    }

    /// One line for the player, or nothing when there is nothing to say.
    ///
    /// Success is deliberately quiet on the failure channel — an autosave
    /// that shouts every two minutes trains you to stop reading it, and the
    /// whole value of this line is that you read it when it appears.
    pub fn complaint(&self) -> Option<String> {
        if self.impossible {
            return Some("THIS WORLD CANNOT BE SAVED. CHECK THE SAVE FOLDER".into());
        }
        match self.failed.len() {
            0 => None,
            1 => Some(format!(
                "SAVE FAILED: {}",
                self.failed[0].to_uppercase()
            )),
            many => Some(format!("SAVE FAILED: {many} PARTS DID NOT GO")),
        }
    }
}

/// Whether an order is worth saving the world for, right now.
///
/// # Why this is a table and not a scattering of calls
///
/// Every order in the game passes through `CommandLog::record` — thirty call
/// sites in `main.rs` alone — so that is where "something changed" is marked,
/// and it cannot be forgotten by the next stage that adds a verb. Which of
/// those orders also deserves an *immediate* save is a separate question with
/// a separate answer, and it is answered here, once, as a total function over
/// the enum. A test walks every variant.
///
/// The rule: yes for anything you would resent doing twice, no for anything
/// that happens thousands of times. Breaking a block is the whole of mining
/// and would save on every swing; moving is recorded on every change of
/// direction. Those are what the clock is for.
pub fn worth_saving_now(command: &crate::journal::Command) -> bool {
    use crate::journal::Command;
    match command {
        // Money, goods and machines: the things you would be sick about.
        Command::Bank { .. }
        | Command::Print { .. }
        | Command::Electrolyse { .. }
        | Command::Repair { .. }
        | Command::Spud { .. }
        | Command::Gift { .. }
        | Command::Salvage { .. } => true,
        // A crew sent out, and the civic acts that only happen once.
        Command::Dispatch { .. }
        | Command::Found { .. }
        | Command::Take { .. }
        | Command::Stand { .. } => true,
        // Everything that happens by the thousand, and everything that is
        // cheap to do again. The clock covers these.
        Command::Break { .. }
        | Command::Place { .. }
        | Command::Cancel
        | Command::Move { .. }
        | Command::Advance { .. }
        | Command::Fire { .. }
        | Command::Fell { .. }
        | Command::Pump { .. }
        | Command::Scout(_)
        | Command::Intrude(_)
        | Command::Talk { .. }
        | Command::Wheel { .. }
        | Command::Pilot { .. }
        | Command::Admin(_) => false,
    }
}

const MANIFEST: &str = "manifest.dat";
const MANIFEST_MAGIC: &[u8; 4] = b"VXKP";
const MANIFEST_VERSION: u32 = 1;

/// Where a rolled-back generation is kept, and where the wreck is put.
const PREVIOUS: &str = "previous";
const TORN: &str = "torn";

/// A file as the manifest last saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Written {
    name: String,
    len: u64,
    digest: u64,
}

/// What a completed save left behind: a generation number and the shape of
/// every file in it.
///
/// Written **last**, atomically, which is the whole trick. Each file being
/// atomic on its own says nothing about the thirty-six of them together, and
/// the thirty-six together is what a save is. The manifest landing is the
/// instant the save becomes true; a save interrupted before it is a save that
/// never happened, and one interrupted after it is one that wholly did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    pub generation: u64,
    files: Vec<Written>,
}

/// What the loader made of a save directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// No manifest. Either a fresh world or one written before stage 54 —
    /// **not** a torn one, and treating it as torn would break every world
    /// anybody already has. It loads as it always did and gains a manifest on
    /// its next save.
    Unstamped,
    /// The manifest agrees with the disk.
    Whole { generation: u64 },
    /// It does not, and this is what the rollback is for.
    Torn { generation: u64, disagreed: Vec<String> },
}

/// A hash that only has to notice damage.
///
/// FNV-1a, eight lines and no dependency. This is not a signature and is not
/// trying to be: nothing here defends against somebody editing their own save
/// on their own machine, which is theirs to do. It defends against a file
/// that got half written, which is a different problem with a much cheaper
/// answer. The length alone catches a truncation; the digest catches the
/// rarer case of a block landing wrong.
fn digest(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

fn shape_of(directory: &Path, name: &str) -> Option<Written> {
    let bytes = std::fs::read(directory.join(name)).ok()?;
    Some(Written {
        name: name.to_string(),
        len: bytes.len() as u64,
        digest: digest(&bytes),
    })
}

/// Stamp a completed save, from what it actually produced.
///
/// Deliberately measured off the disk rather than off what the writers think
/// they wrote: a manifest derived from intention would agree with a save that
/// failed, which is the exact failure it exists to catch.
pub fn seal(directory: &Path, generation: u64) -> std::io::Result<Manifest> {
    let files: Vec<Written> = FILES
        .iter()
        .filter_map(|(name, _)| shape_of(directory, name))
        .collect();

    let mut file = begin(directory, MANIFEST)?;
    file.write_all(MANIFEST_MAGIC)?;
    file.write_all(&MANIFEST_VERSION.to_le_bytes())?;
    file.write_all(&generation.to_le_bytes())?;
    file.write_all(&(files.len() as u32).to_le_bytes())?;
    for written in &files {
        let name = written.name.as_bytes();
        file.write_all(&(name.len() as u32).to_le_bytes())?;
        file.write_all(name)?;
        file.write_all(&written.len.to_le_bytes())?;
        file.write_all(&written.digest.to_le_bytes())?;
    }
    file.commit()?;
    Ok(Manifest { generation, files })
}

fn read_manifest(directory: &Path) -> std::io::Result<Option<Manifest>> {
    let path = directory.join(MANIFEST);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut at = 0usize;
    let mut take = |count: usize| -> std::io::Result<&[u8]> {
        let end = at.checked_add(count).ok_or_else(|| {
            std::io::Error::other("a manifest longer than memory")
        })?;
        let slice = bytes
            .get(at..end)
            .ok_or_else(|| std::io::Error::other("a manifest that stops early"))?;
        at = end;
        Ok(slice)
    };
    if take(4)? != MANIFEST_MAGIC {
        return Err(std::io::Error::other("not a manifest"));
    }
    let version = u32::from_le_bytes(take(4)?.try_into().expect("four bytes"));
    if version != MANIFEST_VERSION {
        return Ok(None);
    }
    let generation = u64::from_le_bytes(take(8)?.try_into().expect("eight bytes"));
    let count = u32::from_le_bytes(take(4)?.try_into().expect("four bytes"));
    if count as usize > FILES.len() {
        return Err(std::io::Error::other("a manifest naming files that cannot exist"));
    }
    let mut files = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let length = u32::from_le_bytes(take(4)?.try_into().expect("four bytes"));
        if length > 64 {
            return Err(std::io::Error::other("an implausible file name"));
        }
        let name = String::from_utf8(take(length as usize)?.to_vec())
            .map_err(|_| std::io::Error::other("a file name that is not text"))?;
        let len = u64::from_le_bytes(take(8)?.try_into().expect("eight bytes"));
        let stamp = u64::from_le_bytes(take(8)?.try_into().expect("eight bytes"));
        files.push(Written {
            name,
            len,
            digest: stamp,
        });
    }
    Ok(Some(Manifest { generation, files }))
}

/// Read the manifest and compare it against what is on the disk.
pub fn inspect(directory: &Path) -> Verdict {
    let manifest = match read_manifest(directory) {
        Ok(Some(manifest)) => manifest,
        // No manifest at all, or one this build does not understand: an old
        // world, not a broken one.
        Ok(None) => return Verdict::Unstamped,
        Err(error) => {
            log::warn!("the save manifest is unreadable: {error}");
            return Verdict::Torn {
                generation: 0,
                disagreed: vec![MANIFEST.to_string()],
            };
        }
    };
    let disagreed: Vec<String> = manifest
        .files
        .iter()
        .filter(|written| shape_of(directory, &written.name).as_ref() != Some(*written))
        .map(|written| written.name.clone())
        .collect();
    if disagreed.is_empty() {
        Verdict::Whole {
            generation: manifest.generation,
        }
    } else {
        Verdict::Torn {
            generation: manifest.generation,
            disagreed,
        }
    }
}

/// Take a cheap copy of the current save, to fall back to.
///
/// # Copied ledgers, linked ground
///
/// The thirty-six sidecar files are **copied**. All of them together are a few
/// kilobytes, and copying makes the backup independent: it survives not only a
/// torn save but a file damaged in place, which a link would not.
///
/// The region files are **hard linked**, because they are the whole world and
/// copying them every save is not a thing you can do. A link is safe for them
/// for a specific reason: `vx_world::save::write_atomically` publishes a
/// region by *rename*, which replaces the directory entry and leaves the old
/// inode alone — so a link taken beforehand still points at the old contents
/// afterwards, with no bytes moved. A filesystem with no links (an exFAT card)
/// falls back to copying and says so, because a slow backup beats none and a
/// silent one beats neither.
///
/// The regions have to be in the snapshot at all: rolling the ledgers back
/// while leaving the ground forward would leave `log.dat` describing a world
/// that is not there, and the replay oracle is precisely the thing that would
/// notice.
///
/// What this does **not** protect against, stated rather than papered over: a
/// region file corrupted in place by the disk itself, since the backup shares
/// its inode. Guarding that means copying the world, and copying the world
/// every couple of minutes is not a backup, it is a stall.
pub fn snapshot(directory: &Path) -> std::io::Result<usize> {
    let previous = directory.join(PREVIOUS);
    let _ = std::fs::remove_dir_all(&previous);
    std::fs::create_dir_all(&previous)?;

    let mut kept = 0;
    let mut copied = 0;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        // A working file from an interrupted write is not part of any save.
        if name.to_string_lossy().ends_with(".writing") {
            continue;
        }
        let to = previous.join(&name);
        // The small ledgers outright; the ground by reference.
        let small = name.to_string_lossy().ends_with(".dat");
        if small {
            std::fs::copy(entry.path(), &to)?;
            kept += 1;
            continue;
        }
        match std::fs::hard_link(entry.path(), &to) {
            Ok(()) => kept += 1,
            Err(_) => {
                std::fs::copy(entry.path(), &to)?;
                kept += 1;
                copied += 1;
            }
        }
    }
    if copied > 0 {
        log::info!("this filesystem has no hard links; copied {copied} region files instead");
    }
    Ok(kept)
}

/// Fall back to the last save that was whole.
///
/// The torn generation is **kept**, under `torn/`, rather than deleted: it is
/// the only evidence of whatever went wrong, and a player who has just lost
/// twenty minutes is owed more than a shrug. Then the backup is renamed into
/// place and the world loads through the ordinary path — one loader, not two.
///
/// Returns whether there was anything to fall back to.
pub fn roll_back(directory: &Path) -> std::io::Result<bool> {
    let previous = directory.join(PREVIOUS);
    if !previous.join(MANIFEST).is_file() {
        return Ok(false);
    }
    let torn = directory.join(TORN);
    let _ = std::fs::remove_dir_all(&torn);
    std::fs::create_dir_all(&torn)?;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        std::fs::rename(entry.path(), torn.join(&name))?;
    }
    for entry in std::fs::read_dir(&previous)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        std::fs::rename(entry.path(), directory.join(&name))?;
    }
    let _ = std::fs::remove_dir_all(&previous);
    Ok(true)
}

/// Put the fleet back together from what was written down.
///
/// # The bug this function is
///
/// The live game and [`crate::session`] each restored the fleet in their own
/// hand-written sequence, seven hundred lines and one file apart. The
/// session's was right. The live game's did this:
///
/// ```text
/// pile::load(&mut base_pile, root);      // reads the base *and* the orphan
/// ...
/// mining.fleet.base = base_pile.base;    // and keeps only the base
/// ```
///
/// `pile.dat` has carried the fleet's **orphaned** stockpile since stage 52 —
/// the goods a broken container was holding, kept aside for the next one you
/// place, so that one stray click is not the most expensive mistake in the
/// game. The live boot wrote them out on every save, read them back correctly,
/// and then dropped them on the floor by grafting across only `.base`. Break
/// your container, save, come back: gone.
///
/// It hid for the best possible reason. There *is* a test —
/// `goods_waiting_for_a_container_survive_a_save` — and it passes. It exercises
/// the session's path, which was never the one that was broken.
///
/// So there is one sequence now and both callers use it. The order matters and
/// is not arbitrary: `pile` first, because `fleet::load` deliberately restores
/// the air side *around* whatever base and orphan are already there; `dig` last
/// and with the world, because restoring a dispatch re-pins its ground and
/// that needs chunks already in place.
pub fn restore_the_fleet(mining: &mut Mining, world: &mut vx_world::World, root: &Path) {
    crate::pile::load(&mut mining.fleet, root);
    crate::fleet::load(&mut mining.fleet, root);
    crate::dig::load(mining, world, root);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("gamingg-keeping-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    #[test]
    fn a_written_file_is_wholly_there() {
        let directory = scratch("atomic");
        let mut file = begin(&directory, "thing.dat").expect("begin");
        file.write_all(b"hello").expect("write");
        file.commit().expect("commit");
        assert_eq!(
            std::fs::read(directory.join("thing.dat")).expect("read"),
            b"hello"
        );
        // And no working file is left lying about for a loader to trip over.
        assert!(!directory.join("thing.dat.writing").exists());
    }

    /// The point of the temporary: a write that fails partway leaves the
    /// *previous* file untouched rather than a stump where it used to be.
    #[test]
    fn a_failed_write_does_not_destroy_what_was_there() {
        let directory = scratch("failed");
        let mut good = begin(&directory, "thing.dat").expect("begin");
        good.write_all(b"the good one").expect("write");
        good.commit().expect("commit");

        // A writer that gives up partway: the bytes went somewhere, but
        // nothing was committed, so nothing was published.
        {
            let mut giving_up = begin(&directory, "thing.dat").expect("begin");
            giving_up.write_all(b"partial").expect("write");
        }
        assert!(
            !directory.join("thing.dat.writing").exists(),
            "an abandoned write left its working file behind"
        );
        assert_eq!(
            std::fs::read(directory.join("thing.dat")).expect("read"),
            b"the good one",
            "a failed write ate the file that was already there"
        );
    }

    #[test]
    fn writing_creates_the_directory_it_needs() {
        let directory = scratch("nested").join("deeper");
        let mut file = begin(&directory, "thing.dat").expect("begin");
        file.write_all(b"x").expect("write");
        file.commit().expect("commit");
        assert!(directory.join("thing.dat").is_file());
    }

    /// **The census in the module docs names exactly the files the code
    /// keeps.**
    ///
    /// That table used to be prose nobody executed, and it had *six* wrong
    /// filenames in it — `map.dat` for `explored.dat`, `bank.dat` for
    /// `vaults.dat`, `ballot.dat` for `elections.dat`, `charter.dat` for
    /// `charters.dat`, `succession.dat` for `stands.dat`, `electrolysis.dat`
    /// for `electrolyser.dat`. Six subsystems documented under names that
    /// have never existed on a disk, in the one table whose entire job is to
    /// say what is on the disk.
    ///
    /// So the table is parsed. `intro.rs` already reads `ROADMAP.md` this way
    /// to build the welcome panel, so the trick is in-house: a document that
    /// is read by a test is a document that cannot drift.
    #[test]
    fn the_census_in_the_docs_names_the_files_the_code_keeps() {
        const DOCS: &str = include_str!("main.rs");
        let mut documented: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for line in DOCS.lines() {
            let Some(row) = line.strip_prefix("//! | ") else { continue };
            let mut rest = row;
            while let Some(open) = rest.find('`') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('`') else { break };
                let name = &after[..close];
                if name.ends_with(".dat") {
                    documented.insert(name.to_string());
                }
                rest = &after[close + 1..];
            }
        }

        let kept: std::collections::BTreeSet<String> =
            FILES.iter().map(|(name, _)| name.to_string()).collect();
        // The manifest names itself in the table but is not one of the files
        // it stamps, which would be circular.
        documented.remove(MANIFEST);

        let undocumented: Vec<&String> = kept.difference(&documented).collect();
        assert!(
            undocumented.is_empty(),
            "saved but not in the census table: {undocumented:?}"
        );
        let invented: Vec<&String> = documented.difference(&kept).collect();
        assert!(
            invented.is_empty(),
            "the census table names files nothing saves: {invented:?}"
        );
    }

    /// Every file is owned by exactly one module, and no name appears twice.
    #[test]
    fn the_file_table_has_no_duplicates() {
        let names: std::collections::BTreeSet<&str> =
            FILES.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), FILES.len(), "a file is listed twice");
        for (name, owner) in FILES {
            assert!(name.ends_with(".dat"), "{name} is not a save file");
            assert!(!owner.is_empty(), "{name} has no owner");
        }
    }

    /// Every order has an answer, and the two kinds are both represented.
    ///
    /// The match in [`worth_saving_now`] is exhaustive, so the compiler
    /// already refuses a new `Command` with no answer — this pins the
    /// *shape* of the answer, so that "make it compile" cannot quietly
    /// become "say false to everything".
    #[test]
    fn the_orders_worth_saving_for_are_the_rare_ones() {
        use crate::journal::Command;
        assert!(worth_saving_now(&Command::Print { recipe: 0 }));
        assert!(worth_saving_now(&Command::Dispatch {
            area: vx_agent::VoxelAabb::new(
                vx_core::BlockPos::new(0, 0, 0),
                vx_core::BlockPos::new(1, 1, 1)
            ),
            method: vx_agent::MineMethod::Adit,
            crew: 1,
        }));
        // And the ones that happen by the thousand do not.
        assert!(!worth_saving_now(&Command::Break {
            at: vx_core::BlockPos::new(0, 0, 0)
        }));
        assert!(!worth_saving_now(&Command::Advance { ticks: 1 }));
        assert!(!worth_saving_now(&Command::Move {
            bits: 0,
            yaw_q: 0,
            pitch_q: 0,
            throttle: 0,
            load: 0,
        }));
    }

    /// A full set, written the way a save writes one.
    fn a_saved_set(directory: &Path) {
        for (name, _) in FILES {
            let mut file = begin(directory, name).expect("begin");
            file.write_all(name.as_bytes()).expect("write");
            file.commit().expect("commit");
        }
    }

    #[test]
    fn a_sealed_save_reads_back_as_whole() {
        let directory = scratch("whole");
        a_saved_set(&directory);
        let manifest = seal(&directory, 7).expect("seal");
        assert_eq!(manifest.generation, 7);
        assert_eq!(inspect(&directory), Verdict::Whole { generation: 7 });
    }

    /// **A save torn by a crash is caught rather than loaded.**
    ///
    /// The scenario this whole part exists for: the process died partway
    /// through writing the set, so one file is short and the rest are from
    /// two different moments. Every loader in this game is tolerant, so
    /// without the manifest this loads silently as a world with somebody
    /// else's wallet in it.
    #[test]
    fn a_torn_save_is_spotted() {
        let directory = scratch("torn");
        a_saved_set(&directory);
        seal(&directory, 3).expect("seal");

        // A kill during `wallet.dat`: the file exists and is the wrong length.
        std::fs::write(directory.join("wallet.dat"), b"wal").expect("truncate");
        match inspect(&directory) {
            Verdict::Torn { generation, disagreed } => {
                assert_eq!(generation, 3);
                assert_eq!(disagreed, ["wallet.dat"]);
            }
            other => panic!("a truncated file read as {other:?}"),
        }
    }

    /// Same length, different bytes — the case the length alone misses.
    #[test]
    fn a_file_that_changed_behind_the_manifest_is_spotted() {
        let directory = scratch("swapped");
        a_saved_set(&directory);
        seal(&directory, 1).expect("seal");
        let same_length = vec![b'x'; "pile.dat".len()];
        std::fs::write(directory.join("pile.dat"), same_length).expect("swap");
        assert!(matches!(inspect(&directory), Verdict::Torn { .. }));
    }

    /// **A world saved before stage 54 is old, not broken.**
    ///
    /// The one way this feature could do more harm than the bug it fixes is
    /// by deciding every existing save is torn and rolling all of them back
    /// to nothing.
    #[test]
    fn a_world_with_no_manifest_is_not_torn() {
        let directory = scratch("unstamped");
        a_saved_set(&directory);
        assert_eq!(inspect(&directory), Verdict::Unstamped);

        // And it gains one on its next save, without anything else changing.
        seal(&directory, 1).expect("seal");
        assert_eq!(inspect(&directory), Verdict::Whole { generation: 1 });
    }

    /// The rollback, end to end: a good save, a snapshot, a bad save on top,
    /// and the good one coming back with the wreck kept beside it.
    #[test]
    fn a_torn_generation_rolls_back_to_the_one_before_it() {
        let directory = scratch("rollback");
        a_saved_set(&directory);
        std::fs::write(directory.join("wallet.dat"), b"the good wallet").expect("write");
        seal(&directory, 1).expect("seal");
        let kept = snapshot(&directory).expect("snapshot");
        assert!(kept >= FILES.len(), "the snapshot missed files: {kept}");

        // Generation two goes wrong halfway through.
        std::fs::write(directory.join("wallet.dat"), b"a torn wallet").expect("write");
        seal(&directory, 2).expect("seal");
        std::fs::write(directory.join("pile.dat"), b"cut off").expect("truncate");
        assert!(matches!(inspect(&directory), Verdict::Torn { .. }));

        assert!(roll_back(&directory).expect("roll back"));
        assert_eq!(inspect(&directory), Verdict::Whole { generation: 1 });
        assert_eq!(
            std::fs::read(directory.join("wallet.dat")).expect("read"),
            b"the good wallet",
            "the rollback did not restore the older generation"
        );
        // And the wreck is kept rather than binned.
        assert!(
            directory.join("torn").join("wallet.dat").is_file(),
            "the torn generation was thrown away instead of set aside"
        );
    }

    /// A snapshot holds the old bytes through a rewrite — for a copied
    /// ledger *and* for a linked region, which is the whole reason the
    /// region can be a link rather than a copy.
    #[test]
    fn a_snapshot_holds_its_own_copy_through_a_rewrite() {
        let directory = scratch("links");
        let mut file = begin(&directory, "wallet.dat").expect("begin");
        file.write_all(b"before").expect("write");
        file.commit().expect("commit");
        // Stand in for a region: not a `.dat`, so it is linked.
        let region = directory.join("r.0.0.vxr");
        std::fs::write(&region, b"old ground").expect("write");
        seal(&directory, 1).expect("seal");
        snapshot(&directory).expect("snapshot");

        let mut again = begin(&directory, "wallet.dat").expect("begin");
        again.write_all(b"after").expect("write");
        again.commit().expect("commit");
        // Republished by rename, exactly as `write_atomically` does it.
        let fresh = directory.join("r.0.0.vxr.tmp");
        std::fs::write(&fresh, b"new ground").expect("write");
        std::fs::rename(&fresh, &region).expect("rename");

        let previous = directory.join("previous");
        assert_eq!(std::fs::read(directory.join("wallet.dat")).unwrap(), b"after");
        assert_eq!(
            std::fs::read(previous.join("wallet.dat")).unwrap(),
            b"before",
            "the copied ledger followed the rewrite"
        );
        assert_eq!(std::fs::read(&region).unwrap(), b"new ground");
        assert_eq!(
            std::fs::read(previous.join("r.0.0.vxr")).unwrap(),
            b"old ground",
            "the linked region followed the rename instead of holding the old inode"
        );
    }

    /// A copied ledger is independent of the file it came from — which a
    /// hard link would not be, and which is why the ledgers are copied.
    #[test]
    fn a_backed_up_ledger_does_not_share_damage_with_the_live_one() {
        let directory = scratch("independent");
        a_saved_set(&directory);
        seal(&directory, 1).expect("seal");
        snapshot(&directory).expect("snapshot");

        // Damage in place, the way a bad write or a stray hand would.
        std::fs::write(directory.join("wallet.dat"), b"ruined").expect("damage");
        assert_eq!(
            std::fs::read(directory.join("previous").join("wallet.dat")).unwrap(),
            b"wallet.dat",
            "the backup shared the damage"
        );
    }

    /// Nothing to fall back to is a fact, not a panic.
    #[test]
    fn rolling_back_with_no_backup_says_so() {
        let directory = scratch("nothing");
        a_saved_set(&directory);
        seal(&directory, 1).expect("seal");
        assert!(!roll_back(&directory).expect("roll back"));
    }

    /// **The goods a broken container was holding survive a reload.**
    ///
    /// Red against the live boot path as it stood: it read the orphan rows and
    /// then grafted across only `.base`. See [`restore_the_fleet`].
    #[test]
    fn goods_waiting_for_a_container_survive_the_restore() {
        let directory = scratch("orphan");
        let mut world = vx_world::World::new(1);

        // A fleet holding goods with nowhere to put them: a container was
        // placed, filled, and then broken.
        let mut mining = Mining::default();
        mining
            .fleet
            .set_base(vx_core::BlockPos::new(4, 70, 4));
        if let Some(base) = mining.fleet.base.as_mut() {
            base.stockpile.add("engine:copper_ore", 91);
        }
        let held = mining.fleet.clear_base();
        assert_eq!(held, 91, "the fixture did not orphan anything");

        crate::pile::save(&mining.fleet, &directory).expect("save");
        crate::fleet::save(&mining.fleet, &directory).expect("save");
        crate::dig::save(&mining, &directory).expect("save");

        let mut restored = Mining::default();
        restore_the_fleet(&mut restored, &mut world, &directory);
        assert_eq!(
            restored.fleet.orphaned().total(),
            91,
            "the goods waiting for a container were dropped by the restore"
        );
        assert!(
            restored.fleet.base.is_none(),
            "a broken container came back declared"
        );
    }
}
