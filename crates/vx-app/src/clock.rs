//! Time of day, and the light that comes with it.
//!
//! One mapping from a fraction of a day to a sun direction, a sky colour and a
//! light level — pure, total, and the only place the palette lives. The
//! renderer never reads a clock; it is *pushed* a [`vx_render::SunUniform`],
//! which is what keeps headless captures reproducible and the pixel-equality
//! tests honest.
//!
//! Time is persisted in its own small file, following the same rule as the
//! skill sheet, the wallet and the map: one concern per file, no migrations.
//! Folding the hour into `wallet.dat` would force a version bump on a loader
//! that rejects unknown versions outright, and would silently wipe a player's
//! credits the first time they ran a new build against an old save.

use std::io::{Read, Write};
use std::path::Path;

use glam::Vec3;
use vx_render::SunUniform;

const MAGIC: &[u8; 4] = b"VXTM";
const VERSION: u32 = 1;

/// Real seconds in one in-game day.
pub const DAY_SECONDS: f32 = 1200.0;

/// A fraction of a day: 0.0 midnight, 0.25 dawn, 0.5 noon, 0.75 dusk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeOfDay(f32);

impl TimeOfDay {
    pub const MIDNIGHT: TimeOfDay = TimeOfDay(0.0);
    pub const DAWN: TimeOfDay = TimeOfDay(0.25);
    pub const NOON: TimeOfDay = TimeOfDay(0.5);
    pub const DUSK: TimeOfDay = TimeOfDay(0.75);

    /// A fresh world opens mid-morning: the first frame a player ever sees
    /// should be daylight, not an explanation.
    pub const START: TimeOfDay = TimeOfDay(0.35);

    /// Wraps into `0..1`, so callers never have to think about the seam.
    pub fn new(fraction: f32) -> Self {
        TimeOfDay(fraction.rem_euclid(1.0))
    }

    pub fn fraction(self) -> f32 {
        self.0
    }

    pub fn advance(self, seconds: f32) -> Self {
        TimeOfDay::new(self.0 + seconds / DAY_SECONDS)
    }

    /// The clock face, for the HUD.
    pub fn hhmm(self) -> (u32, u32) {
        let minutes = (self.0 * 24.0 * 60.0).round() as u32 % (24 * 60);
        (minutes / 60, minutes % 60)
    }

    /// Are the shops open? The single predicate the town keeps hours by.
    pub fn is_daylight(self) -> bool {
        (0.24..0.78).contains(&self.0)
    }
}

impl Default for TimeOfDay {
    fn default() -> Self {
        TimeOfDay::START
    }
}

/// Everything the frame needs to draw this moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyState {
    /// Unit vector toward the key light.
    pub sun_direction: Vec3,
    /// Linear RGB: the clear colour *and* the fog.
    pub sky: [f32; 3],
    pub light: f32,
    /// Never zero — a pitch-black night is unreadable, not atmospheric.
    pub ambient: f32,
}

/// Keyframes: midnight, dawn, noon, dusk, and back to midnight.
const KEYS: [(f32, [f32; 3], f32, f32); 5] = [
    (0.00, [0.02, 0.03, 0.06], 0.05, 0.14),
    (0.25, [0.55, 0.42, 0.38], 0.30, 0.26),
    (0.50, [0.62, 0.74, 0.88], 0.58, 0.42),
    (0.75, [0.62, 0.35, 0.22], 0.30, 0.26),
    (1.00, [0.02, 0.03, 0.06], 0.05, 0.14),
];

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The one mapping from time to light.
pub fn sky_at(time: TimeOfDay) -> SkyState {
    let at = time.fraction();
    let mut window = (&KEYS[0], &KEYS[1]);
    for pair in KEYS.windows(2) {
        if at >= pair[0].0 && at <= pair[1].0 {
            window = (&pair[0], &pair[1]);
            break;
        }
    }
    let (from, to) = window;
    let span = (to.0 - from.0).max(f32::EPSILON);
    let t = ((at - from.0) / span).clamp(0.0, 1.0);

    let sky = [
        lerp(from.1[0], to.1[0], t),
        lerp(from.1[1], to.1[1], t),
        lerp(from.1[2], to.1[2], t),
    ];

    // The sun sweeps a full circle: up in the east at dawn, overhead at noon,
    // down in the west at dusk. After dark the same vector points at the moon
    // instead, so the world keeps its readable shading rather than going flat.
    let angle = (at - 0.25) * std::f32::consts::TAU;
    let sun_direction = Vec3::new(angle.cos(), angle.sin(), 0.29).normalize();

    SkyState {
        sun_direction,
        sky,
        light: lerp(from.2, to.2, t),
        ambient: lerp(from.3, to.3, t),
    }
}

