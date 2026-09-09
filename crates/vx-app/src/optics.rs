//! The optics kit: the lamp you carry and the visors you print.
//!
//! # Light is worn, not placed
//!
//! Stage 17 made the world below genuinely dark, and the honest fix for
//! darkness in this game is equipment, not a settings slider. The basic hand
//! lamp is part of the suit — everyone has it, because falling into a pothole
//! with no way to see the walls is a trap, not a mechanic. Everything better
//! is made at the fabricator: a high beam that throws further, a night vision
//! visor that amplifies what little light there is, and a thermal visor that
//! does not care about light at all and paints warm machinery against cold
//! rock. The counter sells none of it; seeing in the dark belongs to people
//! with a mine, which is who needs it.
//!
//! # One key, one dial
//!
//! `L` cycles Off → Lamp → Night vision → Thermal, skipping anything not
//! owned. A mode is a way of *seeing*, so it lives entirely in the renderer:
//! the lamp is a spot cone in the shader, the visors are colour transforms,
//! and none of it touches the world or the journal. What persists is what you
//! own — name-keyed, like every other possession in this game — plus the mode
//! you left the dial on.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;

/// Printed at the fabricator: a longer, brighter throw for the hand lamp.
pub const HIGH_BEAM: &str = "high beam lamp";
/// Printed at the fabricator: see by amplifying what little light exists.
pub const NIGHT_VISION: &str = "night vision visor";
/// Printed at the fabricator: see heat instead of light.
pub const THERMAL: &str = "thermal visor";

const MAGIC: &[u8; 4] = b"VXOL";
const VERSION: u32 = 1;

/// How the player is currently seeing the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Off,
    Lamp,
    NightVision,
    Thermal,
}

/// The lamp's throw: strength (shader units) and reach (blocks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beam {
    pub strength: f32,
    pub reach: f32,
}

/// The basic suit lamp, owned by everyone.
const SUIT_BEAM: Beam = Beam {
    strength: 0.95,
    reach: 16.0,
};

/// The printed high beam: brighter and nearly twice the reach.
const HIGH_BEAM_THROW: Beam = Beam {
    strength: 1.35,
    reach: 30.0,
};

/// What the player owns and how they are looking at the world.
#[derive(Debug, Default)]
pub struct Optics {
    /// Printed gear, by name. The basic lamp is not in here because it is
    /// not a possession — it is part of the suit.
    pub owned: BTreeSet<String>,
    pub mode: Mode,
}

impl Optics {
    /// Is a mode available to switch to?
    pub fn available(&self, mode: Mode) -> bool {
        match mode {
            Mode::Off | Mode::Lamp => true,
            Mode::NightVision => self.owned.contains(NIGHT_VISION),
            Mode::Thermal => self.owned.contains(THERMAL),
        }
    }

    /// Turn the dial one notch, skipping anything not owned.
    pub fn cycle(&mut self) {
        let order = [Mode::Off, Mode::Lamp, Mode::NightVision, Mode::Thermal];
        let here = order.iter().position(|mode| *mode == self.mode).unwrap_or(0);
        for step in 1..=order.len() {
            let next = order[(here + step) % order.len()];
            if self.available(next) {
                self.mode = next;
                return;
            }
        }
    }

    /// The lamp the player is actually holding when the dial says Lamp:
    /// the printed high beam if owned, the suit lamp otherwise.
    pub fn beam(&self) -> Beam {
        if self.owned.contains(HIGH_BEAM) {
            HIGH_BEAM_THROW
        } else {
            SUIT_BEAM
        }
    }

