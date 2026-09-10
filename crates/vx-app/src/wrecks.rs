//! Wrecks: the machines you lost, and where they are lying.
//!
//! # A loss you can walk to
//!
//! The cheap version of losing a machine is deleting it: the roster row goes,
//! a line appears in the terminal, and the player is told rather than shown.
//! This is the other version. A destroyed machine leaves a **hulk at the spot
//! it died**, drawn in its own scorched skin, pinned on the map, and holding
//! whatever it was carrying when it went. The loss is real — the machine is
//! not coming back — but the trip out to it is worth making.
//!
//! That is the same shape [`crate::arsenal::Crash`] has had since stage 13,
//! when a shot-down caravan started spilling its load on the ground where it
//! fell. A wreck is that idea with a longer life and a verb attached.
//!
//! # Why it lives inside `Mining`
//!
//! A wreck is created by the tick — a hillside, a slug, a trunk coming down,
//! a seized machine asked to work once too often — and the tick is
//! [`crate::mining::Mining::advance`], which is what replay re-runs. Holding
//! the list anywhere else would mean the live game and the journal's replay
//! could disagree about whether a machine died, and therefore about how much
//! ground got cut. So it sits beside the fuel tank, the wear ledger and the
//! integrity ledger, for exactly the reason they do.
//!
//! It still gets a file of its own — `wrecks.dat` — because one concern per
//! save file is the house rule, and a hulk in a field is a different concern
//! from how bent the machines still flying are.
//!
//! # What a hulk is worth
//!
//! Its cargo, in full: rock in a dead drone's bed is rock, and losing it
//! twice would be mean. Plus a share of the machine in [`crate::wear`]'s
//! spare parts — the same good a repair is paid in, so a wreck feeds the
//! bench that keeps the survivors going. Never credits: a machine you flew
//! into a hill should not be a way of getting money out of the garage.

use std::io::{Read, Write};
use std::path::Path;

use glam::{DVec3, Vec3};
use vx_core::BlockPos;
use vx_render::Object;

use crate::mining::MachineRef;

const MAGIC: &[u8; 4] = b"VXWK";
const VERSION: u32 = 1;

/// How near you have to be to strip a hulk, in blocks.
///
/// Deliberately wider than the drill's reach: you are walking round a thing
/// the size of a car with a spanner, not touching one block.
pub const REACH: f64 = 3.0;

/// Spare parts recovered from a hulk, before its cargo.
///
/// A third of what a repair bench would get through in a career, so a wreck
/// is a genuine consolation and nowhere near a refund.
pub const PARTS_PER_WRECK: u64 = 6;

/// The most hulks the world keeps at once.
///
/// A cap for the same reason [`crate::drops`] has one: a save file is not a
/// landfill. The oldest goes when a new one will not fit, which is the least
/// surprising rule available — the one you just lost is never the one that
/// vanishes.
pub const MAX_WRECKS: usize = 64;

/// One dead machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wreck {
    /// Where it came to rest.
    pub at: BlockPos,
    /// What it was. Kept so the hulk is drawn as the right shape, and so the
    /// terminal can say "DIGGER 2" rather than "a machine".
    pub machine: MachineRef,
    /// What it was carrying.
    pub cargo: vx_agent::Stockpile,
    /// Spare parts left in the frame.
    pub parts: u64,
}

impl Wreck {
    /// What the roster and the terminal call it.
    pub fn name(&self) -> String {
        match self.machine {
            MachineRef::Digger(index) => format!("DIGGER {}", index + 1),
            MachineRef::Flier(index) => format!("FLIER {}", index + 1),
            MachineRef::Kestrel => "KESTREL".to_string(),
        }
    }

    /// Everything a walker would carry away from it.
    pub fn haul(&self) -> Vec<(String, u64)> {
        let mut haul: Vec<(String, u64)> = self
            .cargo
            .entries()
            .map(|(name, count)| (name.to_string(), count))
            .collect();
        if self.parts > 0 {
            haul.push((crate::wear::SPARE_PART.to_string(), self.parts));
        }
        haul
    }

    /// The centre of the hulk, in world space.
    pub fn centre(&self) -> DVec3 {
        DVec3::new(
            f64::from(self.at.x) + 0.5,
            f64::from(self.at.y) + 0.4,
            f64::from(self.at.z) + 0.5,
        )
    }
}

/// Every hulk in the world.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Wrecks {
    hulks: Vec<Wreck>,
}