/// Hand a moment to the renderer's uniform.
pub fn sun_uniform(state: SkyState) -> SunUniform {
    // Below the horizon the key light is moonlight: keep the direction usable
    // by flipping it up, so faces still separate at night.
    let direction = if state.sun_direction.y < 0.0 {
        Vec3::new(state.sun_direction.x, -state.sun_direction.y, state.sun_direction.z)
            .normalize()
    } else {
        state.sun_direction
    };
    SunUniform {
        direction: [direction.x, direction.y, direction.z, 0.0],
        sky: [state.sky[0], state.sky[1], state.sky[2], 1.0],
        light: [state.light, state.ambient, 0.0, 0.0],
        // The lamp and the view mode are the player's, not the sky's: main
        // fills them in after asking the clock what the sun is doing.
        ..SunUniform::default()
    }
}

/// Lean the sky and the light towards the season.
///
/// Applied *after* the hour and *under* the weather's own tint, in that
/// order, because that is the order they happen in: the sun decides where the
/// light is coming from, the month decides what it is like, and the cloud in
/// front of it decides how much of it arrives. Reversing the last two would
/// make an overcast January and an overcast July the same afternoon.
///
/// Deliberately small. A winter sky that read as a different game would be a
/// filter rather than a season, and the country is one country.
pub fn tint_for_season(sun: &mut SunUniform, tick: u64) {
    let warmth = vx_world::season::warmth(tick);
    // Winter is pale and flat: the blue drains out of the top of the sky and
    // the whole thing comes down towards the horizon's grey.
    let winter = (-warmth).clamp(0.0, 1.0);
    // Summer is the other way — harder, deeper, more of it.
    let summer = warmth.clamp(0.0, 1.0);

    let pale = [0.70, 0.72, 0.76];
    let deep = [0.30, 0.54, 0.92];
    for channel in 0..3 {
        let towards = pale[channel] * winter + deep[channel] * summer;
        let amount = (winter + summer) * SEASON_SKY;
        sun.sky[channel] = sun.sky[channel] * (1.0 - amount) + towards * amount;
    }
    // A low winter sun is a weaker one, and it fills in less. Summer barely
    // moves: the hour already gives it a long day, and doubling up would
    // blow the highlights out.
    sun.light[0] *= 1.0 - 0.20 * winter + 0.05 * summer;
    sun.light[1] = (sun.light[1] + 0.06 * winter).min(0.6);
}

/// How far the year is allowed to move the sky, at its extremes.
///
/// A third, which is enough to read at a glance and not enough to make a
/// January and a July look like two different games. It moves the fog with
/// it, because the sky and the horizon are one value here by construction —
/// so a summer distance goes blue and a winter one goes flat, which is the
/// half of a season you notice without being told.
const SEASON_SKY: f32 = 0.34;

/// Write the hour beside the world save.
pub fn save(time: TimeOfDay, directory: &Path) -> std::io::Result<()> {
    let mut file = crate::keeping::begin(directory, "clock.dat")?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&time.fraction().to_le_bytes())?;
    file.commit()
}

/// Read it back, tolerating absence and damage — a broken clock is dawn,
/// logged, never a failed world.
pub fn load(directory: &Path) -> TimeOfDay {
    let path = directory.join("clock.dat");
    match read_clock(&path) {
        Ok(Some(time)) => time,
        Ok(None) => TimeOfDay::START,
        Err(error) => {
            log::warn!("could not read {}: {error}; starting fresh", path.display());
            TimeOfDay::START
        }
    }
}

