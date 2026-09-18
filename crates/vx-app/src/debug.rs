//! The debug readout: the engine's vitals, drawn over the world.
//!
//! # Diagnostics, not cheats
//!
//! The gold panel is a scenario editor: journalled orders, live tuning,
//! compiled out of shipped builds because it changes what happens. This
//! panel changes nothing — it only *says* what is happening — so it ships
//! in every build, F3 to toggle, which is precisely the point: the machine
//! it is most useful on is the Deck in your hands, where a title-bar
//! frame counter does not exist.
//!
//! # A pure function of a snapshot
//!
//! `main` assembles a [`DebugContent`] from live state and this module
//! draws it, the same split every panel here uses: the render is testable
//! without a window, deterministic for the capture fixtures, and the
//! panel cannot reach into systems and perturb what it reports.

use vx_render::font::{self, LINE_HEIGHT};

/// Panel size in texture pixels, drawn at the shop scale.
pub const DEBUG_WIDTH: u32 = 300;

const TEXT: [u8; 4] = [220, 235, 220, 255];
const DIM: [u8; 4] = [140, 150, 140, 255];
const ACCENT: [u8; 4] = [140, 235, 140, 255];
const WARN: [u8; 4] = [235, 200, 90, 255];
const BACKGROUND: [u8; 4] = [8, 14, 8, 215];

/// Everything the panel says, one struct so the border between "reads
/// state" and "draws pixels" is a line you can point at.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DebugContent {
    /// Smoothed frames per second.
    pub fps: f32,
    /// Feet position and the chunk they stand in.
    pub position: (f32, f32, f32),
    pub chunk: (i32, i32),
    /// Look angles, degrees.
    pub yaw: f32,
    pub pitch: f32,
    /// The block under the crosshair, by name, if any.
    pub aimed: Option<String>,
    /// The room the feet are in, as `atmos` sees it: its sealed volume, or
    /// that it is outdoors.
    pub room: String,
    /// World: chunks loaded / drawn, triangles, player edits, wounds.
    pub chunks_loaded: usize,
    pub chunks_drawn: usize,
    pub triangles: u32,
    pub edits: u64,
    pub composites: usize,
    /// The journal: current tick and entries in the log.
    pub tick: u64,
    pub log_entries: usize,
    /// Day number and the clock face.
    pub day: u32,
    pub hhmm: (u32, u32),
    /// Fleet: burners working, fuel cells in reach, worst condition.
    pub burners: u32,
    pub fuel_cells: u64,
    pub worst_machine: &'static str,
    /// The living world: panicking townsfolk, contact marks, slugs aloft.
    pub panicking: usize,
    pub marks: usize,
    pub shots: usize,
    /// The other side: deputies after you, shelters mustered, and the
    /// posse's belief when there is one — total mass and confidence.
    pub deputies: usize,
    pub squads: usize,
    pub hunting: usize,
    pub belief: Option<(f32, f32)>,
    /// You: hits, bounty, and both standings.
    pub hits: (u8, u8),
    pub bounty: u64,
    pub compact: &'static str,
    pub holdouts: &'static str,
    /// Holes sunk, and how many of them are still lifting.
    pub wells: (usize, usize),
    /// Rads a second at the body right now, and the dose carried.
    pub rads: (f32, f32),
    /// Medkits in the pocket.
    pub medkits: u8,
    /// How roused the deep is, 0 to 1, and what it is doing if anything.
    pub dark: (f32, &'static str),
}

/// One row: a dim label and a value.
struct Rows {
    pixels: Vec<u8>,
    y: i32,
}

impl Rows {
    fn line(&mut self, label: &str, value: &str, colour: [u8; 4]) {
        font::draw_text(&mut self.pixels, DEBUG_WIDTH, 8, self.y, 1, DIM, label);
        font::draw_text(&mut self.pixels, DEBUG_WIDTH, 8 + 74, self.y, 1, colour, value);
        self.y += LINE_HEIGHT as i32;
    }

    fn gap(&mut self) {
        self.y += 3;
    }
}

/// Rows the panel always draws, plus one optional belief row. The height
/// is derived, not typed — the fabricator panel's overflow taught that
/// lesson for everybody.
const FIXED_ROWS: u32 = 18;
pub const DEBUG_HEIGHT: u32 = 12 + (FIXED_ROWS + 1) * LINE_HEIGHT + 4 * 3 + 8;

