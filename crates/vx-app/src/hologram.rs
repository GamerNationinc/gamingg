//! The drill's cage of light: what the block under the bit looks like.
//!
//! # This is the selection box this game never had
//!
//! Fifty-two stages in, nothing was drawn on the block you were aiming at.
//! No outline, no crosshair, nothing — the only way to know what was under
//! the bit was to open the F3 panel and read the name off a text row. So
//! this module is not decoration bolted onto an existing highlight. It *is*
//! the highlight, and it is written to read as one: quiet on a block you are
//! merely looking at, bright on the block the bit is in, with a plane of
//! light that rises through it as the drill chews.
//!
//! Free on any drill. Nothing here asks the wallet or the skill sheet a
//! question, and the signature is the proof: there is nowhere to pass one in.
//!
//! # Why bars and not a shell
//!
//! The obvious hologram is a translucent box round the block. It is the wrong
//! shape here, for three reasons that all live in `vx-render`: the object
//! pipeline writes depth, it blends unsorted, and it culls back faces. A
//! clear box would z-fight the block's own faces, occlude whatever
//! translucent thing happened to draw after it, and vanish when you stepped
//! inside it. Twelve thin *opaque* bars standing [`SWELL`] off the surface
//! have none of those problems and read as a projected wireframe besides,
//! which is the look that was asked for.
//!
//! The glow is [`vx_render::Object::light`] pushed past 1.0. The shader
//! multiplies `light` into the lighting and clamps at 1.4, so up to a 40%
//! overbright is reachable with no new shader, no new instance attribute and
//! no new pipeline.
//!
//! # The trap
//!
//! `App::frame` walks the whole frame's object list and **overwrites**
//! `object.light` from the column-depth rule, so that a drone standing in a
//! hole is as dark as the hole. Anything from this module must be appended
//! to the list *after* that loop, or the cage is a dull grey box in a dark
//! mine — which is exactly where it is most wanted.
//!
//! # Nothing here is a lever
//!
//! Every function in this file is pure in its arguments and returns
//! geometry. It writes no block, moves no good and touches no ledger, so it
//! is not on the journal and cannot make the replay oracle disagree. There
//! is a test in [`crate::drillmod`] that plays the same session twice, once
//! with the cage on and once with it off, and demands the two come out byte
//! for byte identical.

use glam::Vec3;
use vx_core::BlockPos;
use vx_render::{Object, tiles::slot};

/// How far the bars stand off the block's faces.
///
/// Small enough that the cage hugs the block, large enough that the bars sit
/// clear of its surface rather than fighting it for the same depth.
pub const SWELL: f32 = 0.03;

/// How thick a bar is, in blocks.
pub const BAR: f32 = 0.045;

/// Brightness of a cage on a block you are only looking at.
pub const IDLE_GLOW: f32 = 0.70;

/// Brightness of a cage on a block the bit is all the way through.
///
/// The shader clamps lighting at 1.4, so this is as bright as the game can
/// draw anything.
pub const CUT_GLOW: f32 = 1.40;

/// How far the sonar's rings travel, in blocks. Matches [`crate::sonar::REACH`].
const RING_REACH: f32 = 4.0;

/// How long a ping's rings take to reach that far, in seconds.
pub const RING_SECONDS: f32 = 0.9;

/// Beads on one circle of a ring, and how many circles a ring has.
const BEADS: usize = 14;
const PLANES: usize = 2;

/// The brightness a cage at this much progress is drawn at.
///
/// `phase` is a free-running angle in radians — wall-clock in the live game,
/// a literal in a capture fixture — and puts a slow shimmer on the idle cage
/// so a projected line looks projected rather than painted on. The shimmer
/// fades out as the bit bites: a block being cut should not flicker.
pub fn glow(progress: f32, phase: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    let steady = IDLE_GLOW + (CUT_GLOW - IDLE_GLOW) * progress;
    let shimmer = 0.06 * (1.0 - progress) * phase.sin();
    (steady + shimmer).clamp(0.0, CUT_GLOW)
}