fn read_clock(path: &Path) -> std::io::Result<Option<TimeOfDay>> {
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
    let fraction = f32::from_le_bytes(word);
    if !fraction.is_finite() {
        return Err(std::io::Error::other("time is not a number"));
    }
    Ok(Some(TimeOfDay::new(fraction)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_wraps_and_advances_at_the_documented_rate() {
        let dawn = TimeOfDay::DAWN;
        assert_eq!(dawn.advance(DAY_SECONDS), dawn, "a full day is not a full day");
        let quarter = dawn.advance(DAY_SECONDS / 4.0);
        assert!((quarter.fraction() - 0.5).abs() < 1.0e-5);
        // Past midnight and out the other side.
        assert!(TimeOfDay::new(0.99).advance(DAY_SECONDS * 0.02).fraction() < 0.5);
    }

    #[test]
    fn the_clock_face_reads_the_way_a_clock_does() {
        assert_eq!(TimeOfDay::MIDNIGHT.hhmm(), (0, 0));
        assert_eq!(TimeOfDay::NOON.hhmm(), (12, 0));
        assert_eq!(TimeOfDay::DAWN.hhmm(), (6, 0));
        assert_eq!(TimeOfDay::DUSK.hhmm(), (18, 0));
    }

    #[test]
    fn the_sky_is_continuous_across_midnight() {
        // A visible seam at the wrap would be a bright flash once a day.
        let before = sky_at(TimeOfDay::new(0.999));
        let after = sky_at(TimeOfDay::new(0.001));
        for channel in 0..3 {
            assert!(
                (before.sky[channel] - after.sky[channel]).abs() < 0.02,
                "sky jumps at midnight on channel {channel}"
            );
        }
        assert!((before.light - after.light).abs() < 0.02);
        assert!((before.ambient - after.ambient).abs() < 0.02);
        assert!(
            (before.sun_direction - after.sun_direction).length() < 0.05,
            "the sun jumps at midnight"
        );
    }

    #[test]
    fn noon_is_brighter_than_midnight_and_night_is_never_black() {
        let noon = sky_at(TimeOfDay::NOON);
        let midnight = sky_at(TimeOfDay::MIDNIGHT);
        assert!(noon.light > midnight.light);
        assert!(noon.ambient > midnight.ambient);
        assert!(noon.sky[2] > midnight.sky[2]);
        assert!(midnight.ambient > 0.0, "a pitch-black night is unplayable");
    }

    #[test]
    fn the_sun_is_a_unit_vector_all_day_and_rises_in_the_east() {
        for step in 0..64 {
            let time = TimeOfDay::new(step as f32 / 64.0);
            let state = sky_at(time);
            assert!(
                (state.sun_direction.length() - 1.0).abs() < 1.0e-4,
                "the sun is not a unit vector at {time:?}"
            );
        }
        // Dawn: level and to the east. Noon: overhead.
        assert!(sky_at(TimeOfDay::DAWN).sun_direction.x > 0.9);
        assert!(sky_at(TimeOfDay::NOON).sun_direction.y > 0.9);
    }

    #[test]
    fn the_uniform_keeps_the_light_above_the_horizon() {
        // Below the horizon the key light becomes the moon; a light coming
        // from underneath would leave every top face black.
        let midnight = sun_uniform(sky_at(TimeOfDay::MIDNIGHT));
        assert!(midnight.direction[1] > 0.0, "the night light points into the ground");
        // And the sky drives the fog, so the two are the same value.
        let noon = sun_uniform(sky_at(TimeOfDay::NOON));
        assert_eq!(&noon.sky[0..3], &sky_at(TimeOfDay::NOON).sky[..]);
    }

    #[test]
    fn a_fresh_world_starts_in_daylight() {
        assert!(TimeOfDay::default().is_daylight());
        assert!(!TimeOfDay::MIDNIGHT.is_daylight());
        assert!(TimeOfDay::NOON.is_daylight());
    }

    #[test]
    fn the_clock_round_trips_and_tolerates_damage() {
        let directory = std::env::temp_dir().join(format!("vx-clock-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let evening = TimeOfDay::new(0.71);
        save(evening, &directory).unwrap();
        assert!((load(&directory).fraction() - 0.71).abs() < 1.0e-6);

        std::fs::write(directory.join("clock.dat"), b"NOPE not a clock").unwrap();
        assert_eq!(load(&directory), TimeOfDay::START, "a damaged clock should start fresh");

        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(load(&directory), TimeOfDay::START, "a missing clock should start fresh");
    }
}