    /// What the HUD says about the dial, when it is not off.
    pub fn label(&self) -> Option<&'static str> {
        match self.mode {
            Mode::Off => None,
            Mode::Lamp => {
                if self.owned.contains(HIGH_BEAM) {
                    Some("HIGH BEAM")
                } else {
                    Some("LAMP")
                }
            }
            Mode::NightVision => Some("NIGHT VISION"),
            Mode::Thermal => Some("THERMAL"),
        }
    }

    /// The view-mode number the shader speaks: 0 plain, 1 night vision,
    /// 2 thermal. The lamp is not a view mode — it is a light.
    pub fn shader_mode(&self) -> f32 {
        match self.mode {
            Mode::Off | Mode::Lamp => 0.0,
            Mode::NightVision => 1.0,
            Mode::Thermal => 2.0,
        }
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "optics.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.owned.len() as u32).to_le_bytes())?;
        for name in &self.owned {
            let bytes = name.as_bytes();
            file.write_all(&(bytes.len() as u32).to_le_bytes())?;
            file.write_all(bytes)?;
        }
        let dial: u8 = match self.mode {
            Mode::Off => 0,
            Mode::Lamp => 1,
            Mode::NightVision => 2,
            Mode::Thermal => 3,
        };
        file.write_all(&[dial])?;
        file.commit()
    }

    /// Load, tolerating absence and damage — lost optics settings are a
    /// dial left at Off, not an error.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("optics.dat");
        match read_optics(&path) {
            Ok(Some((owned, mode))) => {
                self.owned = owned;
                // Never restore a dial pointing at gear that is not owned
                // (a hand-edited file, an older save): fall back rather
                // than rendering through a visor that does not exist.
                self.mode = if self.available(mode) { mode } else { Mode::Off };
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("unreadable {}: {error}", path.display());
            }
        }
    }
}

fn read_optics(path: &Path) -> std::io::Result<Option<(BTreeSet<String>, Mode)>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not an optics file"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    file.read_exact(&mut word)?;
    let count = u32::from_le_bytes(word);
    let mut owned = BTreeSet::new();
    for _ in 0..count {
        file.read_exact(&mut word)?;
        let mut name = vec![0u8; u32::from_le_bytes(word) as usize];
        file.read_exact(&mut name)?;
        owned.insert(
            String::from_utf8(name).map_err(|_| std::io::Error::other("garbled gear name"))?,
        );
    }
    let mut dial = [0u8; 1];
    file.read_exact(&mut dial)?;
    let mode = match dial[0] {
        1 => Mode::Lamp,
        2 => Mode::NightVision,
        3 => Mode::Thermal,
        _ => Mode::Off,
    };
    Ok(Some((owned, mode)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dial_skips_gear_you_do_not_own() {
        let mut optics = Optics::default();
        optics.cycle();
        assert_eq!(optics.mode, Mode::Lamp, "everyone owns the suit lamp");
        optics.cycle();
        assert_eq!(optics.mode, Mode::Off, "no visors yet, straight back to off");

        optics.owned.insert(THERMAL.to_string());
        optics.cycle();
        optics.cycle();
        assert_eq!(
            optics.mode,
            Mode::Thermal,
            "night vision is skipped when only thermal is owned"
        );
        optics.cycle();
        assert_eq!(optics.mode, Mode::Off);
    }

    #[test]
    fn the_high_beam_replaces_the_suit_lamp() {
        let mut optics = Optics::default();
        let suit = optics.beam();
        optics.owned.insert(HIGH_BEAM.to_string());
        let printed = optics.beam();
        assert!(printed.reach > suit.reach && printed.strength > suit.strength);
        optics.mode = Mode::Lamp;
        assert_eq!(optics.label(), Some("HIGH BEAM"));
    }

    #[test]
    fn optics_round_trip_through_the_save() {
        let directory = std::env::temp_dir().join(format!("vx-optics-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut optics = Optics::default();
        optics.owned.insert(NIGHT_VISION.to_string());
        optics.mode = Mode::NightVision;
        optics.save(&directory).unwrap();

        let mut loaded = Optics::default();
        loaded.load(&directory);
        assert_eq!(loaded.owned, optics.owned);
        assert_eq!(loaded.mode, Mode::NightVision);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_dial_pointing_at_missing_gear_falls_back_to_off() {
        let directory = std::env::temp_dir().join(format!("vx-optics-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut optics = Optics::default();
        optics.owned.insert(THERMAL.to_string());
        optics.mode = Mode::Thermal;
        optics.save(&directory).unwrap();

        // The same file read back with the gear gone would point at a visor
        // that does not exist; simulate by saving a dial with no gear list.
        let stripped = Optics {
            mode: Mode::Thermal,
            ..Optics::default()
        };
        stripped.save(&directory).unwrap();

        let mut loaded = Optics::default();
        loaded.load(&directory);
        assert_eq!(loaded.mode, Mode::Off, "an unavailable dial did not fall back");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn every_label_is_drawable() {
        for mode in [Mode::Lamp, Mode::NightVision, Mode::Thermal] {
            let mut optics = Optics {
                mode,
                ..Optics::default()
            };
            optics.owned.insert(HIGH_BEAM.to_string());
            for character in optics.label().unwrap().chars() {
                assert!(vx_render::font::knows(character), "undrawable {character:?}");
            }
        }
    }
}
