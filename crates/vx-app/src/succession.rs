//! What grows back, and how long it takes.
//!
//! The forest note assumed this existed. It did not: until stage 36 nothing
//! could disturb a stand, so a ledger would have had nothing to hold. Felling
//! and fire changed that, and this is the other half of the cycle — the part
//! that makes a burn a *disturbance* rather than a scar.
//!
//! **Only disturbed stands are stored.** Untouched forest stays exactly what
//! it has been since stage 35: a pure function of the seed, derived on
//! demand, costing nothing. What is kept is a sparse table of the lattice
//! cells something happened to, keyed the way [`vx_world::flora`] keys its
//! own lattice — the same trick the micro-mask uses to store wounds without
//! storing the blocks that have none.
//!
//! **The four stages are the note's, and so is their ordering.** Meadow,
//! pioneer thicket, mixed, old growth. Real chronosequences run eighty to
//! four hundred years and the note is explicit about what to do with that:
//! compress the absolute durations by one to two orders of magnitude and keep
//! the *sequence* and the *relative* rates honest. So a black-spruce bog comes
//! back fastest — it reseeds itself from an aerial seed bank the fire opened —
//! a hardwood cove takes its time through pioneers to climax, and subalpine
//! conifer is the slowest thing on the map, because at that altitude it is.
//!
//! **And since stage 41 it runs on the growing season.** The clock below is
//! not the calendar: it is [`season::grown_between`], the integral of how
//! fast anything green is growing right now. Away in spring, flat out through
//! summer, tapering through autumn and **stopped** over winter. A stand burnt
//! in October sits black until the spring, and a year still takes a year —
//! the season redistributes the growing, it does not slow it down.
//!
//! Note what that is *not*: it is not a branch that skips [`Ledger::advance`]
//! when it is cold. [`Ledger::due`] has to stay a pure function of
//! `(disturbed_at, tick, species)` or a reload would land on a different
//! stage than the session that saved it, and "how often did we happen to call
//! advance while it was warm" is not a function of anything.
//!
//! **Regrowth is an edit, not a generator change.** Worldgen stays pure; the
//! ledger stamps each stage into the world as blocks, exactly as the wells
//! and the fluid do. That keeps chunk generation free of mutable state and
//! keeps this module's whole effect inspectable as ordinary ground.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use vx_core::{BlockId, BlockPos};
use vx_world::flora::{self, Species, Tree, TreePart};
use vx_world::season;
use vx_world::town::TownSite;
use vx_world::World;

const MAGIC: &[u8; 4] = b"VXSC";
const VERSION: u32 = 1;

/// The base step between one stage and the next, in ticks: one day of the
/// game's own clock — of *growing* days, which in a winter is no days at all.
pub const STEP_TICKS: u64 = 64 * crate::clock::DAY_SECONDS as u64;

/// How many stages there are before a stand is simply forest again.
pub const STAGES: u8 = 4;

/// What a stand looks like at each stage, for a line the player can read.
pub const LABELS: [&str; STAGES as usize] = ["MEADOW", "THICKET", "MIXED", "OLD GROWTH"];

/// How long each forest takes, as a multiple of the base step.
///
/// The note's relative rates: the bog resets hardest and returns fastest off
/// its own seed bank, the cove runs pioneers before climax, and the high
/// country takes the longest because everything up there does.
pub fn pace(species: Species) -> u64 {
    match species {
        Species::BogSpruce => 1,
        Species::Hardwood | Species::Giant | Species::Ancient => 2,
        Species::Spruce | Species::Krummholz => 3,
    }
}

/// One stand that something happened to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stand {
    /// When it was cleared.
    pub disturbed_at: u64,
    /// How many stages have been stamped into the world so far, `0..=STAGES`.
    /// Zero means the ground is as whatever cleared it left it.
    pub stamped: u8,
}

/// Every stand that is not simply what the seed says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    /// Sorted, so a save file and a sweep both have one canonical order.
    stands: BTreeMap<(i32, i32), Stand>,
}

impl Ledger {
    /// The lattice cell a block stands in — the same cell [`vx_world::flora`]
    /// grows one tree in.
    pub fn cell_of(at: BlockPos) -> (i32, i32) {
        (
            at.x.div_euclid(flora::TREE_CELL),
            at.z.div_euclid(flora::TREE_CELL),
        )
    }

    pub fn len(&self) -> usize {
        self.stands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stands.is_empty()
    }

    pub fn stand(&self, cell: (i32, i32)) -> Option<Stand> {
        self.stands.get(&cell).copied()
    }

