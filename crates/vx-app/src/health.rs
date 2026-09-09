//! What the player can take, and what happens when they cannot take any more.
//!
//! # Live-only, like every other reaction to the player
//!
//! Health never reaches the replay oracle. The journal records *orders* and
//! the hash covers *ground*, and being shot is neither: it is a reaction to
//! where the player stood, exactly like a villager's panic, the roost's
//! attention or a contact mark. So this lives in its own small file beside
//! the wallet and the friendship ledger, and a replay re-runs the same
//! orders over the same world without ever learning that anybody was hit.
//!
//! # Quiet, then mend
//!
//! There is no medkit economy and this round does not invent one. Take a
//! round and the count drops; break contact and stay unhit for
//! [`QUIET_SECONDS`] and it climbs back, one hit at a time. That makes
//! *disengaging* the heal — which suits a game whose combat is a posse you
//! provoked and can still walk away from.
//!
//! # Down is arrested, not dead
//!
//! Falling to zero in front of the law ends with a fine, not a grave. The
//! crime systems have been building toward this since permits shipped:
//! crime raises bounty, bounty crosses the warrant threshold, the warrant
//! sends deputies, the deputies put you down, and being put down settles the
//! bill. Dying to something that is *not* the law waits for a round with
//! something else in it.

use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"VXHP";
/// Two carries the medkits you have bought. One byte more, and a version-one
/// file still loads — it simply reports an empty pocket, which is what an
/// older save honestly means.
const VERSION: u32 = 2;

/// Hits a whole person can take.
pub const MAX_HITS: u8 = 6;

/// Seconds without being hit before the count starts climbing back.
pub const QUIET_SECONDS: f32 = 8.0;

/// Seconds per hit recovered, once the quiet has held.
pub const MEND_SECONDS: f32 = 4.0;

/// What one round from a deputy's carbine takes.
pub const ROUND_HITS: u8 = 1;

/// What one medkit puts back.
///
/// Two rather than everything: a medkit is what gets you out of a gallery,
/// not a bed. The bed is in town, it is free, and walking to it is the
/// price.
pub const MEDKIT_HITS: u8 = 2;

/// The most you can carry. Small enough that a fight still has to end.
pub const MEDKITS_MAX: u8 = 5;

/// How the player is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct Health {
    hits: u8,
    /// Seconds since the last hit landed.
    quiet: f32,
    /// Fractional progress toward the next hit back.
    mending: f32,
    /// Medkits in the pocket. Carried rather than piled: a medkit is not a
    /// trade good, it never reaches the base stockpile, and so the replay
    /// oracle never has to hear about one.
    medkits: u8,
}

impl Default for Health {
    fn default() -> Self {
        Health {
            hits: MAX_HITS,
            quiet: QUIET_SECONDS,
            mending: 0.0,
            medkits: 0,
        }
    }
}

impl Health {
    /// Hits left.
    pub fn hits(&self) -> u8 {
        self.hits
    }

    /// Is the player still standing?
    pub fn standing(&self) -> bool {
        self.hits > 0
    }

    /// Has the player taken anything at all?
    pub fn hurt(&self) -> bool {
        self.hits < MAX_HITS
    }

    /// Take damage. Returns whether this is the blow that put them down.
    pub fn take(&mut self, hits: u8) -> bool {
        let standing = self.standing();
        self.hits = self.hits.saturating_sub(hits);
        self.quiet = 0.0;
        self.mending = 0.0;
        standing && !self.standing()
    }

    /// Back on your feet, whole. What arrest, waking up at home and a night
    /// on a ward cot all do.
    ///
    /// Keeps what you are carrying: being patched up is not being robbed.
    pub fn revive(&mut self) {
        let medkits = self.medkits;
        *self = Health::default();
        self.medkits = medkits;
    }

    /// Medkits in the pocket.
    pub fn medkits(&self) -> u8 {
        self.medkits
    }

    /// Buy one, up to what a person can carry. False when the pocket is full.
    pub fn stock_medkit(&mut self) -> bool {
        if self.medkits >= MEDKITS_MAX {
            return false;
        }
        self.medkits += 1;
        true
    }

