//! The wellhead: the first machine whose whole job is patience.
//!
//! # A different kind of machine
//!
//! Every machine before this one is somewhere you *are*. A drone digs where
//! you sent it, the fabricator prints while you watch the bar, the
//! electrolyser wants you standing on a shore. A well is somewhere you were:
//! you carry the head out to a place the ground says something is under,
//! spend the casing, wait out the drilling, and then leave it lifting barrels
//! into your pile while you go and do something else. It is the first thing
//! in this game that *keeps paying* — and, because a reservoir is finite, the
//! first thing that stops.
//!
//! # Why this lives inside the replayed simulation
//!
//! A well puts goods on the base pile, and the pile is what the fleet burns,
//! what the shop sells and what the fabricator eats. Wear taught this lesson
//! in stage 26 and the tank taught it in 20: anything whose arithmetic
//! decides how much ground gets dug has to live inside the call
//! `Command::Advance` re-runs, or a replayed session and its own journal
//! disagree about the world. So [`Wells`] hangs off `Mining`, ticks inside
//! `Mining::advance`, and every number here is a constant rather than a
//! setting.
//!
//! What is *not* in the oracle: the panel, the cursor and the feedback line.
//! Those are live-only, exactly like the electrolyser's, because refusing to
//! spud a hole changes nothing about the world.
//!
//! # The bet
//!
//! [`vx_world::reservoir::reservoir_under`] is pure in the seed, so a dry
//! hole is a *place* rather than a dice roll: the same column is dry in every
//! session of that world, and a player who learns the ground learns something
//! true. The panel says whether the mud log shows a trace before you commit,
//! and nothing else — what fluid, how much, and how deep are all things you
//! find out by drilling, which is what makes drilling worth doing.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::Stockpile;
use vx_core::BlockPos;
use vx_render::font::{self, LINE_HEIGHT};
use vx_world::reservoir::{self, Fluid, Reservoir};

/// What sinking a hole costs, charged up front like the electrolyser's
/// electrodes. Casing and cement: a well that fails still ate both, which is
/// the whole weight behind the word "commitment".
pub const CASING: (&str, u64) = ("engine:copper_bar", 4);
pub const CEMENT: (&str, u64) = ("engine:stone", 24);

/// Journal ticks of drilling per block of depth. At 8 Hz that is about a
/// second and three quarters a metre, so a shallow field is a couple of
/// minutes and a deep one is a job you leave running.
pub const TICKS_PER_BLOCK: u32 = 14;

/// The least drilling any hole takes, however shallow the target.
pub const MIN_DRILL_TICKS: u32 = 240;

/// Ticks between one unit lifted and the next. Four seconds a barrel: slow
/// enough that a field is a supply *line* rather than a windfall, fast
/// enough that coming back to a full pile feels like something happened.
pub const PUMP_PERIOD: u32 = 32;

const MAGIC: &[u8; 4] = b"VXWL";
const VERSION: u32 = 1;

/// Where a hole is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Spudded in and going down.
    Drilling { left: u32 },
    /// On production.
    Pumping,
    /// Either it never found anything, or it found it and lifted the lot.
    /// The same state on purpose: a hole with nothing left in it is a dry
    /// hole, whatever it was yesterday.
    Dry,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Drilling { .. } => "DRILLING",
            Stage::Pumping => "PUMPING",
            Stage::Dry => "DRY HOLE",
        }
    }
}

/// One hole in the ground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Well {
    /// The head on the surface — how the player and the panel find it.
    pub at: BlockPos,
    pub stage: Stage,
    /// What it is into, known only once the string reaches it.
    pub fluid: Option<Fluid>,
    /// Units still in the ground under this head.
    pub remaining: u64,
    /// Units this hole has lifted in its life.
    pub lifted: u64,
    /// How long a hole had to be drilled, kept for the panel's percentage.
    pub total_drill: u32,
    /// Ticks since the last unit came up.
    pumped_for: u32,
}