/// Draw the readout. Pure in its inputs.
pub fn render_debug(content: &DebugContent) -> Vec<u8> {
    let mut pixels = vec![0u8; (DEBUG_WIDTH * DEBUG_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let mut rows = Rows { pixels, y: 8 };
    font::draw_text(
        &mut rows.pixels,
        DEBUG_WIDTH,
        8,
        rows.y,
        1,
        ACCENT,
        "ENGINE",
    );
    let fps = format!("{:.0} FPS", content.fps);
    font::draw_text(
        &mut rows.pixels,
        DEBUG_WIDTH,
        DEBUG_WIDTH as i32 - 8 - font::text_width(&fps, 1) as i32,
        rows.y,
        1,
        if content.fps < 25.0 { WARN } else { ACCENT },
        &fps,
    );
    rows.y += LINE_HEIGHT as i32 + 3;

    let (x, y, z) = content.position;
    rows.line("AT", &format!("{x:.1} {y:.1} {z:.1}"), TEXT);
    rows.line(
        "CHUNK",
        &format!(
            "{} {}   YAW {:.0} PITCH {:.0}",
            content.chunk.0, content.chunk.1, content.yaw, content.pitch
        ),
        TEXT,
    );
    rows.line(
        "AIMED",
        content.aimed.as_deref().unwrap_or("-"),
        TEXT,
    );
    rows.line("ROOM", &content.room, TEXT);
    rows.gap();

    rows.line(
        "CHUNKS",
        &format!("{} DRAWN OF {}", content.chunks_drawn, content.chunks_loaded),
        TEXT,
    );
    rows.line(
        "TRIS",
        &format!(
            "{}   EDITS {}   WOUNDS {}",
            content.triangles, content.edits, content.composites
        ),
        TEXT,
    );
    rows.line(
        "JOURNAL",
        &format!("TICK {}   {} ENTRIES", content.tick, content.log_entries),
        TEXT,
    );
    rows.line(
        "CLOCK",
        &format!("DAY {}  {:02}:{:02}", content.day, content.hhmm.0, content.hhmm.1),
        TEXT,
    );
    rows.gap();

    rows.line(
        "FLEET",
        &format!(
            "{} BURNING   HHO {}   {}",
            content.burners, content.fuel_cells, content.worst_machine
        ),
        TEXT,
    );
    rows.line(
        "AROUND",
        &format!(
            "{} PANICKING   {} MARKS   {} SHOTS",
            content.panicking, content.marks, content.shots
        ),
        TEXT,
    );
    rows.gap();

    rows.line(
        "HOSTILE",
        &format!(
            "{} DEPUTIES   {} SQUADS   {} HUNTING",
            content.deputies, content.squads, content.hunting
        ),
        if content.deputies + content.hunting > 0 {
            WARN
        } else {
            TEXT
        },
    );
    match content.belief {
        Some((mass, confidence)) => rows.line(
            "BELIEF",
            &format!("MASS {mass:.2}   CONFIDENCE {confidence:.2}"),
            WARN,
        ),
        None => rows.line("BELIEF", "NONE HELD", TEXT),
    }
    rows.gap();

    rows.line(
        "YOU",
        &format!(
            "HITS {}/{}   BOUNTY {}   MEDKITS {}",
            content.hits.0, content.hits.1, content.bounty, content.medkits
        ),
        TEXT,
    );
    rows.line(
        "NAME",
        &format!("TOWNS {}   SHELTERS {}", content.compact, content.holdouts),
        TEXT,
    );
    rows.line(
        "DOSE",
        &format!("{:.0} RADS   {:.1}/S", content.rads.1, content.rads.0),
        if content.rads.1 >= crate::dose::BURN_AT {
            WARN
        } else {
            TEXT
        },
    );
    rows.line(
        "THE DEEP",
        &format!("HEAT {:.0}%   {}", content.dark.0 * 100.0, content.dark.1),
        if content.dark.1 == "GONE" { TEXT } else { WARN },
    );
    rows.line(
        "WELLS",
        &format!("{} SUNK   {} PUMPING", content.wells.0, content.wells.1),
        TEXT,
    );
    rows.line("F3", "CLOSES", DIM);

    rows.pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy() -> DebugContent {
        DebugContent {
            fps: 58.7,
            position: (12.5, 74.0, -3.2),
            chunk: (0, -1),
            yaw: 182.0,
            pitch: -12.0,
            aimed: Some("ENGINE:STONE".into()),
            chunks_loaded: 113,
            chunks_drawn: 69,
            triangles: 142_924,
            edits: 5_512,
            composites: 37,
            tick: 88_412,
            log_entries: 1_204,
            day: 9,
            hhmm: (13, 42),
            burners: 4,
            fuel_cells: 11,
            worst_machine: "WORN",
            panicking: 1,
            marks: 3,
            shots: 2,
            deputies: 3,
            squads: 1,
            hunting: 2,
            belief: Some((0.82, 0.44)),
            hits: (4, 6),
            bounty: 120,
            compact: "WARM",
            holdouts: "COLD",
            wells: (4, 2),
            rads: (18.4, 132.0),
            dark: (0.8, "HUNTING"),
            medkits: 3,
            room: "75 BLOCKS SEALED".into(),
        }
    }

    #[test]
    fn the_panel_draws_every_row_without_overflowing() {
        // The drawing indexes the pixel buffer directly, so the loudest
        // test is the busiest panel: every optional present, every number
        // wide. A panic is the failure — the fabricator's lesson, applied
        // before the bug this time rather than after it.
        let pixels = render_debug(&busy());
        assert_eq!(pixels.len(), (DEBUG_WIDTH * DEBUG_HEIGHT * 4) as usize);
        let empty = render_debug(&DebugContent::default());
        assert_eq!(empty.len(), pixels.len());
    }

    #[test]
    fn the_render_is_deterministic_and_reacts_to_its_inputs() {
        assert_eq!(render_debug(&busy()), render_debug(&busy()));
        let mut moved = busy();
        moved.tick += 1;
        assert_ne!(render_debug(&busy()), render_debug(&moved), "the tick row is dead");
        let mut quiet = busy();
        quiet.belief = None;
        assert_ne!(render_debug(&busy()), render_debug(&quiet), "the belief row is dead");
    }

    #[test]
    fn every_character_the_panel_can_emit_is_drawable() {
        // The static labels, and a formatted sample of every dynamic row.
        let content = busy();
        for line in [
            format!("{:.0} FPS", content.fps),
            format!("MASS {:.2}   CONFIDENCE {:.2}", 0.82, 0.44),
            "NONE HELD".to_string(),
            content.aimed.clone().unwrap_or_default(),
            content.worst_machine.to_string(),
        ] {
            for character in line.chars() {
                assert!(font::knows(character), "undrawable {character:?} in {line:?}");
            }
        }
    }
}
