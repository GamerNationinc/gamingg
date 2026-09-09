//! Friendship, kept the way this game keeps everything: as a ledger.
//!
//! # Entries, not numbers
//!
//! What was given and what was said is remembered; the *number* is derived
//! from the entries. That is the same shape as the loot ledger and the bounty
//! sheet, and it buys the same things: caps that cannot be gamed by ordering
//! (a third gift this week scores zero because the week's entries are right
//! there to count), birthdays that triple honestly, and a save format that is
//! a list of facts rather than a running total that could drift from them.
//!
//! # What friendship is for
//!
//! Every tier opens something that already exists — nothing here invented a
//! feature to be the reward. Acquainted deepens what people say. Trusted
//! buys intel: a bearing to a bunker, the stage 19 loot loop fed by talk.
//! Close hands you a key — an actual grant through the permits system, to
//! that person's own door, through the rungs and not around them.
//!
//! # Two numbers, one ledger
//!
//! The civic round adds **trust**, which is not friendship. Friendship is
//! what you buy with gifts and conversation; trust is what you buy with
//! business, and the note is specific that it is business that opens a door
//! — "trust through trade is the per-resident stat that unlocks guest
//! authorization on their building".
//!
//! They live on the same ledger rather than in a second file, because a
//! second file would be this one again: the same sparse map, the same
//! [`PersonKey`], the same save. A `Trade` entry counts toward both, and
//! [`Disposition::trust`] reads only the trade entries — so a stranger you
//! have made rich can be let through the door without ever having been
//! liked, and somebody's oldest friend is not automatically their supplier.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::people::Person;

const MAGIC: &[u8; 4] = b"VXDS";
/// Two adds the `Trade` entry. Version one files still load — the layout is
/// the same list of tagged entries and one simply has no trades in it, so
/// dropping them would throw away friendships somebody earned for nothing.
const VERSION: u32 = 2;
const OLDEST: u32 = 1;

/// A person, addressed the way the permits system addresses beds: town
/// centre and resident index.
pub type PersonKey = ((i32, i32), u8);

/// Gift scores. Two a week count; a birthday gift counts triple.
pub const LOVED: i64 = 60;
pub const LIKED: i64 = 30;
pub const NEUTRAL: i64 = 8;
pub const HATED: i64 = -50;
/// The first conversation of a day.
pub const TALKED: i64 = 2;

/// How many credits of business are worth one point of trust.
///
/// A good haul off a full pile is a few hundred credits, so a solid day's
/// trading is worth a dozen or so points and [`TRUSTED_TRADE`] is a
/// relationship rather than an afternoon.
pub const CREDITS_PER_POINT: u64 = 20;

/// The most one deal can be worth, however big it was. A vault's worth of
/// bars sold in one go is a good day, not a lifetime of dealing.
pub const TRADE_CAP: i64 = 40;

/// Trust enough to be handed a key to their door.
pub const TRUSTED_TRADE: i64 = 400;

/// Gifts that score per rolling week.
pub const GIFTS_PER_WEEK: usize = 2;
const WEEK_DAYS: u32 = 7;

/// The ladder. Thresholds are cumulative points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Stranger,
    Acquainted,
    Friendly,
    Trusted,
    Close,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Stranger => "STRANGER",
            Tier::Acquainted => "ACQUAINTED",
            Tier::Friendly => "FRIENDLY",
            Tier::Trusted => "TRUSTED",
            Tier::Close => "CLOSE",
        }
    }

    pub fn for_points(points: i64) -> Tier {
        match points {
            _ if points >= 1_500 => Tier::Close,
            _ if points >= 700 => Tier::Trusted,
            _ if points >= 300 => Tier::Friendly,
            _ if points >= 100 => Tier::Acquainted,
            _ => Tier::Stranger,
        }
    }
}

/// One thing that happened between you and a person.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// A gift, with the points it scored *at the time* — the preferences are
    /// derived and stable, but scoring at entry keeps the ledger honest even
    /// if a future round re-tunes the tables.
    Gift { day: u32, points: i64 },
    Talk { day: u32 },
    /// A witnessed crime against their town, as negative points scaled from
    /// the bounty billed. Disposition and law stay linked but distinct: the
    /// sheriff wants credits, a neighbour just thinks less of you.
    Crime { day: u32, points: i64 },
    /// Business done across their counter, scored from what it was worth.
    /// Counts toward the friendship total *and* is the only thing that
    /// counts toward trust.
    Trade { day: u32, points: i64 },
}