impl Well {
    /// How far down this hole went, in blocks.
    pub fn depth(&self) -> u32 {
        self.total_drill / TICKS_PER_BLOCK
    }

    /// Fraction of the drilling done, 0 to 1.
    pub fn drilled(&self) -> f32 {
        match self.stage {
            Stage::Drilling { left } => {
                let total = self.total_drill.max(1) as f32;
                ((total - left as f32) / total).clamp(0.0, 1.0)
            }
            _ => 1.0,
        }
    }
}

/// What a run of ticks did to the holes.
///
/// Returned rather than toasted from inside, because the same tick runs
/// under replay where there is nobody to tell.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct WellReport {
    /// Holes that reached their target this run.
    pub struck: Vec<(BlockPos, Fluid)>,
    /// Holes that finished drilling into nothing.
    pub dusters: Vec<BlockPos>,
    /// Holes that lifted their last unit this run.
    pub spent: Vec<BlockPos>,
    /// Units put on the pile.
    pub lifted: u64,
}

impl WellReport {
    pub fn quiet(&self) -> bool {
        self.struck.is_empty()
            && self.dusters.is_empty()
            && self.spent.is_empty()
            && self.lifted == 0
    }
}

/// Every hole this player has sunk.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Wells {
    holes: Vec<Well>,
}

/// Why a hole cannot be spudded here, in the order a person would notice.
pub fn refuse(wells: &Wells, at: BlockPos, pile: Option<&Stockpile>) -> Option<String> {
    if wells.at(at).is_some() {
        return Some("ALREADY SPUDDED".to_string());
    }
    let pile = pile?;
    for (good, need) in [CASING, CEMENT] {
        let held = pile.count(good);
        if held < need {
            let short = need - held;
            let name = match good {
                "engine:copper_bar" => "COPPER BAR",
                "engine:stone" => "STONE",
                other => other,
            };
            return Some(format!("SHORT {short} {name}"));
        }
    }
    None
}

impl Wells {
    pub fn is_empty(&self) -> bool {
        self.holes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.holes.len()
    }

    pub fn all(&self) -> &[Well] {
        &self.holes
    }

    /// The hole under a head, if there is one.
    pub fn at(&self, at: BlockPos) -> Option<&Well> {
        self.holes.iter().find(|hole| hole.at == at)
    }

    /// Forget a hole — what happens when its head is broken off.
    pub fn remove(&mut self, at: BlockPos) -> bool {
        let before = self.holes.len();
        self.holes.retain(|hole| hole.at != at);
        self.holes.len() != before
    }

    /// Sink a hole, charging casing and cement to the pile.
    ///
    /// The world is consulted exactly once, here, and the answer is stored:
    /// what the ground holds cannot change under a running well, and asking
    /// again every tick would make a hole's output depend on when it was
    /// asked rather than on what it hit.
    pub fn spud(
        &mut self,
        at: BlockPos,
        seed: u64,
        pile: &mut Stockpile,
    ) -> Result<(), String> {
        if let Some(reason) = refuse(self, at, Some(pile)) {
            return Err(reason);
        }
        pile.take(CASING.0, CASING.1);
        pile.take(CEMENT.0, CEMENT.1);

        let found = reservoir::reservoir_under(seed, at.x, at.z);
        // A duster still gets drilled, and to a plausible depth: the string
        // goes down until the geologist gives up, not until it finds
        // nothing instantly.
        let target = found.map_or(BlockPos::new(at.x, 24, at.z).y, |body: Reservoir| body.crown());
        let depth = (at.y - target).max(1) as u32;
        let total_drill = (depth * TICKS_PER_BLOCK).max(MIN_DRILL_TICKS);

        self.holes.push(Well {
            at,
            stage: Stage::Drilling { left: total_drill },
            fluid: found.map(|body| body.fluid),
            remaining: found.map_or(0, |body| body.volume()),
            lifted: 0,
            total_drill,
            pumped_for: 0,
        });
        Ok(())
    }