    /// Use one where you stand. Returns whether it went into somebody who
    /// needed it — a medkit is not spent on a whole player, and refusing to
    /// waste it is kinder than letting them.
    pub fn patch(&mut self) -> Result<u8, String> {
        if self.medkits == 0 {
            return Err("NO MEDKITS".to_string());
        }
        if !self.hurt() {
            return Err("NOTHING TO PATCH".to_string());
        }
        self.medkits -= 1;
        let before = self.hits;
        self.hits = (self.hits + MEDKIT_HITS).min(MAX_HITS);
        // A patch does not restart the mend clock: it is help, not a fresh
        // wound.
        Ok(self.hits - before)
    }

    /// One frame of quiet, or of not-quiet. Returns whether a hit came back,
    /// which is worth a line on the HUD.
    pub fn tick(&mut self, dt: f32) -> bool {
        if !self.standing() {
            // Down is down: it does not quietly mend itself while you lie
            // there. Something has to happen to you first.
            return false;
        }
        self.quiet += dt;
        if self.quiet < QUIET_SECONDS || self.hits >= MAX_HITS {
            return false;
        }
        self.mending += dt;
        if self.mending < MEND_SECONDS {
            return false;
        }
        self.mending = 0.0;
        self.hits = (self.hits + 1).min(MAX_HITS);
        true
    }

    /// What the HUD says, or nothing while whole — a bar that is always
    /// there is a bar nobody reads.
    pub fn readout(&self) -> Option<String> {
        if !self.standing() {
            return Some("DOWN".to_string());
        }
        if !self.hurt() {
            return None;
        }
        Some(format!("HITS {}/{MAX_HITS}", self.hits))
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "health.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&[self.hits])?;
        file.write_all(&[self.medkits])?;
        file.commit()
    }

