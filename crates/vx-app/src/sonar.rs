//! The drill's sonar: what is in the four metres of rock around the bit.
//!
//! # A read, never a write
//!
//! Every function here takes the world and gives back a description of it.
//! Nothing in this module breaks a block, moves a good or touches a ledger,
//! so a ping is not an order and is not on the journal — which is what lets
//! it fire on every block the bit touches without the replay oracle ever
//! having to know it happened.
//!
//! # What it is for
//!
//! Ore in this game is buried and there has never been any way to look for
//! it but to dig. The flier's survey pings tell you a *sector* has copper in
//! it, which is a thing you read off a map; this tells you the copper is
//! three blocks behind the wall you are standing at, which is a thing you
//! act on with the tool already in your hand. That is the whole design: the
//! reading is worthless at range and decisive at arm's length.
//!
//! # The order is a total order, on purpose
//!
//! [`ping`] groups by block name and sorts seams first, then by count, then
//! by name. Every tie is broken, so the same ground pings the same way twice
//! — which is what makes the scope panel safe to compare in a test and safe
//! to photograph in a capture.

use std::collections::BTreeMap;

use vx_core::{BlockPos, BlockRegistry};
use vx_render::font::{self, LINE_HEIGHT};
use vx_world::World;

/// How far the ping reaches, in metres, measured from the centre of the
/// block the bit touched. Four, as asked for.
pub const REACH: f64 = 4.0;

/// How long an echo marker stays lit in the world, in seconds.
pub const LINGER: f32 = 4.0;

/// How long the scope panel stays up after the last ping, in seconds.
pub const SCOPE_SECONDS: f32 = 5.0;

/// How many rows the scope has room for.
const ROWS: usize = 6;

/// How many seam cells get a marker in the world.
///
/// Capped because a ping standing in an ore body hears hundreds, and three
/// hundred markers is a magenta fog rather than a reading. The nearest are
/// the ones worth pointing at anyway.
pub const MAX_MARKS: usize = 40;

/// The six ways out of a block.
const AROUND: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// Panel size in texture pixels, shown at [`SCOPE_SCALE`].
pub const SCOPE_WIDTH: u32 = 156;
pub const SCOPE_HEIGHT: u32 = 126;
pub const SCOPE_SCALE: f32 = 2.0;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
/// The cage's cyan and the echo's magenta, so the panel is painted in the
/// same two colours the world is.
const CAGE: [u8; 4] = [60, 235, 245, 255];
const ECHO: [u8; 4] = [245, 80, 215, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 225];
const SCOPE_FACE: [u8; 4] = [16, 26, 30, 255];

/// One kind of block the ping heard back from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Echo {
    /// The block's namespaced name, as the registry knows it.
    pub name: String,
    /// How many cells of it are within reach.
    pub count: u32,
    /// The nearest of those cells.
    pub nearest: BlockPos,
    /// How far away that one is, in metres, centre to centre. Held in
    /// hundredths so an `Echo` is comparable and hashable like every other
    /// record in this game — floats in a sorted key are how a total order
    /// stops being total.
    pub centimetres: u32,
}

impl Echo {
    /// The distance to the nearest cell, in metres.
    pub fn distance(&self) -> f32 {
        self.centimetres as f32 / 100.0
    }

    /// Whether this is something worth walking towards.
    pub fn is_seam(&self) -> bool {
        seam(&self.name)
    }
}

/// Everything one ping heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// The block the bit was on.
    pub centre: BlockPos,
    /// What came back, seams first. See the module note on ordering.
    pub echoes: Vec<Echo>,
    /// How many cells were solid, of the ones in reach.
    pub cells: u32,
    /// Where to hang a marker in the world, nearest first, capped at
    /// [`MAX_MARKS`].
    ///
    /// These are **air cells**, not the seam cells themselves — each one is
    /// the open cell against an exposed face of a seam block. The renderer
    /// draws opaque geometry with a depth test, so a marker sitting inside
    /// solid rock is a marker nobody will ever see; hanging it in the air
    /// against the ore puts it exactly where a player is looking and costs
    /// nothing.
    ///
    /// A seam with no exposed face at all gets no marker and is counted in
    /// [`Reading::buried`] instead. That is the honest division of labour
    /// here: the markers show you what you could reach out and touch, and
    /// the readout tells you what is still behind the wall.
    pub marks: Vec<BlockPos>,
    /// Seam cells heard with no exposed face — the ore still inside the rock.
    pub buried: u32,
}

