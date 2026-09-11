//! Who the townsfolk are: identity, temperament, and what they say.
//!
//! # One personality, both lives
//!
//! The design note's core bet: **a person is one agent with two policies**,
//! and the part both policies share is temperament. The archetype that makes
//! a shopkeeper chatty at the counter is the archetype that will make her
//! slow to panic when the hostiles round arrives; the trait that makes a
//! drifter craven in conversation is the trait that will make him surrender
//! early in a bunker. Character is one derivation, spent twice — and an
//! "unpredictable" person stays deterministic, because the variation is
//! rolled at *creation* and never at runtime.
//!
//! This round spends the civic half. `nerve` sits unread until the combat
//! rounds — deliberately derived *now*, so the day composure lands, every
//! person already has the number and no save changes meaning.
//!
//! # Derived, except where authored
//!
//! The hometown trio — the Mayor, the Sheriff, Old Prat — are authored, with
//! authored gift tables, because the town every player starts in should have
//! people somebody wrote. Everyone else in the world is arithmetic on the
//! site hash, exactly like the town they live in.
//!
//! # Gossip is telemetry wearing a coat
//!
//! Dialogue pools are keyed `(archetype × context)`, and the lines are
//! templates over the *live simulation* — the books' prices, the bounty
//! ledger, the fleet's tank. This is what a derived villager can have that an
//! authored one cannot: things to say that are true.

use vx_world::TownSite;

use crate::economy;

/// How many people a town has. One per dwelling the plans build; the census
/// grows when the plans do, not before — a name with no door to sleep behind
/// would be a ghost, and the permits system already maps beds to exactly
/// these three.
pub const PEOPLE: usize = 3;

/// The six shapes a person comes in.
///
/// Everything a person says and (come the combat rounds) does under pressure
/// is coloured by this one draw — the Animal Crossing lesson, pointed at a
/// frontier. `Proud` and `Craven` are deliberately the combat-relevant pair:
/// one will never surrender, the other will never stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    Steady,
    Chatty,
    Gruff,
    Anxious,
    Proud,
    Craven,
}

impl Archetype {
    pub const ALL: [Archetype; 6] = [
        Archetype::Steady,
        Archetype::Chatty,
        Archetype::Gruff,
        Archetype::Anxious,
        Archetype::Proud,
        Archetype::Craven,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Archetype::Steady => "STEADY",
            Archetype::Chatty => "CHATTY",
            Archetype::Gruff => "GRUFF",
            Archetype::Anxious => "ANXIOUS",
            Archetype::Proud => "PROUD",
            Archetype::Craven => "CRAVEN",
        }
    }
}

/// The one derivation both halves of a person spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Temperament {
    pub archetype: Archetype,
    /// Where composure breaks. Unread until the hostiles round — derived now
    /// so the number exists before anything depends on it.
    pub nerve: u8,
    /// How fast friendship grows: a warm person's ledger earns a small
    /// premium on every kind entry.
    pub warmth: u8,
    /// Which line of a pool this person reaches for.
    pub voice: u8,
}

/// One townsperson.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    /// What they do, which is also what they love getting.
    pub trade: &'static str,
    pub temperament: Temperament,
    /// Two goods they love and one they hate, by name.
    pub loved: [&'static str; 2],
    pub hated: &'static str,
    /// Day of the 28-day year their birthday falls on.
    pub birthday: u32,
}

