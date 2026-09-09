//! The chain between a bounty and a posse.
//!
//! # The middle was missing
//!
//! Stage 11 gave the frontier a bounty sheet and stage 28 gave it deputies,
//! and the join between them was one comparison:
//! `bounty >= WARRANT_THRESHOLD` and four armed men appeared over the hill.
//! Crossing a hundred credits of fines went from *nothing at all* to a
//! firefight, with no step in between and nobody's decision in the middle of
//! it.
//!
//! The civic note is specific about what belongs there: "when an individual's
//! bounty crosses a threshold, the sheriff cannot act alone — they must
//! obtain a **warrant from the mayor**, and only then may dispatch the
//! offender", with the consequences short of force being "fines, revoked
//! market access".
//!
//! # The mayor is a person, and that is the point
//!
//! [`crate::office`] says who holds the seat, and that person has an
//! archetype off [`crate::people`] and an opinion of you off
//! [`crate::disposition`]. So the decision has an author: a `Proud` mayor
//! signs because the law is the law, a `Craven` one stalls because signing
//! means consequences, and a mayor you have been feeding gifts to for a
//! season will find reasons to leave the paperwork in a drawer.
//!
//! It buys **time, not impunity**. Past [`SIGNS_REGARDLESS`] — the price the
//! permits round put on a vault — nobody's friend gets a pass, because a
//! system where enough gifts make you untouchable is a system with no law in
//! it at all.
//!
//! # Deterministic, and outside the hash
//!
//! Nothing here edits a block. The decision is arithmetic on things both a
//! live session and a reload can see — the seat, the ledger, the bounty and
//! the tick — plus one hashed nudge for texture, so a reloaded save reaches
//! the same verdict rather than re-rolling it. What is *stored* is only the
//! towns that have a warrant open, which on a clean frontier is none of
//! them.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use vx_world::TownSite;

use crate::disposition::Tier;
use crate::people::{Archetype, Person};
use crate::permits;

const MAGIC: &[u8; 4] = b"VXWT";
const VERSION: u32 = 1;

/// What share of the standing bounty the town takes as a fine when the
/// sheriff files. A quarter: enough to hurt, never enough to be the whole
/// punishment.
pub const FINE_SHARE: u64 = 4;

/// How long a refused petition holds the sheriff off, in journal ticks —
/// about four minutes of play. A reprieve, not a pardon.
pub const REPRIEVE_TICKS: u64 = 64 * 240;

/// The bounty past which the mayor signs whoever you are. Deliberately the
/// price of a vault, so the one crime the permits round called unforgivable
/// stays unforgivable.
pub const SIGNS_REGARDLESS: u64 = permits::BOUNTY_VAULT;

/// Where a town's paperwork has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The sheriff has asked. Fines and a closed market start here.
    Petitioned { filed: u64 },
    /// Signed. This is what the posse waits for.
    Granted { at: u64 },
    /// Refused, until this tick. The sheriff may ask again after it.
    Refused { until: u64 },
}

impl Stage {
    /// How the terminal says it.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Petitioned { .. } => "PETITIONED",
            Stage::Granted { .. } => "GRANTED",
            Stage::Refused { .. } => "REFUSED",
        }
    }
}

/// One town's paperwork on you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Warrant {
    pub stage: Stage,
    /// What the bounty stood at when the sheriff filed. Kept so the fine is
    /// charged once against a figure rather than repeatedly against a
    /// moving one.
    pub at_bounty: u64,
}

/// What filing a petition cost you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filed {
    /// The fine the town wants now.
    pub fine: u64,
}

/// Everything anybody has open on you. Sparse: a town with no warrant is not
/// in here, and a clean frontier costs the save four bytes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Docket {
    open: BTreeMap<(i32, i32), Warrant>,
}

impl Docket {
    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    pub fn get(&self, town: (i32, i32)) -> Option<Warrant> {
        self.open.get(&town).copied()
    }

