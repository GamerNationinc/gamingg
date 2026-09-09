//! The ballot box: how a seat changes hands.
//!
//! # A seat nobody could ever lose
//!
//! Stage 39 put a mayor and a sheriff in every town, derived off the town's
//! own seed — which was the right way to have them and the wrong way to keep
//! them. [`crate::office::holder`] is a pure function, so the mayor of a town
//! was its mayor forever, and a warrant chain whose decider can never be
//! replaced has one fixed link in it.
//!
//! This module is the overlay that lets the derivation be *wrong*. Everything
//! about an election is arithmetic on the seed and the ledgers except the one
//! fact an election produces — who won — and that is stored the way this
//! project stores everything: **only where it differs**. A frontier nobody has
//! campaigned in has an empty register and every seat still at its derived
//! default, exactly as an unburnt forest has an empty
//! [`crate::succession::Ledger`].
//!
//! # Goodwill is the trade ledger, which already exists
//!
//! The civic note asks for votes cast on "goodwill points, earned through
//! trade interactions — the resident who profits from trading with you votes
//! your way, which reuses the trade ledger rather than inventing a reputation
//! system from nothing." Stage 39 built exactly that ledger:
//! [`crate::disposition::trust`] is what somebody has put across their own
//! counter with you, kept separate from friendship. So a vote reads trust
//! first, friendship second, and nothing here invents a third number.
//!
//! # Polling day is the market day
//!
//! A town already picks a weekday to hold its market
//! ([`crate::schedule::market_weekday`]) and already sends everybody to the
//! square on it. Voting on that day costs no new clock, puts the whole town in
//! one place for it, and stays pure in `(site, day)` — so both sides of the
//! oracle agree on when an election happens without anybody recording it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use vx_world::TownSite;

use crate::disposition::{Disposition, Tier};
use crate::office;
use crate::people::{self, Archetype, PEOPLE};
use crate::permits::Office;

const MAGIC: &[u8; 4] = b"VXBL";
const VERSION: u32 = 2;
/// The oldest file this loader still reads: one without the taken map.
const OLDEST: u32 = 1;

/// How many days a term runs. A week, so a town's politics move on a clock a
/// player can actually watch turn rather than one they take on trust.
pub const TERM_DAYS: u32 = 7;

/// Who can hold a seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Candidate {
    /// A resident, by roster index — the same index the permits system
    /// numbers beds with.
    Resident(usize),
    Player,
}

impl Candidate {
    pub fn is_player(self) -> bool {
        matches!(self, Candidate::Player)
    }
}

/// A term index: how many whole terms have passed.
pub fn term_of(day: u32) -> u32 {
    day / TERM_DAYS
}

/// Is this a polling day in this town?
///
/// The town's own market day, once a term. Pure in `(site, day)`, which is
/// what lets the live game and a replay hold the same election on the same
/// day without an order between them.
pub fn is_polling_day(site: &TownSite, day: u32) -> bool {
    crate::schedule::is_market_day(site, day) && day % TERM_DAYS < 7
}

/// The next day this town goes to the polls, at or after `day`.
pub fn next_poll(site: &TownSite, day: u32) -> u32 {
    (day..day + 14)
        .find(|ahead| is_polling_day(site, *ahead))
        .unwrap_or(day)
}

