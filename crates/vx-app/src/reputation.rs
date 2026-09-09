//! Factions: what whole peoples think of you, and what that changes.
//!
//! # Reputation is the memory the bounty board does not have
//!
//! Bounty is a *bill*: it accrues per crime, a warrant answers it, an
//! arrest settles it, and the ledger starts clean. Reputation is what
//! everybody remembers anyway. Settle every fine you like — a county that
//! has watched you breach four vaults trades with you accordingly, and a
//! shelter whose neighbours you dragged to the board does not care that
//! your paperwork is in order. The two stay linked but distinct, exactly
//! as disposition and law have been since the townsfolk round: bounty is
//! per-incident and settleable, standing is cumulative and lived-with.
//!
//! # Two factions, because two exist
//!
//! **The Compact** is the settled towns — one people already in every
//! mechanical sense: one lattice, one beacon network, one economy hauling
//! between them, one warrant that follows you town to town. **The
//! Holdouts** are whoever holds the shelters — the squads stage 29 raised
//! from each bunker's seed. Nothing else in this world has a flag to fly,
//! and a faction system with more factions than peoples would be a menu,
//! not a world. The military tier, when it arrives, gets its own name;
//! this file grows by adding a field, which is the same promise every
//! name-keyed ledger in this game makes.
//!
//! # Live-only, like every opinion of the player
//!
//! Standing reaches no block and no hash. It is disposition at faction
//! scale, kept in its own small file, and the replay oracle never learns
//! anyone had a reputation at all.

use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"VXRP";
const VERSION: u32 = 1;

/// Standing is clamped here: no amount of virtue makes a saint, no amount
/// of murder makes you worse than hated.
pub const CAP: i64 = 1_000;

// What deeds are worth, signed per faction.
/// Turning a surrendered holder in: civic service to the towns...
pub const CAPTURE_COMPACT: i64 = 25;
/// ...and betrayal to the shelters, worse than a clean kill because the
/// board parades it.
pub const CAPTURE_HOLDOUTS: i64 = -40;
/// Putting a holder down.
pub const KILL_COMPACT: i64 = 8;
pub const KILL_HOLDOUTS: i64 = -60;
/// Breaking a whole shelter, over and above its holders.
pub const CLEARED_HOLDOUTS: i64 = -80;
/// A witnessed crime in a town costs standing at one point per five
/// bounty points billed — the same scaling the disposition ledger uses.
pub const CRIME_DIVISOR: u64 = 5;
/// A load sold across a counter. A trickle on purpose: trade is a life,
/// not a grind, and it should take a season of honest hauling to be known.
pub const TRADE_COMPACT: i64 = 1;
/// A gift to a townsperson: kindness travels.
pub const GIFT_COMPACT: i64 = 2;

/// Winning a seat at a town's ballot box.
///
/// The civic note asks for offices held to tie into the factions, and this is
/// the tie: the Compact is the towns' own opinion of you, and a town that has
/// just elected you has stated one out loud. Sized like a capture rather than
/// a trade — an election is a rare, large event.
pub const ELECTED_COMPACT: i64 = 30;
/// And the Holdouts' view of somebody who has taken the towns' shilling.
pub const ELECTED_HOLDOUTS: i64 = -20;

/// Seizing a town's chairs over its broken deputies.
///
/// The Compact is the towns' own opinion of you, and you have just thrown a
/// town's law down in its own square: worse than a vault, because a vault is
/// against one town's money and this is against every town's order. The
/// shelters, who have never had much use for a mayor, think rather better of
/// it — not enough to make a friend, enough to notice.
pub const TAKEN_COMPACT: i64 = -80;
pub const TAKEN_HOLDOUTS: i64 = 40;

/// Where you stand with a people.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Standing {
    Enemy,
    Cold,
    Neutral,
    Warm,
    Friend,
}