impl Entry {
    fn day(&self) -> u32 {
        match self {
            Entry::Gift { day, .. }
            | Entry::Talk { day }
            | Entry::Crime { day, .. }
            | Entry::Trade { day, .. } => *day,
        }
    }
}

/// What a gift attempt came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Given {
    /// Scored: the points, and whether the birthday triple applied.
    Scored { points: i64, birthday: bool },
    /// The weekly cap: taken politely, worth nothing.
    Enough,
}

/// Every relationship you have.
#[derive(Debug, Default)]
pub struct Disposition {
    ledgers: BTreeMap<PersonKey, Vec<Entry>>,
}

impl Disposition {
    /// The score with a person, derived from the entries every time it is
    /// asked — the ledger is small and the honesty is the point.
    pub fn points(&self, key: PersonKey) -> i64 {
        self.ledgers
            .get(&key)
            .map_or(0, |entries| {
                entries
                    .iter()
                    .map(|entry| match entry {
                        Entry::Gift { points, .. }
                        | Entry::Crime { points, .. }
                        | Entry::Trade { points, .. } => *points,
                        Entry::Talk { .. } => TALKED,
                    })
                    .sum()
            })
            .max(-500)
    }

    pub fn tier(&self, key: PersonKey) -> Tier {
        Tier::for_points(self.points(key))
    }

