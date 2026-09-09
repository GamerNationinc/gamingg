//! What the kestrel sees: contact marks, and how they go stale.
//!
//! A mark is a *report*, not a tracking beacon. It holds the position a
//! contact was seen at, from the moment of sighting, and fades after
//! [`MARK_DECAY`] ticks unless re-sighted. Stale intelligence looking
//! different from fresh intelligence is the same honesty the market page
//! practices with prices — and it is what makes breaking line of sight
//! mean something.
//!
//! Marks are live-side intelligence, like the town books: journal replay
//! never sees them, because they never touch the ground the hash covers.
//!
//! # They do survive a save, since stage 52
//!
//! "Replay never sees them" was read for far too long as "they need not be
//! written down". They are the entire output of a machine the player paid for:
//! fly the kestrel over a valley, learn who is in it, save, and the report was
//! gone — the one bought thing in the game that kept nothing. `marks.dat`
//! (`VXMK`), the tolerant loader every other ledger here has, and the decay
//! clock does the rest: a mark saved thirty seconds before a quit is stale on
//! the way back in, exactly as it would have been.

use std::io::{Read, Write};
use std::path::Path;

use glam::DVec3;
use vx_world::World;

const MAGIC: &[u8; 4] = b"VXMK";
const VERSION: u32 = 1;

/// More sightings than any kestrel could hold. A cap so a damaged file cannot
/// ask for an enormous allocation, and so a hand-edited one cannot assert an
/// intelligence picture the game could never have produced.
const MAX_MARKS: u32 = 65_536;

/// Ticks a mark survives unsighted (30 s at the 8 Hz journal clock).
pub const MARK_DECAY: u64 = 240;

/// How close a fresh sighting must be to an old mark to refresh it rather
/// than spawn a second one.
const SAME_CONTACT: f32 = 2.5;

/// What kind of thing was seen. Deliberately binary — friend, crew,
/// villager and stranger read the same until factions land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Person,
    Machine,
}

/// One sighting: what, where, when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub position: DVec3,
    /// The journal tick it was (last) seen at.
    pub seen: u64,
}

impl Mark {
    /// Ticks since the sighting.
    pub fn age(&self, now: u64) -> u64 {
        now.saturating_sub(self.seen)
    }

    /// Whether the report is still worth showing.
    pub fn live(&self, now: u64) -> bool {
        self.age(now) < MARK_DECAY
    }
}

/// The scout's collected intelligence.
#[derive(Debug, Default)]
pub struct Marks {
    marks: Vec<Mark>,
}

impl Marks {
    /// One scan from an airborne eye: every contact within `radius` with an
    /// unbroken line of sight gets marked or refreshed. The raycast is the
    /// existing sight query, which is what makes a roof — or tree canopy —
    /// real cover from above without any new rule.
    pub fn scan(
        &mut self,
        world: &World,
        eye: DVec3,
        radius: f32,
        contacts: &[(MarkKind, DVec3)],
        now: u64,
    ) {
        for &(kind, at) in contacts {
            let target = at + DVec3::Y * 1.0;
            if (target - eye).length() > f64::from(radius) {
                continue;
            }
            if !vx_world::sight::sees(
                world,
                world.registry(),
                eye,
                target,
                radius + 2.0,
            ) {
                continue;
            }
            match self
                .marks
                .iter_mut()
                .find(|mark| mark.kind == kind && (mark.position - at).length() < f64::from(SAME_CONTACT))
            {
                Some(mark) => {
                    mark.position = at;
                    mark.seen = now;
                }
                None => self.marks.push(Mark {
                    kind,
                    position: at,
                    seen: now,
                }),
            }
        }
    }

    /// Forget everything past its decay.
    pub fn cull(&mut self, now: u64) {
        self.marks.retain(|mark| mark.live(now));
    }

    /// Every report still worth showing.
    pub fn live(&self, now: u64) -> impl Iterator<Item = &Mark> {
        self.marks.iter().filter(move |mark| mark.live(now))
    }

