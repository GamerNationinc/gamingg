//! The electrolyser: a lake, two electrodes, and patience.
//!
//! # Where fuel comes from
//!
//! Every other good in this game is dug out of the ground or made from
//! something that was. Oxyhydrogen is neither: it is water taken apart, and
//! the world has an ocean of water. That would make it free, so the cost is
//! moved to the three things that are not free — the **electrodes**, which
//! wear away into the bath as copper; the **time**, which is real minutes of
//! a machine running; and the **place**, because an electrolyser has to
//! stand where there is water to split.
//!
//! That last one is the interesting constraint. It is the first machine in
//! this game whose *position* is part of whether it works at all, which
//! turns a lake shore from scenery into somewhere worth building.
//!
//! # Built like the fabricator, on purpose
//!
//! One job at a time, materials charged up front, progress on the journal's
//! clock, a panel with rows and a cursor. Not because a second machine had to
//! be identical, but because the fabricator's shape was the right one and a
//! player who has used it already knows how to use this.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::Stockpile;
use vx_core::BlockPos;
use vx_render::font::{self, LINE_HEIGHT};

use crate::skills;

/// How far from the bath the water may be. Two blocks: the machine stands at
/// the shore, not in the middle of the lake and not up on the hill.
pub const WATER_REACH: i32 = 2;

/// What the electrodes are made of.
const ELECTRODE: &str = "engine:copper_bar";

const MAGIC: &[u8; 4] = b"VXEL";
const VERSION: u32 = 1;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [120, 200, 240, 255];
const SHORT: [u8; 4] = [235, 110, 90, 255];
const GOOD: [u8; 4] = [150, 220, 150, 255];
const BACKGROUND: [u8; 4] = [10, 14, 18, 240];

/// One length of run the machine offers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Run {
    pub label: &'static str,
    /// Canisters this run fills.
    pub cells: u32,
    /// Copper bars consumed as electrodes, charged up front.
    pub bars: u64,
    /// Seconds at Fabrication level one.
    pub seconds: f32,
}

/// The runs on offer, shortest first.
///
/// Longer runs are cheaper per canister — a set of electrodes lasts better
/// once it is warm, and more to the point a player who has committed to
/// standing a machine by a lake should be rewarded for using it properly
/// rather than nursing it.
pub const RUNS: &[Run] = &[
    Run {
        label: "SHORT RUN - 4 CELLS",
        cells: 4,
        bars: 2,
        seconds: 24.0,
    },
    Run {
        label: "LONG RUN - 12 CELLS",
        cells: 12,
        bars: 5,
        seconds: 66.0,
    },
    Run {
        label: "FULL BANK - 36 CELLS",
        cells: 36,
        bars: 13,
        seconds: 180.0,
    },
];

/// Look a run up by the index the journal records.
pub fn run(index: usize) -> Option<&'static Run> {
    RUNS.get(index)
}

/// Is there water within reach of a block?
///
/// Asked of the world rather than of the height field, so a pond somebody
/// poured themselves counts exactly as much as the sea does — the machine
/// cares about water, not about geography.
pub fn water_near(world: &vx_world::World, at: BlockPos) -> bool {
    let Some(water) = world.registry().id_of("engine:water") else {
        return false;
    };
    for dx in -WATER_REACH..=WATER_REACH {
        for dy in -WATER_REACH..=WATER_REACH {
            for dz in -WATER_REACH..=WATER_REACH {
                if world.block(at.offset([dx, dy, dz])) == water {
                    return true;
                }
            }
        }
    }
    false
}

/// Why this run cannot start, in the order a person would notice.
pub fn refuse(run: &Run, pile: Option<&Stockpile>, dry_shore: bool) -> Option<String> {
    if dry_shore {
        return Some("NO WATER IN REACH".to_string());
    }
    let held = pile.map_or(0, |pile| pile.count(ELECTRODE));
    if held < run.bars {
        return Some(format!("SHORT {} COPPER BAR", run.bars - held));
    }
    None
}

/// How long a run takes for somebody this practised.
pub fn duration(run: &Run, fabrication: u32) -> f32 {
    skills::bypass_seconds(run.seconds, fabrication)
}

/// A run under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Job {
    pub run: usize,
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

/// What a tick of running produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    Working(f32),
    Done { cells: u32, xp: u64 },
}

/// The machine: where it stands, and what it is making.
#[derive(Debug, Default)]
pub struct Electrolyser {
    /// Where it was placed, once it has been.
    pub at: Option<BlockPos>,
    pub job: Option<Job>,
    pub open: bool,
    pub cursor: usize,
    pub feedback: Option<String>,
}

impl Electrolyser {
    pub fn open_at(&mut self, at: BlockPos) {
        self.open = true;
        self.at = Some(at);
        self.feedback = None;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let last = RUNS.len() as i32 - 1;
        self.cursor = (self.cursor as i32 + delta).clamp(0, last) as usize;
    }