    /// Run every hole for `ticks`, delivering to `pile`.
    ///
    /// Note what is *not* here: fuel, wear, or any dependence on the crew. A
    /// wellhead runs on a slipstream of what it lifts, which is both true of
    /// the real machine and the reason a well is what rescues a fleet that
    /// ran itself dry a long way from water.
    pub fn tick(&mut self, ticks: u32, pile: &mut Stockpile) -> WellReport {
        let mut report = WellReport::default();
        if ticks == 0 || self.holes.is_empty() {
            return report;
        }

        for hole in &mut self.holes {
            for _ in 0..ticks {
                match hole.stage {
                    Stage::Dry => break,
                    Stage::Drilling { left } => {
                        let left = left.saturating_sub(1);
                        if left > 0 {
                            hole.stage = Stage::Drilling { left };
                            continue;
                        }
                        match hole.fluid {
                            Some(fluid) if hole.remaining > 0 => {
                                hole.stage = Stage::Pumping;
                                report.struck.push((hole.at, fluid));
                            }
                            _ => {
                                hole.stage = Stage::Dry;
                                report.dusters.push(hole.at);
                                break;
                            }
                        }
                    }
                    Stage::Pumping => {
                        hole.pumped_for += 1;
                        if hole.pumped_for < PUMP_PERIOD {
                            continue;
                        }
                        hole.pumped_for = 0;
                        let Some(fluid) = hole.fluid else {
                            hole.stage = Stage::Dry;
                            break;
                        };
                        pile.add(fluid.product(), 1);
                        hole.lifted += 1;
                        hole.remaining -= 1;
                        report.lifted += 1;
                        if hole.remaining == 0 {
                            hole.stage = Stage::Dry;
                            report.spent.push(hole.at);
                            break;
                        }
                    }
                }
            }
        }
        report
    }

    /// Every hole still lifting, for the terminal's roster.
    pub fn producing(&self) -> impl Iterator<Item = &Well> {
        self.holes
            .iter()
            .filter(|hole| hole.stage == Stage::Pumping)
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "wells.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.holes.len() as u32).to_le_bytes())?;
        for hole in &self.holes {
            file.write_all(&hole.at.x.to_le_bytes())?;
            file.write_all(&hole.at.y.to_le_bytes())?;
            file.write_all(&hole.at.z.to_le_bytes())?;
            let (tag, left) = match hole.stage {
                Stage::Drilling { left } => (0u8, left),
                Stage::Pumping => (1, 0),
                Stage::Dry => (2, 0),
            };
            file.write_all(&[tag])?;
            file.write_all(&left.to_le_bytes())?;
            let fluid = match hole.fluid {
                None => 0u8,
                Some(Fluid::Oil) => 1,
                Some(Fluid::Gas) => 2,
            };
            file.write_all(&[fluid])?;
            file.write_all(&hole.remaining.to_le_bytes())?;
            file.write_all(&hole.lifted.to_le_bytes())?;
            file.write_all(&hole.total_drill.to_le_bytes())?;
            file.write_all(&hole.pumped_for.to_le_bytes())?;
        }
        file.commit()
    }

    /// Load the holes, tolerating absence and damage.
    ///
    /// Persisted for the tank's reason, word for word: replay re-derives a
    /// well from tick zero, so a session that reloaded with no holes would
    /// put different goods on the pile than its own journal says it did, and
    /// the ground would differ by exactly the digging that difference bought.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("wells.dat");
        match read_wells(&path) {
            Ok(Some(holes)) => self.holes = holes,
            Ok(None) => {}
            Err(error) => log::warn!("unreadable {}: {error}", path.display()),
        }
    }
}