    /// Test convenience; the live game asks `live()` instead.
    #[allow(dead_code)]
    /// Write the report to `marks.dat`.
    ///
    /// In sighting order, which is the order they were made in and therefore
    /// stable: the same report writes the same bytes twice.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file =
            std::io::BufWriter::new(std::fs::File::create(directory.join("marks.dat"))?);
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.marks.len() as u32).to_le_bytes())?;
        for mark in &self.marks {
            file.write_all(&[match mark.kind {
                MarkKind::Person => 0u8,
                MarkKind::Machine => 1,
            }])?;
            file.write_all(&mark.position.x.to_le_bytes())?;
            file.write_all(&mark.position.y.to_le_bytes())?;
            file.write_all(&mark.position.z.to_le_bytes())?;
            file.write_all(&mark.seen.to_le_bytes())?;
        }
        file.flush()
    }

    /// Read it back, tolerating absence and damage.
    ///
    /// A damaged report costs you your intelligence and never your world, the
    /// same bargain every side file here makes.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("marks.dat");
        match read(&path) {
            Ok(Some(marks)) => {
                self.marks = marks;
                if self.is_empty() {
                    log::debug!("the scout's report came back empty");
                }
            }
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged marks at {}: {error}", path.display()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

fn read(path: &Path) -> std::io::Result<Option<Vec<Mark>>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a marks file"));
    }
    if read_u32(&mut file)? != VERSION {
        return Ok(None);
    }
    let count = read_u32(&mut file)?;
    if count > MAX_MARKS {
        return Err(std::io::Error::other("implausible sighting count"));
    }
    let mut marks = Vec::with_capacity(count.min(1024) as usize);
    for _ in 0..count {
        let mut kind = [0u8; 1];
        file.read_exact(&mut kind)?;
        let kind = match kind[0] {
            0 => MarkKind::Person,
            1 => MarkKind::Machine,
            _ => return Err(std::io::Error::other("unknown mark kind")),
        };
        let position = DVec3::new(read_f64(&mut file)?, read_f64(&mut file)?, read_f64(&mut file)?);
        // A sighting that is not a number would land on the map at no
        // coordinate at all and compare false against every distance test.
        if !position.is_finite() {
            return Err(std::io::Error::other("a sighting that is not a number"));
        }
        marks.push(Mark {
            kind,
            position,
            seen: read_u64(&mut file)?,
        });
    }
    Ok(Some(marks))
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_u64(file: &mut impl Read) -> std::io::Result<u64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