/// What one resident thinks of one of their neighbours, before anything the
/// player did.
///
/// Derived and stable: a town's own politics do not change because somebody
/// walked into it. Nobody votes for themselves out of vanity alone — the
/// bonus is small — and nobody's regard for a neighbour is a coin flip either.
pub fn regard(site: &TownSite, voter: usize, of: usize) -> i64 {
    if voter == of {
        // A person's opinion of themselves. Enough to matter in a town of
        // three, not enough to decide it.
        return 30;
    }
    let hash = vx_world::seed::finalise(
        site.seed
            ^ 0xba11_0700_0000_0001
            ^ (voter as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (of as u64 + 1).wrapping_mul(0xc2b2_ae3d_27d4_eb4f),
    );
    (hash % 41) as i64
}

/// How much a person's own standing recommends them for a seat.
///
/// A proud person looks like a mayor and a craven one does not, which is the
/// same six archetypes spent a fourth time.
pub fn bearing(archetype: Archetype) -> i64 {
    match archetype {
        Archetype::Proud => 22,
        Archetype::Steady => 18,
        Archetype::Gruff => 10,
        Archetype::Chatty => 12,
        Archetype::Anxious => 4,
        Archetype::Craven => 0,
    }
}

/// What the player brings to one voter, and why.
///
/// Trust dominates, because the note says it should: business is what buys a
/// vote. Friendship helps. A bounty in this town is a candidate who looks
/// like trouble, and it is deliberately steep — the frontier will elect
/// somebody it likes, not somebody it is frightened of.
pub fn standing_with(friends: &Disposition, town: (i32, i32), voter: usize, bounty: u64) -> i64 {
    let key = (town, voter as u8);
    let trust = (friends.trust(key) / 8).min(40);
    let friendship = match friends.tier(key) {
        Tier::Stranger => 0,
        Tier::Acquainted => 6,
        Tier::Friendly => 14,
        Tier::Trusted => 22,
        Tier::Close => 32,
    };
    let trouble = (bounty / 4) as i64;
    trust + friendship - trouble
}

/// What the incumbent's record costs them.
///
/// A mayor with paper standing against him looks weak in front of the town he
/// is supposed to run, which is the join back to stage 39's warrant chain.
pub fn record(has_warrant: bool) -> i64 {
    if has_warrant {
        -25
    } else {
        0
    }
}

/// Everything an election needs to know that this module cannot derive.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    pub site: &'a TownSite,
    pub friends: &'a Disposition,
    /// The player's bounty in this town.
    pub bounty: u64,
    /// Is there a warrant standing against the town's own incumbent? (Stage
    /// 39's docket answers this for the player; a resident incumbent under a
    /// cloud is the same idea pointed the other way.)
    pub incumbent_troubled: bool,
    /// Is the player on the ballot for this seat at all?
    pub standing: bool,
    /// Which seat is being polled. Only read when the player is defending
    /// one, because a player has no roster index to look the office up from.
    pub seat: Office,
    /// Did the player found this town? A settler owes the founder the roof
    /// over their head — see [`crate::charter::FOUNDER`].
    pub founder: bool,
}

/// One voter's choice, with the arithmetic that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vote {
    pub voter: usize,
    pub cast_for: Candidate,
    /// What the winner scored with this voter, for the panel to print.
    pub score: i64,
}

/// How much being the one already in the chair is worth, against an
/// outsider.
///
/// Modest, because it is only ever weighed against the player: see [`poll`]
/// for why residents never contest each other.
pub const INCUMBENCY: i64 = 20;

