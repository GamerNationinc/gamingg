//! Skills and levels: the player's own progression.
//!
//! # Name-keyed on purpose
//!
//! Skills are entries in a map keyed by name, not variants of an enum — the
//! same reasoning as block names and stockpile keys, and a deliberate door:
//! the design wants the eventual breadth of a full RPG skill sheet (combat,
//! faction standing, and whatever the villages bring), and those must land as
//! *data*, not as rewrites of everything that touches a skill.
//!
//! # The curve
//!
//! The classic long-tail shape: each level costs about ten percent more than
//! the one before, on top of a flat floor, levels 1 to 99. Early levels fall
//! out of a few minutes of drilling; the last ones are a career. The shape is
//! a genre convention; the constants are ours.
//!
//! # Player knowledge, not world truth
//!
//! Like the map's explored set, skills persist in their own small file beside
//! the region files (`player.dat`) with a tolerant loader: a damaged file
//! logs, resets, and never takes the world down.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::OnceLock;

/// The skills that exist at launch. Anything else added at runtime simply
/// works — this list only drives the HUD ordering and the effect functions.
pub const MINING: &str = "mining";
pub const PROSPECTING: &str = "prospecting";
pub const LOGISTICS: &str = "logistics";
/// Picking locks. Levelled by doing it, which means the only way to get good
/// at bypassing a door is to bypass easier ones first.
pub const SECURITY: &str = "security";
/// Working the fabricator: what you have printed teaches you to print
/// faster, and unlocks the patterns a novice cannot hold in tolerance.
pub const FABRICATION: &str = "fabrication";

/// The level cap.
pub const MAX_LEVEL: u32 = 99;

const MAGIC: &[u8; 4] = b"VXPL";
const VERSION: u32 = 1;

/// Cumulative XP required to *reach* each level; `TABLE[1] = 0`.
fn table() -> &'static [u64; (MAX_LEVEL + 1) as usize] {
    static TABLE: OnceLock<[u64; (MAX_LEVEL + 1) as usize]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut cumulative = [0u64; (MAX_LEVEL + 1) as usize];
        let mut total = 0u64;
        for level in 2..=MAX_LEVEL {
            // The cost of going from level-1 to level: a flat floor plus a
            // ~10%-per-level exponential term.
            let cost = 25.0 + 75.0 * 1.10f64.powi(level as i32 - 2);
            total += cost as u64;
            cumulative[level as usize] = total;
        }
        cumulative
    })
}

/// Cumulative XP needed to reach `level`. Level 1 is free; past the cap costs
/// what the cap costs.
pub fn xp_for_level(level: u32) -> u64 {
    table()[level.clamp(1, MAX_LEVEL) as usize]
}

/// The level `xp` earns. Saturates at [`MAX_LEVEL`].
pub fn level_for_xp(xp: u64) -> u32 {
    // The table is small; a linear scan is clearer than a binary search and
    // this is nowhere near hot.
    let table = table();
    let mut level = 1;
    while level < MAX_LEVEL && xp >= table[(level + 1) as usize] {
        level += 1;
    }
    level
}

/// Fraction of the way from the current level to the next, for the HUD bar.
pub fn progress_to_next(xp: u64) -> f32 {
    let level = level_for_xp(xp);
    if level >= MAX_LEVEL {
        return 1.0;
    }
    let floor = xp_for_level(level);
    let ceiling = xp_for_level(level + 1);
    (xp - floor) as f32 / (ceiling - floor) as f32
}

// --- Effects: small pure functions from level to stat. ---
// Kept here together so "what does levelling actually do" has one address.

/// Hardness-units the handheld drill chews per second.
pub fn drill_power(mining_level: u32) -> f32 {
    1.25 * (1.0 + 0.04 * (mining_level.saturating_sub(1)) as f32)
}

/// How deep the flier's scanner senses, in blocks below the surface.
pub fn scan_depth(prospecting_level: u32) -> i32 {
    (vx_agent::SCAN_DEPTH + (prospecting_level / 4) as i32).min(44)
}