impl Wrecks {
    pub fn is_empty(&self) -> bool {
        self.hulks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hulks.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Wreck> {
        self.hulks.iter()
    }

    /// Record a machine going down.
    pub fn add(&mut self, wreck: Wreck) {
        if self.hulks.len() >= MAX_WRECKS {
            self.hulks.remove(0);
        }
        self.hulks.push(wreck);
    }

    /// The nearest hulk within [`REACH`] of a position, if any.
    ///
    /// Nearest rather than first, so standing between two of them strips the
    /// one you are actually looking at rather than the one that happened to
    /// be recorded earlier.
    pub fn near(&self, position: DVec3) -> Option<usize> {
        self.hulks
            .iter()
            .enumerate()
            .map(|(index, hulk)| (index, (hulk.centre() - position).length()))
            .filter(|(_, distance)| *distance <= REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// A hulk at exactly this cell, if there is one.
    ///
    /// The journal's [`crate::journal::Command::Salvage`] names a *position*,
    /// not an index — indices are a live list's business and would not
    /// survive a replay — so this is how the replay arm finds the same hulk
    /// the live game stripped.
    pub fn at(&self, at: BlockPos) -> Option<usize> {
        self.hulks.iter().position(|hulk| hulk.at == at)
    }

    /// Strip a hulk and remove it. Returns what came off it.
    pub fn strip(&mut self, index: usize) -> Option<Wreck> {
        (index < self.hulks.len()).then(|| self.hulks.remove(index))
    }

    /// Every hulk, as drawable boxes.
    ///
    /// Built from the live rigs through [`crate::rig::Rig::wreck`], so a
    /// machine that gains a part gains it in its own hulk too.
    pub fn objects(&self, relative: impl Fn(DVec3) -> Vec3) -> Vec<Object> {
        let digger = crate::rig::Rig::wreck(crate::rig::Rig::digger());
        let flier = crate::rig::Rig::wreck(crate::rig::Rig::flier());
        let kestrel = crate::rig::Rig::wreck(crate::rig::Rig::kestrel());
        let mut objects = Vec::new();
        for hulk in &self.hulks {
            let rig = match hulk.machine {
                MachineRef::Digger(_) => &digger,
                MachineRef::Flier(_) => &flier,
                MachineRef::Kestrel => &kestrel,
            };
            // The yaw is the hulk's own cell rather than a stored angle:
            // a wreck does not turn, and a number that never changes is a
            // number that can be derived instead of saved.
            let yaw = f32::from((hulk.at.x.rem_euclid(4) + hulk.at.z.rem_euclid(4)) as i16) * 0.4;
            let feet = relative(DVec3::new(
                f64::from(hulk.at.x) + 0.5,
                f64::from(hulk.at.y),
                f64::from(hulk.at.z) + 0.5,
            ));
            objects.extend(
                rig.objects_pitched(feet, yaw, crate::rig::Rig::WRECK_TILT, 0.0)
                    .into_iter()
                    .map(Object::already_relative),
            );
        }
        objects
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "wrecks.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.hulks.len() as u32).to_le_bytes())?;
        for hulk in &self.hulks {
            file.write_all(&hulk.at.x.to_le_bytes())?;
            file.write_all(&hulk.at.y.to_le_bytes())?;
            file.write_all(&hulk.at.z.to_le_bytes())?;
            let (kind, index) = match hulk.machine {
                MachineRef::Digger(index) => (0u8, index as u32),
                MachineRef::Flier(index) => (1u8, index as u32),
                MachineRef::Kestrel => (2u8, 0),
            };
            file.write_all(&[kind])?;
            file.write_all(&index.to_le_bytes())?;
            file.write_all(&hulk.parts.to_le_bytes())?;
            let rows: Vec<(&str, u64)> = hulk.cargo.entries().collect();
            file.write_all(&(rows.len() as u32).to_le_bytes())?;
            for (name, count) in rows {
                let bytes = name.as_bytes();
                file.write_all(&(bytes.len() as u32).to_le_bytes())?;
                file.write_all(bytes)?;
                file.write_all(&count.to_le_bytes())?;
            }
        }
        file.commit()
    }

    /// Read them back, tolerating absence and damage — no file is a world
    /// nothing has crashed in, which is generous and harmless.
    pub fn load(&mut self, directory: &Path) {
        match read(&directory.join("wrecks.dat")) {
            Ok(Some(wrecks)) => *self = wrecks,
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged wreck list: {error}"),
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Wrecks>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
    file.read_exact(&mut word)?;
    let count = u32::from_le_bytes(word);
    // A count is a length claim from a file on disk, so it is bounded before
    // it is trusted: stage 54's rule, after a torn save nearly allocated a
    // gigabyte off four damaged bytes.
    if count as usize > MAX_WRECKS {
        return Err(std::io::Error::other("implausible wreck count"));
    }
    let mut hulks = Vec::new();
    for _ in 0..count {
        let mut axis = [0u8; 4];
        file.read_exact(&mut axis)?;
        let x = i32::from_le_bytes(axis);
        file.read_exact(&mut axis)?;
        let y = i32::from_le_bytes(axis);
        file.read_exact(&mut axis)?;
        let z = i32::from_le_bytes(axis);
        let mut kind = [0u8; 1];
        file.read_exact(&mut kind)?;
        file.read_exact(&mut word)?;
        let index = u32::from_le_bytes(word) as usize;
        let machine = match kind[0] {
            0 => MachineRef::Digger(index),
            1 => MachineRef::Flier(index),
            2 => MachineRef::Kestrel,
            other => return Err(std::io::Error::other(format!("unknown machine {other}"))),
        };
        let mut long = [0u8; 8];
        file.read_exact(&mut long)?;
        let parts = u64::from_le_bytes(long);
        file.read_exact(&mut word)?;
        let rows = u32::from_le_bytes(word);
        if rows > 4_096 {
            return Err(std::io::Error::other("implausible cargo"));
        }
        let mut cargo = vx_agent::Stockpile::new();
        for _ in 0..rows {
            file.read_exact(&mut word)?;
            let length = u32::from_le_bytes(word) as usize;
            if length > 256 {
                return Err(std::io::Error::other("implausible good name"));
            }
            let mut name = vec![0u8; length];
            file.read_exact(&mut name)?;
            let name = String::from_utf8(name)
                .map_err(|_| std::io::Error::other("good name is not text"))?;
            file.read_exact(&mut long)?;
            cargo.add(name, u64::from_le_bytes(long));
        }
        hulks.push(Wreck {
            at: BlockPos::new(x, y, z),
            machine,
            cargo,
            parts,
        });
    }
    Ok(Some(Wrecks { hulks }))
}

/// Strip a hulk into the player's pack, dropping what will not fit.
///
/// One function, called by the live game and by the journal's replay arm, so
/// the two cannot drift about where a wreck's rock ended up. Exactly the
/// bargain [`crate::drill::deposit`] struck in stage 55 for a cut block, and
/// it reuses that stage's overflow rule wholesale: a full pack does not refuse
/// the haul and does not eat it — what will not fit lies on the floor beside
/// the hulk, and a second trip is a second trip rather than a loss.
///
/// Returns how many goods went into the pack and how many went on the ground.
pub fn recover(
    wreck: &Wreck,
    pack: &mut crate::pack::Pack,
    drops: &mut crate::drops::Drops,
    capacity: u64,
    world: &vx_world::World,
) -> (u64, u64) {
    let floor = crate::drops::Drops::settle(world, wreck.at);
    let mut packed = 0;
    let mut spilled = 0;
    for (name, count) in wreck.haul() {
        for _ in 0..count {
            if pack.stow(&name, capacity) {
                packed += 1;
            } else {
                drops.shed(floor, &name, 1);
                spilled += 1;
            }
        }
    }
    (packed, spilled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hulk(at: BlockPos) -> Wreck {
        let mut cargo = vx_agent::Stockpile::new();
        cargo.add("engine:stone", 12);
        cargo.add("engine:copper_ore", 3);
        Wreck {
            at,
            machine: MachineRef::Digger(1),
            cargo,
            parts: PARTS_PER_WRECK,
        }
    }

    #[test]
    fn a_hulk_carries_what_the_machine_was_carrying_plus_parts() {
        let wreck = hulk(BlockPos::new(4, 64, -2));
        let haul = wreck.haul();
        let total: u64 = haul.iter().map(|(_, count)| count).sum();
        assert_eq!(total, 12 + 3 + PARTS_PER_WRECK);
        assert!(haul
            .iter()
            .any(|(name, _)| name == crate::wear::SPARE_PART));
        assert_eq!(wreck.name(), "DIGGER 2");
    }

    #[test]
    fn you_have_to_walk_to_it() {
        let mut wrecks = Wrecks::default();
        wrecks.add(hulk(BlockPos::new(0, 64, 0)));
        let close = DVec3::new(0.5, 64.4, 1.5);
        let far = DVec3::new(0.5, 64.4, 40.0);
        assert_eq!(wrecks.near(close), Some(0));
        assert_eq!(wrecks.near(far), None, "stripped a hulk from across a field");
    }

    /// Standing between two hulks strips the near one.
    #[test]
    fn the_nearest_hulk_wins() {
        let mut wrecks = Wrecks::default();
        wrecks.add(hulk(BlockPos::new(0, 64, 0)));
        wrecks.add(hulk(BlockPos::new(2, 64, 0)));
        let beside_the_second = DVec3::new(2.4, 64.4, 0.5);
        assert_eq!(wrecks.near(beside_the_second), Some(1));
    }

    #[test]
    fn stripping_takes_the_hulk_away() {
        let mut wrecks = Wrecks::default();
        wrecks.add(hulk(BlockPos::new(0, 64, 0)));
        assert_eq!(wrecks.len(), 1);
        let stripped = wrecks.strip(0).expect("nothing to strip");
        assert_eq!(stripped.parts, PARTS_PER_WRECK);
        assert!(wrecks.is_empty(), "the hulk stayed after it was stripped");
        assert_eq!(wrecks.strip(0), None);
    }

    /// The replay arm finds a hulk by *position*, because that is what the
    /// order carries — an index into a live list would not survive.
    #[test]
    fn a_hulk_is_findable_by_the_cell_it_lies_in() {
        let mut wrecks = Wrecks::default();
        wrecks.add(hulk(BlockPos::new(7, 70, 7)));
        wrecks.add(hulk(BlockPos::new(-3, 62, 11)));
        assert_eq!(wrecks.at(BlockPos::new(-3, 62, 11)), Some(1));
        assert_eq!(wrecks.at(BlockPos::new(0, 0, 0)), None);
    }

    #[test]
    fn the_oldest_goes_when_the_yard_is_full() {
        let mut wrecks = Wrecks::default();
        for n in 0..MAX_WRECKS as i32 + 10 {
            wrecks.add(hulk(BlockPos::new(n, 64, 0)));
        }
        assert_eq!(wrecks.len(), MAX_WRECKS);
        // The newest is kept and the oldest is not.
        assert!(wrecks.at(BlockPos::new(MAX_WRECKS as i32 + 9, 64, 0)).is_some());
        assert!(wrecks.at(BlockPos::new(0, 64, 0)).is_none());
    }

    #[test]
    fn a_wreck_is_drawn_without_its_rotor() {
        let whole = crate::rig::Rig::flier();
        let spinning = whole.parts.iter().filter(|part| part.spin.is_some()).count();
        assert!(spinning > 0, "the flier has no moving part to lose");
        let broken = crate::rig::Rig::wreck(crate::rig::Rig::flier());
        assert_eq!(broken.parts.len(), whole.parts.len() - spinning);
        assert!(
            broken
                .parts
                .iter()
                .all(|part| part.tile == vx_render::tiles::slot::WRECK),
            "a hulk was drawn in the machine's own paint"
        );
    }

    #[test]
    fn wrecks_round_trip() {
        let directory = std::env::temp_dir()
            .join(format!("vx-wrecks-{}-{}", std::process::id(), line!()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();

        let mut wrecks = Wrecks::default();
        wrecks.add(hulk(BlockPos::new(4, 64, -2)));
        let mut flier = hulk(BlockPos::new(-40, 120, 900));
        flier.machine = MachineRef::Flier(0);
        flier.parts = 0;
        wrecks.add(flier);
        wrecks.save(&directory).unwrap();

        let mut back = Wrecks::default();
        back.load(&directory);
        assert_eq!(back, wrecks);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_torn_list_resets_rather_than_failing() {
        let directory = std::env::temp_dir()
            .join(format!("vx-wrecks-{}-{}", std::process::id(), line!()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("wrecks.dat"), b"not a wreck list at all").unwrap();

        let mut wrecks = Wrecks::default();
        wrecks.load(&directory);
        assert!(wrecks.is_empty());

        std::fs::remove_dir_all(&directory).ok();
    }

    /// A damaged length field must not be believed. Stage 54's rule.
    #[test]
    fn an_implausible_count_is_refused() {
        let directory = std::env::temp_dir()
            .join(format!("vx-wrecks-{}-{}", std::process::id(), line!()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(directory.join("wrecks.dat"), bytes).unwrap();

        let mut wrecks = Wrecks::default();
        wrecks.load(&directory);
        assert!(wrecks.is_empty());

        std::fs::remove_dir_all(&directory).ok();
    }
}