/// The twelve bars of the cage round one block.
///
/// `progress` is how far through the block the bit is, `0.0` for a block you
/// are only aiming at. `phase` drives the shimmer; see [`glow`].
pub fn cage(block: BlockPos, progress: f32, phase: f32) -> Vec<Object> {
    let light = glow(progress, phase);
    let low = Vec3::new(block.x as f32, block.y as f32, block.z as f32) - Vec3::splat(SWELL);
    let high = low + Vec3::splat(1.0 + SWELL * 2.0);
    let mut bars = Vec::with_capacity(12);

    // Four bars along each axis, one down each edge of the box. Written as a
    // sweep over the two axes that are *not* the bar's own, so the twelve
    // come out of one rule rather than twelve hand-typed corners.
    for axis in 0..3usize {
        let (a, b) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        for corner in 0..4usize {
            let mut min = low;
            let mut max = high;
            for (index, which) in [(a, corner & 1), (b, (corner >> 1) & 1)] {
                if which == 0 {
                    max[index] = low[index] + BAR;
                } else {
                    min[index] = high[index] - BAR;
                }
            }
            let mut bar = Object::box_between(min, max, slot::HOLOGRAM);
            bar.light = light;
            bars.push(bar);
        }
    }
    bars
}

/// The plane of light crossing the block at the height the bit has reached.
///
/// This is the drill's progress bar, drawn on the rock instead of parked in
/// the corner of the screen. It is [`None`] for a block that is only being
/// aimed at, because a plane sitting on the floor of an untouched block
/// reads as a mistake.
pub fn scan_plane(block: BlockPos, progress: f32) -> Option<Object> {
    if progress <= 0.0 {
        return None;
    }
    let progress = progress.clamp(0.0, 1.0);
    let low = Vec3::new(block.x as f32, block.y as f32, block.z as f32) - Vec3::splat(SWELL);
    let span = 1.0 + SWELL * 2.0;
    let height = low.y + span * progress;
    let mut plane = Object::box_between(
        Vec3::new(low.x, height - BAR * 0.5, low.z),
        Vec3::new(low.x + span, height + BAR * 0.5, low.z + span),
        slot::HOLOGRAM,
    );
    plane.light = CUT_GLOW;
    Some(plane)
}

/// The sonar's rings, expanding from the block that was pinged.
///
/// Pure in `age` — seconds since the ping — so the live game feeds it a
/// wall clock and a capture fixture feeds it a literal and both draw the
/// same picture. Empty once the rings have run their course, which is what
/// makes "stop drawing it" the caller's whole job.
pub fn rings(centre: BlockPos, age: f32) -> Vec<Object> {
    if !(0.0..RING_SECONDS).contains(&age) {
        return Vec::new();
    }
    let travel = (age / RING_SECONDS).clamp(0.0, 1.0);
    let middle = Vec3::new(centre.x as f32, centre.y as f32, centre.z as f32) + Vec3::splat(0.5);
    let mut ring = Vec::with_capacity(2 * PLANES * BEADS);

    // Two fronts, the second a beat behind the first, because one ring reads
    // as a shape and two read as something travelling.
    for lag in [0.0f32, 0.35] {
        let front = travel - lag;
        if front <= 0.0 {
            continue;
        }
        let radius = 0.5 + (RING_REACH - 0.5) * front;
        // Fading, in the one currency this renderer has: brightness.
        let light = CUT_GLOW * (1.0 - front).max(0.0);
        let bead = BAR * (0.8 + 1.2 * (1.0 - front));

        // Beads on two circles rather than bars on two squares. A square
        // ring lying flat is four long bars, and a player standing level
        // with it — which is where a player drilling is — sees two lines
        // across the screen instead of a ring. Short beads on a circle read
        // as a ring from any angle, and the second circle standing upright
        // means there is always one of them turned towards the eye.
        for plane in 0..PLANES {
            for step in 0..BEADS {
                let angle = step as f32 / BEADS as f32 * std::f32::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                let offset = if plane == 0 {
                    Vec3::new(cos * radius, 0.0, sin * radius)
                } else {
                    Vec3::new(cos * radius, sin * radius, 0.0)
                };
                let at = middle + offset;
                // Cyan, not magenta: the pulse is the drill's, and the
                // magenta is reserved for what came *back*. Two colours for
                // two meanings, or the picture is a magenta cloud with no
                // grammar in it.
                let mut dot = Object::box_between(
                    at - Vec3::splat(bead),
                    at + Vec3::splat(bead),
                    slot::HOLOGRAM,
                );
                dot.light = light;
                ring.push(dot);
            }
        }
    }
    ring
}

