//! The fabricator: raw stock in, anything out.
//!
//! # Why a printer and not a crafting table
//!
//! A grid of shaped recipes is a memory game about where to put the sticks.
//! This game already has a better vocabulary for the same idea: everything it
//! owns is a **name-keyed row** — blocks, skills, upgrade lines, machines,
//! goods. So the fabricator takes named goods out of a pile and puts named
//! things back, and "it can make anything" means precisely "adding a thing is
//! adding a row". That is a promise the codebase can actually keep.
//!
//! # The pile is the inventory
//!
//! There is no player-carried inventory and this round does not invent one.
//! The printer reads and writes the fleet's base stockpile — the pile the
//! flier ferries into, the pile the shop sells out of, the pile the drill now
//! fills. One pile, three doors, no transfer minigame.
//!
//! # What an upgrade may be made of
//!
//! This round the fabricator learned to print *upgrades* — the same lines
//! the counter sells for credits, bought instead with ore and time. That
//! turned up a rule worth writing on the wall: **the inputs of a recipe are
//! oracle state, the refusal to start it is not.** Replay re-runs
//! `Command::Print` by taking `recipe.inputs` off the pile, so a cost that
//! varied with anything live — a wallet level, the hour, the weather — would
//! have the two sides take different amounts and the pile would drift.
//! [`refuse`] is only ever asked live, so *that* may look at anything it
//! likes. So the parts cost a flat price and the gate stiffens instead: each
//! mark on a line demands more Fabrication than the last.
//!
//! # Materials and time, never credits
//!
//! Buying a drone with credits and printing one out of copper are two
//! different routes to the same machine, and that is the point: the counter
//! is for people with money, the printer is for people with a mine. Nothing
//! here charges credits. Fuel plugs into this later — the fuel loop is a
//! stage away — and when it does, it costs the printer time rather than
//! changing what a recipe means.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::Stockpile;
use vx_core::BlockPos;

use crate::skills;

const MAGIC: &[u8; 4] = b"VXPR";
const VERSION: u32 = 1;

