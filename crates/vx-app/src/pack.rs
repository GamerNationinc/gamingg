//! What you are carrying, on your back, right now.
//!
//! # Why this file exists
//!
//! Fifty-four stages in, there was no player inventory. Everything you broke
//! went straight onto [`vx_agent::Stockpile`] on `Fleet::base` — a container
//! standing somewhere else entirely, possibly hundreds of blocks away — and
//! the movement system then slowed you down *for carrying it*. That is the
//! oldest entry in the README's rough-edge list, unchanged since stage 16:
//!
//! > Mining sixty blocks makes you walk at 0.55× until you sell, with the
//! > "pack" sat in a container somewhere else entirely.
//!
//! So this module is not a new mechanic so much as pointing a thing that has
//! always existed at the thing it was always describing. The load byte on
//! `Command::Move` has ridden the wire since stage 10b; what fills it is what
//! changes here.
//!
//! # Weight, not count
//!
//! [`Stockpile::total`] sums raw counts, and that sum *was* the load. Nothing
//! in the game had ever ascribed mass to anything — `economy::BASE_PRICE` says
//! what a good is worth, `BlockDef::hardness` says what it costs to cut, and
//! that was the whole list. [`WEIGHTS`] is the third such table, and it is
//! what makes a haul a decision: a pack of leaves and a pack of uranium are
//! the same number of blocks and not remotely the same walk home.
//!
//! [`UNIT`] pins the scale to the old behaviour. One block of plain stone
//! weighs `UNIT`, and the base capacity is the old count times `UNIT`, so a
//! fresh player still carries exactly sixty-four stone. Everything else is
//! measured against that rock.
//!
//! # Why the table is here and not on `BlockDef`
//!
//! Mass is an app concern — `vx-core` describes what a block *is*, and how
//! heavy it feels in a player's hands is a rule of this game rather than a
//! property of the voxel. Keeping it here also keeps the cross-crate surface
//! still. The cost is drift: a block added later could quietly weigh nothing.
//! [`every_block_the_game_knows_has_a_weight`] is the guard, in the shape of
//! `salvage`'s equivalent.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::Stockpile;

const MAGIC: &[u8; 4] = b"VXPK";
const VERSION: u32 = 1;

/// Longest good name accepted, so a damaged file cannot ask for a huge buffer.
const MAX_NAME: u32 = 64;

/// A cap on how many kinds a pack can hold: more than the registry has, and
/// far short of an allocation a damaged file could weaponise.
const MAX_ROWS: u32 = 4_096;

/// What one block of plain stone weighs.
///
/// The whole scale hangs off this. Capacities are the old block counts times
/// `UNIT`, so a fresh player carries sixty-four stone exactly as before and
/// every existing capacity curve keeps its meaning.
pub const UNIT: u64 = 10;

/// What anything unlisted weighs.
///
/// Stone, deliberately: a middling rock is the least surprising guess, and
/// the drift test below means nothing the registry knows can reach this in
/// practice. It exists for goods that are not blocks at all, should the
/// economy ever grow one.
pub const UNLISTED: u64 = UNIT;