/// Run the poll for one seat and say who won.
///
/// **A poll is a referendum on you, not a contest among neighbours.** In a
/// town of three, the man in the chair is not swapped for the man next to him
/// — they have lived beside each other for years and nothing about that
/// changed this week. What can change is that somebody turned up from outside
/// who the town would rather have. So when nobody is standing against the
/// incumbent, the incumbent is returned; when the player is standing, every
/// voter weighs what the player is worth to *them* against what they think of
/// the sitting holder.
///
/// That is also what keeps [`Register`] sparse in the way this project means
/// it: a town the player has never campaigned in re-elects its derived seat
/// for ever and is never written down at all.
///
/// Pure in everything it reads. The register is not consulted here — the
/// caller passes the incumbent in, because who *is* seated is the register's
/// business and who *should be* is this function's.
pub fn poll(field: &Field<'_>, incumbent: Candidate) -> (Candidate, Vec<Vote>) {
    let sitting = |voter: usize| -> i64 {
        match incumbent {
            Candidate::Resident(index) => {
                let person = people::person(field.site, index);
                regard(field.site, voter, index)
                    + bearing(person.temperament.archetype)
                    + INCUMBENCY
                    + record(field.incumbent_troubled)
            }
            // A player defending a seat is judged the same way they won it,
            // with the chair worth the same to them as it is to anybody —
            // and with the founder's due on top, if this is their town.
            Candidate::Player => {
                standing_with(field.friends, field.site.centre, voter, field.bounty)
                    + INCUMBENCY
                    + record(field.incumbent_troubled)
                    + if field.founder { crate::charter::FOUNDER } else { 0 }
            }
        }
    };

    // A player who has taken their name off the ballot while holding the
    // seat is not defending it — that is what withdrawing *means*. The town
    // does not hold a contest over a chair nobody is sitting in; it gives it
    // back to the man the seed always said should have it.
    let native = Candidate::Resident(office::holder(field.site, field.seat));
    if incumbent.is_player() && !field.standing {
        let votes = (0..PEOPLE)
            .map(|voter| Vote {
                voter,
                cast_for: native,
                score: sitting(voter),
            })
            .collect();
        return (native, votes);
    }

    // Otherwise exactly one candidate can be challenging. Against a
    // resident it is the player, and only with their name in. Against a
    // *player* in the chair it is the man the seed always said should have
    // it: a town that elected an outsider, or was founded by one, or was
    // taken by one, weighs that outsider against its own every term. This is
    // what makes a taken town a town you have to keep, and the founder's due
    // the thing that keeps a founder in through the first quiet terms.
    let challenger = if incumbent.is_player() {
        Some(native)
    } else if field.standing {
        Some(Candidate::Player)
    } else {
        None
    };

    let mut votes = Vec::with_capacity(PEOPLE);
    let mut for_challenger = 0usize;
    for voter in 0..PEOPLE {
        let held = sitting(voter);
        let (cast_for, score) = match challenger {
            Some(Candidate::Player) => {
                let theirs = standing_with(field.friends, field.site.centre, voter, field.bounty);
                if theirs > held {
                    (Candidate::Player, theirs)
                } else {
                    (incumbent, held)
                }
            }
            Some(Candidate::Resident(index)) => {
                // The native stands on who he is, without the chair's own
                // weight: the incumbency bonus is the player's this term.
                let person = people::person(field.site, index);
                let theirs = regard(field.site, voter, index) + bearing(person.temperament.archetype);
                if theirs > held {
                    (Candidate::Resident(index), theirs)
                } else {
                    (incumbent, held)
                }
            }
            None => (incumbent, held),
        };
        if Some(cast_for) == challenger {
            for_challenger += 1;
        }
        votes.push(Vote {
            voter,
            cast_for,
            score,
        });
    }

    // A majority, and a dead heat keeps the incumbent: a town that cannot
    // make up its mind does not change its mind.
    let winner = match challenger {
        Some(who) if for_challenger * 2 > PEOPLE => who,
        _ => incumbent,
    };
    (winner, votes)
}

/// A seat that is not what the seed said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub holder: Candidate,
    /// The term they won, so a panel can say how long they have had it.
    pub since: u32,
}

/// Every seat anywhere that has changed hands, and nothing else.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Register {
    seats: BTreeMap<((i32, i32), u8), Seat>,
    /// The last term each seat has actually polled, so an election is held
    /// once rather than every tick of its polling day.
    ///
    /// Per seat, not per town — stage 40 kept this per town, which meant the
    /// first office polled on a market day quietly closed the ballot for the
    /// second. Found on the way past in 43, when a taken town's sheriff's
    /// chair never came up for a vote.
    polled: BTreeMap<((i32, i32), u8), u32>,
    /// Which seats the player is standing for. An order sets this, so it is
    /// state the journal replays rather than a derivation.
    standing: std::collections::BTreeSet<((i32, i32), u8)>,
    /// Towns whose chairs were seized rather than won, and the day. Only
    /// the panels read it; a seized seat polls like any other.
    taken: BTreeMap<(i32, i32), u32>,
}

fn slot(office: Office) -> u8 {
    match office {
        Office::Mayor => 0,
        Office::Sheriff => 1,
    }
}

fn office_of_slot(slot: u8) -> Office {
    match slot {
        0 => Office::Mayor,
        _ => Office::Sheriff,
    }
}

impl Register {
    /// Who holds this seat: whoever won it, or whoever the seed said.
    pub fn seated(&self, site: &TownSite, office: Office) -> Candidate {
        self.seats
            .get(&(site.centre, slot(office)))
            .map(|seat| seat.holder)
            .unwrap_or(Candidate::Resident(office::holder(site, office)))
    }

    /// Does the player hold this seat?
    pub fn player_holds(&self, town: (i32, i32), office: Office) -> bool {
        matches!(
            self.seats.get(&(town, slot(office))),
            Some(Seat { holder: Candidate::Player, .. })
        )
    }