/// What a finished print turns into.
///
/// Split by *where the thing lives*, because that is what decides whether a
/// replay can reproduce it: goods land in the pile, which a replay carries;
/// everything else is live-side state, which it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// A named good, straight back onto the pile.
    Good { name: &'static str, count: u64 },
    /// Slugs into the satchel.
    Slugs(u32),
    /// A charged cell: swap it in and the kestrel flies now rather than
    /// after its recharge.
    Cell,
    /// A machine, onto the garage's books — the same place the counter puts
    /// one, because it is the same machine.
    Machine(&'static str),
    /// An intrusion module.
    Module(&'static str),
    /// A piece of the optics kit: a better lamp or a visor.
    Optic(&'static str),
    /// The arcade cartridge: a toy, and nearly the last thing on the ladder.
    Cartridge,
    /// A town charter: the paper that founds a town, filed on open ground.
    Charter,
    /// One mark on an upgrade line — the same line the counter sells, paid
    /// for in materials instead of credits.
    Upgrade(&'static str),
}

/// One row of the catalogue.
#[derive(Debug, Clone, Copy)]
pub struct Recipe {
    /// What the panel calls it.
    pub label: &'static str,
    pub output: Output,
    /// What it eats, by good name.
    pub inputs: &'static [(&'static str, u64)],
    /// Seconds at Fabrication level one.
    pub seconds: f32,
    /// The Fabrication level that may attempt it at all. A hard floor, like
    /// every other gate in this game: below it the panel says so rather than
    /// letting you find out slowly.
    pub floor: u32,
}

/// Everything the fabricator knows how to make, easiest first.
///
/// The ladder is deliberate: ammunition and building stock at the bottom so
/// a printer pays for itself the day you place it, modules in the middle,
/// whole machines at the top. Adding a row is the whole cost of adding a
/// thing, which is what "it can make anything" has to mean to be true.
pub const CATALOGUE: &[Recipe] = &[
    Recipe {
        label: "SLUGS X8",
        output: Output::Slugs(8),
        inputs: &[("engine:copper_bar", 2), ("engine:stone", 8)],
        seconds: 14.0,
        floor: 1,
    },
    Recipe {
        label: "COPPER BAR",
        output: Output::Good {
            name: "engine:copper_bar",
            count: 1,
        },
        // The refinery in your pocket. Logs stand in for fuel until the fuel
        // loop lands and gives the smelt something better to burn.
        inputs: &[("engine:copper_ore", 3), ("engine:log", 1)],
        seconds: 10.0,
        floor: 1,
    },
    Recipe {
        label: "PLANKS X4",
        output: Output::Good {
            name: "engine:plank",
            count: 4,
        },
        inputs: &[("engine:log", 1)],
        seconds: 6.0,
        floor: 1,
    },
    Recipe {
        label: "PLANKS X12",
        output: Output::Good {
            name: "engine:plank",
            count: 12,
        },
        // Prime timber is heartwood off an emergent stem, and it mills like
        // it: three times the planks out of one block, which is the whole
        // reason a giant is worth the walk and the danger.
        inputs: &[("engine:prime_timber", 1)],
        seconds: 9.0,
        floor: 1,
    },
    Recipe {
        label: "PUMP",
        output: Output::Good {
            name: "engine:pump",
            count: 1,
        },
        // A housing, a rotor and a run of pipe. Cheap on purpose: the pump is
        // the answer to a mine you flooded on yourself, and a rescue you
        // cannot afford is not a rescue.
        inputs: &[("engine:copper_bar", 2), ("engine:plank", 2)],
        seconds: 14.0,
        floor: 3,
    },
    Recipe {
        label: "SPARE PARTS X4",
        output: Output::Good {
            name: crate::wear::SPARE_PART,
            count: 4,
        },
        // Low on the ladder on purpose: a fleet that cannot be mended is a
        // fleet that dies of old age, and nobody should meet that wall
        // before they can print their way out of it.
        inputs: &[("engine:copper_bar", 2), ("engine:stone", 6)],
        seconds: 16.0,
        floor: 3,
    },
    Recipe {
        label: "METAL WALL X2",
        output: Output::Good {
            name: "engine:metal_wall",
            count: 2,
        },
        inputs: &[("engine:copper_bar", 1), ("engine:stone", 4)],
        seconds: 12.0,
        floor: 4,
    },
    Recipe {
        label: "HIGH BEAM LAMP",
        output: Output::Optic(crate::optics::HIGH_BEAM),
        inputs: &[("engine:copper_bar", 3), ("engine:copper_ore", 4)],
        seconds: 20.0,
        floor: 6,
    },
    Recipe {
        label: "DRILL HEAD",
        output: Output::Upgrade(crate::wallet::DRILL),
        inputs: &[("engine:copper_bar", 4), ("engine:stone", 12)],
        seconds: 30.0,
        floor: 7,
    },
    Recipe {
        label: "CARGO RACK",
        output: Output::Upgrade(crate::wallet::CARGO),
        inputs: &[("engine:copper_bar", 3), ("engine:plank", 6)],
        seconds: 30.0,
        floor: 7,
    },
    Recipe {
        label: "PACK FRAME",
        output: Output::Upgrade(crate::wallet::PACK),
        inputs: &[("engine:plank", 8), ("engine:copper_bar", 2)],
        seconds: 26.0,
        floor: 7,
    },
    Recipe {
        label: "CHARGED CELL",
        output: Output::Cell,
        inputs: &[("engine:copper_bar", 2), ("engine:copper_ore", 6)],
        seconds: 25.0,
        floor: 8,
    },
    Recipe {
        label: "WELLHEAD",
        output: Output::Good {
            name: "engine:wellhead",
            count: 1,
        },
        // Printed rather than bought, because a well is not a machine you
        // own — it is a machine you *leave somewhere*, and the ones worth
        // leaving are a long way from any shop counter.
        inputs: &[
            ("engine:copper_bar", 10),
            ("engine:stone", 20),
            ("engine:plank", 4),
        ],
        seconds: 45.0,
        floor: 10,
    },
    Recipe {
        label: "NIGHT VISION VISOR",
        output: Output::Optic(crate::optics::NIGHT_VISION),
        inputs: &[("engine:copper_bar", 6), ("engine:copper_ore", 8)],
        seconds: 40.0,
        floor: 12,
    },
    Recipe {
        label: "LEAD LINING",
        output: Output::Upgrade(crate::wallet::SHIELD),
        // Stone and bars: the fiction is lead sheet beaten into the suit,
        // and the game has no lead — what it has is heavy rock and metal,
        // and a lot of both.
        inputs: &[("engine:copper_bar", 6), ("engine:stone", 24)],
        seconds: 38.0,
        floor: 13,
    },
    Recipe {
        label: "LAMP REFLECTOR",
        output: Output::Upgrade(crate::wallet::LAMP),
        inputs: &[("engine:copper_bar", 5), ("engine:copper_ore", 6)],
        seconds: 35.0,
        floor: 14,
    },
    Recipe {
        label: "PRESS ROLLERS",
        output: Output::Upgrade(crate::wallet::PRESS),
        inputs: &[("engine:copper_bar", 8), ("engine:stone", 20)],
        seconds: 50.0,
        floor: 16,
    },
    Recipe {
        label: "THERMAL VISOR",
        output: Output::Optic(crate::optics::THERMAL),
        inputs: &[
            ("engine:copper_bar", 9),
            ("engine:copper_ore", 10),
            ("engine:log", 2),
        ],
        seconds: 55.0,
        floor: 18,
    },
    Recipe {
        label: "LIGHT COIL",
        output: Output::Module(crate::intrusion::LIGHT_COIL),
        inputs: &[("engine:copper_bar", 6), ("engine:copper_ore", 4)],
        seconds: 60.0,
        floor: 20,
    },
    Recipe {
        label: "POCKET ARCADE",
        output: Output::Cartridge,
        // The dearest thing on the ladder that does no work at all, which is
        // exactly the point of it: everything else here is a tool, and this
        // is the one row you print because you want to rather than because
        // you need to.
        inputs: &[
            ("engine:copper_bar", 12),
            ("engine:copper_ore", 6),
            ("engine:plank", 2),
        ],
        seconds: 70.0,
        floor: 22,
    },
    Recipe {
        label: "KESTREL",
        output: Output::Machine(crate::garage::KESTREL),
        inputs: &[
            ("engine:copper_bar", 14),
            ("engine:copper_ore", 8),
            ("engine:plank", 4),
        ],
        seconds: 90.0,
        floor: 25,
    },
    Recipe {
        label: "GROUND DRONE",
        output: Output::Machine(crate::garage::DRONE),
        inputs: &[
            ("engine:copper_bar", 20),
            ("engine:stone", 30),
            ("engine:log", 10),
        ],
        seconds: 150.0,
        floor: 30,
    },
    Recipe {
        label: "TOWN CHARTER",
        output: Output::Charter,
        // The dearest thing on the ladder, and the only row whose product is
        // a place rather than a thing: file it on open ground a cell clear
        // of anybody and there is a town there, with your name in both of
        // its chairs. Dear in bars because a town is dear.
        inputs: &[
            ("engine:copper_bar", 20),
            ("engine:plank", 8),
            ("engine:copper_ore", 8),
        ],
        seconds: 90.0,
        floor: 30,
    },
];

/// Look a recipe up by the index the journal records.
pub fn recipe(index: usize) -> Option<&'static Recipe> {
    CATALOGUE.get(index)
}

/// How much more Fabrication each mark on a line demands than the last.
///
/// This is where the *price* of a repeat upgrade lives, because the price
/// itself cannot rise: inputs are re-run by the replay oracle and must be
/// constant, while this gate is only ever asked live.
pub const FLOOR_STEP: u32 = 5;

/// The Fabrication this recipe demands right now — its printed floor, plus
/// a step for every mark already fitted on the line it upgrades.
pub fn effective_floor(recipe: &Recipe, kit: &crate::wallet::Wallet) -> u32 {
    match recipe.output {
        Output::Upgrade(line) => recipe.floor + kit.upgrade(line) * FLOOR_STEP,
        _ => recipe.floor,
    }
}

/// Why a print cannot start, or `None` if it can.
///
/// Ordered so the message names what the player can most usefully fix: the
/// skill floor first, because no amount of ore fixes that, then the stock.
pub fn refuse(
    recipe: &Recipe,
    pile: Option<&Stockpile>,
    fabrication: u32,
    kit: &crate::wallet::Wallet,
) -> Option<String> {
    if let Output::Upgrade(line) = recipe.output {
        let owned = kit.upgrade(line);
        if owned >= crate::wallet::MAX_UPGRADE {
            return Some(format!("FITTED ALREADY - {owned} OF {}", crate::wallet::MAX_UPGRADE));
        }
    }
    let floor = effective_floor(recipe, kit);
    if fabrication < floor {
        return Some(format!("NEEDS FABRICATION {floor}"));
    }
    let Some(pile) = pile else {
        return Some("NO BASE PILE TO DRAW ON".into());
    };
    for (name, needed) in recipe.inputs {
        let held = pile.count(name);
        if held < *needed {
            return Some(format!(
                "SHORT {} {}",
                needed - held,
                crate::shop::display_name(name)
            ));
        }
    }
    None
}

/// Seconds this print takes at a level. Levelling buys speed, never
/// certainty — the same curve every timed job in this game runs on.
pub fn duration(recipe: &Recipe, fabrication: u32, press_level: u32) -> f32 {
    // Two independent speedups: what you learned, and what you fitted.
    // Both live-only — print timing never reaches the journal, whose
    // `Print` arm moves the pile in one go.
    skills::bypass_seconds(recipe.seconds, fabrication)
        * crate::wallet::press_multiplier(press_level)
}

/// A print under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Job {
    pub recipe: usize,
    pub done: f32,
    pub total: f32,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        if self.total <= 0.0 {
            return 1.0;
        }
        (self.done / self.total).clamp(0.0, 1.0)
    }
}