/// A temperament from a bare seed, for people who have no town roster to
/// come from — the deputies a warrant sends, and whoever else turns up
/// later. The same four properties off the same kind of stream, so a
/// deputy's nerve means exactly what a shopkeeper's nerve means.
pub fn temperament_from(seed: u64) -> Temperament {
    let stream = |salt: u64| vx_world::seed::finalise(seed ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    Temperament {
        archetype: pick(&Archetype::ALL, stream(0x01)),
        nerve: (stream(0x02) % 256) as u8,
        warmth: (stream(0x03) % 256) as u8,
        voice: (stream(0x04) % 256) as u8,
    }
}

/// The person's own hash stream, salted per property.
fn hash(site: &TownSite, index: usize, salt: u64) -> u64 {
    vx_world::seed::finalise(
        site.seed
            ^ (index as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ salt.wrapping_mul(0xc2b2_ae3d_27d4_eb4f),
    )
}

fn pick<T: Copy>(options: &[T], roll: u64) -> T {
    options[(roll % options.len() as u64) as usize]
}

/// Given names, built to be drawable and short.
const HEADS: [&str; 12] = [
    "MER", "DOR", "SAL", "WREN", "HAL", "IDA", "BREN", "OTT", "NEL", "GAR", "PIP", "VER",
];
const TAILS: [&str; 8] = ["A", "IS", "TON", "", "ET", "O", "NA", "RIC"];

/// What the townsfolk of each speciality do for a living, one per person
/// slot. The trades are also the gift bias: a smelterman loves bars the way
/// the note's smith loves fine ore.
fn trades(site: &TownSite) -> [&'static str; PEOPLE] {
    match site.speciality {
        vx_world::Speciality::Mine => ["FOREMAN", "POWDERMAN", "ASSAYER"],
        vx_world::Speciality::Refinery => ["SMELTERMAN", "GAUGER", "STOKER"],
        vx_world::Speciality::Depot => ["CLERK", "TALLYMAN", "OSTLER"],
        // Nobody lives in the Ruined City. These are the people who work the
        // Outpost and go home somewhere else, which is why none of them makes
        // anything — see `economy::made_by`.
        vx_world::Speciality::City => ["FACTOR", "PURSER", "MARSHAL"],
    }
}

/// What each trade would love to be handed, in preference order. The derived
/// pick draws its two loved goods from the front of this list and its hated
/// one from the back half of the catalogue.
fn trade_bias(trade: &str) -> [&'static str; 2] {
    match trade {
        "FOREMAN" | "POWDERMAN" | "ASSAYER" => ["engine:copper_ore", "engine:hho_cell"],
        "SMELTERMAN" | "GAUGER" | "STOKER" => ["engine:copper_bar", "engine:copper_ore"],
        _ => ["engine:copper_bar", "engine:log"],
    }
}

/// The people of a town, in resident order — the same order the permits
/// system numbers beds, which is what lets a Close friendship grant a key to
/// the right door.
pub fn roster(site: &TownSite) -> Vec<Person> {
    (0..PEOPLE).map(|index| person(site, index)).collect()
}

/// One person of a town.
pub fn person(site: &TownSite, index: usize) -> Person {
    let temperament = Temperament {
        archetype: pick(&Archetype::ALL, hash(site, index, 0x01)),
        nerve: (hash(site, index, 0x02) % 256) as u8,
        warmth: (hash(site, index, 0x03) % 256) as u8,
        voice: (hash(site, index, 0x04) % 256) as u8,
    };

    // The hometown trio are authored people, not arithmetic. Their names
    // come from the offices the permits round gave them, their tables from
    // this table.
    if site.is_home() {
        let (name, trade, loved, hated) = match index {
            0 => (
                "THE MAYOR",
                "MAYOR",
                ["engine:copper_bar", "engine:hho_cell"],
                "engine:stone",
            ),
            1 => (
                "THE SHERIFF",
                "SHERIFF",
                ["engine:hho_cell", "engine:metal_wall"],
                "engine:copper_ore",
            ),
            _ => (
                "OLD PRAT",
                "LAYABOUT",
                ["engine:log", "engine:stone"],
                "engine:copper_bar",
            ),
        };
        return Person {
            name: name.to_string(),
            trade,
            temperament: Temperament {
                // Authored temperaments too: the mayor is proud, the sheriff
                // steady, and Old Prat has never been anxious about anything.
                archetype: match index {
                    0 => Archetype::Proud,
                    1 => Archetype::Steady,
                    _ => Archetype::Chatty,
                },
                ..temperament
            },
            loved,
            hated,
            birthday: (hash(site, index, 0x05) % YEAR_DAYS as u64) as u32,
        };
    }

    let trade = trades(site)[index % PEOPLE];
    let bias = trade_bias(trade);
    // The second loved good sometimes wanders off the trade bias, so not
    // every assayer in the world loves the same two things.
    let stray = pick(&economy::GOODS, hash(site, index, 0x06));
    let loved = if hash(site, index, 0x07).is_multiple_of(3) && stray != bias[0] {
        [bias[0], stray]
    } else {
        bias
    };
    // Hated: drawn from the catalogue, never something they love.
    let mut hated = pick(&economy::GOODS, hash(site, index, 0x08));
    if loved.contains(&hated) {
        hated = if loved.contains(&"engine:stone") {
            "engine:log"
        } else {
            "engine:stone"
        };
    }

    let head = pick(&HEADS, hash(site, index, 0x09));
    let tail = pick(&TAILS, hash(site, index, 0x0a));
    Person {
        name: format!("{head}{tail} THE {trade}"),
        trade,
        temperament,
        loved,
        hated,
        birthday: (hash(site, index, 0x05) % YEAR_DAYS as u64) as u32,
    }
}

/// Days in this world's year: four weeks of seven. Short on purpose — a
/// birthday nobody lives long enough to see is authored content thrown away.
pub const YEAR_DAYS: u32 = 28;

// ---------------------------------------------------------------------------
// Speech
// ---------------------------------------------------------------------------

/// What the world knows right now, handed in so a line can be true.
///
/// A struct rather than a lookup so speech stays a pure function — the same
/// facts always produce the same line, which is what makes the panel render
/// deterministic and the whole thing testable without a world.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub town: String,
    /// The local ore price, in credits.
    pub ore_price: u32,
    /// Your standing on the sheriff's board.
    pub bounty: u64,
    /// Is the fleet dry right now?
    pub fleet_dry: bool,
    /// A bearing to the nearest bunker, when one is known to the teller.
    pub bunker: Option<String>,
}

/// What a person of this friendship tier will talk about.
///
/// Order is depth: strangers get pleasantries, acquaintances get the market,
/// and only the trusted mention what their uncle kept in the hills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Smalltalk,
    Prices,
    Bounty,
    Intel,
}