impl Standing {
    /// Band a total. The thresholds are deliberately far apart: standings
    /// are seasons, not moods.
    pub fn of(points: i64) -> Standing {
        if points <= -400 {
            Standing::Enemy
        } else if points <= -100 {
            Standing::Cold
        } else if points < 100 {
            Standing::Neutral
        } else if points < 400 {
            Standing::Warm
        } else {
            Standing::Friend
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Standing::Enemy => "ENEMY",
            Standing::Cold => "COLD",
            Standing::Neutral => "NEUTRAL",
            Standing::Warm => "WARM",
            Standing::Friend => "FRIEND",
        }
    }
}

/// How the Compact's counters shade their prices for you, as a percentage
/// applied in your favour or against it.
///
/// Warm towns pay a little more for your ore and charge a little less for
/// their goods; cold ones do the opposite. Small on purpose — a few
/// percent reads as a relationship, a big number reads as an exploit.
pub fn price_shade(standing: Standing) -> i64 {
    match standing {
        Standing::Enemy => -6,
        Standing::Cold => -3,
        Standing::Neutral => 0,
        Standing::Warm => 3,
        Standing::Friend => 6,
    }
}

/// Shade a selling price by the Compact's opinion: what the counter pays
/// *you*. Never below one credit — a town that pays nothing is a town
/// that broke the economy's floor.
pub fn shaded_sell(price: u64, standing: Standing) -> u64 {
    let shade = price_shade(standing);
    let shifted = price as i64 + price as i64 * shade / 100;
    shifted.max(1) as u64
}

/// Your name across the county, and among the shelters.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reputation {
    compact: i64,
    holdouts: i64,
}

impl Reputation {
    pub fn compact(&self) -> Standing {
        Standing::of(self.compact)
    }

    pub fn holdouts(&self) -> Standing {
        Standing::of(self.holdouts)
    }

    pub fn compact_points(&self) -> i64 {
        self.compact
    }

    pub fn holdouts_points(&self) -> i64 {
        self.holdouts
    }

    /// Move the Compact's opinion. Returns the new band when it changed —
    /// a band crossing is worth a toast, a point is not.
    pub fn with_compact(&mut self, points: i64) -> Option<Standing> {
        let before = self.compact();
        self.compact = (self.compact + points).clamp(-CAP, CAP);
        let after = self.compact();
        (before != after).then_some(after)
    }

    /// Move the Holdouts' opinion, same contract.
    pub fn with_holdouts(&mut self, points: i64) -> Option<Standing> {
        let before = self.holdouts();
        self.holdouts = (self.holdouts + points).clamp(-CAP, CAP);
        let after = self.holdouts();
        (before != after).then_some(after)
    }

    /// A witnessed crime against a town, scaled off the bounty bill.
    pub fn crime(&mut self, billed: u64) -> Option<Standing> {
        self.with_compact(-((billed / CRIME_DIVISOR).max(1) as i64))
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "reputation.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&self.compact.to_le_bytes())?;
        file.write_all(&self.holdouts.to_le_bytes())?;
        file.commit()
    }

