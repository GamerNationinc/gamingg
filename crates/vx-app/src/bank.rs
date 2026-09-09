//! Banks: the one building in town whose business is holding your things.
//!
//! # Two problems, one strongroom
//!
//! A trade run starts with goods in the wrong place. Until now the only store
//! this game had was the fleet's base pile, which lives wherever you put the
//! container — so hauling to a market meant carrying everything and selling in
//! one lump, and staging a load *near* a town was not a thing you could do.
//! A vault fixes both halves: leave goods in the town you mean to sell them in
//! and come back when the price is right, or park what you cannot afford to
//! lose somewhere with a door on it.
//!
//! # Per town, and derived from nothing
//!
//! Each town keeps its own books, keyed by the town's centre — the same key
//! the economy uses, so a vault and a market always agree about which town
//! they belong to. Unlike almost everything else here the contents are *not*
//! derived: they are exactly what somebody put there, which is the whole
//! point of a bank.
//!
//! # The heaviest lock in the game finally has something behind it
//!
//! Stage 11 built three grades of lockbox and the third has never been
//! stamped anywhere: "endgame; nothing stamps one yet". The bank carries it.
//! Getting in illegitimately is possible, slow, loud and expensive — which is
//! the shape that round was designed around, and it took a building worth
//! robbing to give it a subject.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use vx_agent::Stockpile;
use vx_core::BlockPos;
use vx_render::font::{self, LINE_HEIGHT};

const MAGIC: &[u8; 4] = b"VXVA";
const VERSION: u32 = 1;

/// How much one town's vault holds, in units of anything.
///
/// Finite, because a warehouse with no wall is a pocket. Generous, because
/// the point is staging a trade run rather than rationing shelf space.
pub const CAPACITY: u64 = 6_000;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [220, 190, 90, 255];
const SHORT: [u8; 4] = [235, 110, 90, 255];
const BACKGROUND: [u8; 4] = [14, 12, 8, 240];

/// Every town's books, and the panel that reads them.
#[derive(Debug, Default)]
pub struct Bank {
    /// Keyed by town centre, exactly as the economy keys its markets.
    vaults: BTreeMap<(i32, i32), Stockpile>,
    pub open: bool,
    /// Which town's strongroom is open, and the box it was opened at.
    pub town: Option<(i32, i32)>,
    pub at: Option<BlockPos>,
    pub cursor: usize,
    pub feedback: Option<String>,
}

impl Bank {
    /// What a town holds for you.
    pub fn vault(&self, town: (i32, i32)) -> Option<&Stockpile> {
        self.vaults.get(&town)
    }

    /// How full a town's vault is.
    pub fn stored(&self, town: (i32, i32)) -> u64 {
        self.vaults.get(&town).map_or(0, |vault| vault.total())
    }

    /// Room left in a town's vault.
    pub fn room(&self, town: (i32, i32)) -> u64 {
        CAPACITY.saturating_sub(self.stored(town))
    }

    pub fn open_at(&mut self, at: BlockPos, town: (i32, i32)) {
        self.open = true;
        self.at = Some(at);
        self.town = Some(town);
        self.cursor = 0;
        self.feedback = None;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Move goods in, returning how many were actually taken.
    ///
    /// Returns the amount rather than a yes/no because the vault's capacity
    /// can bite mid-deposit, and a caller that journalled "all of it" while
    /// the vault took two thirds would put the log and the world at odds.
    pub fn deposit(&mut self, town: (i32, i32), good: &str, amount: u64, pile: &mut Stockpile) -> u64 {
        let moved = amount.min(pile.count(good)).min(self.room(town));
        if moved == 0 {
            return 0;
        }
        pile.take(good, moved);
        self.vaults.entry(town).or_default().add(good, moved);
        moved
    }

    /// Move goods out, returning how many were actually handed over.
    pub fn withdraw(&mut self, town: (i32, i32), good: &str, amount: u64, pile: &mut Stockpile) -> u64 {
        let Some(vault) = self.vaults.get_mut(&town) else {
            return 0;
        };
        let moved = amount.min(vault.count(good));
        if moved == 0 {
            return 0;
        }
        vault.take(good, moved);
        pile.add(good, moved);
        moved
    }

    /// The goods this panel lists: everything in the pile or in the vault,
    /// in one stable order so the cursor means the same thing twice.
    pub fn rows(&self, town: (i32, i32), pile: Option<&Stockpile>) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        if let Some(pile) = pile {
            names.extend(pile.entries().map(|(name, _)| name.to_string()));
        }
        if let Some(vault) = self.vaults.get(&town) {
            names.extend(vault.entries().map(|(name, _)| name.to_string()));
        }
        names.sort();
        names.dedup();
        names
    }