fn read_wells(path: &Path) -> std::io::Result<Option<Vec<Well>>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a wells file"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    file.read_exact(&mut word)?;
    let count = u32::from_le_bytes(word) as usize;

    let mut holes = Vec::with_capacity(count);
    for _ in 0..count {
        let mut int = || -> std::io::Result<i32> {
            let mut bytes = [0u8; 4];
            file.read_exact(&mut bytes)?;
            Ok(i32::from_le_bytes(bytes))
        };
        let at = BlockPos::new(int()?, int()?, int()?);
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte)?;
        let tag = byte[0];
        let mut word = [0u8; 4];
        file.read_exact(&mut word)?;
        let left = u32::from_le_bytes(word);
        file.read_exact(&mut byte)?;
        let fluid = match byte[0] {
            1 => Some(Fluid::Oil),
            2 => Some(Fluid::Gas),
            _ => None,
        };
        let mut long = [0u8; 8];
        file.read_exact(&mut long)?;
        let remaining = u64::from_le_bytes(long);
        file.read_exact(&mut long)?;
        let lifted = u64::from_le_bytes(long);
        file.read_exact(&mut word)?;
        let total_drill = u32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let pumped_for = u32::from_le_bytes(word);

        holes.push(Well {
            at,
            stage: match tag {
                0 => Stage::Drilling { left },
                1 => Stage::Pumping,
                _ => Stage::Dry,
            },
            fluid,
            remaining,
            lifted,
            total_drill,
            pumped_for,
        });
    }
    Ok(Some(holes))
}

/// The live half: which head the player has open, and what it last said.
///
/// Deliberately *not* on `Mining`. Opening a panel, moving off it and being
/// refused change nothing about the world, so none of it belongs in the
/// oracle — the same split the electrolyser makes between where it stands
/// and what its panel is showing.
#[derive(Debug, Default)]
pub struct Panel {
    pub open: bool,
    pub at: Option<BlockPos>,
    pub feedback: Option<String>,
}

impl Panel {
    pub fn open_at(&mut self, at: BlockPos) {
        self.open = true;
        self.at = Some(at);
        self.feedback = None;
    }

    pub fn close(&mut self) {
        self.open = false;
    }
}

// ---------------------------------------------------------------- the panel

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [230, 180, 90, 255];
const SHORT: [u8; 4] = [235, 110, 90, 255];
const GOOD: [u8; 4] = [150, 220, 150, 255];
const BACKGROUND: [u8; 4] = [10, 14, 18, 240];

pub const WELL_WIDTH: u32 = 260;
/// Derived, not typed. The fabricator's panel overran a hand-written height
/// the moment its catalogue grew, and the lesson was cheap exactly once.
pub const WELL_HEIGHT: u32 = 6 + LINE_HEIGHT * 8 + 20;

/// Everything the panel draws, snapshotted.
///
/// A struct rather than a pile of arguments for the reason every panel in
/// this game takes one: the renderer is pure, so the panel can be drawn in a
/// test without a world, a pile or a machine anywhere near it.
#[derive(Debug, Clone, PartialEq)]
pub struct WellContent {
    /// Where the head stands.
    pub at: BlockPos,
    /// Does the mud log show anything under this column? All the ground will
    /// say before somebody spends the casing.
    pub trace: bool,
    /// The hole here, if it has been spudded.
    pub hole: Option<Well>,
    /// Why the spud row is refused, if it is.
    pub refusal: Option<String>,
    /// The line under the rows: the last thing that happened.
    pub feedback: Option<String>,
}

