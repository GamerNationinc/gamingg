//! The drill mod's two switches: the cage of light, and the sonar ping.
//!
//! # Two switches, because they are two different things
//!
//! The cage ([`crate::hologram`]) draws the block you are aiming at. The
//! ping ([`crate::sonar`]) reads the four metres of ground *around* it.
//! Bundling them behind one key would mean a player who wants the outline
//! but finds the ping busy has to give up both, and a player who wants the
//! ping in a dark shaft has to accept a cyan box in every screenshot. So:
//! `H` for the cage, `P` for the ping, each remembered where you left it.
//!
//! Both are free. There is no purchase, no upgrade line and no skill gate —
//! the cage is on any basic drill, which is what was asked for, and the
//! ping is on the same terms because a mod that reads the rock is worth
//! nothing to a player who cannot afford it yet and is drowning in ore by
//! the time they can.
//!
//! # It is a lens, not a lever
//!
//! Neither switch changes anything the world can see. No block moves, no
//! good moves, no number changes; the whole feature is a way of *looking*.
//! That is why it is on disk but not on the wire: `drillmod.dat` remembers
//! where you left the switches, and the journal never hears about them at
//! all. [`crate::optics`] made exactly this argument for the lamp and the
//! visors, and this file follows it.
//!
//! The claim gets a test rather than a comment — see
//! `the_drill_mod_is_a_lens_not_a_lever` in [`crate::session`], which plays
//! the same session twice with the switches on and off and demands
//! identical journal bytes and an identical world hash. If a pretty effect
//! could ever change what happens, that test goes red.

use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"VXDM";
const VERSION: u32 = 1;

/// Journal ticks between pings, so sweeping the trigger across a wall does
/// not machine-gun the scope.
///
/// Counted in ticks and not in seconds because a tick is the unit the
/// headless fixture and the live game share: a capture reproduces the same
/// pings in the same order, which a wall clock could never promise.
pub const PING_REST: u64 = 3;

/// Where the player left the drill mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switches {
    /// Draw the cage on the block under the bit.
    pub cage: bool,
    /// Ping the ground around every new block the bit touches.
    pub ping: bool,
}

impl Default for Switches {
    /// Both on. A feature nobody can see is a feature nobody has, and both
    /// of these are free — so the default is the one that shows the player
    /// what their drill can do, and the keys are there to turn it down.
    fn default() -> Self {
        Switches {
            cage: true,
            ping: true,
        }
    }
}

impl Switches {
    /// Flip the cage, and say what happened.
    pub fn toggle_cage(&mut self) -> &'static str {
        self.cage = !self.cage;
        if self.cage {
            "DRILL HOLOGRAM ON"
        } else {
            "DRILL HOLOGRAM OFF"
        }
    }

    /// Flip the ping, and say what happened.
    pub fn toggle_ping(&mut self) -> &'static str {
        self.ping = !self.ping;
        if self.ping {
            "DRILL SONAR ON"
        } else {
            "DRILL SONAR OFF"
        }
    }

    /// Both switches, as the terminal reports them.
    pub fn lines(&self) -> Vec<String> {
        let state = |on: bool| if on { "ON" } else { "OFF" };
        vec![
            format!("HOLOGRAM {} - H", state(self.cage)),
            format!("SONAR {} - P", state(self.ping)),
        ]
    }

    /// Write them out.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file =
            std::io::BufWriter::new(std::fs::File::create(directory.join("drillmod.dat"))?);
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&[u8::from(self.cage), u8::from(self.ping)])?;
        file.flush()
    }

    /// Read them back, tolerating absence and damage.
    ///
    /// A missing file means a world saved before this stage, or a fresh one:
    /// both get the default, which is both switches on. A damaged file is a
    /// warning and the default — never a failed world, for a pair of bools
    /// about how the game looks.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("drillmod.dat");
        match read(&path) {
            Ok(Some(switches)) => *self = switches,
            Ok(None) => {}
            Err(error) => {
                log::warn!("ignoring damaged drill mod at {}: {error}", path.display())
            }
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Switches>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a drill mod file"));
    }
    let mut version = [0u8; 4];
    file.read_exact(&mut version)?;
    if u32::from_le_bytes(version) != VERSION {
        return Ok(None);
    }
    let mut switches = [0u8; 2];
    file.read_exact(&mut switches)?;
    Ok(Some(Switches {
        cage: switches[0] != 0,
        ping: switches[1] != 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_render::font;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("gamingg-drillmod-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    /// Free on any drill: the default is both on, so a player who never
    /// finds the keys still gets the feature.
    #[test]
    fn both_switches_start_on() {
        let switches = Switches::default();
        assert!(switches.cage && switches.ping);
    }

    #[test]
    fn both_switches_survive_a_save() {
        let directory = scratch("round-trip");
        let mut switches = Switches::default();
        switches.toggle_cage();
        switches.save(&directory).expect("save");

        let mut read_back = Switches::default();
        read_back.load(&directory);
        assert_eq!(read_back, switches);
        assert!(!read_back.cage, "the cage came back on");
        assert!(read_back.ping, "the ping came back off");
    }

    /// The house rule, for the smallest file in the game as much as for the
    /// biggest: absent is the default, damaged is a warning and the default,
    /// and neither is ever a world that will not load.
    #[test]
    fn a_missing_or_corrupt_file_leaves_both_switches_on() {
        let directory = scratch("corrupt");
        let mut switches = Switches::default();
        switches.toggle_cage();
        switches.toggle_ping();
        switches.load(&directory);
        assert_eq!(switches, Switches { cage: false, ping: false }, "absence overwrote");

        std::fs::write(directory.join("drillmod.dat"), b"not a drill mod at all").expect("write");
        let mut fresh = Switches::default();
        fresh.load(&directory);
        assert_eq!(fresh, Switches::default(), "a damaged file was believed");

        // And an unknown version is the same story, not a panic.
        let mut future = MAGIC.to_vec();
        future.extend_from_slice(&99u32.to_le_bytes());
        future.extend_from_slice(&[0, 0]);
        std::fs::write(directory.join("drillmod.dat"), future).expect("write");
        let mut later = Switches::default();
        later.load(&directory);
        assert_eq!(later, Switches::default());
    }

    #[test]
    fn a_switch_says_which_way_it_went() {
        let mut switches = Switches::default();
        assert_eq!(switches.toggle_cage(), "DRILL HOLOGRAM OFF");
        assert_eq!(switches.toggle_cage(), "DRILL HOLOGRAM ON");
        assert_eq!(switches.toggle_ping(), "DRILL SONAR OFF");
        assert_eq!(switches.toggle_ping(), "DRILL SONAR ON");
    }

    /// Everything the player can be shown has to be drawable.
    #[test]
    fn every_line_the_switches_say_is_drawable() {
        let mut switches = Switches::default();
        let mut all: Vec<String> = switches.lines();
        for _ in 0..2 {
            all.push(switches.toggle_cage().to_string());
            all.push(switches.toggle_ping().to_string());
            all.extend(switches.lines());
        }
        for line in all {
            for character in line.chars() {
                assert!(font::knows(character), "{character:?} in {line:?} is not drawable");
            }
        }
    }
}