/// What a tick of printing produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    Working(f32),
    /// Finished: hand this to whatever owns the output.
    Done { output: Output, xp: u64 },
}

/// The fabricator: where it stands, and what it is making.
#[derive(Debug, Default)]
pub struct Printer {
    /// Where it was placed, once it has been. `None` means it is still a
    /// block in the palette rather than a machine in the world.
    pub at: Option<BlockPos>,
    pub job: Option<Job>,
    /// The panel's cursor and last word.
    pub open: bool,
    pub cursor: usize,
    pub feedback: Option<String>,
}

impl Printer {
    pub fn open_at(&mut self, at: BlockPos) {
        self.open = true;
        self.at = Some(at);
        self.feedback = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.feedback = None;
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let last = CATALOGUE.len() as i32 - 1;
        self.cursor = (self.cursor as i32 + delta).clamp(0, last.max(0)) as usize;
    }

    /// Start a print, taking its inputs off the pile *now*.
    ///
    /// Charging up front is the honest order: queue three drones on one bar
    /// and the pile would be lying about what it holds. It also means a
    /// cancelled print costs you the materials, which is what makes starting
    /// one a decision.
    pub fn begin(
        &mut self,
        index: usize,
        pile: &mut Stockpile,
        fabrication: u32,
        kit: &crate::wallet::Wallet,
    ) -> Result<(), String> {
        let Some(recipe) = recipe(index) else {
            return Err("NO SUCH PATTERN".into());
        };
        if self.job.is_some() {
            return Err("ALREADY PRINTING".into());
        }
        if let Some(reason) = refuse(recipe, Some(pile), fabrication, kit) {
            return Err(reason);
        }
        // Exactly `recipe.inputs`, every time, whatever is fitted: this is
        // the arithmetic the journal's replay arm re-runs.
        for (name, needed) in recipe.inputs {
            pile.take(name, *needed);
        }
        self.job = Some(Job {
            recipe: index,
            done: 0.0,
            total: duration(recipe, fabrication, kit.upgrade(crate::wallet::PRESS)),
        });
        Ok(())
    }