/// Carry capacity multiplier for drones and fliers.
pub fn capacity(base: u64, logistics_level: u32) -> u64 {
    base + base * 2 * u64::from(logistics_level.saturating_sub(1)) / 100
}

/// XP granted per point of hardness the player drills through.
pub const MINING_XP_PER_HARDNESS: f32 = 10.0;
/// XP for completing a sector sweep, plus per ping found in it.
pub const SWEEP_XP: u64 = 100;
pub const PING_XP: u64 = 40;
/// XP per block a flier lands in the base.
pub const DELIVERY_XP: u64 = 2;
/// XP for talking a lock open, by grade. A house teaches you little; the
/// sheriff's door teaches you a great deal.
pub const BYPASS_XP: [u64; 3] = [120, 900, 4_000];

/// Seconds to talk a lock open, given the grade's base time and your level.
///
/// Time is the whole cost of a bypass — you stand still and exposed while it
/// runs, so the danger is being *seen*, not being refused. Levelling buys speed
/// rather than certainty: there is no roll, which keeps house rule 2 intact.
pub fn bypass_seconds(base: f32, security_level: u32) -> f32 {
    base / (1.0 + 0.03 * (security_level.saturating_sub(1)) as f32)
}

/// The player's skill sheet.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Skills {
    xp: BTreeMap<String, u64>,
    /// The skill that most recently earned XP — what the HUD's bar shows.
    recent: Option<String>,
}

impl Skills {
    pub fn new() -> Self {
        Skills::default()
    }

    pub fn xp(&self, skill: &str) -> u64 {
        self.xp.get(skill).copied().unwrap_or(0)
    }

    pub fn level(&self, skill: &str) -> u32 {
        level_for_xp(self.xp(skill))
    }

    /// Grant XP. Returns the new level if this crossed a boundary.
    pub fn add_xp(&mut self, skill: &str, amount: u64) -> Option<u32> {
        if amount == 0 {
            return None;
        }
        let before = self.level(skill);
        let entry = self.xp.entry(skill.to_string()).or_insert(0);
        *entry = entry.saturating_add(amount);
        self.recent = Some(skill.to_string());
        let after = level_for_xp(*entry);
        (after > before).then_some(after)
    }

    /// The skill whose bar the HUD should show: the last one that moved.
    pub fn recent(&self) -> Option<&str> {
        self.recent.as_deref()
    }

    /// Write the sheet beside the world save.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "player.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.xp.len() as u32).to_le_bytes())?;
        for (name, xp) in &self.xp {
            let bytes = name.as_bytes();
            file.write_all(&(bytes.len() as u32).to_le_bytes())?;
            file.write_all(bytes)?;
            file.write_all(&xp.to_le_bytes())?;
        }
        file.commit()
    }

    /// Load the sheet, tolerating absence and damage — a corrupt file is a
    /// fresh start, logged, never a failed world.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("player.dat");
        match read_skills(&path) {
            Ok(Some(xp)) => {
                log::info!("loaded {} skills", xp.len());
                self.xp = xp;
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting fresh", path.display());
                self.xp.clear();
            }
        }
    }
}