/// What each thing weighs, against [`UNIT`].
///
/// Read it as a shape rather than a spreadsheet: leaves and grass are nothing,
/// timber is light for its bulk, rock is the reference, worked metal and ore
/// are heavy, and uranium is the heaviest thing a person can pick up. That
/// ordering is the mechanic; the exact numbers are tuning.
pub const WEIGHTS: &[(&str, u64)] = &[
    // Nothing at all: foliage and litter.
    ("engine:leaves", 1),
    ("engine:needles", 1),
    ("engine:bog_needles", 1),
    ("engine:grass", 1),
    ("engine:tall_grass", 1),
    ("engine:snowy_grass", 1),
    ("engine:ash", 2),
    ("engine:ember", 2),
    ("engine:sphagnum", 2),
    ("engine:snowy_sphagnum", 2),
    // Gas, in cylinders. Bulky and light, which is exactly why a town runs
    // out of oxyhydrogen rather than running out of room for it.
    ("engine:hho_cell", 3),
    ("engine:gas_cell", 3),
    // Timber. Light for its size — the reason a logging run is a different
    // kind of trip from a mining one.
    ("engine:plank", 4),
    ("engine:roof", 4),
    ("engine:catwalk", 4),
    ("engine:footing", 4),
    ("engine:bog_log", 5),
    ("engine:log", 6),
    ("engine:spruce_log", 6),
    ("engine:prime_timber", 7),
    ("engine:ancient_log", 12),
    // Loose ground.
    ("engine:sand", 8),
    ("engine:snowy_sand", 8),
    ("engine:dirt", 8),
    // Rock: the reference.
    ("engine:stone", 10),
    ("engine:ice", 10),
    ("engine:water", 10),
    ("engine:bedrock", 10),
    ("engine:gas_shale", 10),
    ("engine:oil_sand", 11),
    // Fittings and machinery: awkward more than heavy.
    ("engine:supply_cache", 12),
    ("engine:chest", 14),
    ("engine:container", 15),
    ("engine:mailbox", 15),
    ("engine:ward_cot", 15),
    ("engine:roost", 15),
    ("engine:wellhead", 16),
    ("engine:pump", 16),
    ("engine:printer", 18),
    ("engine:electrolyser", 18),
    ("engine:counter", 18),
    ("engine:beacon", 18),
    ("engine:mast", 18),
    ("engine:permit_box_i", 15),
    ("engine:permit_box_ii", 15),
    ("engine:permit_box_iii", 15),
    // Metal, worked or ruined.
    ("engine:rusted_metal", 18),
    ("engine:metal_wall", 20),
    ("engine:rampart", 20),
    ("engine:bunker_shell", 20),
    ("engine:vault", 24),
    // The heavy end: what you came for.
    ("engine:oil_barrel", 22),
    ("engine:copper_ore", 18),
    ("engine:copper_bar", 25),
    ("engine:uranium_ore", 30),
];

/// What one of `name` weighs.
///
/// A linear scan over fifty-odd rows, called once per mined block and once per
/// panel row. Sorting it into a map would buy nothing measurable and would
/// cost the table its readability, which is the part that matters.
pub fn weight_of(name: &str) -> u64 {
    WEIGHTS
        .iter()
        .find(|(good, _)| *good == name)
        .map_or(UNLISTED, |(_, weight)| *weight)
}

/// What you can carry before the frame gives out, in [`UNIT`]s.
///
/// The two curves that have existed since stage 25 and have only ever been
/// applied to a number nobody could see — `skills::capacity` for LOGISTICS and
/// `wallet::pack_capacity` for the PACK line — times `UNIT`, then the
/// exoskeleton on top.
pub fn capacity(logistics: u32, pack_marks: u32, exo_marks: u32) -> u64 {
    let blocks = crate::wallet::pack_capacity(
        crate::skills::capacity(vx_agent::DEFAULT_CAPACITY, logistics),
        pack_marks,
    );
    crate::wallet::exo_capacity(blocks.saturating_mul(UNIT), exo_marks)
}

/// The load you can carry without feeling it.
///
/// Three tenths of the cap bare, rising a tenth per mark of the exoskeleton:
/// the powered frame's whole point is that the first stretch is free, and five
/// marks makes almost the whole pack free. Under this you walk at full speed;
/// over it you slow; at the cap nothing more goes in. That is *slow first,
/// then a hard stop*.
pub fn comfortable(capacity: u64, exo_marks: u32) -> u64 {
    capacity * (3 + exo_marks.min(crate::wallet::MAX_UPGRADE) as u64) / 10
}

/// What you are carrying.
///
/// A [`Stockpile`] and nothing else — the same type the base pile and the
/// house chest use, because a pile is a pile wherever it lives. The cap is
/// caller-side, exactly as `Stockpile`'s own docs insist it must be.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pack {
    goods: Stockpile,
}

impl Pack {
    /// An empty pack.
    pub fn new() -> Self {
        Self::default()
    }

    /// What it all weighs.
    pub fn load(&self) -> u64 {
        self.goods
            .entries()
            .map(|(name, count)| weight_of(name).saturating_mul(count))
            .fold(0u64, u64::saturating_add)
    }

    /// How many of `name` are in there.
    pub fn count(&self, name: &str) -> u64 {
        self.goods.count(name)
    }

    /// How many things, ignoring what they weigh.
    pub fn total(&self) -> u64 {
        self.goods.total()
    }

