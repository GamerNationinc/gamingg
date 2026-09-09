//! The player's house furniture: one chest, one mailbox.
//!
//! Singular on purpose. The chest is the *home warehouse* — the landing spot
//! for mail and the buffer that is not the fleet's working pile. Goods flow
//! mailbox → chest → base pile → sell or ship; the shop does not sell straight
//! from the chest, because the base pile is where working stock lives and one
//! pile per job keeps every counter honest about what it is spending.
//!
//! # The chest is movable, and its contents are not blocks
//!
//! Breaking the chest block "packs it up": `chest_at` goes to `None` and the
//! contents stay right here in this struct, because they were never stored in
//! the world to begin with — the block is a marker, the side table is the
//! truth, exactly the pattern the fleet's base container established. Placing
//! a chest anywhere puts it back down, contents intact. Only one may stand at
//! a time; the place hook enforces it at the door.
//!
//! The mailbox has no such dance: it is registered unbreakable, town furniture
//! like the counter and the beacon, which deletes every "what if the mailbox
//! is gone when the mail lands" edge case in one line of registry data.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::Stockpile;
use vx_core::BlockPos;
use vx_render::font;

const MAGIC: &[u8; 4] = b"VXHM";
const VERSION: u32 = 1;

/// Panel size in texture pixels, displayed at the shop's scale.
pub const HOME_WIDTH: u32 = 240;
pub const HOME_HEIGHT: u32 = 150;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 235];

/// The player's house furniture and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Homestead {
    /// Where the chest block stands, or `None` while it is packed up.
    pub chest_at: Option<BlockPos>,
    /// The chest's contents. Survives the block being broken.
    pub chest: Stockpile,
    /// Goods that have arrived by mail and not been collected.
    pub mailbox: Stockpile,
}

impl Homestead {
    /// A fresh homestead: the chest stands where the house plan put it.
    pub fn new() -> Self {
        let site = vx_world::town::home_site();
        Homestead {
            chest_at: Some(vx_world::town::chest_position(&site)),
            chest: Stockpile::new(),
            mailbox: Stockpile::new(),
        }
    }

    /// Empty the mailbox into the chest. Works while the chest is packed —
    /// a stockpile is a stockpile wherever it lives.
    ///
    /// Returns how many goods moved.
    pub fn collect(&mut self) -> u64 {
        let moved = self.mailbox.total();
        let arrived: Vec<(String, u64)> = self
            .mailbox
            .entries()
            .map(|(name, count)| (name.to_string(), count))
            .collect();
        for (name, count) in arrived {
            self.mailbox.take(&name, count);
            self.chest.add(name, count);
        }
        moved
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "homestead.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;

        match self.chest_at {
            Some(at) => {
                file.write_all(&[1u8])?;
                file.write_all(&at.x.to_le_bytes())?;
                file.write_all(&at.y.to_le_bytes())?;
                file.write_all(&at.z.to_le_bytes())?;
            }
            None => {
                file.write_all(&[0u8])?;
                file.write_all(&[0u8; 12])?;
            }
        }
        write_pile(&mut file, &self.chest)?;
        write_pile(&mut file, &self.mailbox)?;
        file.commit()
    }

    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("homestead.dat");
        match read_homestead(&path) {
            Ok(Some(read)) => *self = read,
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting fresh", path.display());
                *self = Homestead::new();
            }
        }
    }
}

impl Default for Homestead {
    fn default() -> Self {
        Homestead::new()
    }
}

fn write_pile(file: &mut impl Write, pile: &Stockpile) -> std::io::Result<()> {
    let entries: Vec<(&str, u64)> = pile.entries().collect();
    file.write_all(&(entries.len() as u32).to_le_bytes())?;
    for (name, count) in entries {
        file.write_all(&(name.len() as u32).to_le_bytes())?;
        file.write_all(name.as_bytes())?;
        file.write_all(&count.to_le_bytes())?;
    }
    Ok(())
}