    /// What this person will trust you with, which is a different question
    /// from what they think of you: only business counts, and only upward —
    /// a crime lowers their opinion, but it does not un-buy the goods.
    pub fn trust(&self, key: PersonKey) -> i64 {
        self.ledgers.get(&key).map_or(0, |entries| {
            entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Trade { points, .. } => Some(*points),
                    _ => None,
                })
                .sum()
        })
    }

    /// Enough business done to be let through their door.
    pub fn trusted_with_a_key(&self, key: PersonKey) -> bool {
        self.trust(key) >= TRUSTED_TRADE
    }

    /// Business done, in credits. Returns what it was worth.
    ///
    /// No weekly cap and no birthday multiplier: this is not a kindness, it
    /// is a transaction, and the only limit is [`TRADE_CAP`] on any one deal
    /// so a single enormous sale does not buy a key outright.
    pub fn trade(&mut self, key: PersonKey, credits: u64, day: u32) -> i64 {
        let points = ((credits / CREDITS_PER_POINT) as i64).clamp(1, TRADE_CAP);
        self.ledgers
            .entry(key)
            .or_default()
            .push(Entry::Trade { day, points });
        points
    }

    /// Hand a person a good on a day. The caller has already taken the good
    /// off the pile; this decides only what it was worth.
    pub fn gift(&mut self, key: PersonKey, person: &Person, good: &str, day: u32) -> Given {
        let entries = self.ledgers.entry(key).or_default();
        let this_week = entries
            .iter()
            .filter(|entry| {
                matches!(entry, Entry::Gift { .. })
                    && entry.day() + WEEK_DAYS > day
                    && entry.day() <= day
            })
            .count();
        if this_week >= GIFTS_PER_WEEK {
            return Given::Enough;
        }

        let base = if person.loved.contains(&good) {
            LOVED
        } else if person.loved.iter().any(|loved| {
            // A near miss: something in the same family as a loved good
            // (bars for an ore-lover) lands as liked rather than neutral.
            related(loved, good)
        }) {
            LIKED
        } else if person.hated == good {
            HATED
        } else {
            NEUTRAL
        };
        // Warmth is a small premium on kindness, never a discount on insult.
        let warmed = if base > 0 {
            base + i64::from(person.temperament.warmth / 64)
        } else {
            base
        };
        let birthday = day % crate::people::YEAR_DAYS == person.birthday;
        let points = if birthday { warmed * 3 } else { warmed };
        entries.push(Entry::Gift { day, points });
        Given::Scored { points, birthday }
    }

    /// A conversation. The first each day is worth something; the rest are
    /// free in both senses.
    pub fn talk(&mut self, key: PersonKey, day: u32) -> bool {
        let entries = self.ledgers.entry(key).or_default();
        if entries
            .iter()
            .any(|entry| matches!(entry, Entry::Talk { day: d } if *d == day))
        {
            return false;
        }
        entries.push(Entry::Talk { day });
        true
    }

    /// Every ledger in a town torn up: trust, friendship, all of it.
    ///
    /// What taking a town costs you with the people who live in it. Not a
    /// crime entry the years soften — the book is simply gone, and the
    /// business that would refill it starts from a blank page.
    pub fn forfeit(&mut self, town: (i32, i32)) {
        for index in 0..crate::people::PEOPLE as u8 {
            self.ledgers.remove(&(town, index));
        }
    }

    /// A crime the town saw, spread across everyone who lives there. The
    /// scale is deliberately soft: neighbours forgive faster than the law.
    pub fn crime(&mut self, town: (i32, i32), billed: u64, day: u32) {
        let points = -((billed / 4) as i64).max(1);
        for index in 0..crate::people::PEOPLE as u8 {
            self.ledgers
                .entry((town, index))
                .or_default()
                .push(Entry::Crime { day, points });
        }
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "friends.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.ledgers.len() as u32).to_le_bytes())?;
        for (((x, z), index), entries) in &self.ledgers {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&[*index])?;
            file.write_all(&(entries.len() as u32).to_le_bytes())?;
            for entry in entries {
                match entry {
                    Entry::Gift { day, points } => {
                        file.write_all(&[0u8])?;
                        file.write_all(&day.to_le_bytes())?;
                        file.write_all(&points.to_le_bytes())?;
                    }
                    Entry::Talk { day } => {
                        file.write_all(&[1u8])?;
                        file.write_all(&day.to_le_bytes())?;
                    }
                    Entry::Crime { day, points } => {
                        file.write_all(&[2u8])?;
                        file.write_all(&day.to_le_bytes())?;
                        file.write_all(&points.to_le_bytes())?;
                    }
                    Entry::Trade { day, points } => {
                        file.write_all(&[3u8])?;
                        file.write_all(&day.to_le_bytes())?;
                        file.write_all(&points.to_le_bytes())?;
                    }
                }
            }
        }
        file.commit()
    }

    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("friends.dat");
        match read_ledgers(&path) {
            Ok(Some(ledgers)) => self.ledgers = ledgers,
            Ok(None) => {}
            Err(error) => log::warn!("unreadable {}: {error}", path.display()),
        }
    }
}

/// Are two goods close enough that loving one likes the other?
fn related(loved: &str, given: &str) -> bool {
    matches!(
        (loved, given),
        ("engine:copper_ore", "engine:copper_bar") | ("engine:copper_bar", "engine:copper_ore")
    )
}

type Ledgers = BTreeMap<PersonKey, Vec<Entry>>;