    /// Something took this stand down. The clock starts now.
    ///
    /// Disturbing a stand that is already coming back restarts it: a fire
    /// through a thicket is a fire through a thicket, and the ground does not
    /// get credit for the years it had banked.
    pub fn disturb(&mut self, at: BlockPos, tick: u64) {
        self.stands.insert(
            Self::cell_of(at),
            Stand {
                disturbed_at: tick,
                stamped: 0,
            },
        );
    }

    /// Which stage a stand should be showing by now: `0` meadow through
    /// `STAGES - 1` old growth. A stand nobody disturbed is old growth, which
    /// is what the seed said in the first place.
    pub fn due(&self, cell: (i32, i32), tick: u64, species: Species) -> u8 {
        let Some(stand) = self.stands.get(&cell) else {
            return STAGES - 1;
        };
        let step = STEP_TICKS * pace(species);
        // Growing ticks, not calendar ticks. The two are the same over a
        // whole year and nothing like each other inside one.
        let age = season::grown_between(stand.disturbed_at, tick);
        ((age / step.max(1)) as u8).min(STAGES - 1)
    }

    /// Move every stand on that is due, stamping the new stage into the
    /// world.
    ///
    /// Returns the stands that changed, so the caller can say so.
    pub fn advance(
        &mut self,
        world: &mut World,
        tick: u64,
        sites: &[TownSite],
    ) -> Vec<((i32, i32), u8)> {
        let cells: Vec<(i32, i32)> = self.stands.keys().copied().collect();
        let mut moved = Vec::new();
        for cell in cells {
            let Some(tree) = tree_of(world, cell, sites) else {
                // Nothing grows here at all — bare rock, a town's plot, the
                // beach. The stand has nothing to come back as.
                self.stands.remove(&cell);
                continue;
            };
            let Some(stand) = self.stands.get(&cell).copied() else {
                continue;
            };
            let due = self.due(cell, tick, tree.species);
            if due < stand.stamped {
                continue;
            }
            stamp(world, &tree, due);
            if due + 1 >= STAGES {
                // Grown. It is what the seed says again, so the ledger can
                // forget it — which is what keeps the table sparse over a
                // long game.
                self.stands.remove(&cell);
            } else if let Some(entry) = self.stands.get_mut(&cell) {
                entry.stamped = due + 1;
            }
            moved.push((cell, due));
        }
        moved
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "stands.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.stands.len() as u32).to_le_bytes())?;
        for ((x, z), stand) in &self.stands {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&stand.disturbed_at.to_le_bytes())?;
            file.write_all(&[stand.stamped])?;
        }
        file.commit()
    }

    /// A missing or unreadable file is an untouched country, not an error.
    pub fn load(&mut self, directory: &Path) {
        let Ok(mut file) = std::fs::File::open(directory.join("stands.dat")) else {
            return;
        };
        let mut magic = [0u8; 4];
        let mut word = [0u8; 4];
        if file.read_exact(&mut magic).is_err() || &magic != MAGIC {
            return;
        }
        if file.read_exact(&mut word).is_err() || u32::from_le_bytes(word) != VERSION {
            return;
        }
        if file.read_exact(&mut word).is_err() {
            return;
        }
        let count = u32::from_le_bytes(word);
        self.stands.clear();
        for _ in 0..count {
            let mut x = [0u8; 4];
            let mut z = [0u8; 4];
            let mut when = [0u8; 8];
            let mut stage = [0u8; 1];
            if file.read_exact(&mut x).is_err()
                || file.read_exact(&mut z).is_err()
                || file.read_exact(&mut when).is_err()
                || file.read_exact(&mut stage).is_err()
            {
                return;
            }
            self.stands.insert(
                (i32::from_le_bytes(x), i32::from_le_bytes(z)),
                Stand {
                    disturbed_at: u64::from_le_bytes(when),
                    stamped: stage[0].min(STAGES),
                },
            );
        }
    }
}

/// The tree worldgen says belongs in this cell.
fn tree_of(world: &World, cell: (i32, i32), sites: &[TownSite]) -> Option<Tree> {
    let generator = world.generator();
    let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, sites);
    let natural_at = |x: i32, z: i32| generator.natural_height_at(x, z);
    let min = (cell.0 * flora::TREE_CELL, cell.1 * flora::TREE_CELL);
    let max = (min.0 + flora::TREE_CELL - 1, min.1 + flora::TREE_CELL - 1);
    flora::trees_overlapping(world.seed(), min, max, &height_at, &natural_at, sites)
        .into_iter()
        .find(|tree| Ledger::cell_of(tree.base) == cell)
}