/// Draw the wellhead panel. Pure in its input, like every panel here.
pub fn render_well(content: &WellContent) -> Vec<u8> {
    let mut pixels = vec![0u8; (WELL_WIDTH * WELL_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    font::draw_text(&mut pixels, WELL_WIDTH, margin, y, 1, ACCENT, "WELLHEAD");
    let where_at = format!("{} {}", content.at.x, content.at.z);
    font::draw_text(
        &mut pixels,
        WELL_WIDTH,
        WELL_WIDTH as i32 - margin - font::text_width(&where_at, 1) as i32,
        y,
        1,
        DIM,
        &where_at,
    );
    y += LINE_HEIGHT as i32 + 3;

    let row = |pixels: &mut Vec<u8>, y: &mut i32, label: &str, value: &str, colour: [u8; 4]| {
        font::draw_text(pixels, WELL_WIDTH, margin, *y, 1, DIM, label);
        font::draw_text(pixels, WELL_WIDTH, margin + 96, *y, 1, colour, value);
        *y += LINE_HEIGHT as i32;
    };

    match &content.hole {
        None => {
            row(
                &mut pixels,
                &mut y,
                "MUD LOG",
                if content.trace { "SHOWS TRACE" } else { "NO SHOW" },
                if content.trace { GOOD } else { DIM },
            );
            row(&mut pixels, &mut y, "STATUS", "NOT SPUDDED", TEXT);
            row(
                &mut pixels,
                &mut y,
                "CASING",
                &format!("{} COPPER BAR", CASING.1),
                TEXT,
            );
            row(
                &mut pixels,
                &mut y,
                "CEMENT",
                &format!("{} STONE", CEMENT.1),
                TEXT,
            );
        }
        Some(hole) => {
            let status = match hole.stage {
                Stage::Drilling { .. } => {
                    format!("DRILLING {}%", (hole.drilled() * 100.0).round() as u32)
                }
                other => other.name().to_string(),
            };
            row(
                &mut pixels,
                &mut y,
                "STATUS",
                &status,
                match hole.stage {
                    Stage::Dry => SHORT,
                    Stage::Pumping => GOOD,
                    Stage::Drilling { .. } => TEXT,
                },
            );
            row(
                &mut pixels,
                &mut y,
                "DEPTH",
                &format!("{} BLOCKS", hole.depth()),
                TEXT,
            );
            let found = match (hole.stage, hole.fluid) {
                // What is down there stays unknown until the string is in
                // it: the panel never spoils the drilling.
                (Stage::Drilling { .. }, _) => "---".to_string(),
                (_, Some(fluid)) => fluid.name().to_string(),
                (_, None) => "NOTHING".to_string(),
            };
            row(&mut pixels, &mut y, "FOUND", &found, TEXT);
            row(&mut pixels, &mut y, "LIFTED", &hole.lifted.to_string(), TEXT);
            let left = match hole.stage {
                Stage::Drilling { .. } => "---".to_string(),
                _ => hole.remaining.to_string(),
            };
            row(&mut pixels, &mut y, "IN GROUND", &left, TEXT);
        }
    }

    y += 3;
    let action = match (&content.hole, &content.refusal) {
        (Some(_), _) => "RUNNING - NOTHING TO DO".to_string(),
        (None, Some(reason)) => reason.clone(),
        (None, None) => "ENTER - SPUD IN".to_string(),
    };
    let colour = match (&content.hole, &content.refusal) {
        (None, Some(_)) => SHORT,
        (None, None) => ACCENT,
        _ => DIM,
    };
    font::draw_text(&mut pixels, WELL_WIDTH, margin, y, 1, colour, &action);
    y += LINE_HEIGHT as i32;

    if let Some(line) = &content.feedback {
        font::draw_text(&mut pixels, WELL_WIDTH, margin, y, 1, TEXT, line);
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stocked() -> Stockpile {
        let mut pile = Stockpile::default();
        pile.add(CASING.0, 40);
        pile.add(CEMENT.0, 400);
        pile
    }

    /// A column somewhere over a real field, found the way a player would:
    /// by looking.
    fn over_a_field(seed: u64) -> BlockPos {
        for x in (-2000..2000).step_by(23) {
            for z in (-2000..2000).step_by(23) {
                if reservoir::reservoir_under(seed, x, z).is_some() {
                    return BlockPos::new(x, 90, z);
                }
            }
        }
        panic!("no field within two kilometres of the origin");
    }

    #[test]
    fn a_hole_over_a_field_strikes_and_then_lifts() {
        let seed = 909;
        let at = over_a_field(seed);
        let mut pile = stocked();
        let mut wells = Wells::default();
        wells.spud(at, seed, &mut pile).unwrap();

        // Drilling first, and nothing on the pile while it goes down.
        let hole = *wells.at(at).unwrap();
        assert!(matches!(hole.stage, Stage::Drilling { .. }));
        let report = wells.tick(hole.total_drill - 1, &mut pile);
        assert!(report.quiet(), "a hole paid out before it reached anything");

        // The tick it lands, it says so.
        let report = wells.tick(1, &mut pile);
        assert_eq!(report.struck.len(), 1, "the string reached nothing");
        let fluid = report.struck[0].1;

        // Then it lifts on its own clock, and the pile grows.
        let before = pile.count(fluid.product());
        wells.tick(PUMP_PERIOD * 3, &mut pile);
        assert_eq!(
            pile.count(fluid.product()),
            before + 3,
            "the well did not lift on its period"
        );
    }

    #[test]
    fn a_dry_hole_costs_exactly_as_much_as_a_good_one() {
        // The bet has to be real: casing and cement are spent before anybody
        // knows, and a duster keeps them.
        let seed = 909;
        let mut pile = stocked();
        let mut wells = Wells::default();
        let bars = pile.count(CASING.0);

        // A column with nothing under it.
        let mut dry = None;
        for x in (-800..800).step_by(17) {
            if reservoir::reservoir_under(seed, x, 0).is_none() {
                dry = Some(BlockPos::new(x, 90, 0));
                break;
            }
        }
        let at = dry.expect("nowhere dry in this world, which cannot be");
        wells.spud(at, seed, &mut pile).unwrap();
        assert_eq!(pile.count(CASING.0), bars - CASING.1);

        let total = wells.at(at).unwrap().total_drill;
        let report = wells.tick(total, &mut pile);
        assert_eq!(report.dusters, vec![at]);
        assert_eq!(wells.at(at).unwrap().stage, Stage::Dry);
        assert!(report.lifted == 0);
    }

    #[test]
    fn a_field_runs_out() {
        // The thing that makes a well a supply line rather than a cheat: it
        // ends, and it says when.
        let seed = 909;
        let at = over_a_field(seed);
        let mut pile = stocked();
        let mut wells = Wells::default();
        wells.spud(at, seed, &mut pile).unwrap();

        let hole = *wells.at(at).unwrap();
        let lifetime = hole.total_drill + hole.remaining as u32 * PUMP_PERIOD;
        let fluid = hole.fluid.unwrap();
        let volume = hole.remaining;

        let mut report = WellReport::default();
        let mut left = lifetime;
        while left > 0 {
            let step = left.min(600);
            let round = wells.tick(step, &mut pile);
            report.lifted += round.lifted;
            report.spent.extend(round.spent);
            left -= step;
        }
        assert_eq!(report.lifted, volume, "the field did not give up its whole volume");
        assert_eq!(report.spent, vec![at]);
        assert_eq!(wells.at(at).unwrap().stage, Stage::Dry);
        assert_eq!(pile.count(fluid.product()), volume);

        // And it stays dry: another hour changes nothing.
        let after = wells.tick(30_000, &mut pile);
        assert!(after.quiet());
    }

    #[test]
    fn two_identical_runs_of_the_same_orders_agree() {
        // The oracle's requirement, checked directly: everything here is
        // constant, so two runs of the same ticks put the same goods on the
        // same pile.
        let seed = 4242;
        let at = over_a_field(seed);
        let run = || {
            let mut pile = stocked();
            let mut wells = Wells::default();
            wells.spud(at, seed, &mut pile).unwrap();
            wells.tick(9_000, &mut pile);
            (wells, pile.total())
        };
        let (first, first_total) = run();
        let (second, second_total) = run();
        assert_eq!(first, second);
        assert_eq!(first_total, second_total);
    }

    #[test]
    fn the_same_ticks_in_one_call_or_many_reach_the_same_place() {
        // Frames are not reproducible; tick counts are. A well that cared
        // how the ticks were grouped would diverge on any machine with a
        // different frame rate — the exact bug the journal exists to
        // prevent.
        let seed = 4242;
        let at = over_a_field(seed);
        let mut bulk_pile = stocked();
        let mut bulk = Wells::default();
        bulk.spud(at, seed, &mut bulk_pile).unwrap();
        bulk.tick(4_000, &mut bulk_pile);

        let mut split_pile = stocked();
        let mut split = Wells::default();
        split.spud(at, seed, &mut split_pile).unwrap();
        for _ in 0..400 {
            split.tick(10, &mut split_pile);
        }
        assert_eq!(bulk, split);
        assert_eq!(bulk_pile.total(), split_pile.total());
    }

    #[test]
    fn a_hole_cannot_be_spudded_twice_or_on_an_empty_pile() {
        let seed = 909;
        let at = over_a_field(seed);
        let mut pile = stocked();
        let mut wells = Wells::default();
        wells.spud(at, seed, &mut pile).unwrap();
        assert_eq!(
            wells.spud(at, seed, &mut pile),
            Err("ALREADY SPUDDED".to_string())
        );

        let mut empty = Stockpile::default();
        let elsewhere = BlockPos::new(at.x + 40, at.y, at.z);
        assert!(wells.spud(elsewhere, seed, &mut empty).is_err());
        assert!(wells.at(elsewhere).is_none());
    }

    #[test]
    fn the_holes_survive_a_save() {
        let seed = 909;
        let at = over_a_field(seed);
        let mut pile = stocked();
        let mut wells = Wells::default();
        wells.spud(at, seed, &mut pile).unwrap();
        wells.tick(wells.at(at).unwrap().total_drill + PUMP_PERIOD * 5, &mut pile);

        let directory = std::env::temp_dir().join(format!("vx-wells-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        wells.save(&directory).unwrap();

        let mut loaded = Wells::default();
        loaded.load(&directory);
        assert_eq!(loaded, wells);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn every_panel_state_draws_inside_the_frame() {
        // The fabricator's overflow, pinned: the busiest panel this can draw
        // has to fit the height the constant claims.
        let at = BlockPos::new(120, 88, -64);
        let states = [
            WellContent {
                at,
                trace: false,
                hole: None,
                refusal: Some("SHORT 4 COPPER BAR".into()),
                feedback: Some("NOTHING UNDER THIS ONE".into()),
            },
            WellContent {
                at,
                trace: true,
                hole: None,
                refusal: None,
                feedback: None,
            },
            WellContent {
                at,
                trace: true,
                hole: Some(Well {
                    at,
                    stage: Stage::Drilling { left: 300 },
                    fluid: Some(Fluid::Oil),
                    remaining: 900,
                    lifted: 0,
                    total_drill: 900,
                    pumped_for: 0,
                }),
                refusal: None,
                feedback: Some("SPUDDED IN".into()),
            },
            WellContent {
                at,
                trace: true,
                hole: Some(Well {
                    at,
                    stage: Stage::Pumping,
                    fluid: Some(Fluid::Gas),
                    remaining: 12_345,
                    lifted: 98_765,
                    total_drill: 1_400,
                    pumped_for: 7,
                }),
                refusal: None,
                feedback: Some("STRUCK GAS AT 62 BLOCKS".into()),
            },
        ];
        for content in states {
            let pixels = render_well(&content);
            assert_eq!(pixels.len(), (WELL_WIDTH * WELL_HEIGHT * 4) as usize);
            assert!(
                pixels.chunks_exact(4).any(|texel| texel != BACKGROUND),
                "a panel state drew nothing at all"
            );
        }
    }
}