impl Default for Reading {
    /// A ping that heard nothing, at the origin. `BlockPos` has no `Default`
    /// of its own — a position with no meaning is exactly the bug that
    /// convention is there to prevent — so this one is written out, and it
    /// is only ever the empty panel a fixture draws before the first ping.
    fn default() -> Self {
        Reading {
            centre: BlockPos::new(0, 0, 0),
            echoes: Vec::new(),
            cells: 0,
            marks: Vec::new(),
            buried: 0,
        }
    }
}

impl Reading {
    /// The seams only — what gets an echo marker in the world.
    pub fn seams(&self) -> impl Iterator<Item = &Echo> {
        self.echoes.iter().filter(|echo| echo.is_seam())
    }

    pub fn is_empty(&self) -> bool {
        self.echoes.is_empty()
    }
}

/// Whether a block is worth lighting up.
///
/// Two rules rather than one list: anything whose name ends in `_ore`, plus
/// the named exceptions — the two hydrocarbons are seams by any sensible
/// reading and are not called ore by anybody. Written as a rule and not as a
/// table so that a block added by a later stage, or by a mod, is a seam on
/// the day it is registered rather than on the day somebody remembers to
/// come back here.
pub fn seam(name: &str) -> bool {
    let bare = name.split_once(':').map_or(name, |(_, rest)| rest);
    bare.ends_with("_ore") || matches!(bare, "oil_sand" | "gas_shale")
}

/// Read the ground within [`REACH`] of a block.
///
/// Sweeps the box that contains the sphere, keeps the cells actually inside
/// it, and groups what it finds by name. Air is not an echo; the block at
/// the centre is, because the thing you are drilling is part of what is
/// around you and leaving it out reads as a bug.
pub fn ping(world: &World, registry: &BlockRegistry, centre: BlockPos) -> Reading {
    let span = REACH.ceil() as i32;
    let limit = REACH * REACH;
    // Name -> (count, nearest cell, its squared distance). A `BTreeMap` and
    // not a hash map: the iteration order feeds the sort, and a sort that
    // starts from an arbitrary order is a sort that has to be total by
    // itself. It is anyway — but two independent reasons to be deterministic
    // are cheaper than one.
    let mut heard: BTreeMap<String, (u32, BlockPos, f64)> = BTreeMap::new();
    let mut found: Vec<(u64, BlockPos)> = Vec::new();
    let mut buried = 0;
    let mut cells = 0;

    for dy in -span..=span {
        for dz in -span..=span {
            for dx in -span..=span {
                let away = (dx * dx + dy * dy + dz * dz) as f64;
                if away > limit {
                    continue;
                }
                let at = centre.offset([dx, dy, dz]);
                let id = world.block(at);
                let Some(def) = registry.get(id) else { continue };
                if def.name == "engine:air" {
                    continue;
                }
                cells += 1;
                if seam(&def.name) {
                    // The open cell against an exposed face, if there is
                    // one. Sorted on the squared distance in whole units and
                    // then on the position, so the marker list is as total
                    // an order as the echo list.
                    match AROUND
                        .into_iter()
                        .map(|side| at.offset(side))
                        .find(|beside| !world.is_solid(*beside))
                    {
                        Some(open) => found.push((away as u64, open)),
                        None => buried += 1,
                    }
                }
                let entry = heard
                    .entry(def.name.clone())
                    .or_insert((0, at, f64::INFINITY));
                entry.0 += 1;
                if away < entry.2 {
                    entry.1 = at;
                    entry.2 = away;
                }
            }
        }
    }

    let mut echoes: Vec<Echo> = heard
        .into_iter()
        .map(|(name, (count, nearest, away))| Echo {
            name,
            count,
            nearest,
            centimetres: (away.sqrt() * 100.0).round() as u32,
        })
        .collect();
    // Seams first, then the commonest, then by name. Every tie broken.
    echoes.sort_by(|a, b| {
        b.is_seam()
            .cmp(&a.is_seam())
            .then(b.count.cmp(&a.count))
            .then(a.name.cmp(&b.name))
    });

    found.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then((a.1.x, a.1.y, a.1.z).cmp(&(b.1.x, b.1.y, b.1.z)))
    });
    let marks = found
        .into_iter()
        .take(MAX_MARKS)
        .map(|(_, at)| at)
        .collect();

    Reading {
        centre,
        echoes,
        cells,
        marks,
        buried,
    }
}