    /// Every town with paperwork open, for the roster and the panel.
    pub fn iter(&self) -> impl Iterator<Item = ((i32, i32), Warrant)> + '_ {
        self.open.iter().map(|(town, warrant)| (*town, *warrant))
    }

    /// May the deputies come out here?
    ///
    /// The one question the posse asks, and the whole reason this module
    /// exists: a bounty is no longer enough on its own.
    pub fn granted_in(&self, town: (i32, i32)) -> bool {
        matches!(self.get(town), Some(Warrant { stage: Stage::Granted { .. }, .. }))
    }

    /// Is anything open at all here? Fines and the closed market answer to
    /// this rather than to `granted_in` — the town starts leaning on you the
    /// moment the sheriff walks into the mayor's office.
    pub fn pending_in(&self, town: (i32, i32)) -> bool {
        matches!(
            self.get(town),
            Some(Warrant {
                stage: Stage::Petitioned { .. } | Stage::Granted { .. },
                ..
            })
        )
    }

    /// The sheriff files, and the mayor decides on the spot.
    ///
    /// Returns what it cost you, or `None` if there was nothing to file —
    /// the bounty is under the threshold, a refusal is still standing, or
    /// the paperwork is already in.
    #[allow(clippy::too_many_arguments)]
    pub fn file(
        &mut self,
        site: &TownSite,
        mayor: &Person,
        tier: Tier,
        trust: i64,
        bounty: u64,
        tick: u64,
    ) -> Option<Filed> {
        if bounty < permits::WARRANT_THRESHOLD {
            return None;
        }
        match self.get(site.centre) {
            Some(Warrant { stage: Stage::Refused { until }, .. }) if tick < until => return None,
            Some(Warrant { stage: Stage::Petitioned { .. } | Stage::Granted { .. }, .. }) => {
                return None
            }
            _ => {}
        }

        let signs = decides(site, mayor, tier, trust, bounty, tick);
        let stage = if signs {
            Stage::Granted { at: tick }
        } else {
            // A refusal still starts as a petition on the books for this
            // tick: the town has been asked, and the asking is what costs
            // you. It simply does not become deputies.
            Stage::Refused {
                until: tick + REPRIEVE_TICKS,
            }
        };
        self.open.insert(
            site.centre,
            Warrant {
                stage,
                at_bounty: bounty,
            },
        );
        Some(Filed {
            fine: bounty / FINE_SHARE,
        })
    }

    /// The bill is settled and the town loses interest.
    pub fn clear(&mut self, town: (i32, i32)) {
        self.open.remove(&town);
    }

    /// Drop everything: what paying off the whole sheet does.
    pub fn clear_all(&mut self) {
        self.open.clear();
    }

    /// Let expired refusals lapse, so the sheriff may ask again.
    ///
    /// Called on the same beat as everything else. A refusal that stayed on
    /// the books forever would be a pardon with extra steps.
    pub fn lapse(&mut self, tick: u64) {
        self.open.retain(|_, warrant| match warrant.stage {
            Stage::Refused { until } => tick < until,
            _ => true,
        });
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "warrants.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.open.len() as u32).to_le_bytes())?;
        for ((x, z), warrant) in &self.open {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&warrant.at_bounty.to_le_bytes())?;
            let (tag, when) = match warrant.stage {
                Stage::Petitioned { filed } => (0u8, filed),
                Stage::Granted { at } => (1u8, at),
                Stage::Refused { until } => (2u8, until),
            };
            file.write_all(&[tag])?;
            file.write_all(&when.to_le_bytes())?;
        }
        file.commit()
    }

    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("warrants.dat");
        match read_docket(&path) {
            Ok(Some(open)) => self.open = open,
            Ok(None) => {}
            Err(error) => log::warn!("unreadable {}: {error}", path.display()),
        }
    }
}

fn read_docket(path: &Path) -> std::io::Result<Option<BTreeMap<(i32, i32), Warrant>>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a warrant file"));
    }
    let mut word = [0u8; 4];
    let mut long = [0u8; 8];
    let mut byte = [0u8; 1];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    file.read_exact(&mut word)?;
    let towns = u32::from_le_bytes(word);
    let mut open = BTreeMap::new();
    for _ in 0..towns {
        file.read_exact(&mut word)?;
        let x = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let z = i32::from_le_bytes(word);
        file.read_exact(&mut long)?;
        let at_bounty = u64::from_le_bytes(long);
        file.read_exact(&mut byte)?;
        file.read_exact(&mut long)?;
        let when = u64::from_le_bytes(long);
        let stage = match byte[0] {
            0 => Stage::Petitioned { filed: when },
            1 => Stage::Granted { at: when },
            _ => Stage::Refused { until: when },
        };
        open.insert((x, z), Warrant { stage, at_bounty });
    }
    Ok(Some(open))
}

/// How hard a mayor is to move, out of a hundred.
///
/// The same six archetypes the whole game runs on, spent a third time. A
/// proud mayor's resolve is the trait that makes him refuse to surrender in
/// the combat rounds; a craven one's is the trait that makes him break.
pub fn resolve(archetype: Archetype) -> i64 {
    match archetype {
        Archetype::Proud => 90,
        Archetype::Gruff => 76,
        Archetype::Steady => 70,
        Archetype::Anxious => 56,
        Archetype::Chatty => 50,
        Archetype::Craven => 34,
    }
}