    /// Read it back. A missing or damaged file is a whole player, which is
    /// generous and harmless — the same line every other ledger here draws.
    pub fn load(&mut self, directory: &Path) {
        match read(&directory.join("health.dat")) {
            Ok(Some((hits, medkits))) => {
                *self = Health::default();
                // A save that says you were down puts you back on your feet:
                // waking up in a fresh session already beaten is a worse bug
                // than forgetting a scratch.
                self.hits = hits.clamp(1, MAX_HITS);
                self.medkits = medkits.min(MEDKITS_MAX);
            }
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged health file: {error}"),
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<(u8, u8)>> {
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
    let mut hits = [0u8; 1];
    file.read_exact(&mut hits)?;
    // The medkit byte is version two's. A file that stops here is a version
    // one file and an empty pocket, which is what it honestly says.
    let mut medkits = [0u8; 1];
    let carried = match file.read_exact(&mut medkits) {
        Ok(()) => medkits[0],
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => 0,
        Err(error) => return Err(error),
    };
    Ok(Some((hits[0], carried)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_player_says_nothing_and_a_hurt_one_speaks_up() {
        let mut health = Health::default();
        assert_eq!(health.hits(), MAX_HITS);
        assert!(health.standing() && !health.hurt());
        assert_eq!(health.readout(), None, "a whole player has a bar to read");

        assert!(!health.take(1), "one round should not put anybody down");
        assert!(health.hurt());
        assert_eq!(health.readout().as_deref(), Some("HITS 5/6"));
    }

    #[test]
    fn the_blow_that_downs_you_reports_itself_exactly_once() {
        // The caller hangs an arrest off this bool, so it must be true on
        // the transition and never again.
        let mut health = Health::default();
        for _ in 0..MAX_HITS - 1 {
            assert!(!health.take(1));
        }
        assert!(health.take(1), "the last hit did not report going down");
        assert!(!health.standing());
        assert!(!health.take(1), "reported going down twice");
        assert_eq!(health.readout().as_deref(), Some("DOWN"));
    }

    #[test]
    fn breaking_contact_is_the_heal() {
        let mut health = Health::default();
        health.take(3);
        assert_eq!(health.hits(), 3);

        // Still under fire: the clock keeps resetting and nothing comes back.
        for _ in 0..200 {
            health.tick(0.1);
            health.take(0);
        }
        assert_eq!(health.hits(), 3, "mended while still being shot at");

        // Quiet: the first hit takes the wait plus the mend, and the rest
        // come one mend apart.
        let mut seconds = 0.0;
        while health.hits() == 3 {
            health.tick(0.1);
            seconds += 0.1;
            assert!(seconds < 60.0, "never mended at all");
        }
        assert!(
            seconds >= QUIET_SECONDS,
            "mended before the quiet had held: {seconds}s"
        );
    }

    #[test]
    fn being_down_does_not_quietly_fix_itself() {
        let mut health = Health::default();
        health.take(MAX_HITS);
        for _ in 0..1_000 {
            health.tick(0.1);
        }
        assert!(!health.standing(), "stood back up on their own");
        health.revive();
        assert_eq!(health.hits(), MAX_HITS);
    }

    #[test]
    fn health_round_trips_and_never_loads_you_already_beaten() {
        // A directory of its own: tests run as threads in one process, and
        // two of them sharing `health.dat` is a race that only shows up
        // under some orderings.
        let directory =
            std::env::temp_dir().join(format!("vx-health-trip-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut health = Health::default();
        health.take(2);
        health.save(&directory).unwrap();
        let mut read_back = Health::default();
        read_back.load(&directory);
        assert_eq!(read_back.hits(), 4);

        // A session that ended with you on the floor starts with you upright
        // but not whole: waking up already down is unplayable.
        let mut downed = Health::default();
        downed.take(MAX_HITS);
        downed.save(&directory).unwrap();
        let mut woken = Health::default();
        woken.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert!(woken.standing(), "woke up already beaten");
    }

    #[test]
    fn a_missing_or_damaged_file_is_a_whole_player() {
        let directory = std::env::temp_dir().join(format!("vx-health-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut health = Health::default();
        health.load(&directory);
        assert_eq!(health, Health::default());

        std::fs::write(directory.join("health.dat"), b"junk").unwrap();
        let mut health = Health::default();
        health.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(health, Health::default());
    }

    #[test]
    fn every_readout_is_drawable() {
        let mut health = Health::default();
        for _ in 0..=MAX_HITS {
            if let Some(line) = health.readout() {
                for character in line.chars() {
                    assert!(vx_render::font::knows(character), "undrawable {character:?}");
                }
            }
            health.take(1);
        }
    }

    #[test]
    fn a_medkit_is_spent_on_somebody_who_needs_it_and_nobody_else() {
        let mut health = Health::default();
        assert!(health.stock_medkit());
        assert_eq!(health.medkits(), 1);

        // Whole: the kit stays in the pocket rather than being wasted.
        assert_eq!(health.patch(), Err("NOTHING TO PATCH".to_string()));
        assert_eq!(health.medkits(), 1);

        health.take(3);
        assert_eq!(health.patch(), Ok(MEDKIT_HITS));
        assert_eq!(health.hits(), MAX_HITS - 3 + MEDKIT_HITS);
        assert_eq!(health.medkits(), 0);
        assert_eq!(health.patch(), Err("NO MEDKITS".to_string()));
    }

    #[test]
    fn a_patch_never_overheals_and_a_pocket_has_a_bottom_and_a_top() {
        let mut health = Health::default();
        for _ in 0..MEDKITS_MAX + 3 {
            health.stock_medkit();
        }
        assert_eq!(health.medkits(), MEDKITS_MAX, "the pocket has no limit");

        health.take(1);
        assert_eq!(health.patch(), Ok(1), "a patch healed past whole");
        assert_eq!(health.hits(), MAX_HITS);
    }

    #[test]
    fn resting_puts_you_right_and_leaves_your_pockets_alone() {
        // The ward cot's promise: whole again, and nobody went through your
        // bag while you were under.
        let mut health = Health::default();
        health.stock_medkit();
        health.stock_medkit();
        health.take(5);
        assert!(health.hurt());

        health.revive();
        assert_eq!(health.hits(), MAX_HITS);
        assert_eq!(health.medkits(), 2, "the ward kept your kit");
    }

    #[test]
    fn the_medkits_survive_a_save_and_an_older_file_still_loads() {
        let directory =
            std::env::temp_dir().join(format!("vx-health-kit-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut health = Health::default();
        health.take(2);
        health.stock_medkit();
        health.stock_medkit();
        health.save(&directory).unwrap();

        let mut loaded = Health::default();
        loaded.load(&directory);
        assert_eq!(loaded.hits(), MAX_HITS - 2);
        assert_eq!(loaded.medkits(), 2);

        // A version-one file — magic, version, one byte of hits — is an
        // empty pocket rather than a refusal.
        let path = directory.join("health.dat");
        let mut older = Vec::new();
        older.extend_from_slice(MAGIC);
        older.extend_from_slice(&VERSION.to_le_bytes());
        older.push(4);
        std::fs::write(&path, older).unwrap();
        let mut old = Health::default();
        old.load(&directory);
        assert_eq!(old.hits(), 4);
        assert_eq!(old.medkits(), 0);

        std::fs::remove_dir_all(&directory).ok();
    }
}
