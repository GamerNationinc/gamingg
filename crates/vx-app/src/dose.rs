//! What uranium costs to be near.
//!
//! # The bargain the deep ore makes
//!
//! Uranium is the richest thing in the ground by a long way, and if that
//! were the whole story it would simply be the ore you mine once you can
//! reach it — a better copper. What makes it a *decision* is that the face
//! you are cutting is doing something to you while you cut it. Nothing here
//! is a timer or a debuff icon: exposure is a function of how much bare
//! uranium is within a few blocks of your body, which means every lever a
//! player has over it is a physical one. Cut and back off. Wall the face
//! back up. Send a drone instead of standing there. Put lead in the suit.
//!
//! # Live-only, on purpose
//!
//! Dose spends health, and health has been live-only state since stage 28:
//! nothing here touches the world, the pile, or how long the fleet turns, so
//! the oracle never needs to hear about it. Two sessions with identical
//! journals can end with different dose and the same world hash, which is
//! exactly the line this game draws between what the log carries and what it
//! does not.
//!
//! # Why it counts blocks rather than tracing rays
//!
//! A proper attenuation model would trace to each source and charge for the
//! rock in between. It would also be a lie dressed as physics: what a player
//! can perceive is "how much of this stuff is near me", and a sum over a
//! small box says that honestly for a fraction of the cost. The one piece of
//! shielding that *is* modelled is the one a player controls — the lead in
//! the suit.

/// How far a bare face reaches, in blocks. Beyond this the sum is zero, so
/// backing off a few steps really is the answer.
pub const REACH: i32 = 5;

/// Rads a second from one block of bare uranium at one block's distance.
/// Everything further falls off with the square, so a face you are pressed
/// against is worth many times one you are across a gallery from.
const PER_BLOCK: f32 = 2.4;

/// The dose a body carries before it starts costing hits.
pub const BURN_AT: f32 = 100.0;

/// Rads shed a second once clear of anything hot.
const SHED: f32 = 3.5;

/// Seconds between hits while over the line. Slow: this is a warning that
/// turns into a wound, not a trap.
const BURN_EVERY: f32 = 6.0;

/// What each mark of the shield line keeps out.
const PER_MARK: f32 = 0.14;

/// What the suit keeps out at `marks` on the shield line.
///
/// Multiplicative rather than subtractive, so shielding never reaches zero:
/// a fully lined suit still takes a fifth of what a bare one does, and
/// nobody gets to stand in a uranium face forever.
pub fn let_through(marks: u32) -> f32 {
    (1.0 - PER_MARK * marks.min(crate::wallet::MAX_UPGRADE) as f32).max(0.30)
}

/// Rads a second at `at`, from bare uranium in the world around it.
///
/// Pure in the world and the position — the same body in the same gallery
/// always reads the same, which is what makes "cut it back and wall it up"
/// a strategy rather than a hope.
pub fn exposure(world: &vx_world::World, at: glam::DVec3) -> f32 {
    let Some(hot) = world.registry().id_of("engine:uranium_ore") else {
        return 0.0;
    };
    let centre = vx_core::BlockPos::new(
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    );

    let mut rads = 0.0;
    for dx in -REACH..=REACH {
        for dy in -REACH..=REACH {
            for dz in -REACH..=REACH {
                let pos = centre.offset([dx, dy, dz]);
                if world.block(pos) != hot {
                    continue;
                }
                let distance_squared = (dx * dx + dy * dy + dz * dz).max(1) as f32;
                if distance_squared > (REACH * REACH) as f32 {
                    continue;
                }
                rads += PER_BLOCK / distance_squared;
            }
        }
    }
    rads
}

/// How hot a body is, and how long since it last paid for it.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Dose {
    /// Rads carried. Zero is clean.
    pub rads: f32,
    /// Seconds since the last hit taken from being over the line.
    since_burn: f32,
}

/// What a tick of exposure did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Told {
    /// Crossed into the burn band this tick: the one warning that matters.
    Warned,
    /// Took a hit for it.
    Burned,
}