    pub fn move_cursor(&mut self, delta: i32, rows: usize) {
        if rows == 0 {
            self.cursor = 0;
            return;
        }
        let last = rows as i32 - 1;
        self.cursor = (self.cursor as i32 + delta).clamp(0, last) as usize;
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "vaults.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.vaults.len() as u32).to_le_bytes())?;
        for ((x, z), vault) in &self.vaults {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            let goods: Vec<(&str, u64)> = vault.entries().collect();
            file.write_all(&(goods.len() as u32).to_le_bytes())?;
            for (name, count) in goods {
                let bytes = name.as_bytes();
                file.write_all(&(bytes.len() as u32).to_le_bytes())?;
                file.write_all(bytes)?;
                file.write_all(&count.to_le_bytes())?;
            }
        }
        file.commit()
    }

    /// Load the books, tolerating absence and damage.
    ///
    /// Name-keyed on disk, like every other store here: a vault holding a
    /// good whose mod has gone reads back as a name nothing resolves, not as
    /// whatever now occupies that number.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("vaults.dat");
        match read_vaults(&path) {
            Ok(Some(vaults)) => self.vaults = vaults,
            Ok(None) => {}
            Err(error) => log::warn!("unreadable {}: {error}", path.display()),
        }
    }
}

type Books = BTreeMap<(i32, i32), Stockpile>;

fn read_vaults(path: &Path) -> std::io::Result<Option<Books>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a vault file"));
    }
    let mut word = [0u8; 4];
    let mut long = [0u8; 8];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    file.read_exact(&mut word)?;
    let towns = u32::from_le_bytes(word);
    let mut vaults = Books::new();
    for _ in 0..towns {
        file.read_exact(&mut word)?;
        let x = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let z = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let goods = u32::from_le_bytes(word);
        let mut vault = Stockpile::new();
        for _ in 0..goods {
            file.read_exact(&mut word)?;
            let mut name = vec![0u8; u32::from_le_bytes(word) as usize];
            file.read_exact(&mut name)?;
            file.read_exact(&mut long)?;
            let name =
                String::from_utf8(name).map_err(|_| std::io::Error::other("garbled good name"))?;
            vault.add(name, u64::from_le_bytes(long));
        }
        vaults.insert((x, z), vault);
    }
    Ok(Some(vaults))
}

pub const VAULT_WIDTH: u32 = 260;
pub const VAULT_HEIGHT: u32 = 168;