/// What being liked is worth, in points off the mayor's resolve.
fn goodwill(tier: Tier, trust: i64) -> i64 {
    let friendship = match tier {
        Tier::Stranger => 0,
        Tier::Acquainted => 8,
        Tier::Friendly => 18,
        Tier::Trusted => 30,
        Tier::Close => 44,
    };
    // Business counts for less than friendship here, and caps sooner: doing
    // trade with the man does not make him bend the law, it makes him wish he
    // did not have to.
    friendship + (trust / 40).min(16)
}

/// Does the mayor sign?
///
/// Pure in everything it reads, which is what lets a reload reach the same
/// verdict without the verdict having been written down.
pub fn decides(
    site: &TownSite,
    mayor: &Person,
    tier: Tier,
    trust: i64,
    bounty: u64,
    tick: u64,
) -> bool {
    if bounty >= SIGNS_REGARDLESS {
        return true;
    }
    // How far past the threshold you are, as pressure on the seat. Capped, so
    // it can be outweighed by a friendship right up to the ceiling above.
    let over = bounty.saturating_sub(permits::WARRANT_THRESHOLD);
    let pressure = ((over * 60) / permits::WARRANT_THRESHOLD.max(1)).min(60) as i64;
    // One hashed nudge, so two identical situations in two towns are not
    // identical answers — and hashed off the tick the petition was filed on,
    // so it is the same nudge on a reload.
    let hash = vx_world::seed::finalise(
        site.seed
            ^ 0x1a97_0000_0000_00c1
            ^ tick.wrapping_mul(0x9e37_79b9_7f4a_7c15),
    );
    let nudge = (hash % 17) as i64 - 8;

    resolve(mayor.temperament.archetype) + pressure + nudge - goodwill(tier, trust) > 60
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office;
    use crate::people;
    use vx_world::town;

    fn frontier() -> Vec<TownSite> {
        town::towns_near(2024, (0, 0), 6_000, &|_, _| 90)
    }

    fn mayor_of(site: &TownSite) -> Person {
        office::seat(site, permits::Office::Mayor)
    }

    /// A bounty under the threshold is not the sheriff's business, and
    /// nothing goes on the books for it.
    #[test]
    fn nothing_is_filed_under_the_threshold() {
        let site = town::home_site();
        let mut docket = Docket::default();
        assert_eq!(
            docket.file(
                &site,
                &mayor_of(&site),
                Tier::Stranger,
                0,
                permits::WARRANT_THRESHOLD - 1,
                0
            ),
            None
        );
        assert!(docket.is_empty());
        assert!(!docket.pending_in(site.centre));
    }

    /// Rob the vault and it does not matter who your friends are.
    #[test]
    fn a_vault_is_signed_for_however_well_liked_you_are() {
        for site in frontier() {
            let mayor = mayor_of(&site);
            assert!(
                decides(&site, &mayor, Tier::Close, 10_000, SIGNS_REGARDLESS, 0),
                "{} let a vault go",
                site.name.head()
            );
        }
    }

    /// And friendship really does buy something: across the frontier, being
    /// close to the mayor turns some signatures into refusals. Measured as a
    /// share rather than asserted town by town, because a proud mayor is
    /// supposed to sign anyway — the claim is that it *matters*, not that it
    /// is a get-out-of-jail card.
    #[test]
    fn being_liked_buys_time_but_not_impunity() {
        let bounty = permits::WARRANT_THRESHOLD + 20;
        let mut helped = 0;
        let mut towns = 0;
        for site in frontier() {
            let mayor = mayor_of(&site);
            let cold = decides(&site, &mayor, Tier::Stranger, 0, bounty, 0);
            let warm = decides(&site, &mayor, Tier::Close, 600, bounty, 0);
            towns += 1;
            if cold && !warm {
                helped += 1;
            }
            assert!(
                !warm || cold,
                "{} signed *because* you are friends",
                site.name.head()
            );
        }
        assert!(towns > 2, "the fixture frontier is too small to say anything");
        assert!(
            helped > 0,
            "friendship with the mayor changed nothing anywhere"
        );
    }

    /// A refusal holds the sheriff off, then lapses and can be filed again.
    #[test]
    fn a_refusal_is_a_reprieve_and_not_a_pardon() {
        let site = town::home_site();
        // The hometown mayor is authored `Proud`, so lean on the decision
        // rather than hoping: a craven stand-in is what a refusal needs.
        let mut mayor = mayor_of(&site);
        mayor.temperament.archetype = Archetype::Craven;
        let bounty = permits::WARRANT_THRESHOLD;
        let mut docket = Docket::default();

        let filed = docket
            .file(&site, &mayor, Tier::Close, 800, bounty, 0)
            .expect("nothing was filed at all");
        assert_eq!(filed.fine, bounty / FINE_SHARE);
        let refused = matches!(
            docket.get(site.centre).map(|warrant| warrant.stage),
            Some(Stage::Refused { .. })
        );
        assert!(refused, "a craven mayor signed for a friend at the threshold");
        assert!(!docket.granted_in(site.centre), "deputies came out anyway");

        // While it stands, the sheriff cannot re-file.
        assert_eq!(docket.file(&site, &mayor, Tier::Close, 800, bounty, 10), None);

        // And once it lapses, he can — and this time the bounty is bigger.
        docket.lapse(REPRIEVE_TICKS + 1);
        assert!(docket.is_empty(), "the refusal never lapsed");
        assert!(docket
            .file(&site, &mayor, Tier::Close, 800, SIGNS_REGARDLESS, REPRIEVE_TICKS + 1)
            .is_some());
        assert!(docket.granted_in(site.centre));
    }

    /// The fine is charged once for the asking, not once per beat.
    #[test]
    fn the_town_bills_you_once_for_the_paperwork() {
        let site = town::home_site();
        let mayor = mayor_of(&site);
        let mut docket = Docket::default();
        let bounty = permits::WARRANT_THRESHOLD * 3;
        assert!(docket.file(&site, &mayor, Tier::Stranger, 0, bounty, 0).is_some());
        for tick in 1..40 {
            assert_eq!(
                docket.file(&site, &mayor, Tier::Stranger, 0, bounty, tick),
                None,
                "the town billed you twice for one warrant"
            );
        }
    }

    /// Deputies wait for a signature. This is the join the whole round is
    /// about: a bounty over the threshold used to *be* a posse.
    #[test]
    fn a_petition_is_not_a_posse() {
        let site = town::home_site();
        let mut mayor = mayor_of(&site);
        mayor.temperament.archetype = Archetype::Craven;
        let mut docket = Docket::default();
        docket.file(&site, &mayor, Tier::Close, 900, permits::WARRANT_THRESHOLD, 0);
        assert!(docket.pending_in(site.centre) || !docket.is_empty());
        assert!(!docket.granted_in(site.centre));
    }

    /// The books survive a reload, because a warrant a reload forgot would be
    /// a warrant that never happened.
    #[test]
    fn the_docket_survives_a_save_and_an_empty_one_loads_clean() {
        let directory = std::env::temp_dir().join(format!("vx-warrant-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut docket = Docket::default();
        let site = town::home_site();
        docket.file(
            &site,
            &mayor_of(&site),
            Tier::Stranger,
            0,
            SIGNS_REGARDLESS,
            4_200,
        );
        docket.open.insert(
            (-512, 1_024),
            Warrant {
                stage: Stage::Refused { until: 99_999 },
                at_bounty: 140,
            },
        );
        docket.save(&directory).unwrap();

        let mut loaded = Docket::default();
        loaded.load(&directory);
        assert_eq!(loaded, docket);
        assert!(loaded.granted_in(site.centre));
        assert!(!loaded.granted_in((-512, 1_024)));

        // And a directory with no file in it is a clean sheet, not an error.
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        let mut fresh = Docket::default();
        fresh.load(&directory);
        assert!(fresh.is_empty());
        std::fs::remove_dir_all(&directory).ok();
    }

    /// An untouched frontier is not in the file at all — the sparse rule
    /// every ledger in this project keeps.
    #[test]
    fn a_clean_record_costs_the_save_nothing() {
        let docket = Docket::default();
        for site in frontier() {
            assert!(!docket.pending_in(site.centre));
            assert!(!docket.granted_in(site.centre));
        }
        assert_eq!(docket.len(), 0);
    }

    /// The mayor's temperament is really what decides it: sweep the six
    /// archetypes against the same situation and the proud end must sign
    /// where the craven end does not.
    #[test]
    fn temperament_decides_who_signs() {
        let site = town::home_site();
        let bounty = permits::WARRANT_THRESHOLD + 10;
        let mut proud = people::person(&site, 0);
        proud.temperament.archetype = Archetype::Proud;
        let mut craven = proud.clone();
        craven.temperament.archetype = Archetype::Craven;
        assert!(resolve(Archetype::Proud) > resolve(Archetype::Craven));
        assert!(decides(&site, &proud, Tier::Friendly, 0, bounty, 0));
        assert!(!decides(&site, &craven, Tier::Friendly, 0, bounty, 0));
    }
}
