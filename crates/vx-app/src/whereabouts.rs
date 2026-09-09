//! Where you were standing and which way you were facing, on disk.
//!
//! # Why this file exists
//!
//! Every load put you back in bed.
//!
//! `App`'s boot builds the body at `town::spawn_position(&home)`
//! unconditionally and the camera at a yaw of ninety degrees — "you wake up in
//! your own house, facing the door" — and nothing anywhere read a saved
//! position, because nothing anywhere wrote one. The ground you changed came
//! back, the pile came back (since stage 50), the wallet, the skills, the
//! books, the chest, the tank and the wear ledger all came back. *You* did
//! not. Walk two hundred blocks to another town, sell your load, save, quit,
//! come back — and you are stood in Stonehaven in your own kitchen, two
//! hundred blocks from where you left off, with the walk to do again.
//!
//! It went unseen for the same reason the ditch did: every test that had ever
//! saved and loaded a world did it at spawn, where being put back at spawn
//! looks exactly like working.
//!
//! # What is and is not in here
//!
//! Where you are and where you are looking, and nothing else. Not your
//! stance — a save taken mid-crawl should not reload you mid-crawl in a place
//! the ceiling has since been dug out of — and not your velocity, because
//! reloading into a fall you started before quitting is a way to die at a
//! loading screen. You arrive stood up and still, which is what every game
//! that has ever done this does, and for the same reason.
//!
//! One concern per file, like every other side file here: absent means the
//! spawn, damaged is logged and means the spawn, and neither is ever a world
//! that fails to open.

use std::io::{Read, Write};
use std::path::Path;

use glam::DVec3;

const MAGIC: &[u8; 4] = b"VXYO";
const VERSION: u32 = 1;

/// Where the player was, and where they were looking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Whereabouts {
    pub position: DVec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// Write it to `whereabouts.dat`.
pub fn save(at: Whereabouts, directory: &Path) -> std::io::Result<()> {
    let mut file = crate::keeping::begin(directory, "whereabouts.dat")?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&at.position.x.to_le_bytes())?;
    file.write_all(&at.position.y.to_le_bytes())?;
    file.write_all(&at.position.z.to_le_bytes())?;
    file.write_all(&at.yaw.to_le_bytes())?;
    file.write_all(&at.pitch.to_le_bytes())?;
    file.commit()
}

/// Read it back, tolerating absence and damage.
///
/// `None` means "use the spawn": a fresh world, a world saved before this
/// round, or a file that will not parse. A world three thousand kilometres
/// wide can hold a position that is merely *odd*, so the only thing rejected
/// here is a number that is not a number — `NaN` in a position would put the
/// body outside every comparison the physics makes and wedge it there.
pub fn load(directory: &Path) -> Option<Whereabouts> {
    let path = directory.join("whereabouts.dat");
    match read(&path) {
        Ok(found) => found,
        Err(error) => {
            log::warn!(
                "ignoring damaged whereabouts at {}: {error}; starting at the spawn",
                path.display()
            );
            None
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Whereabouts>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a whereabouts file"));
    }
    if read_u32(&mut file)? != VERSION {
        return Ok(None);
    }
    let at = Whereabouts {
        position: DVec3::new(read_f64(&mut file)?, read_f64(&mut file)?, read_f64(&mut file)?),
        yaw: read_f32(&mut file)?,
        pitch: read_f32(&mut file)?,
    };
    if !at.position.is_finite() || !at.yaw.is_finite() || !at.pitch.is_finite() {
        return Err(std::io::Error::other("a whereabouts that is not a number"));
    }
    Ok(Some(at))
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_f32(file: &mut impl Read) -> std::io::Result<f32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(f32::from_le_bytes(word))
}

fn read_f64(file: &mut impl Read) -> std::io::Result<f64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(f64::from_le_bytes(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("vx-where-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// The bug, pinned: somewhere that is emphatically not the spawn comes
    /// back as itself.
    #[test]
    fn where_you_stood_comes_back() {
        let directory = scratch("round");
        let stood = Whereabouts {
            position: DVec3::new(-147.5, 122.0, -140.25),
            yaw: -2.75,
            pitch: 0.4,
        };
        save(stood, &directory).unwrap();
        let back = load(&directory).expect("nothing came back");
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(back, stood);
    }

    /// Three thousand kilometres out, to the millimetre. The body has been
    /// `f64` since stage 42 and the file has to be too, or a save is a
    /// teleport of a few centimetres every time you quit.
    #[test]
    fn a_position_a_long_way_out_keeps_every_digit() {
        let directory = scratch("far");
        let stood = Whereabouts {
            position: DVec3::new(3_000_000.125, 71.5, -2_999_999.875),
            yaw: 1.25,
            pitch: -0.5,
        };
        save(stood, &directory).unwrap();
        let back = load(&directory).expect("nothing came back");
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(back.position, stood.position, "the far position drifted");
    }

    #[test]
    fn a_missing_or_damaged_file_means_the_spawn() {
        let directory = scratch("damaged");
        assert!(load(&directory).is_none(), "a missing file invented a place");

        std::fs::write(directory.join("whereabouts.dat"), b"NOPE and then some").unwrap();
        assert!(load(&directory).is_none(), "a damaged file invented a place");

        // Truncated: the magic is right and the rest is not there.
        std::fs::write(directory.join("whereabouts.dat"), b"VXYO\x01").unwrap();
        assert!(load(&directory).is_none(), "a truncated file invented a place");
        std::fs::remove_dir_all(&directory).ok();
    }

    /// A `NaN` position would be outside every comparison the sweep makes and
    /// would wedge the body there for good, so it is refused at the door.
    #[test]
    fn a_position_that_is_not_a_number_is_refused() {
        let directory = scratch("nan");
        save(
            Whereabouts {
                position: DVec3::new(f64::NAN, 70.0, 0.0),
                yaw: 0.0,
                pitch: 0.0,
            },
            &directory,
        )
        .unwrap();
        assert!(load(&directory).is_none(), "a NaN position was accepted");
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn the_same_place_writes_the_same_bytes() {
        let directory = scratch("stable");
        let stood = Whereabouts {
            position: DVec3::new(1.0, 2.0, 3.0),
            yaw: 0.5,
            pitch: 0.25,
        };
        save(stood, &directory).unwrap();
        let first = std::fs::read(directory.join("whereabouts.dat")).unwrap();
        save(stood, &directory).unwrap();
        let second = std::fs::read(directory.join("whereabouts.dat")).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(first, second);
    }
}