fn read_skills(path: &Path) -> std::io::Result<Option<BTreeMap<String, u64>>> {
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

    let mut skills = BTreeMap::new();
    for _ in 0..count {
        file.read_exact(&mut word)?;
        let length = u32::from_le_bytes(word) as usize;
        if length > 256 {
            return Err(std::io::Error::other("skill name implausibly long"));
        }
        let mut name = vec![0u8; length];
        file.read_exact(&mut name)?;
        let mut xp = [0u8; 8];
        file.read_exact(&mut xp)?;
        skills.insert(
            String::from_utf8(name).map_err(std::io::Error::other)?,
            u64::from_le_bytes(xp),
        );
    }
    Ok(Some(skills))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_is_strictly_monotonic_and_saturates() {
        for level in 2..=MAX_LEVEL {
            assert!(
                xp_for_level(level) > xp_for_level(level - 1),
                "level {level} costs nothing over {y}",
                y = level - 1
            );
        }
        assert_eq!(xp_for_level(0), xp_for_level(1), "below 1 clamps");
        assert_eq!(xp_for_level(200), xp_for_level(MAX_LEVEL), "above cap clamps");
        assert_eq!(level_for_xp(u64::MAX), MAX_LEVEL, "no overflow at the extreme");
    }

    #[test]
    fn level_and_xp_are_inverses_on_the_table() {
        for level in 1..=MAX_LEVEL {
            let floor = xp_for_level(level);
            assert_eq!(level_for_xp(floor), level, "at exactly the boundary");
            if level < MAX_LEVEL {
                assert_eq!(level_for_xp(xp_for_level(level + 1) - 1), level, "just below next");
            }
        }
    }

    #[test]
    fn early_levels_are_quick_and_late_levels_are_careers() {
        // The shape promise: the first level is a handful of blocks, the last
        // is orders of magnitude more.
        let first = xp_for_level(2);
        let last = xp_for_level(MAX_LEVEL) - xp_for_level(MAX_LEVEL - 1);
        assert!(first <= 150, "level 2 costs {first}, too grindy to feel");
        assert!(
            last > first * 1000,
            "the tail is too short: first {first}, last {last}"
        );
    }

    #[test]
    fn adding_xp_reports_level_ups_exactly_once() {
        let mut skills = Skills::new();
        assert_eq!(skills.level(MINING), 1);

        // Almost to level 2, then over.
        let boundary = xp_for_level(2);
        assert_eq!(skills.add_xp(MINING, boundary - 1), None);
        assert_eq!(skills.add_xp(MINING, 1), Some(2));
        assert_eq!(skills.add_xp(MINING, 1), None, "reported the same level twice");
        assert_eq!(skills.level(MINING), 2);
        assert_eq!(skills.recent(), Some(MINING));
    }

    #[test]
    fn unknown_skills_are_level_one_not_errors() {
        // The Skyrim-breadth door: a skill nobody has heard of yet is simply
        // unpractised.
        let mut skills = Skills::new();
        assert_eq!(skills.level("speechcraft"), 1);
        skills.add_xp("speechcraft", 500);
        assert!(skills.level("speechcraft") > 1);
    }

    #[test]
    fn effects_move_with_levels() {
        assert!(drill_power(10) > drill_power(1));
        assert!(scan_depth(20) > scan_depth(1));
        assert_eq!(scan_depth(1), vx_agent::SCAN_DEPTH);
        assert!(scan_depth(99) <= 44, "scan depth should cap");
        assert!(capacity(64, 50) > capacity(64, 1));
        assert_eq!(capacity(64, 1), 64, "level 1 is the base game");
    }

    #[test]
    fn the_sheet_round_trips_through_disk() {
        let directory = std::env::temp_dir().join(format!("vx-skills-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut skills = Skills::new();
        skills.add_xp(MINING, 12_345);
        skills.add_xp(PROSPECTING, 678);
        skills.save(&directory).unwrap();

        let mut loaded = Skills::new();
        loaded.load(&directory);
        assert_eq!(loaded.xp(MINING), 12_345);
        assert_eq!(loaded.xp(PROSPECTING), 678);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_corrupt_sheet_resets_rather_than_failing() {
        let directory = std::env::temp_dir().join(format!("vx-skills-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("player.dat"), b"not a sheet").unwrap();

        let mut skills = Skills::new();
        skills.add_xp(MINING, 99);
        skills.load(&directory);
        assert_eq!(skills.xp(MINING), 0);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn progress_runs_zero_to_one_between_levels() {
        let floor = xp_for_level(5);
        let ceiling = xp_for_level(6);
        assert_eq!(progress_to_next(floor), 0.0);
        assert!(progress_to_next((floor + ceiling) / 2) > 0.3);
        assert!(progress_to_next(ceiling - 1) > 0.9);
        assert_eq!(progress_to_next(u64::MAX), 1.0);
    }
}