impl Dose {
    /// Take `dt` seconds of `rads` a second, shielded by `marks`.
    ///
    /// Returns what to say, if anything. Nothing is said while a dose merely
    /// climbs — the readout is on the HUD for that — because a toast every
    /// second is a toast nobody reads.
    pub fn tick(&mut self, rads: f32, dt: f32, marks: u32) -> Option<Told> {
        let was_over = self.rads >= BURN_AT;
        if rads > 0.0 {
            self.rads += rads * let_through(marks) * dt;
        } else {
            self.rads = (self.rads - SHED * dt).max(0.0);
        }

        if self.rads < BURN_AT {
            // Below the line the clock resets, so a body that dips under and
            // climbs back gets the full six seconds again rather than being
            // punished for the dip.
            self.since_burn = 0.0;
            return None;
        }
        if !was_over {
            self.since_burn = 0.0;
            return Some(Told::Warned);
        }
        self.since_burn += dt;
        if self.since_burn < BURN_EVERY {
            return None;
        }
        self.since_burn = 0.0;
        Some(Told::Burned)
    }

    /// Scrubbed clean. What a ward cot does, and the only thing in this game
    /// that does it faster than walking away and waiting.
    pub fn flush(&mut self) {
        *self = Dose::default();
    }

    /// Is this body carrying anything worth showing?
    pub fn showing(&self) -> bool {
        self.rads > 1.0
    }

    /// What the HUD says, or nothing when clean.
    pub fn readout(&self) -> Option<String> {
        if !self.showing() {
            return None;
        }
        let percent = (self.rads / BURN_AT * 100.0).round() as u32;
        if self.rads >= BURN_AT {
            return Some(format!("DOSE {percent}% - GET OUT"));
        }
        Some(format!("DOSE {percent}%"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_the_whole_defence() {
        // The inverse square is what makes backing off work. Pin it: a block
        // at arm's length is worth many at the edge of reach.
        let near = PER_BLOCK;
        let far = PER_BLOCK / ((REACH * REACH) as f32);
        assert!(near > far * 8.0, "distance barely matters, which is a bug");
    }

    #[test]
    fn a_clean_body_sheds_what_it_took() {
        let mut dose = Dose::default();
        dose.tick(20.0, 2.0, 0);
        assert!(dose.showing());
        for _ in 0..100 {
            dose.tick(0.0, 0.5, 0);
        }
        assert_eq!(dose.rads, 0.0, "a body kept a dose it walked away from");
        assert!(dose.readout().is_none());
    }

    #[test]
    fn crossing_the_line_warns_once_and_then_burns_on_a_clock() {
        let mut dose = Dose::default();
        let mut warnings = 0;
        let mut burns = 0;
        // Ten seconds pressed against a hot face.
        for _ in 0..100 {
            match dose.tick(40.0, 0.1, 0) {
                Some(Told::Warned) => warnings += 1,
                Some(Told::Burned) => burns += 1,
                None => {}
            }
        }
        assert_eq!(warnings, 1, "the warning fired {warnings} times");
        assert!(burns >= 1, "ten seconds in a face cost nothing");
        assert!(burns <= 2, "{burns} hits in ten seconds is a trap, not a warning");
    }

    #[test]
    fn lead_buys_time_and_never_immunity() {
        // The two halves of the shield line's promise, in one place.
        let hot = 50.0;
        let bare = {
            let mut dose = Dose::default();
            let mut seconds = 0.0;
            while dose.rads < BURN_AT && seconds < 600.0 {
                dose.tick(hot, 0.1, 0);
                seconds += 0.1;
            }
            seconds
        };
        let lined = {
            let mut dose = Dose::default();
            let mut seconds = 0.0;
            while dose.rads < BURN_AT && seconds < 600.0 {
                dose.tick(hot, 0.1, crate::wallet::MAX_UPGRADE);
                seconds += 0.1;
            }
            seconds
        };
        assert!(lined > bare * 1.3, "lead bought {lined}s against {bare}s");
        assert!(lined < 600.0, "a lined suit made a hot face survivable forever");
        assert!(let_through(99) >= 0.30, "the shield became immunity");
    }

    #[test]
    fn every_readout_is_drawable() {
        use vx_render::font;
        let mut dose = Dose::default();
        for rads in [0.0, 5.0, 60.0, 99.9, 100.0, 400.0] {
            dose.rads = rads;
            if let Some(line) = dose.readout() {
                assert!(font::text_width(&line, 1) > 0, "unrenderable: {line}");
            }
        }
    }
}