    /// Run for `dt` seconds.
    pub fn work(&mut self, dt: f32) -> Option<Progress> {
        let job = self.job.as_mut()?;
        job.done += dt;
        if job.done < job.total {
            return Some(Progress::Working(job.fraction()));
        }
        let index = job.recipe;
        self.job = None;
        let recipe = recipe(index)?;
        Some(Progress::Done {
            output: recipe.output,
            // What it taught you scales with what it cost: the floor is the
            // honest measure of how hard the pattern was.
            xp: 40 + u64::from(recipe.floor) * 30,
        })
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "printer.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        // Where it stands persists; a half-finished print does not. The
        // materials are already spent, so resuming would be the only fair
        // option and it is not worth a format — finishing on load is.
        match self.at {
            Some(at) => {
                file.write_all(&[1u8])?;
                file.write_all(&at.x.to_le_bytes())?;
                file.write_all(&at.y.to_le_bytes())?;
                file.write_all(&at.z.to_le_bytes())?;
            }
            None => file.write_all(&[0u8; 13])?,
        }
        file.commit()
    }

    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("printer.dat");
        match read_printer(&path) {
            Ok(Some(at)) => self.at = at,
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting fresh", path.display());
                self.at = None;
            }
        }
    }
}

fn read_printer(path: &Path) -> std::io::Result<Option<Option<BlockPos>>> {
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
    let mut flag = [0u8; 1];
    file.read_exact(&mut flag)?;
    let mut at = [0u8; 12];
    file.read_exact(&mut at)?;
    let placed = (flag[0] != 0).then(|| {
        let word = |n: usize| i32::from_le_bytes([at[n], at[n + 1], at[n + 2], at[n + 3]]);
        BlockPos::new(word(0), word(4), word(8))
    });
    Ok(Some(placed))
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_optics_climb_the_ladder_in_order() {
        // The catalogue is easiest-first, and the three optic patterns sit at
        // the floors the fiction says: the beam early, the visors on the way
        // to the coil. A resort here renumbers journal recipes — that is a
        // VERSION bump, which is what this test makes loud.
        let labels: Vec<&str> = CATALOGUE.iter().map(|recipe| recipe.label).collect();
        assert_eq!(
            labels,
            vec![
                "SLUGS X8",
                "COPPER BAR",
                "PLANKS X4",
                "PLANKS X12",
                "PUMP",
                "SPARE PARTS X4",
                "METAL WALL X2",
                "HIGH BEAM LAMP",
                "DRILL HEAD",
                "CARGO RACK",
                "PACK FRAME",
                "CHARGED CELL",
                "WELLHEAD",
                "NIGHT VISION VISOR",
                "LEAD LINING",
                "LAMP REFLECTOR",
                "PRESS ROLLERS",
                "THERMAL VISOR",
                "LIGHT COIL",
                "POCKET ARCADE",
                "KESTREL",
                "GROUND DRONE",
                "TOWN CHARTER",
            ]
        );
        let mut floor = 0;
        for recipe in CATALOGUE {
            assert!(recipe.floor >= floor, "{} breaks the ladder", recipe.label);
            floor = recipe.floor;
        }
    }
    use super::*;

    /// A wallet with nothing fitted: the identity every older test assumed.
    fn stock() -> crate::wallet::Wallet {
        crate::wallet::Wallet::new()
    }

    fn stocked() -> Stockpile {
        let mut pile = Stockpile::new();
        pile.add("engine:copper_bar", 40);
        pile.add("engine:copper_ore", 40);
        pile.add("engine:stone", 60);
        pile.add("engine:log", 20);
        pile.add("engine:plank", 8);
        pile
    }

    fn index_of(label: &str) -> usize {
        CATALOGUE
            .iter()
            .position(|recipe| recipe.label == label)
            .expect("no such recipe")
    }

    #[test]
    fn an_upgrade_part_costs_the_same_whatever_is_already_fitted() {
        // The invariant this round turned up, pinned. Replay re-runs
        // `Command::Print` by taking `recipe.inputs` off the pile, so if the
        // price rose with what you own, the live session and its replay
        // would take different amounts and the pile would drift apart.
        // Repeat purchases are gated by Fabrication instead — see below.
        let head = index_of("DRILL HEAD");
        let recipe = recipe(head).unwrap();

        let charge = |kit: &crate::wallet::Wallet| {
            let mut pile = Stockpile::new();
            pile.add("engine:copper_bar", 100);
            pile.add("engine:stone", 100);
            let mut printer = Printer::default();
            printer.begin(head, &mut pile, 99, kit).expect("refused");
            (
                100 - pile.count("engine:copper_bar"),
                100 - pile.count("engine:stone"),
            )
        };

        let bare = charge(&stock());
        let mut fitted = stock();
        fitted.raise(crate::wallet::DRILL);
        fitted.raise(crate::wallet::DRILL);
        assert_eq!(bare, charge(&fitted), "the price moved with the wallet");
        // And it is exactly what the row says, which is what replay takes.
        assert_eq!(bare, (recipe.inputs[0].1, recipe.inputs[1].1));
    }

    #[test]
    fn each_mark_on_a_line_demands_more_fabrication_than_the_last() {
        let head = index_of("DRILL HEAD");
        let recipe = recipe(head).unwrap();
        let mut kit = stock();
        let mut last = 0;
        for mark in 0..crate::wallet::MAX_UPGRADE {
            let floor = effective_floor(recipe, &kit);
            assert!(floor > last, "mark {mark} was no dearer than the last");
            // Exactly at the floor it goes; one short and it refuses.
            let mut plenty = Stockpile::new();
            plenty.add("engine:copper_bar", 100);
            plenty.add("engine:stone", 100);
            assert!(refuse(recipe, Some(&plenty), floor, &kit).is_none());
            assert!(refuse(recipe, Some(&plenty), floor - 1, &kit).is_some());
            last = floor;
            kit.raise(crate::wallet::DRILL);
        }
        // Five of five: no floor in the world buys a sixth.
        let mut plenty = Stockpile::new();
        plenty.add("engine:copper_bar", 100);
        plenty.add("engine:stone", 100);
        assert!(
            refuse(recipe, Some(&plenty), 999, &kit).is_some(),
            "printed a sixth mark on a five-mark line"
        );
    }

    #[test]
    fn every_upgrade_line_the_wallet_knows_can_be_printed_or_bought() {
        // A line with no way to raise it is dead content. The press is the
        // deliberate exception in the other direction: the workshop is the
        // only place that sells it, so it must have a row here.
        let printable: Vec<&str> = CATALOGUE
            .iter()
            .filter_map(|recipe| match recipe.output {
                Output::Upgrade(line) => Some(line),
                _ => None,
            })
            .collect();
        assert!(printable.contains(&crate::wallet::PRESS), "the press is unreachable");
        for line in printable {
            assert!(
                crate::wallet::LINES.contains(&line),
                "{line} is printed but the wallet does not list it"
            );
        }
    }

    #[test]
    fn the_rollers_speed_every_print_and_stack_with_the_skill() {
        let bar = index_of("COPPER BAR");
        let recipe = recipe(bar).unwrap();
        let plain = duration(recipe, 1, 0);
        let rolled = duration(recipe, 1, crate::wallet::MAX_UPGRADE);
        assert!(rolled < plain, "the rollers did nothing");
        // Skill and fitment are independent multipliers, so the best of both
        // beats either alone.
        let both = duration(recipe, 40, crate::wallet::MAX_UPGRADE);
        assert!(both < rolled && both < duration(recipe, 40, 0));
    }

    #[test]
    fn the_catalogue_is_a_ladder_and_every_row_costs_something() {
        let mut floor = 0;
        for recipe in CATALOGUE {
            assert!(!recipe.inputs.is_empty(), "{} is free", recipe.label);
            assert!(recipe.seconds > 0.0, "{} is instant", recipe.label);
            assert!(
                recipe.floor >= floor,
                "{} sits below an easier row",
                recipe.label
            );
            floor = recipe.floor;
            for character in recipe.label.chars() {
                assert!(
                    vx_render::font::knows(character),
                    "undrawable {character:?} in {}",
                    recipe.label
                );
            }
        }
    }

    #[test]
    fn a_print_takes_its_materials_up_front_and_pays_out_at_the_end() {
        let mut pile = stocked();
        let before = pile.count("engine:copper_bar");
        let mut printer = Printer::default();
        let slugs = index_of("SLUGS X8");

        printer.begin(slugs, &mut pile, 1, &stock()).expect("refused a slug run");
        assert!(
            pile.count("engine:copper_bar") < before,
            "the bars were never spent"
        );
        // Halfway: nothing yet.
        let total = duration(&CATALOGUE[slugs], 1, 0);
        assert!(matches!(
            printer.work(total - 0.5),
            Some(Progress::Working(_))
        ));
        match printer.work(1.0) {
            Some(Progress::Done { output, xp }) => {
                assert_eq!(output, Output::Slugs(8));
                assert!(xp > 0);
            }
            other => panic!("the run never finished: {other:?}"),
        }
        assert!(printer.job.is_none(), "a finished job is still queued");
    }

    #[test]
    fn one_bar_cannot_buy_two_drones() {
        // Charging up front is what stops a queue conjuring materials.
        let mut pile = stocked();
        let mut printer = Printer::default();
        let bar = index_of("COPPER BAR");
        printer.begin(bar, &mut pile, 1, &stock()).expect("refused a smelt");
        assert!(printer.begin(bar, &mut pile, 1, &stock()).is_err(), "printed two at once");
    }

    #[test]
    fn the_floor_and_the_stock_are_both_hard_gates() {
        let mut pile = stocked();
        let drone = index_of("GROUND DRONE");
        let recipe = &CATALOGUE[drone];

        // Levelled but empty-handed.
        let empty = Stockpile::new();
        let short = refuse(recipe, Some(&empty), 99, &stock()).expect("printed a drone out of nothing");
        assert!(short.starts_with("SHORT"), "wrong refusal: {short}");

        // Stocked but green.
        let green = refuse(recipe, Some(&pile), 1, &stock()).expect("a novice printed a drone");
        assert!(green.contains("FABRICATION"), "wrong refusal: {green}");

        // No base at all is its own answer.
        assert!(refuse(recipe, None, 99, &stock()).is_some());

        // Both in hand: allowed, and it actually starts.
        assert!(refuse(recipe, Some(&pile), 99, &stock()).is_none());
        let mut printer = Printer::default();
        assert!(printer.begin(drone, &mut pile, 99, &stock()).is_ok());
    }

    #[test]
    fn levelling_buys_speed_and_only_speed() {
        let recipe = &CATALOGUE[index_of("COPPER BAR")];
        let green = duration(recipe, 1, 0);
        let sharp = duration(recipe, 80, 0);
        assert!(sharp < green, "levelling bought nothing");
        assert!(sharp > 0.5, "a print became free");
    }

    #[test]
    fn where_it_stands_survives_the_trip_to_disk() {
        let directory =
            std::env::temp_dir().join(format!("gamingg-printer-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");

        let printer = Printer {
            at: Some(BlockPos::new(-7, 73, 12)),
            ..Printer::default()
        };
        printer.save(&directory).expect("save");

        let mut back = Printer::default();
        back.load(&directory);
        assert_eq!(back.at, Some(BlockPos::new(-7, 73, 12)));

        let _ = std::fs::remove_dir_all(&directory);
    }
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

use vx_render::font::{self, LINE_HEIGHT};

/// Panel size in texture pixels; drawn at [`PRINT_SCALE`].
///
/// The height is *derived* from the catalogue rather than typed: this round
/// added five rows and a hand-tuned number quietly drew the last of them off
/// the bottom of the texture. A panel that sizes itself cannot go stale when
/// the table grows, and a test holds the arithmetic to the drawing.
pub const PRINT_WIDTH: u32 = 250;
pub const PRINT_HEIGHT: u32 = HEADER + CATALOGUE.len() as u32 * LINE_HEIGHT + FOOTER;

/// Header: the title row and the gap under it.
const HEADER: u32 = LINE_HEIGHT + 12;
/// Footer: the cost line, the job bar, the feedback line and the hint.
const FOOTER: u32 = LINE_HEIGHT * 5 + 12;
pub const PRINT_SCALE: f32 = 2.0;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const GOOD: [u8; 4] = [120, 220, 120, 255];
const SHORT: [u8; 4] = [235, 90, 70, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 235];
const BAR_BACK: [u8; 4] = [45, 48, 55, 255];

/// Draw the fabricator's panel: the catalogue, what it costs, and what is
/// on the bed right now.
pub fn render_printer(
    printer: &Printer,
    pile: Option<&Stockpile>,
    fabrication: u32,
    kit: &crate::wallet::Wallet,
) -> Vec<u8> {
    let mut pixels = vec![0u8; (PRINT_WIDTH * PRINT_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    font::draw_text(&mut pixels, PRINT_WIDTH, margin, y, 1, ACCENT, "FABRICATOR");
    let level = format!("FAB {fabrication}");
    font::draw_text(
        &mut pixels,
        PRINT_WIDTH,
        PRINT_WIDTH as i32 - margin - font::text_width(&level, 1) as i32,
        y,
        1,
        DIM,
        &level,
    );
    y += LINE_HEIGHT as i32 + 3;

    for (index, recipe) in CATALOGUE.iter().enumerate() {
        let selected = index == printer.cursor;
        if selected {
            font::draw_text(&mut pixels, PRINT_WIDTH, margin, y, 1, ACCENT, ">");
        }
        // Red when it cannot be made right now, so the shelf reads at a
        // glance rather than one refusal at a time.
        let blocked = refuse(recipe, pile, fabrication, kit).is_some();
        let colour = if blocked {
            SHORT
        } else if selected {
            TEXT
        } else {
            DIM
        };
        font::draw_text(&mut pixels, PRINT_WIDTH, margin + 10, y, 1, colour, recipe.label);
        // An upgrade part says what it is worth: three of five fitted is
        // the whole reason to print a fourth.
        if let Output::Upgrade(line) = recipe.output {
            let marks = format!("{}/{}", kit.upgrade(line), crate::wallet::MAX_UPGRADE);
            font::draw_text(
                &mut pixels,
                PRINT_WIDTH,
                PRINT_WIDTH as i32 - margin - font::text_width(&marks, 1) as i32,
                y,
                1,
                colour,
                &marks,
            );
        }
        y += LINE_HEIGHT as i32;
    }

    y += 3;
    // What the highlighted row eats, spelled out — the number you are
    // actually deciding on.
    if let Some(recipe) = CATALOGUE.get(printer.cursor) {
        let cost: Vec<String> = recipe
            .inputs
            .iter()
            .map(|(name, count)| format!("{count} {}", crate::shop::display_name(name)))
            .collect();
        font::draw_text(
            &mut pixels,
            PRINT_WIDTH,
            margin,
            y,
            1,
            DIM,
            &format!("EATS {}", cost.join(", ")),
        );
        y += LINE_HEIGHT as i32;
    }

    if let Some(job) = &printer.job {
        let label = recipe(job.recipe).map_or("", |recipe| recipe.label);
        font::draw_text(
            &mut pixels,
            PRINT_WIDTH,
            margin,
            y,
            1,
            GOOD,
            &format!("PRINTING {label}"),
        );
        // The bar, because a print is long enough to want watching.
        let left = (margin + 150) as u32;
        let width = PRINT_WIDTH - left - margin as u32;
        let filled = (width as f32 * job.fraction()) as u32;
        for py in y as u32 + 1..y as u32 + 6 {
            for px in left..left + width {
                let at = ((py * PRINT_WIDTH + px) * 4) as usize;
                let texel = if px < left + filled { GOOD } else { BAR_BACK };
                pixels[at..at + 4].copy_from_slice(&texel);
            }
        }
        y += LINE_HEIGHT as i32;
    }

    if let Some(feedback) = &printer.feedback {
        font::draw_text(&mut pixels, PRINT_WIDTH, margin, y, 1, TEXT, feedback);
    }

    font::draw_text(
        &mut pixels,
        PRINT_WIDTH,
        margin,
        PRINT_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
        1,
        DIM,
        "ARROWS PICK. ENTER PRINTS. E LEAVES.",
    );
    pixels
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    /// A wallet with nothing fitted.
    fn stock() -> crate::wallet::Wallet {
        crate::wallet::Wallet::new()
    }

    fn stocked() -> Stockpile {
        let mut pile = Stockpile::new();
        pile.add("engine:copper_bar", 40);
        pile.add("engine:copper_ore", 40);
        pile.add("engine:stone", 60);
        pile.add("engine:log", 20);
        pile.add("engine:plank", 8);
        pile
    }

    #[test]
    fn the_panel_has_room_for_every_row_it_draws() {
        // The guard for the bug this round shipped and caught: five new
        // rows overran a hand-typed panel height. Drawing indexes the pixel
        // buffer directly, so the loudest possible test is simply to draw
        // the busiest panel there is — every row present, a job running, a
        // line of feedback under it — and let a panic be the failure.
        let printer = Printer {
            feedback: Some("FITTED - DRILL NOW 3 OF 5".into()),
            job: Some(Job {
                recipe: 0,
                done: 3.0,
                total: 10.0,
            }),
            cursor: CATALOGUE.len() - 1,
            ..Printer::default()
        };
        let pile = stocked();
        let pixels = render_printer(&printer, Some(&pile), 40, &stock());
        assert_eq!(pixels.len(), (PRINT_WIDTH * PRINT_HEIGHT * 4) as usize);

        // And the height is genuinely derived: adding a row must move it.
        assert!(
            PRINT_HEIGHT > CATALOGUE.len() as u32 * LINE_HEIGHT,
            "the panel is shorter than the rows it lists"
        );
    }

    #[test]
    fn the_panel_is_deterministic_and_reacts_to_the_cursor() {
        let mut printer = Printer::default();
        let pile = {
            let mut pile = Stockpile::new();
            pile.add("engine:log", 4);
            pile
        };
        let first = render_printer(&printer, Some(&pile), 3, &stock());
        assert_eq!(first, render_printer(&printer, Some(&pile), 3, &stock()));
        printer.move_cursor(1);
        assert_ne!(first, render_printer(&printer, Some(&pile), 3, &stock()));
    }

    #[test]
    fn every_word_the_panel_can_say_is_drawable() {
        // The bitmap font has no lower case and a short punctuation set, so
        // a label it cannot draw is a hole in the panel.
        let mut lines = vec![
            "FABRICATOR".to_string(),
            "ARROWS PICK. ENTER PRINTS. E LEAVES.".to_string(),
            "NO BASE PILE TO DRAW ON".to_string(),
            "ALREADY PRINTING".to_string(),
            "NO SUCH PATTERN".to_string(),
        ];
        for recipe in CATALOGUE {
            lines.push(recipe.label.to_string());
            lines.push(format!("PRINTING {}", recipe.label));
            lines.push(format!("NEEDS FABRICATION {}", recipe.floor));
            for (name, count) in recipe.inputs {
                lines.push(format!("EATS {count} {}", crate::shop::display_name(name)));
                lines.push(format!("SHORT {count} {}", crate::shop::display_name(name)));
            }
        }
        for line in lines {
            for character in line.chars() {
                assert!(font::knows(character), "undrawable {character:?} in {line:?}");
            }
        }
    }
}