/// The reading as rows of text, for the scope and for the terminal.
///
/// Seams carry their range because the range is the actionable part; common
/// rock carries only its count, because "STONE 4.2M" is a sentence about
/// nothing.
pub fn lines(reading: &Reading) -> Vec<String> {
    if reading.is_empty() {
        return vec!["NOTHING IN RANGE".to_string()];
    }
    reading
        .echoes
        .iter()
        .map(|echo| {
            let name = crate::shop::display_name(&echo.name);
            if echo.is_seam() {
                format!("{name} {} AT {:.1}M", echo.count, echo.distance())
            } else {
                format!("{name} {}", echo.count)
            }
        })
        .collect()
}

/// One line summarising a reading, for the greeting strip and the log.
pub fn headline(reading: &Reading) -> String {
    match reading.seams().next() {
        Some(echo) => format!(
            "PING: {} AT {:.1}M",
            crate::shop::display_name(&echo.name),
            echo.distance()
        ),
        None => "PING: NO SEAM IN RANGE".to_string(),
    }
}

fn fill(pixels: &mut [u8], x: i32, y: i32, width: u32, height: u32, colour: [u8; 4]) {
    for row in 0..height as i32 {
        for column in 0..width as i32 {
            let (px, py) = (x + column, y + row);
            if px < 0 || py < 0 || px >= SCOPE_WIDTH as i32 || py >= SCOPE_HEIGHT as i32 {
                continue;
            }
            let index = ((py as u32 * SCOPE_WIDTH + px as u32) * 4) as usize;
            pixels[index..index + 4].copy_from_slice(&colour);
        }
    }
}