/// The line a person says, given who they are, how well they know you, and
/// what is true.
///
/// Bounty outranks everything — a wanted face is the most interesting thing
/// in the room whoever is looking at it — then intel at Trusted, then the
/// market, then weather-grade pleasantries.
pub fn line_for(person: &Person, tier: crate::disposition::Tier, facts: &Facts) -> String {
    use crate::disposition::Tier;

    let context = if facts.bounty > 0 {
        Context::Bounty
    } else if tier >= Tier::Trusted && facts.bunker.is_some() {
        Context::Intel
    } else if tier >= Tier::Acquainted {
        Context::Prices
    } else {
        Context::Smalltalk
    };

    let archetype = person.temperament.archetype;
    let voice = person.temperament.voice as usize;
    match context {
        Context::Bounty => {
            let lines: [String; 6] = [
                format!("BOARD SAYS {} ON YOU. NOT MY BUSINESS.", facts.bounty),
                format!("{} CREDITS, THE SHERIFF SAYS. EVERYONE KNOWS.", facts.bounty),
                format!("SHERIFFS BOARD SAYS {} FOR YOU. ID WALK ON.", facts.bounty),
                format!("THEY SAY YOURE WORTH {}. THEY SAY A LOT.", facts.bounty),
                format!("{} ON YOUR HEAD AND YOU WALK AROUND SMILING.", facts.bounty),
                format!("PLEASE - IM NOT LOOKING. NOBODY SAW {} OF ANYTHING.", facts.bounty),
            ];
            match archetype {
                Archetype::Steady => lines[0].clone(),
                Archetype::Chatty => lines[1].clone(),
                Archetype::Gruff => lines[2].clone(),
                Archetype::Anxious => lines[5].clone(),
                Archetype::Proud => lines[4].clone(),
                Archetype::Craven => lines[3].clone(),
            }
        }
        Context::Intel => {
            let bearing = facts.bunker.clone().unwrap_or_default();
            [
                format!("MY UNCLE KEPT A SHELTER IN THE HILLS. {bearing}, IF IT STANDS."),
                format!("OLD WORKS OUT {bearing}. SEALED, BUT YOU HAVE A DRILL."),
            ][voice % 2]
                .clone()
        }
        Context::Prices => {
            let price = facts.ore_price;
            let town = &facts.town;
            match archetype {
                Archetype::Chatty => format!(
                    "ORES AT {price} A LOAD - WAS HALF THAT BEFORE THE {town} RUN STOPPED."
                ),
                Archetype::Gruff => format!("ORE PAYS {price}. DIG OR DONT."),
                Archetype::Anxious => {
                    format!("{price} FOR ORE CANT LAST. IT NEVER LASTS.")
                }
                Archetype::Proud => {
                    format!("I REMEMBER WHEN {town} SET THE PRICE. NOW LOOK - {price}.")
                }
                _ => format!("ORES {price} AT THE COUNTER TODAY."),
            }
        }
        Context::Smalltalk => {
            if facts.fleet_dry {
                return match archetype {
                    Archetype::Gruff => "YOUR MACHINES ARE STANDING IDLE. FUEL COSTS.".into(),
                    _ => "QUIET OUT THERE. YOUR CREWS STOPPED, I NOTICE.".into(),
                };
            }
            let pool: &[&str] = match archetype {
                Archetype::Steady => &["FINE MORNING FOR IT.", "THE MAST NEVER STOPS HUMMING."],
                Archetype::Chatty => &[
                    "YOU DIG, I TALK. EVERYONE HAS A TRADE.",
                    "STAY FOR A STORY SOMETIME.",
                ],
                Archetype::Gruff => &["MM.", "YOURE STANDING IN MY LIGHT."],
                Archetype::Anxious => &[
                    "HEARD THE ROOST GO UP TWICE LAST NIGHT. TWICE.",
                    "YOU WALK QUIET. WHY DO YOU WALK QUIET?",
                ],
                Archetype::Proud => &[
                    "MY FAMILY RAISED HALF THIS TOWN.",
                    "YOU WORK HARD. NOT AS HARD AS I DID.",
                ],
                Archetype::Craven => &[
                    "WHATEVER YOU WANT - ITS YOURS, NO TROUBLE.",
                    "I KEEP OUT OF THINGS. ITS SERVED ME.",
                ],
            };
            pool[voice % pool.len()].to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disposition::Tier;

    fn far_site() -> TownSite {
        // Any derived town: the hometown is authored and tested separately.
        vx_world::town::towns_near(2024, (2000, 300), 2000, &|_, _| 90)
            .into_iter()
            .find(|site| !site.is_home())
            .expect("a derived town near the fixture point")
    }

    #[test]
    fn a_person_is_the_same_person_every_time() {
        let site = far_site();
        for index in 0..PEOPLE {
            assert_eq!(person(&site, index), person(&site, index));
        }
        // And the roster varies within a town: three identical neighbours
        // would mean the salt is not reaching the hash.
        let names: std::collections::BTreeSet<String> =
            roster(&site).into_iter().map(|person| person.name).collect();
        assert_eq!(names.len(), PEOPLE, "the town is one person three times");
    }

    #[test]
    fn the_hometown_trio_are_authored() {
        let home = vx_world::town::home_site();
        let people = roster(&home);
        assert_eq!(people[0].name, "THE MAYOR");
        assert_eq!(people[1].name, "THE SHERIFF");
        assert_eq!(people[2].name, "OLD PRAT");
        assert_eq!(people[0].temperament.archetype, Archetype::Proud);
    }

    #[test]
    fn nobody_hates_what_they_love() {
        // Sample widely: this is exactly the kind of rule a rare hash roll
        // breaks silently.
        let ground = |_: i32, _: i32| 90;
        for site in vx_world::town::towns_near(2024, (0, 0), 8_000, &ground) {
            for someone in roster(&site) {
                assert!(
                    !someone.loved.contains(&someone.hated),
                    "{} both loves and hates {}",
                    someone.name,
                    someone.hated
                );
                assert!(someone.birthday < YEAR_DAYS);
            }
        }
    }

    #[test]
    fn preferences_lean_toward_the_trade() {
        // A statistical claim, not a per-person one: across many towns, the
        // people of the mines love ore more often than the people of the
        // depots do. This is the note's "a smith loves fine ore" made
        // checkable.
        let ground = |_: i32, _: i32| 90;
        let mut mine_ore = 0;
        let mut depot_ore = 0;
        for site in vx_world::town::towns_near(2024, (0, 0), 12_000, &ground) {
            for someone in roster(&site) {
                if site.is_home() {
                    continue;
                }
                let loves_ore = someone.loved.contains(&"engine:copper_ore");
                match site.speciality {
                    vx_world::Speciality::Mine if loves_ore => mine_ore += 1,
                    vx_world::Speciality::Depot if loves_ore => depot_ore += 1,
                    _ => {}
                }
            }
        }
        assert!(
            mine_ore > depot_ore,
            "miners ({mine_ore}) do not out-love ore against clerks ({depot_ore})"
        );
    }

    #[test]
    fn every_line_is_drawable_whatever_is_true() {
        let site = far_site();
        let facts_variants = [
            Facts::default(),
            Facts {
                town: "REDREACH".into(),
                ore_price: 14,
                bounty: 0,
                fleet_dry: true,
                bunker: None,
            },
            Facts {
                town: "STONEHAVEN".into(),
                ore_price: 9,
                bounty: 560,
                fleet_dry: false,
                bunker: Some("NW 412M".into()),
            },
            Facts {
                town: "COLDFORK".into(),
                ore_price: 22,
                bounty: 0,
                fleet_dry: false,
                bunker: Some("S 96M".into()),
            },
        ];
        for index in 0..PEOPLE {
            let someone = person(&site, index);
            for tier in [Tier::Stranger, Tier::Acquainted, Tier::Trusted, Tier::Close] {
                for facts in &facts_variants {
                    let line = line_for(&someone, tier, facts);
                    assert!(!line.is_empty());
                    for character in line.chars() {
                        assert!(
                            vx_render::font::knows(character),
                            "undrawable {character:?} in {line:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn depth_is_earned() {
        // A stranger hears pleasantries; a trusted friend hears where the
        // shelter is. The ladder of contexts is the tier ladder, asserted.
        let site = far_site();
        let someone = person(&site, 0);
        let facts = Facts {
            town: "REDREACH".into(),
            ore_price: 14,
            bounty: 0,
            fleet_dry: false,
            bunker: Some("NE 200M".into()),
        };
        let stranger = line_for(&someone, Tier::Stranger, &facts);
        let trusted = line_for(&someone, Tier::Trusted, &facts);
        assert!(!stranger.contains("200M"), "a stranger gave up the shelter");
        assert!(trusted.contains("200M"), "a trusted friend kept the secret");
    }
}