fn read_pile(file: &mut impl Read) -> std::io::Result<Stockpile> {
    let mut pile = Stockpile::new();
    let count = read_u32(file)?;
    if count > 4_096 {
        return Err(std::io::Error::other("implausible pile size"));
    }
    for _ in 0..count {
        let length = read_u32(file)? as usize;
        if length > 256 {
            return Err(std::io::Error::other("implausible name length"));
        }
        let mut bytes = vec![0u8; length];
        file.read_exact(&mut bytes)?;
        let name =
            String::from_utf8(bytes).map_err(|_| std::io::Error::other("name is not text"))?;
        let mut word = [0u8; 8];
        file.read_exact(&mut word)?;
        pile.add(name, u64::from_le_bytes(word));
    }
    Ok(pile)
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_homestead(path: &Path) -> std::io::Result<Option<Homestead>> {
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
    if read_u32(&mut file)? != VERSION {
        return Err(std::io::Error::other("unknown version"));
    }

    let mut flag = [0u8; 1];
    file.read_exact(&mut flag)?;
    let mut coords = [0u8; 12];
    file.read_exact(&mut coords)?;
    let chest_at = (flag[0] == 1).then(|| {
        let word = |i: usize| i32::from_le_bytes(coords[i * 4..i * 4 + 4].try_into().unwrap());
        BlockPos::new(word(0), word(1), word(2))
    });

    let chest = read_pile(&mut file)?;
    let mailbox = read_pile(&mut file)?;
    Ok(Some(Homestead {
        chest_at,
        chest,
        mailbox,
    }))
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// The chest panel's cursor and feedback, the shop's idiom.
#[derive(Debug, Default)]
pub struct HomePanel {
    pub open: bool,
    pub cursor: usize,
    pub feedback: Option<String>,
}

impl HomePanel {
    pub fn open_at_chest(&mut self) {
        self.open = true;
        self.cursor = 0;
        self.feedback = None;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn move_cursor(&mut self, delta: i32, rows: usize) {
        if rows == 0 {
            return;
        }
        let last = rows as i32 - 1;
        self.cursor = (self.cursor as i32 + delta).clamp(0, last) as usize;
    }

    /// Move the selected stack from the chest into the fleet's base pile.
    pub fn confirm(&mut self, homestead: &mut Homestead, base: Option<&mut Stockpile>) {
        let Some(base) = base else {
            self.feedback = Some("NO BASE CONTAINER SET".into());
            return;
        };
        let Some((name, count)) = homestead
            .chest
            .entries()
            .nth(self.cursor)
            .map(|(name, count)| (name.to_string(), count))
        else {
            self.feedback = Some("CHEST IS EMPTY".into());
            return;
        };
        homestead.chest.take(&name, count);
        base.add(&name, count);
        let label = short_name(&name);
        self.feedback = Some(format!("{count} {label} TO THE BASE PILE"));
        self.cursor = 0;
    }
}

/// "engine:copper_ore" reads as "COPPER ORE" on a panel.
fn short_name(name: &str) -> String {
    name.rsplit(':')
        .next()
        .unwrap_or(name)
        .replace('_', " ")
        .to_uppercase()
}

/// Draw the chest panel. Pure in its inputs, like every panel here.
pub fn render_homestead(panel: &HomePanel, homestead: &Homestead) -> Vec<u8> {
    let mut pixels = vec![0u8; (HOME_WIDTH * HOME_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    font::draw_text(&mut pixels, HOME_WIDTH, margin, y, 1, ACCENT, "YOUR CHEST");
    let waiting = homestead.mailbox.total();
    if waiting > 0 {
        let note = format!("MAILBOX: {waiting} WAITING");
        font::draw_text(
            &mut pixels,
            HOME_WIDTH,
            HOME_WIDTH as i32 - margin - font::text_width(&note, 1) as i32,
            y,
            1,
            TEXT,
            &note,
        );
    }
    y += 14;

    let rows: Vec<(String, u64)> = homestead
        .chest
        .entries()
        .map(|(name, count)| (short_name(name), count))
        .collect();
    if rows.is_empty() {
        font::draw_text(&mut pixels, HOME_WIDTH, margin, y, 1, DIM, "EMPTY. FOR NOW.");
        y += 12;
    }
    for (index, (label, count)) in rows.iter().enumerate() {
        let tint = if index == panel.cursor { ACCENT } else { TEXT };
        let marker = if index == panel.cursor { ">" } else { " " };
        let line = format!("{marker} {count} {label}");
        font::draw_text(&mut pixels, HOME_WIDTH, margin, y, 1, tint, &line);
        y += 12;
    }

    if let Some(feedback) = &panel.feedback {
        y += 4;
        font::draw_text(&mut pixels, HOME_WIDTH, margin, y, 1, ACCENT, feedback);
        y += 12;
    }

    let _ = y;
    font::draw_text(
        &mut pixels,
        HOME_WIDTH,
        margin,
        HOME_HEIGHT as i32 - 14,
        1,
        DIM,
        "ARROWS PICK. ENTER MOVES. E LEAVES.",
    );
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaking_the_chest_packs_its_contents_and_replacing_it_unpacks_them() {
        let mut homestead = Homestead::new();
        homestead.chest.add("engine:copper_ore", 40);

        // The break hook does exactly this: forget the block, keep the goods.
        homestead.chest_at = None;
        assert_eq!(homestead.chest.count("engine:copper_ore"), 40, "packing spilled");

        // And the place hook does this.
        let elsewhere = BlockPos::new(300, 64, -120);
        homestead.chest_at = Some(elsewhere);
        assert_eq!(homestead.chest.count("engine:copper_ore"), 40, "moving spilled");
    }

    #[test]
    fn collecting_the_mailbox_moves_everything_into_the_chest_even_while_packed() {
        let mut homestead = Homestead::new();
        homestead.chest_at = None; // packed up
        homestead.mailbox.add("engine:log", 20);
        homestead.mailbox.add("engine:stone", 20);

        assert_eq!(homestead.collect(), 40);
        assert!(homestead.mailbox.is_empty());
        assert_eq!(homestead.chest.count("engine:log"), 20);
        assert_eq!(homestead.chest.count("engine:stone"), 20);

        // A second collection has nothing to say.
        assert_eq!(homestead.collect(), 0);
    }

    #[test]
    fn the_homestead_survives_a_round_trip_through_disk() {
        let directory = std::env::temp_dir().join(format!(
            "vx-homestead-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let mut homestead = Homestead::new();
        homestead.chest_at = Some(BlockPos::new(7, 73, -3));
        homestead.chest.add("engine:copper_bar", 5);
        homestead.mailbox.add("engine:copper_ore", 20);
        homestead.save(&directory).unwrap();

        let mut read_back = Homestead::new();
        read_back.load(&directory);

        // Damage tolerance: garbage loads as a fresh homestead, not a panic.
        std::fs::write(directory.join("homestead.dat"), b"junkjunkjunk").unwrap();
        let mut damaged = Homestead::new();
        damaged.chest.add("engine:log", 99);
        damaged.load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        assert_eq!(read_back, homestead);
        assert_eq!(damaged, Homestead::new(), "damage did not reset cleanly");
    }

    #[test]
    fn a_packed_chest_round_trips_as_packed() {
        let directory = std::env::temp_dir().join(format!(
            "vx-homestead-packed-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let mut homestead = Homestead::new();
        homestead.chest_at = None;
        homestead.chest.add("engine:stone", 12);
        homestead.save(&directory).unwrap();

        let mut read_back = Homestead::new();
        read_back.load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        assert_eq!(read_back.chest_at, None, "the packed state was lost");
        assert_eq!(read_back.chest.count("engine:stone"), 12);
    }

    #[test]
    fn the_panel_renders_deterministically_and_reacts_to_contents() {
        let panel = HomePanel::default();
        let empty = Homestead::new();
        let mut full = Homestead::new();
        full.chest.add("engine:copper_ore", 60);
        full.mailbox.add("engine:log", 20);

        assert_eq!(
            render_homestead(&panel, &empty),
            render_homestead(&panel, &empty),
            "the panel is not a pure function of its inputs"
        );
        assert_ne!(
            render_homestead(&panel, &empty),
            render_homestead(&panel, &full),
            "contents do not show"
        );
    }

    #[test]
    fn confirm_without_a_base_refuses_and_keeps_the_goods() {
        let mut panel = HomePanel::default();
        let mut homestead = Homestead::new();
        homestead.chest.add("engine:stone", 30);

        panel.confirm(&mut homestead, None);

        assert_eq!(homestead.chest.count("engine:stone"), 30);
        assert_eq!(panel.feedback.as_deref(), Some("NO BASE CONTAINER SET"));
    }

    #[test]
    fn confirm_moves_the_selected_stack_into_the_base_pile() {
        let mut panel = HomePanel::default();
        let mut homestead = Homestead::new();
        homestead.chest.add("engine:copper_ore", 25);
        let mut base = Stockpile::new();

        panel.confirm(&mut homestead, Some(&mut base));

        assert_eq!(homestead.chest.count("engine:copper_ore"), 0);
        assert_eq!(base.count("engine:copper_ore"), 25);
    }

    #[test]
    fn every_panel_label_is_drawable() {
        for name in ["engine:copper_ore", "engine:log", "engine:stone", "engine:copper_bar"] {
            for character in short_name(name).chars() {
                assert!(font::knows(character), "undrawable {character:?} in {name}");
            }
        }
    }
}