fn read_f64(file: &mut impl Read) -> std::io::Result<f64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(f64::from_le_bytes(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockPos, ChunkPos};

    fn open_world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 2);
        world
    }

    /// High-air fixture: eye and contacts well above any terrain, so only
    /// the rules under test decide.
    const SKY: f32 = 200.0;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("vx-marks-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// The report survives a save. It never did: the kestrel is a machine you
    /// pay for and its whole output was thrown away every time you quit.
    #[test]
    fn what_the_scout_saw_survives_a_save() {
        let directory = scratch("round");
        let world = open_world();
        let mut marks = Marks::default();
        marks.scan(
            &world,
            DVec3::new(0.5, f64::from(SKY), 0.5),
            60.0,
            &[
                (MarkKind::Person, DVec3::new(4.5, f64::from(SKY), 3.5)),
                (MarkKind::Machine, DVec3::new(-8.5, f64::from(SKY), 11.5)),
            ],
            120,
        );
        let before: Vec<Mark> = marks.live(120).copied().collect();
        assert_eq!(before.len(), 2, "the fixture saw nothing to save");
        marks.save(&directory).unwrap();

        let mut back = Marks::default();
        back.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        let after: Vec<Mark> = back.live(120).copied().collect();
        assert_eq!(after, before);
    }

    /// And it keeps ageing across the save rather than arriving fresh: a
    /// sighting saved just before a quit is stale on the way back in, exactly
    /// as it would have been had nobody quit.
    #[test]
    fn a_saved_sighting_is_still_as_old_as_it_was() {
        let directory = scratch("age");
        let world = open_world();
        let mut marks = Marks::default();
        marks.scan(
            &world,
            DVec3::new(0.5, f64::from(SKY), 0.5),
            60.0,
            &[(MarkKind::Person, DVec3::new(4.5, f64::from(SKY), 3.5))],
            10,
        );
        marks.save(&directory).unwrap();
        let mut back = Marks::default();
        back.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(back.live(10).count(), 1);
        assert_eq!(
            back.live(10 + MARK_DECAY).count(),
            0,
            "a saved mark stopped decaying"
        );
    }

    #[test]
    fn a_missing_or_damaged_report_is_no_report_at_all() {
        let directory = scratch("damaged");
        let mut marks = Marks::default();
        marks.load(&directory);
        assert!(marks.is_empty(), "a missing file invented a sighting");

        std::fs::write(directory.join("marks.dat"), b"NOPE and then some").unwrap();
        let mut marks = Marks::default();
        marks.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert!(marks.is_empty(), "a damaged file invented a sighting");
    }

    #[test]
    fn a_mark_decays_exactly_on_schedule() {
        let world = open_world();
        let mut marks = Marks::default();
        let eye = DVec3::new(0.5, f64::from(SKY) + 8.0, 0.5);
        let seen_at = 100;
        marks.scan(
            &world,
            eye,
            24.0,
            &[(MarkKind::Person, DVec3::new(4.5, f64::from(SKY), 4.5))],
            seen_at,
        );
        assert_eq!(marks.live(seen_at).count(), 1, "the contact was not marked");

        let last_tick = seen_at + MARK_DECAY - 1;
        assert_eq!(marks.live(last_tick).count(), 1, "faded a tick early");
        assert_eq!(
            marks.live(seen_at + MARK_DECAY).count(),
            0,
            "held past its decay"
        );

        marks.cull(seen_at + MARK_DECAY);
        assert!(marks.is_empty(), "cull left a stale report behind");
    }

    #[test]
    fn resighting_refreshes_instead_of_duplicating() {
        let world = open_world();
        let mut marks = Marks::default();
        let eye = DVec3::new(0.5, f64::from(SKY) + 8.0, 0.5);
        let walker = |t: f64| DVec3::new(4.5 + t, f64::from(SKY), 4.5);
        marks.scan(&world, eye, 24.0, &[(MarkKind::Person, walker(0.0))], 100);
        marks.scan(&world, eye, 24.0, &[(MarkKind::Person, walker(1.0))], 110);
        assert_eq!(
            marks.live(110).count(),
            1,
            "a moving contact left a trail of marks"
        );
        assert_eq!(marks.live(110).next().unwrap().seen, 110);
    }

    #[test]
    fn under_a_roof_is_unseen() {
        let mut world = open_world();
        let stone = world.registry().id_of("engine:stone").unwrap();
        // A slab of roof between the sky eye and the contact under it.
        for x in 2..8 {
            for z in 2..8 {
                world.set_block(BlockPos::new(x, SKY as i32 + 4, z), stone);
            }
        }
        let mut marks = Marks::default();
        let eye = DVec3::new(4.5, f64::from(SKY) + 8.0, 4.5);
        let sheltered = DVec3::new(4.5, f64::from(SKY), 4.5);
        let exposed = DVec3::new(20.5, f64::from(SKY), 4.5);
        marks.scan(
            &world,
            eye,
            24.0,
            &[
                (MarkKind::Person, sheltered),
                (MarkKind::Person, exposed),
            ],
            50,
        );
        let seen: Vec<DVec3> = marks.live(50).map(|mark| mark.position).collect();
        assert_eq!(seen, vec![exposed], "cover from above did not count: {seen:?}");
    }

    #[test]
    fn out_of_radius_is_out_of_the_report() {
        let world = open_world();
        let mut marks = Marks::default();
        let eye = DVec3::new(0.5, f64::from(SKY), 0.5);
        marks.scan(
            &world,
            eye,
            24.0,
            &[(MarkKind::Machine, DVec3::new(60.5, f64::from(SKY), 0.5))],
            10,
        );
        assert!(marks.is_empty(), "marked something beyond the scanner");
    }
}