/// Draw the panel. Pure in its inputs, like every other panel here.
///
/// `age` is seconds since the ping, and drives the sweep on the scope face
/// so the picture reads as live rather than as a table. A capture fixture
/// passes a literal and gets the same pixels every time.
pub fn render_scope(reading: &Reading, age: f32) -> Vec<u8> {
    let mut pixels = vec![0u8; (SCOPE_WIDTH * SCOPE_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }
    let margin = 5i32;

    font::draw_text(&mut pixels, SCOPE_WIDTH, margin, margin, 1, CAGE, "SONAR");
    let range = format!("{:.0}M", REACH);
    font::draw_text(
        &mut pixels,
        SCOPE_WIDTH,
        SCOPE_WIDTH as i32 - margin - font::text_width(&range, 1) as i32,
        margin,
        1,
        DIM,
        &range,
    );

    // The scope face: a square of dark glass with the rings on it, the
    // player at the middle, and every seam plotted where it actually is.
    let face = 44u32;
    let face_x = margin;
    let face_y = margin + LINE_HEIGHT as i32 + 2;
    fill(&mut pixels, face_x, face_y, face, face, SCOPE_FACE);
    let middle = (face_x + face as i32 / 2, face_y + face as i32 / 2);
    // Two rings, at half reach and full reach, so the plot has a scale.
    for ring in [face as i32 / 4, face as i32 / 2 - 1] {
        for step in 0..(ring * 8).max(1) {
            let angle = step as f32 / (ring * 8).max(1) as f32 * std::f32::consts::TAU;
            let x = middle.0 + (angle.cos() * ring as f32).round() as i32;
            let y = middle.1 + (angle.sin() * ring as f32).round() as i32;
            fill(&mut pixels, x, y, 1, 1, DIM);
        }
    }
    // The sweep: one spoke, turning once per second.
    let sweep = age * std::f32::consts::TAU;
    for step in 0..(face as i32 / 2) {
        let x = middle.0 + (sweep.cos() * step as f32).round() as i32;
        let y = middle.1 + (sweep.sin() * step as f32).round() as i32;
        fill(&mut pixels, x, y, 1, 1, CAGE);
    }
    fill(&mut pixels, middle.0 - 1, middle.1 - 1, 3, 3, CAGE);
    // Every seam, plotted from the centre in blocks. `x` runs right and `z`
    // runs down, which is the same convention the minimap uses.
    let scale = (face as f32 * 0.5 - 2.0) / REACH as f32;
    for echo in reading.seams() {
        let dx = (echo.nearest.x - reading.centre.x) as f32 * scale;
        let dz = (echo.nearest.z - reading.centre.z) as f32 * scale;
        let x = middle.0 + dx.round() as i32;
        let y = middle.1 + dz.round() as i32;
        fill(&mut pixels, x - 1, y - 1, 3, 3, ECHO);
    }

    // How much came back, beside the face — the one thing short enough to
    // sit there without running off the panel.
    let heard = format!("{} CELLS", reading.cells);
    font::draw_text(
        &mut pixels,
        SCOPE_WIDTH,
        face_x + face as i32 + 6,
        face_y + 2,
        1,
        DIM,
        &heard,
    );
    let seams = reading.seams().count();
    if seams > 0 {
        font::draw_text(
            &mut pixels,
            SCOPE_WIDTH,
            face_x + face as i32 + 6,
            face_y + 2 + LINE_HEIGHT as i32,
            1,
            ECHO,
            &format!("{seams} SEAM"),
        );
    }
    if reading.buried > 0 {
        font::draw_text(
            &mut pixels,
            SCOPE_WIDTH,
            face_x + face as i32 + 6,
            face_y + 2 + LINE_HEIGHT as i32 * 2,
            1,
            DIM,
            &format!("{} IN ROCK", reading.buried),
        );
    }

    // The rows go *under* the face, across the full width. Beside it they
    // ran off the edge — a name, a count and a range is twenty-odd
    // characters, and the panel is twenty-six wide.
    let mut y = face_y + face as i32 + 4;
    let rows = lines(reading);
    for (index, line) in rows.iter().take(ROWS).enumerate() {
        if y + LINE_HEIGHT as i32 > SCOPE_HEIGHT as i32 - margin {
            break;
        }
        let seam_row = reading
            .echoes
            .get(index)
            .is_some_and(|echo| echo.is_seam());
        let colour = if seam_row { ECHO } else { TEXT };
        font::draw_text(&mut pixels, SCOPE_WIDTH, margin, y, 1, colour, line);
        y += LINE_HEIGHT as i32;
    }
    if rows.len() > ROWS && y + LINE_HEIGHT as i32 <= SCOPE_HEIGHT as i32 - margin {
        let more = format!("+{} MORE", rows.len() - ROWS);
        font::draw_text(&mut pixels, SCOPE_WIDTH, margin, y, 1, DIM, &more);
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;

    /// A world of solid stone round the origin, so a test can put one ore
    /// block where it wants it and know everything else.
    fn stone_world() -> World {
        let mut world = World::new(7);
        world.load_around(ChunkPos::new(0, 0), 2);
        let stone = world.registry().id_of("engine:stone").expect("stone");
        for y in 60..70 {
            for z in -8..=8 {
                for x in -8..=8 {
                    world.set_block(BlockPos::new(x, y, z), stone);
                }
            }
        }
        world
    }

    fn put(world: &mut World, at: BlockPos, name: &str) {
        let id = world.registry().id_of(name).expect(name);
        world.set_block(at, id);
    }

    /// The headline behaviour: drill a bit of stone, and the copper behind
    /// the wall is in the reading with its range on it.
    #[test]
    fn a_ping_finds_the_seam_behind_the_wall() {
        let mut world = stone_world();
        let bit = BlockPos::new(0, 65, 0);
        put(&mut world, BlockPos::new(3, 65, 0), "engine:copper_ore");

        let reading = ping(&world, world.registry(), bit);
        let copper = reading
            .echoes
            .iter()
            .find(|echo| echo.name == "engine:copper_ore")
            .expect("the copper was not heard");
        assert_eq!(copper.count, 1);
        assert_eq!(copper.nearest, BlockPos::new(3, 65, 0));
        assert!((copper.distance() - 3.0).abs() < 0.01, "{}m", copper.distance());

        // And it is the first row, because a seam outranks a thousand
        // blocks of stone.
        assert_eq!(reading.echoes[0].name, "engine:copper_ore");
        assert!(headline(&reading).contains("COPPER ORE"));
    }

    /// Four metres means four metres. This is the number the player was
    /// promised, so it gets a test on both sides of the line.
    #[test]
    fn the_ping_reaches_four_metres_and_no_further() {
        let mut world = stone_world();
        let bit = BlockPos::new(0, 65, 0);
        put(&mut world, BlockPos::new(0, 65, 4), "engine:copper_ore");
        put(&mut world, BlockPos::new(0, 65, -5), "engine:uranium_ore");

        let reading = ping(&world, world.registry(), bit);
        assert!(
            reading.echoes.iter().any(|e| e.name == "engine:copper_ore"),
            "the block at four metres was missed"
        );
        assert!(
            !reading.echoes.iter().any(|e| e.name == "engine:uranium_ore"),
            "the block at five metres was heard anyway"
        );
    }

    /// Air is not an echo, and the block under the bit is.
    #[test]
    fn the_bit_hears_itself_but_not_the_empty_air() {
        let mut world = stone_world();
        let bit = BlockPos::new(0, 65, 0);
        put(&mut world, bit, "engine:copper_ore");
        // Hollow out one cell, which should simply not be counted.
        let air = world.registry().id_of("engine:air").expect("air");
        world.set_block(BlockPos::new(1, 65, 0), air);

        let reading = ping(&world, world.registry(), bit);
        assert!(!reading.echoes.iter().any(|e| e.name == "engine:air"));
        let copper = reading
            .echoes
            .iter()
            .find(|e| e.name == "engine:copper_ore")
            .expect("the bit did not hear its own block");
        assert_eq!(copper.centimetres, 0, "the block under the bit is not at zero");
    }

    /// The same ground twice is the same reading twice, byte for byte. This
    /// is what makes the scope safe to photograph and safe to compare.
    #[test]
    fn the_same_ground_pings_the_same_way_twice() {
        let mut world = stone_world();
        for (index, name) in ["engine:copper_ore", "engine:uranium_ore", "engine:gas_shale"]
            .iter()
            .enumerate()
        {
            put(&mut world, BlockPos::new(index as i32 + 1, 66, 2), name);
        }
        let bit = BlockPos::new(0, 65, 0);
        let first = ping(&world, world.registry(), bit);
        let second = ping(&world, world.registry(), bit);
        assert_eq!(first, second);
        assert_eq!(render_scope(&first, 0.4), render_scope(&second, 0.4));
    }

    /// Ties are broken all the way down, so two seams with the same count
    /// still come out in a fixed order.
    #[test]
    fn every_tie_in_the_order_is_broken() {
        let mut world = stone_world();
        put(&mut world, BlockPos::new(1, 65, 0), "engine:uranium_ore");
        put(&mut world, BlockPos::new(-1, 65, 0), "engine:copper_ore");
        let reading = ping(&world, world.registry(), BlockPos::new(0, 65, 0));
        let seams: Vec<&str> = reading.seams().map(|e| e.name.as_str()).collect();
        assert_eq!(seams, ["engine:copper_ore", "engine:uranium_ore"]);
    }

    /// The rule, not the list: an ore is a seam by its name, and the two
    /// hydrocarbons are named exceptions. Plain rock is not.
    #[test]
    fn a_seam_is_anything_worth_walking_towards() {
        assert!(seam("engine:copper_ore"));
        assert!(seam("engine:uranium_ore"));
        assert!(seam("engine:oil_sand"));
        assert!(seam("engine:gas_shale"));
        // And a block a later stage or a mod adds is a seam on the day it
        // is registered, without anybody editing this function.
        assert!(seam("somemod:tungsten_ore"));
        assert!(!seam("engine:stone"));
        assert!(!seam("engine:dirt"));
        assert!(!seam("engine:copper_bar"), "a bar is not a seam");
    }

    /// The house convention: every string the player can be shown must be
    /// drawable, or it renders as a row of filled boxes.
    #[test]
    fn every_line_of_a_reading_is_drawable() {
        let mut world = stone_world();
        put(&mut world, BlockPos::new(2, 65, 1), "engine:copper_ore");
        put(&mut world, BlockPos::new(-2, 64, 1), "engine:oil_sand");
        let reading = ping(&world, world.registry(), BlockPos::new(0, 65, 0));

        let mut all = lines(&reading);
        all.push(headline(&reading));
        all.extend(lines(&Reading::default()));
        all.push(headline(&Reading::default()));
        for line in all {
            for character in line.chars() {
                assert!(font::knows(character), "{character:?} in {line:?} is not drawable");
            }
        }
    }

    /// Every seam cell gets a marker, not just the nearest one — the point
    /// of the thing is to show you where the body *is*, and one dot in the
    /// middle of a seam says nothing about its shape. Capped, or an ore
    /// body is a fog.
    #[test]
    fn every_seam_cell_within_reach_gets_a_marker() {
        let mut world = stone_world();
        let wanted = [
            BlockPos::new(2, 65, 0),
            BlockPos::new(3, 65, 0),
            BlockPos::new(2, 66, 0),
            BlockPos::new(0, 65, -3),
        ];
        for at in wanted {
            put(&mut world, at, "engine:copper_ore");
        }
        // Buried in solid stone: heard, counted, and given no marker,
        // because a marker inside rock is one nobody can see.
        let reading = ping(&world, world.registry(), BlockPos::new(0, 65, 0));
        assert!(reading.marks.is_empty(), "a buried seam was marked anyway");
        assert_eq!(reading.buried, wanted.len() as u32);

        // Open the face of one of them and it earns a marker, hung in the
        // air cell against it rather than inside the ore.
        let air = world.registry().id_of("engine:air").expect("air");
        world.set_block(BlockPos::new(1, 65, 0), air);
        let opened = ping(&world, world.registry(), BlockPos::new(0, 65, 0));
        assert_eq!(
            opened.marks,
            vec![BlockPos::new(1, 65, 0)],
            "the exposed seam was not marked in the open cell beside it"
        );
        assert_eq!(opened.buried, 3, "the other three are still in the rock");
    }

    /// An ore body does not become a magenta fog.
    #[test]
    fn the_marker_list_is_capped() {
        let mut solid = stone_world();
        for z in -4..=4 {
            for y in 61..=69 {
                for x in -4..=4 {
                    put(&mut solid, BlockPos::new(x, y, z), "engine:copper_ore");
                }
            }
        }
        // Hollow a gallery through it, so there are exposed faces to mark.
        let air = solid.registry().id_of("engine:air").expect("air");
        for x in -4..=4 {
            solid.set_block(BlockPos::new(x, 65, 0), air);
            solid.set_block(BlockPos::new(x, 66, 0), air);
        }
        let dense = ping(&solid, solid.registry(), BlockPos::new(0, 64, 0));
        assert_eq!(dense.marks.len(), MAX_MARKS, "the cap did not hold");
    }

    /// A row that runs off the edge of the panel is a row nobody can read.
    /// The font is fixed width, so this is arithmetic rather than an
    /// eyeball — and it caught the first layout, which put the rows beside
    /// the scope face where a name plus a range did not fit.
    #[test]
    fn no_row_runs_off_the_edge_of_the_scope() {
        let mut world = stone_world();
        put(&mut world, BlockPos::new(2, 65, 1), "engine:copper_ore");
        put(&mut world, BlockPos::new(-3, 63, 2), "engine:uranium_ore");
        put(&mut world, BlockPos::new(3, 66, -2), "engine:oil_sand");
        let reading = ping(&world, world.registry(), BlockPos::new(0, 65, 0));

        let room = SCOPE_WIDTH - 10;
        for line in lines(&reading) {
            let width = font::text_width(&line, 1);
            assert!(width <= room, "{line:?} is {width}px in a {room}px panel");
        }
    }

    /// A ping with nothing in it still says something.
    #[test]
    fn an_empty_reading_says_so() {
        let reading = Reading::default();
        assert_eq!(lines(&reading), ["NOTHING IN RANGE"]);
        // And the panel still draws, at the size everything else expects.
        let pixels = render_scope(&reading, 0.0);
        assert_eq!(pixels.len(), (SCOPE_WIDTH * SCOPE_HEIGHT * 4) as usize);
    }

    /// The sweep moves, which is the one thing on the panel that is allowed
    /// to differ between two frames.
    #[test]
    fn the_scope_sweep_turns() {
        let reading = Reading::default();
        assert_ne!(render_scope(&reading, 0.0), render_scope(&reading, 0.25));
    }
}