    /// Start a run, charging the electrodes to the pile up front.
    pub fn begin(
        &mut self,
        index: usize,
        pile: &mut Stockpile,
        fabrication: u32,
        dry_shore: bool,
    ) -> Result<(), String> {
        if self.job.is_some() {
            return Err("ALREADY RUNNING".into());
        }
        let run = run(index).ok_or_else(|| "NO SUCH RUN".to_string())?;
        if let Some(reason) = refuse(run, Some(pile), dry_shore) {
            return Err(reason);
        }
        pile.take(ELECTRODE, run.bars);
        self.job = Some(Job {
            run: index,
            done: 0.0,
            total: duration(run, fabrication),
        });
        Ok(())
    }

    /// Advance the run by `dt` seconds.
    pub fn work(&mut self, dt: f32) -> Option<Progress> {
        let job = self.job.as_mut()?;
        job.done += dt;
        if job.done < job.total {
            return Some(Progress::Working(job.fraction()));
        }
        let index = job.run;
        self.job = None;
        let finished = run(index)?;
        Some(Progress::Done {
            cells: finished.cells,
            xp: 30 + finished.cells as u64 * 6,
        })
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "electrolyser.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        match self.at {
            Some(at) => {
                file.write_all(&[1u8])?;
                file.write_all(&at.x.to_le_bytes())?;
                file.write_all(&at.y.to_le_bytes())?;
                file.write_all(&at.z.to_le_bytes())?;
            }
            None => file.write_all(&[0u8])?,
        }
        file.commit()
    }

    /// Load where the machine stands. A run in progress is not persisted —
    /// the electrodes were already spent, and a bank of gas half made is the
    /// kind of state a save has no business inventing.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("electrolyser.dat");
        match read_machine(&path) {
            Ok(Some(at)) => self.at = at,
            Ok(None) => {}
            Err(error) => log::warn!("unreadable {}: {error}", path.display()),
        }
    }
}

#[allow(clippy::type_complexity)]
fn read_machine(path: &Path) -> std::io::Result<Option<Option<BlockPos>>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not an electrolyser file"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    let mut flag = [0u8; 1];
    file.read_exact(&mut flag)?;
    if flag[0] == 0 {
        return Ok(Some(None));
    }
    let mut read = || -> std::io::Result<i32> {
        let mut bytes = [0u8; 4];
        file.read_exact(&mut bytes)?;
        Ok(i32::from_le_bytes(bytes))
    };
    Ok(Some(Some(BlockPos::new(read()?, read()?, read()?))))
}

pub const FUEL_WIDTH: u32 = 250;
pub const FUEL_HEIGHT: u32 = 132;