/// Put a stage's worth of vegetation on the ground.
///
/// Each stage is the *same tree* the seed describes, at a fraction of its
/// height — so a stand comes back as itself rather than as a generic shrub,
/// and the last stage is bit-for-bit what worldgen would have grown.
fn stamp(world: &mut World, tree: &Tree, stage: u8) {
    let air = BlockId::AIR;
    let reach = flora::CANOPY_REACH;
    // Clear whatever is standing there now, up to the full crown.
    for y in tree.base.y + 1..=tree.base.y + tree.height + 4 {
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                let at = BlockPos::new(tree.base.x + dx, y, tree.base.z + dz);
                let name = world.registry().get_or_air(world.block(at)).name.clone();
                if crate::fire::fuel(&name).is_some() || name == "engine:ember" {
                    world.set_block(at, air);
                }
            }
        }
    }

    // The ash goes back to soil at the first green.
    if let (Some(grass), Some(ash)) = (
        world.registry().id_of("engine:grass"),
        world.registry().id_of("engine:ash"),
    ) {
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                let at = BlockPos::new(tree.base.x + dx, tree.base.y, tree.base.z + dz);
                if world.block(at) == ash {
                    world.set_block(at, grass);
                }
            }
        }
    }

    let height = match stage {
        // Meadow: nothing standing but grass.
        0 => 0,
        // A thicket of saplings, crowded and low.
        1 => 2,
        // Half a forest.
        2 => (tree.height / 2).max(3),
        // What the seed said all along.
        _ => tree.height,
    };

    if height == 0 {
        if let Some(tuft) = world.registry().id_of("engine:tall_grass") {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    if (dx + dz).rem_euclid(2) != 0 {
                        continue;
                    }
                    let at = BlockPos::new(tree.base.x + dx, tree.base.y + 1, tree.base.z + dz);
                    if world.block(at).is_air() {
                        world.set_block(at, tuft);
                    }
                }
            }
        }
        return;
    }

    let young = Tree {
        base: tree.base,
        height,
        species: tree.species,
    };
    let blocks = world.generator().blocks();
    for y in young.base.y + 1..=young.base.y + height + 4 {
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                let (x, z) = (young.base.x + dx, young.base.z + dz);
                let Some(part) = flora::tree_part_at(&young, x, y, z) else {
                    continue;
                };
                let block = match (part, young.species) {
                    (TreePart::Trunk, Species::Spruce) => blocks.spruce_log,
                    (TreePart::Trunk, Species::BogSpruce) => blocks.bog_log,
                    (TreePart::Trunk, Species::Ancient) => blocks.ancient_log,
                    (TreePart::Trunk, _) => blocks.log,
                    (TreePart::Leaves, Species::BogSpruce) => blocks.bog_needles,
                    (TreePart::Leaves, species) if species.conifer() => blocks.needles,
                    (TreePart::Leaves, _) => blocks.leaves,
                };
                let at = BlockPos::new(x, y, z);
                if world.block(at).is_air() {
                    world.set_block(at, block);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;

    const WOODS: (i32, i32) = (96, 96);

    fn woods() -> (World, Vec<TownSite>) {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(WOODS.0 / 16, WOODS.1 / 16), 3);
        let sites = world
            .towns_overlapping((WOODS.0 - 80, WOODS.1 - 80), (WOODS.0 + 80, WOODS.1 + 80));
        (world, sites)
    }

    fn a_tree(world: &World, sites: &[TownSite]) -> Tree {
        let generator = world.generator();
        let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, sites);
        let natural_at = |x: i32, z: i32| generator.natural_height_at(x, z);
        flora::trees_overlapping(
            world.seed(),
            (WOODS.0 - 24, WOODS.1 - 24),
            (WOODS.0 + 24, WOODS.1 + 24),
            &height_at,
            &natural_at,
            sites,
        )
        .into_iter()
        .find(|tree| {
            tree.height >= 5
                && (tree.base.x - WOODS.0).abs() <= 24
                && (tree.base.z - WOODS.1).abs() <= 24
                && world.block(BlockPos::new(tree.base.x, tree.base.y + 1, tree.base.z))
                    != BlockId::AIR
        })
        .expect("no standing tree in the loaded square")
    }

    /// How much of a tree is standing in its cell.
    fn standing(world: &World, tree: &Tree) -> i32 {
        let mut count = 0;
        for y in tree.base.y + 1..=tree.base.y + tree.height + 4 {
            for dx in -flora::CANOPY_REACH..=flora::CANOPY_REACH {
                for dz in -flora::CANOPY_REACH..=flora::CANOPY_REACH {
                    let at = BlockPos::new(tree.base.x + dx, y, tree.base.z + dz);
                    let name = world.registry().get_or_air(world.block(at)).name.clone();
                    if crate::fire::fuel(&name).is_some() && name != "engine:tall_grass" {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    #[test]
    fn a_disturbed_stand_comes_back_through_every_stage() {
        let (mut world, sites) = woods();
        let tree = a_tree(&world, &sites);
        let grown = standing(&world, &tree);
        assert!(grown > 10, "the tree was not there to begin with");

        // Something cleared it — a fire, a saw, it does not matter which.
        for y in tree.base.y + 1..=tree.base.y + tree.height + 4 {
            for dx in -flora::CANOPY_REACH..=flora::CANOPY_REACH {
                for dz in -flora::CANOPY_REACH..=flora::CANOPY_REACH {
                    let at = BlockPos::new(tree.base.x + dx, y, tree.base.z + dz);
                    let name = world.registry().get_or_air(world.block(at)).name.clone();
                    if crate::fire::fuel(&name).is_some() {
                        world.set_block(at, BlockId::AIR);
                    }
                }
            }
        }
        assert_eq!(standing(&world, &tree), 0, "the clearing left wood standing");

        let mut ledger = Ledger::default();
        ledger.disturb(tree.base, 0);
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger.due(Ledger::cell_of(tree.base), 0, tree.species), 0);

        // Walk the *growing* clock, not the calendar: since stage 41 a stage
        // costs a stand so many days of growing weather, and how many days of
        // January that is depends on when it was cleared.
        let step = STEP_TICKS * pace(tree.species);
        let mut sizes = Vec::new();
        for stage in 0..STAGES {
            let tick = season::when_grown(0, step * stage as u64);
            ledger.advance(&mut world, tick, &sites);
            sizes.push(standing(&world, &tree));
        }

        // It grew, monotonically, and ended as the tree the seed describes.
        for pair in sizes.windows(2) {
            assert!(pair[1] >= pair[0], "the stand went backwards: {sizes:?}");
        }
        assert!(sizes[0] < sizes[2], "nothing grew at all: {sizes:?}");
        assert_eq!(sizes[0], 0, "the meadow had trees in it: {sizes:?}");
        assert_eq!(
            *sizes.last().unwrap(),
            grown,
            "the grown stand is not the stand the seed describes"
        );
        // And a stand that is forest again is not worth remembering.
        assert!(ledger.is_empty(), "the ledger kept a finished stand");
    }

    #[test]
    fn the_bog_beats_the_cove_and_the_cove_beats_the_high_country() {
        // The note's ordering, and the only thing about the durations that
        // has to be true.
        assert!(pace(Species::BogSpruce) < pace(Species::Hardwood));
        assert!(pace(Species::Hardwood) < pace(Species::Spruce));

        // Same disturbance, same tick, different forests, different stages.
        let mut ledger = Ledger::default();
        let at = BlockPos::new(0, 80, 0);
        ledger.disturb(at, 0);
        let cell = Ledger::cell_of(at);
        let tick = season::when_grown(0, STEP_TICKS * 2);
        assert!(
            ledger.due(cell, tick, Species::BogSpruce) > ledger.due(cell, tick, Species::Spruce),
            "the high country came back as fast as the bog"
        );
    }

    #[test]
    fn disturbing_a_stand_twice_starts_the_clock_over() {
        let mut ledger = Ledger::default();
        let at = BlockPos::new(40, 80, -16);
        let cell = Ledger::cell_of(at);
        ledger.disturb(at, 0);
        let step = STEP_TICKS * pace(Species::Hardwood);
        // Two stages' worth of *growing*, which is some number of calendar
        // days depending on what the weather was doing.
        let two = season::when_grown(0, step * 2);
        assert_eq!(ledger.due(cell, two, Species::Hardwood), 2);
        // Burned through again: it does not get the years it had banked.
        ledger.disturb(at, two);
        assert_eq!(ledger.due(cell, two, Species::Hardwood), 0);
        assert_eq!(
            ledger.due(cell, season::when_grown(two, step), Species::Hardwood),
            1
        );
        // And a cell nobody ever touched is simply forest.
        assert_eq!(
            ledger.due((999, 999), 0, Species::Hardwood),
            STAGES - 1,
            "an untouched stand is not old growth"
        );
    }

    #[test]
    fn the_ledger_survives_a_save_and_an_empty_one_loads_clean() {
        let directory =
            std::env::temp_dir().join(format!("vx-stands-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut ledger = Ledger::default();
        ledger.disturb(BlockPos::new(-412, 97, 883), 12_345);
        ledger.disturb(BlockPos::new(40, 80, -16), 99);
        ledger.save(&directory).unwrap();

        let mut loaded = Ledger::default();
        loaded.load(&directory);
        assert_eq!(loaded, ledger, "the ledger did not survive the wire");

        // A world nobody has burned yet reads as an untouched country.
        let empty = std::env::temp_dir().join(format!("vx-stands-none-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let mut fresh = Ledger::default();
        fresh.load(&empty);
        assert!(fresh.is_empty());
    }

    #[test]
    fn a_stand_with_nowhere_to_grow_is_forgotten() {
        // A cell out at sea grows nothing, and the ledger should not hold a
        // row for it for ever.
        let (mut world, sites) = woods();
        let mut ledger = Ledger::default();
        ledger.disturb(BlockPos::new(WOODS.0 + 4_000, 60, WOODS.1 + 4_000), 0);
        ledger.advance(&mut world, STEP_TICKS * 8, &sites);
        assert!(ledger.is_empty(), "a stand that cannot grow was kept");
    }

    /// There is one calendar, not two. `vx-world` writes the day length and
    /// the year length where the weather can see them; the app's own clock
    /// and roster write the same numbers for the schedule and for birthdays.
    /// If either side ever drifts, a stand's growing season would stop
    /// lining up with the month the player is looking at.
    #[test]
    fn the_world_and_the_app_keep_one_calendar() {
        assert_eq!(
            season::DAY_TICKS, STEP_TICKS,
            "a day of growing is not a day"
        );
        assert_eq!(
            season::DAY_TICKS,
            64 * crate::clock::DAY_SECONDS as u64,
            "the world's day is not the clock's day"
        );
        assert_eq!(
            season::YEAR_DAYS,
            crate::people::YEAR_DAYS as u64,
            "the seasons and the birthdays are counting different years"
        );
        // And one season is one term of office, which is why an election and
        // a season turn together.
        assert_eq!(
            season::SEASON_DAYS,
            crate::ballot::TERM_DAYS as u64,
            "a term is no longer a season"
        );
    }

    /// A stand burnt in the autumn sits black until the spring — the thing a
    /// year is *for*. The same burn set in the spring is a thicket inside the
    /// week.
    #[test]
    fn a_stand_burnt_in_the_autumn_waits_out_the_winter() {
        let cell = (12, -7);
        let step = STEP_TICKS * pace(Species::Hardwood);

        // Cleared on the last day before winter.
        let autumn = 3 * season::SEASON_TICKS - season::DAY_TICKS;
        let mut ledger = Ledger::default();
        ledger.disturb(BlockPos::new(cell.0 * flora::TREE_CELL, 80, cell.1 * flora::TREE_CELL), autumn);
        let cell = Ledger::cell_of(BlockPos::new(
            cell.0 * flora::TREE_CELL,
            80,
            cell.1 * flora::TREE_CELL,
        ));
        // A week later — the whole of winter — it is still exactly as the
        // fire left it. Not slower: stopped.
        for day in 1..=7 {
            let then = autumn + season::DAY_TICKS * day;
            assert_eq!(
                season::Season::of(then),
                season::Season::Winter,
                "day {day} was not winter"
            );
            assert_eq!(
                ledger.due(cell, then, Species::Hardwood),
                0,
                "the burn healed on winter day {day}"
            );
        }
        // And then it comes back. By the end of the following spring it has
        // moved on, because the ground has thawed and the clock has restarted.
        let spring_end = autumn + season::DAY_TICKS * 14;
        assert_eq!(season::Season::of(spring_end), season::Season::Spring);
        assert!(
            ledger.due(cell, spring_end, Species::Hardwood) > 0,
            "the burn never came back at all"
        );

        // The same clearing set in the spring is a stage on inside the week,
        // and by high summer it has moved twice.
        let spring = season::SEASON_TICKS - season::DAY_TICKS * 3;
        let mut ledger = Ledger::default();
        ledger.disturb(BlockPos::new(0, 80, 0), spring);
        let home = Ledger::cell_of(BlockPos::new(0, 80, 0));
        assert!(
            ledger.due(home, spring + step, Species::Hardwood) >= 1,
            "a spring clearing did not move in a growing week"
        );

        // Over a whole year the two land in the same place: the season
        // decides *when* it grows, never *whether* it does.
        let mut late = Ledger::default();
        late.disturb(BlockPos::new(0, 80, 0), autumn);
        let mut early = Ledger::default();
        early.disturb(BlockPos::new(0, 80, 0), spring);
        assert_eq!(
            late.due(home, autumn + season::YEAR_TICKS, Species::Hardwood),
            early.due(home, spring + season::YEAR_TICKS, Species::Hardwood),
            "a year of growing depends on the month it started in"
        );
    }
}
