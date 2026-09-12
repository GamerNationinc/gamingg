//! The ship on the Outpost's pad, and where it is in the air.
//!
//! Presentation only. What a sale at the city does to the books is
//! `Command::Sell`'s business; this is the picture of it leaving. Not
//! journalled and not saved, for the reason a caravan's position is a lerp:
//! where the ship is *now* is a function of when it left, and nothing else
//! in the game asks.

/// How long a launch is on screen for, in journal ticks. Ten seconds: long
/// enough to look up from the counter and see it go.
pub const FLIGHT_TICKS: u64 = 64 * 10;

/// How high it is when it leaves the frame. Above the fog, so it goes rather
/// than stops.
const CEILING: f64 = 220.0;

/// One launch: the tick it lifted, and what went up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub depart: u64,
    /// The manifest, as it was read out: "40 COPPER ORE".
    pub manifest: String,
}

impl Launch {
    /// Height above the pad at `now`, or `None` once it is gone. Quadratic in
    /// the time, so it leaves the pad slowly and the frame fast.
    pub fn altitude_at(&self, now: u64) -> Option<f64> {
        let flown = now.saturating_sub(self.depart);
        if flown >= FLIGHT_TICKS {
            return None;
        }
        let t = flown as f64 / FLIGHT_TICKS as f64;
        Some(CEILING * t * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_lifts_off_climbs_and_is_gone() {
        let launch = Launch {
            depart: 1_000,
            manifest: "40 COPPER ORE".into(),
        };
        assert_eq!(launch.altitude_at(1_000), Some(0.0));
        assert_eq!(launch.altitude_at(900), Some(0.0), "airborne before it left");
        let mut last = -1.0;
        for tick in 1_000..1_000 + FLIGHT_TICKS {
            let height = launch.altitude_at(tick).expect("gone early");
            assert!(height >= last, "it came back down at tick {tick}");
            last = height;
        }
        assert!(last > 100.0, "it never got above the fog: {last}");
        assert_eq!(launch.altitude_at(1_000 + FLIGHT_TICKS), None);
    }
}