/// Draw the panel. Pure in its inputs, like every other panel here.
pub fn render_electrolyser(
    machine: &Electrolyser,
    pile: Option<&Stockpile>,
    fabrication: u32,
    dry_shore: bool,
) -> Vec<u8> {
    let mut pixels = vec![0u8; (FUEL_WIDTH * FUEL_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    font::draw_text(&mut pixels, FUEL_WIDTH, margin, y, 1, ACCENT, "ELECTROLYSER");
    let held = pile.map_or(0, |pile| pile.count(crate::fuel::CELL));
    let banner = format!("HHO {held}");
    font::draw_text(
        &mut pixels,
        FUEL_WIDTH,
        FUEL_WIDTH as i32 - margin - font::text_width(&banner, 1) as i32,
        y,
        1,
        DIM,
        &banner,
    );
    y += LINE_HEIGHT as i32 + 3;

    for (index, offer) in RUNS.iter().enumerate() {
        let selected = index == machine.cursor;
        if selected {
            font::draw_text(&mut pixels, FUEL_WIDTH, margin, y, 1, ACCENT, ">");
        }
        let blocked = refuse(offer, pile, dry_shore).is_some();
        let colour = if blocked {
            SHORT
        } else if selected {
            TEXT
        } else {
            DIM
        };
        font::draw_text(&mut pixels, FUEL_WIDTH, margin + 10, y, 1, colour, offer.label);
        y += LINE_HEIGHT as i32;
    }
    y += 3;

    if let Some(offer) = RUNS.get(machine.cursor) {
        let line = format!(
            "EATS {} COPPER BAR, {}S",
            offer.bars,
            duration(offer, fabrication).round() as i32
        );
        font::draw_text(&mut pixels, FUEL_WIDTH, margin, y, 1, DIM, &line);
        y += LINE_HEIGHT as i32;
        if let Some(reason) = refuse(offer, pile, dry_shore) {
            font::draw_text(&mut pixels, FUEL_WIDTH, margin, y, 1, SHORT, &reason);
            y += LINE_HEIGHT as i32;
        }
    }

    if let Some(job) = machine.job {
        let label = run(job.run).map_or("RUNNING", |offer| offer.label);
        font::draw_text(&mut pixels, FUEL_WIDTH, margin, y, 1, GOOD, label);
        let bar_x = margin + font::text_width(label, 1) as i32 + 8;
        let width = FUEL_WIDTH as i32 - bar_x - margin;
        if width > 8 {
            let filled = (width as f32 * job.fraction()) as i32;
            for step in 0..width {
                let colour = if step < filled { GOOD } else { DIM };
                for row in 0..3 {
                    let px = ((y + row + 2) * FUEL_WIDTH as i32 + bar_x + step) as usize * 4;
                    if px + 4 <= pixels.len() {
                        pixels[px..px + 4].copy_from_slice(&colour);
                    }
                }
            }
        }
    } else if let Some(note) = &machine.feedback {
        font::draw_text(&mut pixels, FUEL_WIDTH, margin, y, 1, TEXT, note);
    }

    font::draw_text(
        &mut pixels,
        FUEL_WIDTH,
        margin,
        FUEL_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
        1,
        DIM,
        "ARROWS PICK. ENTER RUNS. E LEAVES.",
    );
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stocked(bars: u64) -> Stockpile {
        let mut pile = Stockpile::new();
        pile.add(ELECTRODE, bars);
        pile
    }

    #[test]
    fn a_dry_shore_refuses_every_run() {
        // The whole point of the machine's siting rule: away from water it
        // does nothing at all, however full the pile is.
        let plenty = stocked(500);
        for offer in RUNS {
            assert_eq!(
                refuse(offer, Some(&plenty), true).as_deref(),
                Some("NO WATER IN REACH")
            );
            assert!(refuse(offer, Some(&plenty), false).is_none());
        }
    }

    /// Ice is not water to the machine: a frozen lake beside an
    /// electrolyser is `NO WATER IN REACH` until the thaw.
    #[test]
    fn a_frozen_shore_is_a_dry_one() {
        let mut world = vx_world::World::new(1);
        world.load_around(vx_core::ChunkPos::new(0, 0), 1);
        let water = world.registry().id_of("engine:water").unwrap();
        let ice = world.registry().id_of("engine:ice").unwrap();
        let top = world.surface_y(0, 0).unwrap();
        let machine = BlockPos::new(0, top + 3, 0);
        let lake = BlockPos::new(1, top + 3, 0);
        world.set_block(lake, water);
        assert!(water_near(&world, machine));
        world.set_block(lake, ice);
        assert!(!water_near(&world, machine), "the electrolyser found water under the ice");
    }

    #[test]
    fn electrodes_are_charged_up_front() {
        let mut pile = stocked(6);
        let mut machine = Electrolyser::default();
        machine.begin(0, &mut pile, 1, false).unwrap();
        assert_eq!(pile.count(ELECTRODE), 4, "the electrodes were not spent");
        // And a second run cannot start on top of the first.
        assert!(machine.begin(0, &mut pile, 1, false).is_err());
        assert_eq!(pile.count(ELECTRODE), 4, "a refused run still charged");
    }

    #[test]
    fn a_run_finishes_into_canisters() {
        let mut pile = stocked(20);
        let mut machine = Electrolyser::default();
        machine.begin(1, &mut pile, 1, false).unwrap();
        let total = duration(&RUNS[1], 1);
        assert!(matches!(machine.work(total * 0.5), Some(Progress::Working(_))));
        match machine.work(total) {
            Some(Progress::Done { cells, xp }) => {
                assert_eq!(cells, RUNS[1].cells);
                assert!(xp > 0);
            }
            other => panic!("the run did not finish: {other:?}"),
        }
        assert!(machine.job.is_none());
    }

    #[test]
    fn levelling_buys_speed_and_nothing_else() {
        for offer in RUNS {
            let green = duration(offer, 1);
            let veteran = duration(offer, 40);
            assert!(veteran < green, "forty levels bought no speed");
            assert_eq!(offer.cells, offer.cells, "the yield is not a function of skill");
        }
    }

    #[test]
    fn longer_runs_are_cheaper_by_the_canister() {
        let mut best = f32::MAX;
        for offer in RUNS {
            let per_cell = offer.bars as f32 / offer.cells as f32;
            assert!(
                per_cell <= best,
                "{} costs more per canister than a shorter run",
                offer.label
            );
            best = per_cell;
        }
    }

    #[test]
    fn the_position_survives_a_save() {
        let directory = std::env::temp_dir().join(format!("vx-electro-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let machine = Electrolyser {
            at: Some(BlockPos::new(-40, 63, 128)),
            ..Electrolyser::default()
        };
        machine.save(&directory).unwrap();
        let mut loaded = Electrolyser::default();
        loaded.load(&directory);
        assert_eq!(loaded.at, machine.at);
        assert!(loaded.job.is_none());
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn the_panel_is_deterministic_and_drawable() {
        let pile = stocked(9);
        let machine = Electrolyser {
            open: true,
            cursor: 1,
            ..Electrolyser::default()
        };
        assert_eq!(
            render_electrolyser(&machine, Some(&pile), 4, false),
            render_electrolyser(&machine, Some(&pile), 4, false)
        );
        assert_ne!(
            render_electrolyser(&machine, Some(&pile), 4, false),
            render_electrolyser(&machine, Some(&pile), 4, true),
            "the dry-shore refusal never reached the panel"
        );
        for offer in RUNS {
            for character in offer.label.chars() {
                assert!(font::knows(character), "undrawable {character:?}");
            }
        }
    }
}