    /// Nothing in it.
    pub fn is_empty(&self) -> bool {
        self.goods.is_empty()
    }

    /// Every row, in name order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, u64)> {
        self.goods.entries()
    }

    /// Whether one more of `name` would fit.
    pub fn room_for(&self, name: &str, capacity: u64) -> bool {
        self.load().saturating_add(weight_of(name)) <= capacity
    }

    /// Put one of `name` in, if it fits. `true` when it went in.
    ///
    /// One at a time on purpose: the caller is a block that just broke, and
    /// the answer decides whether it lands on your back or on the floor.
    pub fn stow(&mut self, name: &str, capacity: u64) -> bool {
        if !self.room_for(name, capacity) {
            return false;
        }
        self.goods.add(name, 1);
        true
    }

    /// Put `amount` of `name` in, and say how many actually fitted.
    ///
    /// What a drop does when you walk over it: a stack of forty may go in as
    /// eleven, and the rest stays on the ground.
    pub fn take_in(&mut self, name: &str, amount: u64, capacity: u64) -> u64 {
        let weight = weight_of(name).max(1);
        let room = capacity.saturating_sub(self.load());
        let fits = (room / weight).min(amount);
        if fits > 0 {
            self.goods.add(name, fits);
        }
        fits
    }

    /// Empty it, handing back what was in it.
    pub fn drain(&mut self) -> impl Iterator<Item = (String, u64)> {
        self.goods.drain()
    }
}

/// Tip the whole pack into the fleet's pile, and say how many things moved.
///
/// The other half of a carrying limit: something has to empty it. A fleet
/// with no base declared takes nothing and the pack keeps everything, which
/// is the honest answer — there is nowhere to put it down.
///
/// Free rather than a method because the replay runs it too, over its own
/// pack and its own fleet, and neither side may reach for anything else.
pub fn tip(pack: &mut Pack, fleet: &mut vx_agent::Fleet) -> u64 {
    let Some(base) = fleet.base.as_mut() else {
        return 0;
    };
    let mut moved = 0u64;
    for (name, count) in pack.drain() {
        base.stockpile.add(name, count);
        moved = moved.saturating_add(count);
    }
    moved
}