/// Draw the strongroom's ledger: what you are carrying beside what is on
/// deposit, one row per good.
pub fn render_vault(bank: &Bank, town: (i32, i32), name: &str, pile: Option<&Stockpile>) -> Vec<u8> {
    let mut pixels = vec![0u8; (VAULT_WIDTH * VAULT_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    font::draw_text(&mut pixels, VAULT_WIDTH, margin, y, 1, ACCENT, name);
    let held = format!("{}/{}", bank.stored(town), CAPACITY);
    font::draw_text(
        &mut pixels,
        VAULT_WIDTH,
        VAULT_WIDTH as i32 - margin - font::text_width(&held, 1) as i32,
        y,
        1,
        if bank.room(town) == 0 { SHORT } else { DIM },
        &held,
    );
    y += LINE_HEIGHT as i32 + 2;
    font::draw_text(&mut pixels, VAULT_WIDTH, margin + 10, y, 1, DIM, "GOOD");
    font::draw_text(&mut pixels, VAULT_WIDTH, margin + 130, y, 1, DIM, "CARRIED");
    font::draw_text(&mut pixels, VAULT_WIDTH, margin + 196, y, 1, DIM, "HELD");
    y += LINE_HEIGHT as i32 + 1;

    let rows = bank.rows(town, pile);
    if rows.is_empty() {
        font::draw_text(&mut pixels, VAULT_WIDTH, margin, y, 1, DIM, "NOTHING TO BANK");
    }
    for (index, good) in rows.iter().enumerate().take(9) {
        let selected = index == bank.cursor;
        if selected {
            font::draw_text(&mut pixels, VAULT_WIDTH, margin, y, 1, ACCENT, ">");
        }
        let colour = if selected { TEXT } else { DIM };
        font::draw_text(
            &mut pixels,
            VAULT_WIDTH,
            margin + 10,
            y,
            1,
            colour,
            &crate::shop::display_name(good),
        );
        let carried = pile.map_or(0, |pile| pile.count(good)).to_string();
        let stored = bank
            .vault(town)
            .map_or(0, |vault| vault.count(good))
            .to_string();
        font::draw_text(&mut pixels, VAULT_WIDTH, margin + 130, y, 1, colour, &carried);
        font::draw_text(&mut pixels, VAULT_WIDTH, margin + 196, y, 1, colour, &stored);
        y += LINE_HEIGHT as i32;
    }

    if let Some(note) = &bank.feedback {
        font::draw_text(
            &mut pixels,
            VAULT_WIDTH,
            margin,
            VAULT_HEIGHT as i32 - LINE_HEIGHT as i32 * 2 - 4,
            1,
            TEXT,
            note,
        );
    }
    font::draw_text(
        &mut pixels,
        VAULT_WIDTH,
        margin,
        VAULT_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
        1,
        DIM,
        "ENTER DEPOSITS. BACKSPACE DRAWS. E LEAVES.",
    );
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pile() -> Stockpile {
        let mut pile = Stockpile::new();
        pile.add("engine:copper_ore", 120);
        pile.add("engine:copper_bar", 8);
        pile
    }

    #[test]
    fn goods_go_in_and_come_back_out() {
        let mut bank = Bank::default();
        let mut carried = pile();
        let town = (0, 0);

        assert_eq!(bank.deposit(town, "engine:copper_ore", 100, &mut carried), 100);
        assert_eq!(carried.count("engine:copper_ore"), 20);
        assert_eq!(bank.stored(town), 100);

        assert_eq!(bank.withdraw(town, "engine:copper_ore", 40, &mut carried), 40);
        assert_eq!(carried.count("engine:copper_ore"), 60);
        assert_eq!(bank.stored(town), 60);
    }

    #[test]
    fn one_town_cannot_spend_another_towns_deposit() {
        // The whole reason a vault is keyed by town: goods left in Stonehaven
        // are in Stonehaven, and walking to the next market does not bring
        // them along.
        let mut bank = Bank::default();
        let mut carried = pile();
        bank.deposit((0, 0), "engine:copper_ore", 50, &mut carried);
        assert_eq!(bank.withdraw((512, 0), "engine:copper_ore", 50, &mut carried), 0);
        assert_eq!(bank.stored((0, 0)), 50);
        assert_eq!(bank.stored((512, 0)), 0);
    }

    #[test]
    fn a_full_vault_takes_what_it_can_and_says_how_much() {
        let mut bank = Bank::default();
        let mut carried = Stockpile::new();
        carried.add("engine:stone", CAPACITY + 500);
        let town = (0, 0);
        let moved = bank.deposit(town, "engine:stone", CAPACITY + 500, &mut carried);
        assert_eq!(moved, CAPACITY, "the vault took more than it holds");
        assert_eq!(carried.count("engine:stone"), 500, "the overflow was eaten");
        assert_eq!(bank.room(town), 0);
        assert_eq!(bank.deposit(town, "engine:stone", 100, &mut carried), 0);
    }

    #[test]
    fn drawing_on_an_empty_vault_moves_nothing() {
        let mut bank = Bank::default();
        let mut carried = Stockpile::new();
        assert_eq!(bank.withdraw((0, 0), "engine:copper_bar", 5, &mut carried), 0);
        assert_eq!(carried.total(), 0, "the bank invented goods");
    }

    #[test]
    fn the_books_survive_a_save() {
        let directory = std::env::temp_dir().join(format!("vx-vault-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut bank = Bank::default();
        let mut carried = pile();
        bank.deposit((0, 0), "engine:copper_ore", 60, &mut carried);
        bank.deposit((512, -512), "engine:copper_bar", 5, &mut carried);
        bank.save(&directory).unwrap();

        let mut loaded = Bank::default();
        loaded.load(&directory);
        assert_eq!(loaded.stored((0, 0)), 60);
        assert_eq!(loaded.stored((512, -512)), 5);
        assert_eq!(
            loaded.vault((0, 0)).unwrap().count("engine:copper_ore"),
            60
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn the_ledger_lists_both_sides_and_draws() {
        let mut bank = Bank::default();
        let mut carried = pile();
        let town = (0, 0);
        bank.deposit(town, "engine:copper_ore", 30, &mut carried);
        let rows = bank.rows(town, Some(&carried));
        assert!(rows.contains(&"engine:copper_ore".to_string()));
        assert!(rows.contains(&"engine:copper_bar".to_string()));
        assert_eq!(
            rows.len(),
            2,
            "a good in both places was listed twice: {rows:?}"
        );

        let picture = render_vault(&bank, town, "STONEHAVEN VAULT", Some(&carried));
        assert_eq!(
            picture,
            render_vault(&bank, town, "STONEHAVEN VAULT", Some(&carried))
        );
        for character in "ENTER DEPOSITS. BACKSPACE DRAWS. E LEAVES.".chars() {
            assert!(font::knows(character), "undrawable {character:?}");
        }
    }
}