/// A marker on a seam the ping found: a small cube in the open air against
/// the ore, at a position [`crate::sonar::ping`] has already chosen.
///
/// Drawn in the sonar's magenta rather than the cage's cyan, so an echo is
/// never mistaken for the pulse that fired it. It is deliberately smaller
/// than a block: an echo is a *report* that there is copper there, not the
/// copper itself, and drawing it block-sized would look like the ore had
/// already been dug out and replaced with a hologram.
pub fn echo(block: BlockPos, age: f32) -> Option<Object> {
    if !(0.0..crate::sonar::LINGER).contains(&age) {
        return None;
    }
    let size = 0.34;
    let middle = Vec3::new(block.x as f32, block.y as f32, block.z as f32) + Vec3::splat(0.5);
    let mut marker = Object::box_between(
        middle - Vec3::splat(size * 0.5),
        middle + Vec3::splat(size * 0.5),
        slot::SONAR,
    );
    // Steady for most of its life, then out. A marker that faded from the
    // first frame would be hard to see at exactly the moment it matters.
    let left = 1.0 - (age / crate::sonar::LINGER).clamp(0.0, 1.0);
    marker.light = CUT_GLOW * (left * 3.0).min(1.0);
    Some(marker)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spread(objects: &[Object]) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for object in objects {
            min = min.min(object.bounds_min);
            max = max.max(object.bounds_max);
        }
        (min, max)
    }

    /// Twelve bars, one down each edge, and the whole cage sits inside the
    /// block swollen by `SWELL` — never wider, never offset by a block.
    #[test]
    fn the_cage_hugs_the_block_it_is_drawn_on() {
        let block = BlockPos::new(4, 70, -9);
        let bars = cage(block, 0.0, 0.0);
        assert_eq!(bars.len(), 12, "a box has twelve edges");

        let (min, max) = spread(&bars);
        let low = Vec3::new(4.0, 70.0, -9.0) - Vec3::splat(SWELL);
        let high = low + Vec3::splat(1.0 + SWELL * 2.0);
        assert!((min - low).abs().max_element() < 1e-5, "cage starts at {min}");
        assert!((max - high).abs().max_element() < 1e-5, "cage ends at {max}");
    }

    /// Every bar is a bar: thin on two axes and long on the third. A cage
    /// whose "edges" were block-sized boxes would be a solid cyan cube.
    #[test]
    fn every_edge_is_thin_on_two_axes() {
        for bar in cage(BlockPos::new(0, 0, 0), 0.0, 0.0) {
            let size = bar.bounds_max - bar.bounds_min;
            let thin = [size.x, size.y, size.z]
                .iter()
                .filter(|side| **side <= BAR * 1.001)
                .count();
            assert_eq!(thin, 2, "a bar of {size} is not a bar");
        }
    }

    /// The plane climbs with the bit, and there is none at all on a block
    /// nobody has touched.
    #[test]
    fn the_scan_plane_rises_with_progress() {
        let block = BlockPos::new(0, 40, 0);
        assert!(scan_plane(block, 0.0).is_none(), "an untouched block has a plane");

        let low = scan_plane(block, 0.25).expect("a quarter through");
        let high = scan_plane(block, 0.75).expect("three quarters through");
        assert!(
            high.bounds_min.y > low.bounds_min.y,
            "the plane did not rise: {} then {}",
            low.bounds_min.y,
            high.bounds_min.y
        );
        // And it stays inside the block it belongs to.
        assert!(high.bounds_max.y <= 41.0 + SWELL * 2.0 + BAR);
    }

    /// The whole point of the choice: it is free. There is no drill, no
    /// wallet and no skill sheet in any signature here, so a cage cannot be
    /// something you buy — and a test that says so out loud stops a later
    /// stage quietly gating it.
    #[test]
    fn the_cage_asks_nothing_about_the_drill_you_own() {
        let block = BlockPos::new(-3, 12, 88);
        let first = cage(block, 0.4, 1.0);
        let second = cage(block, 0.4, 1.0);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.bounds_min, b.bounds_min);
            assert!((a.light - b.light).abs() < 1e-6);
        }
    }

    /// Brighter the deeper the bit is, and never past what the shader can
    /// draw.
    #[test]
    fn the_cage_brightens_as_the_bit_bites() {
        let idle = glow(0.0, 0.0);
        let through = glow(1.0, 0.0);
        assert!(idle < through, "{idle} was not dimmer than {through}");
        assert!((through - CUT_GLOW).abs() < 1e-6);
        for step in 0..=40 {
            let phase = step as f32 * 0.31;
            for progress in [0.0, 0.3, 0.6, 1.0] {
                let light = glow(progress, phase);
                assert!(
                    (0.0..=CUT_GLOW).contains(&light),
                    "glow {light} is outside what the shader clamps to"
                );
            }
        }
    }

    /// A block being cut does not flicker; a block being looked at does.
    #[test]
    fn the_shimmer_dies_away_under_the_bit() {
        let idle_spread = (0..16)
            .map(|step| glow(0.0, step as f32 * 0.4))
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        let cut_spread = (0..16)
            .map(|step| glow(1.0, step as f32 * 0.4))
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        assert!(idle_spread.1 - idle_spread.0 > 0.05, "the idle cage is dead flat");
        assert!(cut_spread.1 - cut_spread.0 < 1e-5, "a block being cut flickers");
    }

    /// Rings go out, get dimmer, and stop. The last one matters most: a
    /// ping that never ended would leave rings in the world for ever.
    #[test]
    fn the_rings_travel_out_and_then_stop() {
        let centre = BlockPos::new(10, 60, 10);
        let early = rings(centre, 0.1);
        let late = rings(centre, 0.8);
        assert!(!early.is_empty(), "no rings at all");

        let (_, early_max) = spread(&early);
        let (_, late_max) = spread(&late);
        assert!(
            late_max.x > early_max.x,
            "the rings did not travel: {} then {}",
            early_max.x,
            late_max.x
        );
        assert!(late_max.x - 10.5 <= RING_REACH + 0.2, "the rings overran their reach");
        assert!(rings(centre, RING_SECONDS + 0.01).is_empty(), "the rings never stop");
        assert!(rings(centre, -0.1).is_empty(), "rings before the ping");
    }

    /// An echo marker is a marker, not a replacement block: small, inside
    /// the block it names, and gone when its time is up.
    #[test]
    fn an_echo_is_smaller_than_the_block_it_names() {
        let block = BlockPos::new(2, 55, 3);
        let marker = echo(block, 0.2).expect("a fresh echo");
        let size = marker.bounds_max - marker.bounds_min;
        assert!(size.max_element() < 0.5, "the echo is {size}, near block sized");
        assert!(marker.bounds_min.x > 2.0 && marker.bounds_max.x < 3.0);
        assert!(echo(block, crate::sonar::LINGER + 0.01).is_none(), "echoes never fade");
    }
}