/// How full the pack is, as the byte the journal carries.
///
/// Zero until [`comfortable`], then a ramp to 255 at the cap. The byte then
/// goes through `movement::mass_from_byte` exactly as it always has, so the
/// *feel* of a heavy walk is the one the game has had since stage 10b and only
/// the thing being weighed has changed.
pub fn load_byte(pack: &Pack, capacity: u64, exo_marks: u32) -> u8 {
    let easy = comfortable(capacity, exo_marks);
    let load = pack.load();
    if load <= easy {
        return 0;
    }
    // Through the existing quantiser rather than a second copy of the same
    // arithmetic: `movement::load_byte` has rounded this number into 255ths
    // since stage 10b, and the whole reason it goes through a byte at all is
    // that the journal carries it.
    let span = capacity.saturating_sub(easy);
    crate::movement::load_byte(load - easy, span)
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// How wide the pack panel is drawn.
pub const PANEL_WIDTH: u32 = 220;
/// And how tall. Twelve rows of goods plus a header, a bar and a footer, which
/// is more kinds than the registry can put in one pack in practice.
pub const PANEL_HEIGHT: u32 = 190;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const HEAVY: [u8; 4] = [235, 110, 70, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 235];
const BAR_BACK: [u8; 4] = [38, 40, 48, 255];

/// Rows the panel will draw before it stops and says how many are left.
const ROWS: usize = 12;

/// Draw the pack. Pure in its inputs, like every panel in this game.
///
/// Two numbers matter and both are on it: what you are carrying against what
/// you can, and where the comfortable line falls — because the difference
/// between those two is the whole of *slow first, then a hard stop*, and a
/// player cannot feel a rule they cannot see.
pub fn render_pack(pack: &Pack, capacity: u64, exo_marks: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (PANEL_WIDTH * PANEL_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    let load = pack.load();
    let easy = comfortable(capacity, exo_marks);
    vx_render::font::draw_text(&mut pixels, PANEL_WIDTH, margin, y, 1, ACCENT, "YOUR PACK");
    let weight = format!("{}/{}", load / UNIT, capacity / UNIT);
    vx_render::font::draw_text(
        &mut pixels,
        PANEL_WIDTH,
        PANEL_WIDTH as i32 - margin - vx_render::font::text_width(&weight, 1) as i32,
        y,
        1,
        if load > easy { HEAVY } else { TEXT },
        &weight,
    );
    y += 12;

    // The bar, with the comfortable line marked on it: under the notch you
    // walk at full speed, over it you slow, at the end nothing more goes in.
    draw_bar(&mut pixels, margin as u32, y as u32, PANEL_WIDTH - 12, load, easy, capacity);
    y += 12;
    let note = if load > capacity.saturating_sub(UNIT) {
        "FULL. WHAT YOU CUT WILL DROP."
    } else if load > easy {
        "HEAVY. YOU ARE WALKING SLOWER."
    } else {
        "COMFORTABLE."
    };
    vx_render::font::draw_text(
        &mut pixels,
        PANEL_WIDTH,
        margin,
        y,
        1,
        if load > easy { HEAVY } else { DIM },
        note,
    );
    y += 14;

    if pack.is_empty() {
        vx_render::font::draw_text(&mut pixels, PANEL_WIDTH, margin, y, 1, DIM, "EMPTY.");
    }
    // Heaviest first: what you would put down to make room is the question a
    // pack panel exists to answer, and name order does not answer it. Ties
    // break on the name so the readout cannot reshuffle between frames.
    let mut rows: Vec<(&str, u64)> = pack.entries().collect();
    rows.sort_by(|(left_name, left), (right_name, right)| {
        (weight_of(right_name) * right)
            .cmp(&(weight_of(left_name) * left))
            .then(left_name.cmp(right_name))
    });
    for (name, count) in rows.iter().take(ROWS) {
        let label = crate::shop::display_name(name);
        vx_render::font::draw_text(
            &mut pixels,
            PANEL_WIDTH,
            margin,
            y,
            1,
            TEXT,
            &format!("{count} {label}"),
        );
        let mass = format!("{}", weight_of(name) * count / UNIT);
        vx_render::font::draw_text(
            &mut pixels,
            PANEL_WIDTH,
            PANEL_WIDTH as i32 - margin - vx_render::font::text_width(&mass, 1) as i32,
            y,
            1,
            DIM,
            &mass,
        );
        y += 11;
    }
    if rows.len() > ROWS {
        vx_render::font::draw_text(
            &mut pixels,
            PANEL_WIDTH,
            margin,
            y,
            1,
            DIM,
            &format!("AND {} MORE KINDS", rows.len() - ROWS),
        );
    }

    vx_render::font::draw_text(
        &mut pixels,
        PANEL_WIDTH,
        margin,
        PANEL_HEIGHT as i32 - 13,
        1,
        DIM,
        "E AT A CONTAINER TIPS IT IN.",
    );
    pixels
}

/// The load bar, with the comfortable line notched into it.
///
/// Clipped like everything else that writes into a panel buffer — see the note
/// on `hud::draw_bar`, which was not, and which this round fixed.
fn draw_bar(pixels: &mut [u8], x: u32, y: u32, width: u32, load: u64, easy: u64, capacity: u64) {
    let span = capacity.max(1);
    let filled = (width as u64 * load.min(span) / span) as u32;
    let notch = (width as u64 * easy.min(span) / span) as u32;
    for py in y..(y + 6).min(PANEL_HEIGHT) {
        for px in x..(x + width).min(PANEL_WIDTH) {
            let along = px - x;
            let texel = if along == notch {
                DIM
            } else if along < filled {
                if load > easy { HEAVY } else { ACCENT }
            } else {
                BAR_BACK
            };
            let at = ((py * PANEL_WIDTH + px) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&texel);
        }
    }
}

/// Write the pack to `pack.dat`.
pub fn save(pack: &Pack, directory: &Path) -> std::io::Result<()> {
    let mut file = crate::keeping::begin(directory, "pack.dat")?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    let rows: Vec<(&str, u64)> = pack.entries().collect();
    file.write_all(&(rows.len() as u32).to_le_bytes())?;
    for (name, count) in rows {
        file.write_all(&(name.len() as u32).to_le_bytes())?;
        file.write_all(name.as_bytes())?;
        file.write_all(&count.to_le_bytes())?;
    }
    file.commit()
}

/// Read it back, tolerating absence and damage.
///
/// No file means an empty pack, which is exactly true of a save written before
/// this round. A damaged file is logged and reset — never a failed world.
pub fn load(directory: &Path) -> Pack {
    let path = directory.join("pack.dat");
    match read(&path) {
        Ok(Some(pack)) => pack,
        Ok(None) => Pack::new(),
        Err(error) => {
            log::warn!("ignoring damaged pack at {}: {error}", path.display());
            Pack::new()
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Pack>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a pack file"));
    }
    let mut version = [0u8; 4];
    file.read_exact(&mut version)?;
    let version = u32::from_le_bytes(version);
    if version == 0 || version > VERSION {
        return Ok(None);
    }
    let mut rows = [0u8; 4];
    file.read_exact(&mut rows)?;
    let rows = u32::from_le_bytes(rows);
    if rows > MAX_ROWS {
        return Err(std::io::Error::other("implausible pack manifest"));
    }
    let mut pack = Pack::new();
    for _ in 0..rows {
        let mut length = [0u8; 4];
        file.read_exact(&mut length)?;
        let length = u32::from_le_bytes(length);
        if length > MAX_NAME {
            return Err(std::io::Error::other("implausible good name"));
        }
        let mut name = vec![0u8; length as usize];
        file.read_exact(&mut name)?;
        let name =
            String::from_utf8(name).map_err(|_| std::io::Error::other("a good's name is not text"))?;
        let mut count = [0u8; 8];
        file.read_exact(&mut count)?;
        pack.goods.add(name, u64::from_le_bytes(count));
    }
    Ok(Some(pack))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("vx-pack-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// The whole point of the round, as one assertion.
    #[test]
    fn ore_is_heavier_than_leaves() {
        let mut ore = Pack::new();
        let mut leaves = Pack::new();
        for _ in 0..20 {
            ore.goods.add("engine:uranium_ore", 1);
            leaves.goods.add("engine:leaves", 1);
        }
        assert_eq!(ore.total(), leaves.total(), "the counts must match");
        assert!(
            ore.load() > leaves.load() * 10,
            "twenty of the heaviest thing in the game weighed {} against {} \
             for twenty of the lightest",
            ore.load(),
            leaves.load()
        );
        // And the ordering the table is *for*, spelled out so a later tuning
        // pass cannot quietly invert it.
        assert!(weight_of("engine:leaves") < weight_of("engine:log"));
        assert!(weight_of("engine:log") < weight_of("engine:stone"));
        assert!(weight_of("engine:stone") < weight_of("engine:copper_ore"));
        assert!(weight_of("engine:copper_ore") < weight_of("engine:copper_bar"));
        assert!(weight_of("engine:copper_bar") < weight_of("engine:uranium_ore"));
    }

    /// The drift guard, in the shape of `salvage`'s: a block added by a later
    /// stage cannot quietly weigh nothing.
    #[test]
    fn every_block_the_game_knows_has_a_weight() {
        let mut registry = vx_core::BlockRegistry::new();
        vx_world::gen::TerrainBlocks::register_builtins(&mut registry);
        let listed: Vec<&str> = WEIGHTS.iter().map(|(name, _)| *name).collect();
        let mut missing = Vec::new();
        for id in 0..u16::MAX {
            let Some(def) = registry.get(vx_core::BlockId(id)) else {
                continue;
            };
            // Air is the one block nobody carries: it is the absence the
            // registry has to name, not a thing with a mass.
            if def.name == "engine:air" {
                continue;
            }
            if !listed.contains(&def.name.as_str()) {
                missing.push(def.name.clone());
            }
        }
        assert!(missing.is_empty(), "blocks with no weight: {missing:?}");
        // The goods the economy trades are the same names, and must also be
        // there — a good the shop sells and the pack cannot weigh would be a
        // hole in the loop rather than in the table.
        for good in crate::economy::GOODS {
            assert!(listed.contains(&good), "{good} has no weight");
        }
        // And nothing in the table is a name nothing answers to, which is how
        // a typo would otherwise hide as a default.
        for (name, weight) in WEIGHTS {
            assert!(registry.id_of(name).is_some(), "{name} is not a block");
            assert!(*weight > 0, "{name} weighs nothing at all");
        }
    }

    /// A fresh player carries exactly what they always did.
    #[test]
    fn the_stock_pack_still_holds_sixty_four_stone() {
        let cap = capacity(1, 0, 0);
        assert_eq!(cap, vx_agent::DEFAULT_CAPACITY * UNIT);
        let mut pack = Pack::new();
        let mut stowed = 0;
        while pack.stow("engine:stone", cap) {
            stowed += 1;
        }
        assert_eq!(stowed, 64, "the stock pack changed size");
        // The same frame full of uranium is a great deal less of it.
        let mut heavy = Pack::new();
        let mut ore = 0;
        while heavy.stow("engine:uranium_ore", cap) {
            ore += 1;
        }
        assert_eq!(ore, 21);
    }

    #[test]
    fn the_pack_slows_first_and_then_stops() {
        let cap = capacity(1, 0, 0);
        let mut pack = Pack::new();
        // The first stretch is free.
        for _ in 0..15 {
            assert!(pack.stow("engine:stone", cap));
        }
        assert_eq!(load_byte(&pack, cap, 0), 0, "fifteen stone should be free");
        // Then it tells, and keeps telling, without ever going backwards.
        let mut last = 0;
        while pack.stow("engine:stone", cap) {
            let now = load_byte(&pack, cap, 0);
            assert!(now >= last, "the load went down as the pack filled");
            last = now;
        }
        assert_eq!(last, 255, "a full pack is not a full load byte");
        // And then it is a hard stop: the next rock does not go in.
        assert!(!pack.stow("engine:stone", cap));
        assert!(!pack.room_for("engine:leaves", cap));
    }

    #[test]
    fn the_frame_carries_more_and_carries_it_easier() {
        let bare = capacity(1, 0, 0);
        let fitted = capacity(1, 0, crate::wallet::MAX_UPGRADE);
        assert!(fitted > bare, "five marks of frame bought nothing");
        // Half a bare pack: heavy on your back, nothing in the frame.
        let mut pack = Pack::new();
        for _ in 0..40 {
            pack.goods.add("engine:stone", 1);
        }
        assert!(load_byte(&pack, bare, 0) > 0);
        assert_eq!(load_byte(&pack, fitted, crate::wallet::MAX_UPGRADE), 0);
    }

    #[test]
    fn a_drop_goes_in_as_far_as_it_fits() {
        let cap = capacity(1, 0, 0);
        let mut pack = Pack::new();
        // 640 units of room, uranium at 30: twenty-one fit and the rest does
        // not, which is what leaves a smaller pile on the floor.
        assert_eq!(pack.take_in("engine:uranium_ore", 40, cap), 21);
        assert_eq!(pack.count("engine:uranium_ore"), 21);
        assert_eq!(pack.take_in("engine:uranium_ore", 40, cap), 0);
        // Something lighter still slips into the gap that is left.
        assert_eq!(pack.load(), 630);
        assert_eq!(pack.take_in("engine:leaves", 40, cap), 10);
    }

    #[test]
    fn a_pack_round_trips_through_disk() {
        let directory = scratch("round-trip");
        let mut pack = Pack::new();
        pack.goods.add("engine:copper_ore", 12);
        pack.goods.add("engine:log", 3);
        save(&pack, &directory).unwrap();
        let read_back = load(&directory);
        assert_eq!(read_back, pack);
        assert_eq!(read_back.load(), pack.load());
    }

    #[test]
    fn a_missing_or_damaged_pack_is_an_empty_pack() {
        let directory = scratch("damage");
        assert!(load(&directory).is_empty(), "a fresh world starts carrying");
        std::fs::write(directory.join("pack.dat"), b"VXPKnonsense").unwrap();
        assert!(load(&directory).is_empty(), "damage should reset, not crash");
    }

    #[test]
    fn an_unlisted_good_still_weighs_something() {
        // Not reachable from the registry — the test above proves that — but
        // a good the economy grew and the table missed must not be free.
        assert_eq!(weight_of("engine:something_new"), UNLISTED);
        assert_ne!(UNLISTED, 0, "an unlisted good would be free to carry");
    }
}