    /// Every seat the player holds anywhere, for the panels and the factions
    /// hook.
    pub fn player_seats(&self) -> impl Iterator<Item = ((i32, i32), Office)> + '_ {
        self.seats
            .iter()
            .filter(|(_, seat)| seat.holder.is_player())
            .map(|((town, slot), _)| (*town, office_of_slot(*slot)))
    }

    /// When they took it.
    pub fn since(&self, town: (i32, i32), office: Office) -> Option<u32> {
        self.seats.get(&(town, slot(office))).map(|seat| seat.since)
    }

    /// Is the player's name on the ballot here?
    pub fn is_standing(&self, town: (i32, i32), office: Office) -> bool {
        self.standing.contains(&(town, slot(office)))
    }

    /// Seat the player, standing, in one chair — a founded town's default,
    /// and half of a taken one.
    pub fn seat_player(&mut self, town: (i32, i32), office: Office, day: u32) {
        self.seats.insert(
            (town, slot(office)),
            Seat {
                holder: Candidate::Player,
                since: term_of(day),
            },
        );
        self.standing.insert((town, slot(office)));
    }

    /// Both chairs to the player, by force. Remembered as taken, so the
    /// panels can say so — the seats themselves are ordinary entries and
    /// the next poll treats them as any player-held seat.
    pub fn seize(&mut self, town: (i32, i32), day: u32) {
        for office in crate::office::OFFICES {
            self.seat_player(town, office, day);
        }
        self.taken.insert(town, day);
    }

    /// The day this town was taken, if it was — and still held.
    pub fn taken(&self, town: (i32, i32)) -> Option<u32> {
        let held = crate::office::OFFICES
            .into_iter()
            .any(|office| self.player_holds(town, office));
        held.then(|| self.taken.get(&town).copied()).flatten()
    }

    /// Put your name in, or take it out. The journal's `Stand` order.
    pub fn stand(&mut self, town: (i32, i32), office: Office, on: bool) {
        if on {
            self.standing.insert((town, slot(office)));
        } else {
            self.standing.remove(&(town, slot(office)));
        }
    }

    /// Has this seat already polled this term?
    pub fn polled_this_term(&self, town: (i32, i32), office: Office, day: u32) -> bool {
        self.polled
            .get(&(town, slot(office)))
            .is_some_and(|term| *term >= term_of(day))
    }

    /// Hold the election, if one is due here today.
    ///
    /// Returns the result when a poll actually ran. Called on the same beat
    /// the warrant chain runs on, from both the live loop and the replay, so
    /// neither side needs an order to know an election happened.
    pub fn hold(&mut self, field: &Field<'_>, office: Office, day: u32) -> Option<Held> {
        let town = field.site.centre;
        if !is_polling_day(field.site, day) || self.polled_this_term(town, office, day) {
            return None;
        }
        let before = self.seated(field.site, office);
        let (winner, votes) = poll(field, before);
        // The bookkeeping goes in either way, so the town does not re-poll
        // for the rest of the day.
        self.polled.insert((town, slot(office)), term_of(day));
        if winner == before {
            return Some(Held {
                office,
                before,
                after: winner,
                changed: false,
                votes,
            });
        }
        // Back to the derived answer means *forgetting* the entry rather than
        // storing it, which is what keeps the register sparse over a long
        // game: a town that elected you and then threw you out costs the save
        // nothing again.
        if winner == Candidate::Resident(office::holder(field.site, office)) {
            self.seats.remove(&(town, slot(office)));
        } else {
            self.seats.insert(
                (town, slot(office)),
                Seat {
                    holder: winner,
                    since: term_of(day),
                },
            );
        }
        Some(Held {
            office,
            before,
            after: winner,
            changed: true,
            votes,
        })
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "elections.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;

        file.write_all(&(self.seats.len() as u32).to_le_bytes())?;
        for (((x, z), slot), seat) in &self.seats {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&[*slot])?;
            match seat.holder {
                Candidate::Player => file.write_all(&[0u8, 0u8])?,
                Candidate::Resident(index) => file.write_all(&[1u8, index as u8])?,
            }
            file.write_all(&seat.since.to_le_bytes())?;
        }

        file.write_all(&(self.polled.len() as u32).to_le_bytes())?;
        for (((x, z), slot), term) in &self.polled {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&[*slot])?;
            file.write_all(&term.to_le_bytes())?;
        }

        file.write_all(&(self.standing.len() as u32).to_le_bytes())?;
        for ((x, z), slot) in &self.standing {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&[*slot])?;
        }

        // Version two: the towns whose chairs were seized, and the day.
        file.write_all(&(self.taken.len() as u32).to_le_bytes())?;
        for ((x, z), day) in &self.taken {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&day.to_le_bytes())?;
        }
        file.commit()
    }

    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("elections.dat");
        match read_register(&path) {
            Ok(Some(register)) => *self = register,
            Ok(None) => {}
            Err(error) => log::warn!("unreadable {}: {error}", path.display()),
        }
    }
}