    /// Read it back. Absence or damage is a stranger nobody has an opinion
    /// of yet, which is generous and harmless.
    pub fn load(&mut self, directory: &Path) {
        match read(&directory.join("reputation.dat")) {
            Ok(Some(reputation)) => *self = reputation,
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged reputation file: {error}"),
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Reputation>> {
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
    let mut long = [0u8; 8];
    file.read_exact(&mut long)?;
    let compact = i64::from_le_bytes(long).clamp(-CAP, CAP);
    file.read_exact(&mut long)?;
    let holdouts = i64::from_le_bytes(long).clamp(-CAP, CAP);
    Ok(Some(Reputation { compact, holdouts }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stranger_is_neutral_with_everybody() {
        let fresh = Reputation::default();
        assert_eq!(fresh.compact(), Standing::Neutral);
        assert_eq!(fresh.holdouts(), Standing::Neutral);
        assert_eq!(price_shade(Standing::Neutral), 0);
    }

    #[test]
    fn the_bands_sit_where_the_thresholds_say_and_stay_ordered() {
        let ladder = [
            (-1_000, Standing::Enemy),
            (-400, Standing::Enemy),
            (-399, Standing::Cold),
            (-100, Standing::Cold),
            (-99, Standing::Neutral),
            (99, Standing::Neutral),
            (100, Standing::Warm),
            (399, Standing::Warm),
            (400, Standing::Friend),
            (1_000, Standing::Friend),
        ];
        for (points, expected) in ladder {
            assert_eq!(Standing::of(points), expected, "at {points}");
        }
        let order = [
            Standing::Enemy,
            Standing::Cold,
            Standing::Neutral,
            Standing::Warm,
            Standing::Friend,
        ];
        for pair in order.windows(2) {
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn only_band_crossings_report_and_totals_clamp() {
        let mut name = Reputation::default();
        assert_eq!(name.with_compact(50), None, "a point made a toast");
        assert_eq!(name.with_compact(60), Some(Standing::Warm));
        assert_eq!(name.with_compact(10), None);
        // No amount of virtue overflows the cap.
        name.with_compact(1_000_000);
        assert_eq!(name.compact_points(), CAP);
        // And a fall from the cap lands where the arithmetic says: a
        // thousand minus fifteen hundred is an enemy, not a cold shoulder.
        assert_eq!(name.with_compact(-CAP - 500), Some(Standing::Enemy));
        assert_eq!(name.compact_points(), -500);
    }

    #[test]
    fn one_deed_moves_the_two_peoples_opposite_ways() {
        // The whole point of factions in one assertion: a capture pleases
        // the towns and damns you to the shelters, with signs pinned.
        // Const blocks, so a rebalance that breaks the shape fails the
        // build rather than a test run — the bounty ladder's precedent.
        const {
            assert!(CAPTURE_COMPACT > 0 && CAPTURE_HOLDOUTS < 0);
            assert!(KILL_COMPACT > 0 && KILL_HOLDOUTS < 0);
        }
        // A capture is worse than a kill to the shelters? No — betrayal
        // reads worse per head to them than dying does, but a kill costs
        // more. The board parades captures; graves are quiet. Pin the
        // actual ordering so a rebalance is a decision, not a drift.
        const { assert!(KILL_HOLDOUTS < CAPTURE_HOLDOUTS) };
        // And to the towns, taking one alive outranks a body.
        const { assert!(CAPTURE_COMPACT > KILL_COMPACT) };
    }

    #[test]
    fn prices_shade_a_little_and_never_to_nothing() {
        assert!(shaded_sell(100, Standing::Friend) > 100);
        assert!(shaded_sell(100, Standing::Enemy) < 100);
        assert_eq!(shaded_sell(100, Standing::Neutral), 100);
        // The floor: even an enemy's ore is worth a credit.
        assert_eq!(shaded_sell(1, Standing::Enemy), 1);
        // And the shade is genuinely a few percent, not a cliff.
        assert_eq!(shaded_sell(100, Standing::Friend), 106);
        assert_eq!(shaded_sell(100, Standing::Enemy), 94);
    }

    #[test]
    fn crimes_scale_off_the_bill_like_the_disposition_ledger() {
        let mut name = Reputation::default();
        name.crime(500);
        assert_eq!(name.compact_points(), -100);
        // Even a petty crime costs at least a point.
        let mut petty = Reputation::default();
        petty.crime(1);
        assert_eq!(petty.compact_points(), -1);
    }

    #[test]
    fn reputation_round_trips_and_tolerates_damage() {
        let directory = std::env::temp_dir().join(format!("vx-rep-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut name = Reputation::default();
        name.with_compact(150);
        name.with_holdouts(-450);
        name.save(&directory).unwrap();
        let mut read_back = Reputation::default();
        read_back.load(&directory);
        assert_eq!(name, read_back);

        std::fs::write(directory.join("reputation.dat"), b"junk").unwrap();
        let mut damaged = Reputation::default();
        damaged.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(damaged, Reputation::default(), "damage was not a stranger");
    }

    #[test]
    fn every_band_name_is_drawable() {
        for band in [
            Standing::Enemy,
            Standing::Cold,
            Standing::Neutral,
            Standing::Warm,
            Standing::Friend,
        ] {
            for character in band.name().chars() {
                assert!(vx_render::font::knows(character), "undrawable {character:?}");
            }
        }
    }
}