fn read_ledgers(path: &Path) -> std::io::Result<Option<Ledgers>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a friendship file"));
    }
    let mut word = [0u8; 4];
    let mut long = [0u8; 8];
    let mut byte = [0u8; 1];
    file.read_exact(&mut word)?;
    let version = u32::from_le_bytes(word);
    // Anything from `OLDEST` up reads with this loop; a file from the future
    // is refused rather than guessed at, as every loader here does.
    if !(OLDEST..=VERSION).contains(&version) {
        return Ok(None);
    }
    file.read_exact(&mut word)?;
    let people = u32::from_le_bytes(word);
    let mut ledgers = Ledgers::new();
    for _ in 0..people {
        file.read_exact(&mut word)?;
        let x = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let z = i32::from_le_bytes(word);
        file.read_exact(&mut byte)?;
        let index = byte[0];
        file.read_exact(&mut word)?;
        let count = u32::from_le_bytes(word);
        let mut entries = Vec::new();
        for _ in 0..count {
            file.read_exact(&mut byte)?;
            file.read_exact(&mut word)?;
            let day = u32::from_le_bytes(word);
            entries.push(match byte[0] {
                0 => {
                    file.read_exact(&mut long)?;
                    Entry::Gift {
                        day,
                        points: i64::from_le_bytes(long),
                    }
                }
                1 => Entry::Talk { day },
                3 => {
                    file.read_exact(&mut long)?;
                    Entry::Trade {
                        day,
                        points: i64::from_le_bytes(long),
                    }
                }
                _ => {
                    file.read_exact(&mut long)?;
                    Entry::Crime {
                        day,
                        points: i64::from_le_bytes(long),
                    }
                }
            });
        }
        ledgers.insert(((x, z), index), entries);
    }
    Ok(Some(ledgers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::people;

    fn someone() -> Person {
        people::person(&vx_world::town::home_site(), 2) // Old Prat: loves logs and stone
    }

    #[test]
    fn the_ladder_matches_the_note() {
        assert_eq!(Tier::for_points(0), Tier::Stranger);
        assert_eq!(Tier::for_points(100), Tier::Acquainted);
        assert_eq!(Tier::for_points(300), Tier::Friendly);
        assert_eq!(Tier::for_points(700), Tier::Trusted);
        assert_eq!(Tier::for_points(1_500), Tier::Close);
        assert_eq!(Tier::for_points(-40), Tier::Stranger);
    }

    #[test]
    fn a_third_weekly_gift_scores_zero() {
        let mut ledger = Disposition::default();
        let key = ((0, 0), 2);
        let prat = someone();
        assert!(matches!(
            ledger.gift(key, &prat, "engine:log", 10),
            Given::Scored { .. }
        ));
        assert!(matches!(
            ledger.gift(key, &prat, "engine:log", 11),
            Given::Scored { .. }
        ));
        let held = ledger.points(key);
        assert_eq!(ledger.gift(key, &prat, "engine:log", 12), Given::Enough);
        assert_eq!(ledger.points(key), held, "the capped gift still scored");
        // A week later the cap has rolled off.
        assert!(matches!(
            ledger.gift(key, &prat, "engine:log", 17),
            Given::Scored { .. }
        ));
    }

    #[test]
    fn loved_liked_neutral_and_hated_are_distinct() {
        let mut ledger = Disposition::default();
        let prat = someone(); // loves log + stone, hates copper bar
        let mut score = |good: &str, day: u32| -> i64 {
            let key = ((day as i32 * 100, 0), 2);
            match ledger.gift(key, &prat, good, day) {
                Given::Scored { points, .. } => points,
                Given::Enough => panic!("capped in a fresh ledger"),
            }
        };
        let loved = score("engine:log", 1);
        let neutral = score("engine:hho_cell", 2);
        let hated = score("engine:copper_bar", 3);
        assert!(loved >= LOVED);
        assert!((NEUTRAL..LIKED).contains(&neutral));
        assert_eq!(hated, HATED, "warmth discounted an insult");
        assert!(loved > neutral && neutral > hated);
    }

    #[test]
    fn birthdays_triple_and_talk_pays_once_a_day() {
        let prat = someone();
        let mut ledger = Disposition::default();
        let key = ((0, 0), 2);
        let birthday = prat.birthday;
        let plain_day = (birthday + 3) % people::YEAR_DAYS;
        let plain = match ledger.gift(((512, 0), 2), &prat, "engine:log", plain_day) {
            Given::Scored { points, .. } => points,
            Given::Enough => unreachable!(),
        };
        match ledger.gift(key, &prat, "engine:log", birthday) {
            Given::Scored { points, birthday } => {
                assert!(birthday);
                assert_eq!(points, plain * 3);
            }
            Given::Enough => unreachable!(),
        }

        assert!(ledger.talk(key, 5));
        assert!(!ledger.talk(key, 5), "the second chat of a day paid");
        assert!(ledger.talk(key, 6));
    }

    #[test]
    fn a_crime_sours_the_whole_town_but_less_than_the_law() {
        let mut ledger = Disposition::default();
        let town = (0, 0);
        ledger.crime(town, 500, 4);
        for index in 0..people::PEOPLE as u8 {
            let points = ledger.points((town, index));
            assert!(points < 0, "somebody did not notice the vault come open");
            assert!(points.abs() < 500, "a neighbour billed like a sheriff");
        }
        // And another town heard nothing.
        assert_eq!(ledger.points(((512, 512), 0)), 0);
    }

    /// Trust is business and friendship is kindness. Gifts do not open a
    /// supplier's door and deals do not make somebody your friend — the two
    /// numbers ride the same ledger and stay separate.
    #[test]
    fn trust_is_bought_with_business_and_friendship_with_gifts() {
        let prat = someone();
        let mut ledger = Disposition::default();
        let key = ((0, 0), 2);

        // Four gifts across four weeks: real friendship, no trust.
        for week in 0..4 {
            ledger.gift(key, &prat, "engine:log", week * 7 + 1);
        }
        assert!(ledger.points(key) > 0, "gifts scored nothing");
        assert_eq!(ledger.trust(key), 0, "a present is not a purchase order");
        assert!(!ledger.trusted_with_a_key(key));

        // And business, which counts toward both — a supplier does warm to
        // you — but only one of them opens the door.
        let mut trader = Disposition::default();
        let other = ((0, 0), 1);
        for day in 0..30 {
            trader.trade(other, 800, day);
        }
        assert!(trader.trust(other) >= TRUSTED_TRADE, "{}", trader.trust(other));
        assert!(trader.trusted_with_a_key(other));
        assert!(trader.points(other) > 0, "business counted for nothing at all");
    }

    /// One enormous sale is a good day, not a relationship, and the smallest
    /// one still counts for something.
    #[test]
    fn a_single_deal_is_capped_at_both_ends() {
        let mut ledger = Disposition::default();
        let key = ((0, 0), 0);
        assert_eq!(ledger.trade(key, 1_000_000, 1), TRADE_CAP);
        let mut small = Disposition::default();
        assert_eq!(small.trade(key, 1, 1), 1);
    }

    /// A friends file written before trade existed still loads, with the
    /// friendships in it intact. Dropping them would have been the easy read
    /// of "tolerant" and the wrong one.
    #[test]
    fn a_ledger_from_before_trade_still_loads() {
        use std::io::Write;
        let directory =
            std::env::temp_dir().join(format!("vx-friends-old-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("friends.dat");
        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(MAGIC).unwrap();
            file.write_all(&OLDEST.to_le_bytes()).unwrap();
            file.write_all(&1u32.to_le_bytes()).unwrap(); // one person
            file.write_all(&0i32.to_le_bytes()).unwrap();
            file.write_all(&0i32.to_le_bytes()).unwrap();
            file.write_all(&[2u8]).unwrap(); // resident index
            file.write_all(&2u32.to_le_bytes()).unwrap(); // two entries
            file.write_all(&[0u8]).unwrap(); // a gift
            file.write_all(&3u32.to_le_bytes()).unwrap();
            file.write_all(&90i64.to_le_bytes()).unwrap();
            file.write_all(&[1u8]).unwrap(); // and a chat
            file.write_all(&3u32.to_le_bytes()).unwrap();
        }
        let mut loaded = Disposition::default();
        loaded.load(&directory);
        let key = ((0, 0), 2);
        assert_eq!(loaded.points(key), 90 + TALKED);
        assert_eq!(loaded.trust(key), 0);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn the_ledger_survives_a_save() {
        let directory =
            std::env::temp_dir().join(format!("vx-friends-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let prat = someone();
        let mut ledger = Disposition::default();
        let key = ((0, 0), 2);
        ledger.gift(key, &prat, "engine:log", 3);
        ledger.talk(key, 3);
        ledger.crime((0, 0), 60, 4);
        ledger.trade(key, 640, 4);
        ledger.save(&directory).unwrap();

        let mut loaded = Disposition::default();
        loaded.load(&directory);
        assert_eq!(loaded.points(key), ledger.points(key));
        assert_eq!(loaded.tier(key), ledger.tier(key));
        assert_eq!(loaded.trust(key), ledger.trust(key));
        assert!(loaded.trust(key) > 0, "the trade did not survive the write");
        std::fs::remove_dir_all(&directory).ok();
    }
}