/// What one election did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub office: Office,
    pub before: Candidate,
    pub after: Candidate,
    pub changed: bool,
    pub votes: Vec<Vote>,
}

fn read_register(path: &Path) -> std::io::Result<Option<Register>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not an election file"));
    }
    let mut word = [0u8; 4];
    let mut pair = [0u8; 2];
    let mut byte = [0u8; 1];
    file.read_exact(&mut word)?;
    let version = u32::from_le_bytes(word);
    if version != VERSION && version != OLDEST {
        return Ok(None);
    }
    let mut register = Register::default();

    file.read_exact(&mut word)?;
    for _ in 0..u32::from_le_bytes(word) {
        file.read_exact(&mut word)?;
        let x = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let z = i32::from_le_bytes(word);
        file.read_exact(&mut byte)?;
        let slot = byte[0];
        file.read_exact(&mut pair)?;
        let holder = if pair[0] == 0 {
            Candidate::Player
        } else {
            Candidate::Resident(pair[1] as usize)
        };
        file.read_exact(&mut word)?;
        let since = u32::from_le_bytes(word);
        register.seats.insert(((x, z), slot), Seat { holder, since });
    }

    file.read_exact(&mut word)?;
    for _ in 0..u32::from_le_bytes(word) {
        file.read_exact(&mut word)?;
        let x = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let z = i32::from_le_bytes(word);
        // Version one kept one term per town; it closes both seats.
        if version >= 2 {
            file.read_exact(&mut byte)?;
            file.read_exact(&mut word)?;
            register.polled.insert(((x, z), byte[0]), u32::from_le_bytes(word));
        } else {
            file.read_exact(&mut word)?;
            let term = u32::from_le_bytes(word);
            register.polled.insert(((x, z), 0), term);
            register.polled.insert(((x, z), 1), term);
        }
    }

    file.read_exact(&mut word)?;
    for _ in 0..u32::from_le_bytes(word) {
        file.read_exact(&mut word)?;
        let x = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let z = i32::from_le_bytes(word);
        file.read_exact(&mut byte)?;
        register.standing.insert(((x, z), byte[0]));
    }

    // Version two: the towns whose chairs were seized. A version-one file
    // simply has none, which is the truth about it.
    if version >= 2 {
        file.read_exact(&mut word)?;
        for _ in 0..u32::from_le_bytes(word) {
            file.read_exact(&mut word)?;
            let x = i32::from_le_bytes(word);
            file.read_exact(&mut word)?;
            let z = i32::from_le_bytes(word);
            file.read_exact(&mut word)?;
            register.taken.insert((x, z), u32::from_le_bytes(word));
        }
    }
    Ok(Some(register))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_world::town;

    fn frontier() -> Vec<TownSite> {
        town::towns_near(2024, (0, 0), 6_000, &|_, _| 90)
    }

    fn field<'a>(site: &'a TownSite, friends: &'a Disposition) -> Field<'a> {
        Field {
            site,
            friends,
            bounty: 0,
            incumbent_troubled: false,
            standing: false,
            seat: Office::Mayor,
            founder: false,
        }
    }

    /// A town polls on its own market day and nowhere else, and the answer is
    /// the same however often you ask — the property that lets both sides of
    /// the oracle hold the same election without an order.
    #[test]
    fn polling_day_is_the_towns_own_market_day() {
        for site in frontier() {
            let mut polls = 0;
            for day in 0..28 {
                let due = is_polling_day(&site, day);
                assert_eq!(due, is_polling_day(&site, day));
                if due {
                    assert!(
                        crate::schedule::is_market_day(&site, day),
                        "{} voted on a day it holds no market",
                        site.name.head()
                    );
                    polls += 1;
                }
            }
            assert_eq!(polls, 4, "a four-week month should hold four polls");
        }
    }

    /// An untouched frontier is every seat at its derived default and an
    /// empty file — the sparse rule every ledger in this project keeps.
    #[test]
    fn an_unelected_frontier_costs_the_save_nothing() {
        let register = Register::default();
        assert!(register.seats.is_empty() && register.standing.is_empty());
        for site in frontier() {
            for office in office::OFFICES {
                assert_eq!(
                    register.seated(&site, office),
                    Candidate::Resident(office::holder(&site, office))
                );
                assert!(!register.player_holds(site.centre, office));
            }
        }
    }

    /// Not standing means not on the ballot: an election you ignored still
    /// happens, and a resident wins it.
    #[test]
    fn an_election_you_ignore_elects_a_local() {
        let site = town::home_site();
        let friends = Disposition::default();
        let mut register = Register::default();
        let day = next_poll(&site, 0);
        let held = register
            .hold(&field(&site, &friends), Office::Mayor, day)
            .expect("no election was held on a polling day");
        assert!(!held.after.is_player());
        assert_eq!(held.votes.len(), PEOPLE);
    }

    /// And business wins one. The note's whole claim: the residents who
    /// profit from trading with you vote your way.
    #[test]
    fn a_town_you_have_traded_with_elects_you() {
        let site = town::home_site();
        let mut friends = Disposition::default();
        for voter in 0..PEOPLE {
            for day in 0..60 {
                friends.trade((site.centre, voter as u8), 1_000, day);
            }
        }
        let mut register = Register::default();
        let mut standing = field(&site, &friends);
        standing.standing = true;
        standing.seat = Office::Sheriff;
        let day = next_poll(&site, 0);
        let held = register
            .hold(&standing, Office::Sheriff, day)
            .expect("no election");
        assert!(held.changed, "the town re-elected its own man");
        assert!(held.after.is_player(), "{:?}", held.votes);
        assert!(register.player_holds(site.centre, Office::Sheriff));
        assert_eq!(
            register.player_seats().collect::<Vec<_>>(),
            vec![(site.centre, Office::Sheriff)]
        );
    }

    /// A bounty is a candidate who looks like trouble, and it is steep enough
    /// to lose an election you would otherwise have won.
    #[test]
    fn a_wanted_candidate_loses_the_room() {
        let site = town::home_site();
        let mut friends = Disposition::default();
        for voter in 0..PEOPLE {
            for day in 0..60 {
                friends.trade((site.centre, voter as u8), 1_000, day);
            }
        }
        let mut clean = field(&site, &friends);
        clean.standing = true;
        clean.seat = Office::Sheriff;
        let mut wanted = clean;
        wanted.bounty = 400;

        let day = next_poll(&site, 0);
        let won = Register::default()
            .hold(&clean, Office::Sheriff, day)
            .expect("no election");
        let lost = Register::default()
            .hold(&wanted, Office::Sheriff, day)
            .expect("no election");
        assert!(won.after.is_player());
        assert!(
            !lost.after.is_player(),
            "a wanted man walked the election anyway"
        );
    }

    /// An incumbent under a cloud loses ground. The join back to stage 39:
    /// paperwork against the man in the chair is a fact the town can see.
    #[test]
    fn paper_on_the_incumbent_costs_him_votes() {
        assert!(record(true) < record(false));
        let site = town::home_site();
        let friends = Disposition::default();
        let seat = Candidate::Resident(office::holder(&site, Office::Mayor));
        let clean = field(&site, &friends);
        let mut troubled = clean;
        troubled.incumbent_troubled = true;
        let (_, steady) = poll(&clean, seat);
        let (_, shaky) = poll(&troubled, seat);
        let backing = |votes: &[Vote]| votes.iter().filter(|vote| vote.cast_for == seat).count();
        assert!(
            backing(&shaky) <= backing(&steady),
            "a warrant made the mayor more popular"
        );
    }

    /// A town polls once a term, not once a tick.
    #[test]
    fn a_town_votes_once_a_term() {
        let site = town::home_site();
        let friends = Disposition::default();
        let mut register = Register::default();
        let day = next_poll(&site, 0);
        assert!(register.hold(&field(&site, &friends), Office::Mayor, day).is_some());
        assert!(register.hold(&field(&site, &friends), Office::Mayor, day).is_none());
        // And again next term.
        let later = next_poll(&site, day + TERM_DAYS);
        assert!(register
            .hold(&field(&site, &friends), Office::Mayor, later)
            .is_some());
    }

    /// Losing a seat forgets it rather than storing a defeat, which is what
    /// keeps the register sparse across a long game.
    #[test]
    fn losing_a_seat_returns_it_to_the_seed() {
        let site = town::home_site();
        let mut friends = Disposition::default();
        for voter in 0..PEOPLE {
            for day in 0..60 {
                friends.trade((site.centre, voter as u8), 1_000, day);
            }
        }
        let mut register = Register::default();
        let mut won = field(&site, &friends);
        won.standing = true;
        won.seat = Office::Sheriff;
        let day = next_poll(&site, 0);
        register.hold(&won, Office::Sheriff, day).expect("no election");
        assert_eq!(register.seats.len(), 1);

        // Withdraw, and the town gives the badge back to the man the seed
        // always said should have it — which means *forgetting* the entry
        // rather than writing a defeat down.
        let mut quit = field(&site, &friends);
        quit.seat = Office::Sheriff;
        let later = next_poll(&site, day + TERM_DAYS);
        let held = register.hold(&quit, Office::Sheriff, later).expect("no election");
        assert!(!held.after.is_player(), "nobody stood and you kept the badge");
        assert_eq!(
            held.after,
            Candidate::Resident(office::holder(&site, Office::Sheriff)),
            "the badge went to somebody the seed never seated"
        );
        assert!(
            register.seats.is_empty(),
            "a lost seat was written down anyway"
        );
    }

    /// Standing is a decision, so it survives a reload — as does a seat and
    /// the term it was won in.
    #[test]
    fn the_register_survives_a_save_and_an_empty_one_loads_clean() {
        let directory = std::env::temp_dir().join(format!("vx-ballot-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut register = Register::default();
        register.seats.insert(
            ((0, 0), 1),
            Seat {
                holder: Candidate::Player,
                since: 3,
            },
        );
        register.seats.insert(
            ((-512, 1_024), 0),
            Seat {
                holder: Candidate::Resident(2),
                since: 1,
            },
        );
        register.polled.insert(((0, 0), 0), 3);
        register.stand((-512, 1_024), Office::Sheriff, true);
        register.save(&directory).unwrap();

        let mut loaded = Register::default();
        loaded.load(&directory);
        assert_eq!(loaded, register);
        assert!(loaded.player_holds((0, 0), Office::Sheriff));
        assert!(loaded.is_standing((-512, 1_024), Office::Sheriff));
        assert_eq!(loaded.since((0, 0), Office::Sheriff), Some(3));

        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        let mut fresh = Register::default();
        fresh.load(&directory);
        assert_eq!(fresh, Register::default());
        std::fs::remove_dir_all(&directory).ok();
    }

    /// A town's own politics do not change because somebody walked into it.
    #[test]
    fn a_towns_regard_for_its_own_is_stable() {
        for site in frontier() {
            for voter in 0..PEOPLE {
                for of in 0..PEOPLE {
                    assert_eq!(regard(&site, voter, of), regard(&site, voter, of));
                }
                assert!(
                    regard(&site, voter, voter) > 0,
                    "nobody thinks anything of themselves"
                );
            }
        }
    }
}

#[cfg(test)]
mod charter_tests {
    use super::*;
    use vx_world::town;

    fn frontier() -> Vec<TownSite> {
        town::towns_near(2024, (0, 0), 6_000, &|_, _| 90)
            .into_iter()
            .filter(|site| !site.is_home())
            .collect()
    }

    /// A founder with no trade on the books keeps both chairs through a
    /// quiet term in every town on the frontier — the founder's due is sized
    /// to carry a seat against anybody's regard for the man the seed named.
    /// The same player in a town they merely hold, with the same empty
    /// books, is thrown out somewhere along the same frontier.
    #[test]
    fn the_founders_due_keeps_a_quiet_founder_in_and_nobody_else() {
        let friends = Disposition::default();
        let mut thrown_out_somewhere = false;
        for site in frontier().into_iter().take(12) {
            for office in office::OFFICES {
                let mut register = Register::default();
                register.seat_player(site.centre, office, 0);
                let day = next_poll(&site, 1);
                let founded = Field {
                    site: &site,
                    friends: &friends,
                    bounty: 0,
                    incumbent_troubled: false,
                    standing: true,
                    seat: office,
                    founder: true,
                };
                let held = register.hold(&founded, office, day).expect("no poll");
                assert!(
                    held.after.is_player(),
                    "{} threw out its founder as {:?}",
                    site.name,
                    office
                );

                let mut register = Register::default();
                register.seat_player(site.centre, office, 0);
                let merely = Field { founder: false, ..founded };
                let held = register.hold(&merely, office, day).expect("no poll");
                if !held.after.is_player() {
                    thrown_out_somewhere = true;
                    // And losing forgets the entry: the register is sparse.
                    assert!(!register.player_holds(site.centre, office));
                }
            }
        }
        assert!(
            thrown_out_somewhere,
            "a player holding a seat with no trade is never voted out anywhere"
        );
    }

    /// A taken town is a town you have to keep. Seized with a vault's worth
    /// of bounty on the sheet and every ledger torn up, the next poll throws
    /// you out; with the fines paid and the trust bought back, it does not.
    #[test]
    fn a_taken_town_throws_you_out_unless_you_pay_and_buy_it_back() {
        let site = frontier()[0];
        let mut register = Register::default();
        register.seize(site.centre, 3);
        assert_eq!(register.taken(site.centre), Some(3));
        for office in office::OFFICES {
            assert!(register.player_holds(site.centre, office));
        }

        let torn_up = Disposition::default();
        let day = next_poll(&site, 4);
        let field = Field {
            site: &site,
            friends: &torn_up,
            bounty: crate::permits::TAKEN_BOUNTY,
            incumbent_troubled: false,
            standing: true,
            seat: Office::Mayor,
            founder: false,
        };
        let held = register.hold(&field, Office::Mayor, day).expect("no poll");
        assert!(!held.after.is_player(), "a town you just took kept you with the sheet unpaid");
        assert!(!register.player_holds(site.centre, Office::Mayor));
        // Losing the chairs forgets that they were taken.
        let mut both = Register::default();
        both.seize(site.centre, 3);
        for office in office::OFFICES {
            both.hold(&Field { seat: office, ..field }, office, day);
        }
        assert_eq!(both.taken(site.centre), None, "a town that threw you out is still 'taken'");

        // Paid down and bought back: every resident traded with, hard.
        let mut bought = Disposition::default();
        for voter in 0..people::PEOPLE as u8 {
            for day in 0..40 {
                bought.trade((site.centre, voter), 900, day);
            }
        }
        let mut register = Register::default();
        register.seize(site.centre, 3);
        let paid = Field {
            friends: &bought,
            bounty: 0,
            ..field
        };
        let held = register.hold(&paid, Office::Mayor, day).expect("no poll");
        assert!(held.after.is_player(), "a town you bought back still threw you out");
    }

    /// The taken map survives the file, and a version-one file loads
    /// without one.
    #[test]
    fn the_taken_map_survives_a_save_and_an_old_file_still_loads() {
        let directory = std::env::temp_dir().join(format!("vx-taken-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let site = frontier()[1];
        let mut register = Register::default();
        register.seize(site.centre, 9);
        register.save(&directory).unwrap();
        let mut back = Register::default();
        back.load(&directory);
        assert_eq!(back, register);
        assert_eq!(back.taken(site.centre), Some(9));

        // A version-one file: the same bytes up to the taken map, with the
        // version word rewritten. It loads, and nothing is taken.
        let path = directory.join("elections.dat");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        let cut = bytes.len() - 4 - 12; // the taken count and one entry
        bytes.truncate(cut);
        std::fs::write(&path, bytes).unwrap();
        let mut old = Register::default();
        old.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(old.taken(site.centre), None);
        assert!(old.player_holds(site.centre, Office::Mayor), "the old file lost its seats");
    }
}
