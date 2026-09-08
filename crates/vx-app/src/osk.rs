//! The on-screen keyboard: typing without a keyboard.
//!
//! # Why a pad cannot simply press a letter
//!
//! Every other control the pad reaches is a [`KeyCode`], synthesized into the
//! same `handle_press` the keyboard drives — that is stage 24's whole design
//! and it is why the pad gained every panel for free. Text is the one thing
//! that does not work that way. Typing arrives on `event.text` of a
//! `WindowEvent`, deliberately: a scancode is a *position*, and reading
//! letters off positions is how a game ends up unusable on half the world's
//! keyboards. `poll_pad` never raises a `WindowEvent`, so no binding of any
//! button to any key could ever produce a character.
//!
//! So the pad gets a grid and a cursor, and what it produces goes into
//! [`crate::terminal::Terminal::type_char`] — the same function the window's
//! text lands in, one function, no second implementation of what a letter is.
//!
//! # A pure render function over a cursor
//!
//! Like every panel in this game: the state is a row, a column and a bool,
//! the drawing is a function of that state, and the tests can hold both
//! without a window. The keys are the ones
//! [`vx_render::font`] can actually draw — asking for a character the
//! terminal would silently drop is a key that does nothing, and a key that
//! does nothing is worse than no key.

use vx_render::font::{self, LINE_HEIGHT};

/// The grid, row by row. Every character here is one the font draws and the
/// terminal accepts, which is what makes a keypress mean something.
///
/// Space is the wide key at the end of the last row, written here as itself.
pub const KEYS: [&str; 4] = [
    "ABCDEFGHIJ",
    "KLMNOPQRST",
    "UVWXYZ0123",
    "456789-,. ",
];

/// Columns and rows in the grid.
pub const COLUMNS: usize = 10;
pub const ROWS: usize = 4;

/// The panel's size in texture pixels.
pub const OSK_WIDTH: u32 = 200;
pub const OSK_HEIGHT: u32 = 76;

/// How far the panel is blown up on screen.
pub const OSK_SCALE: f32 = 2.0;

const CELL_WIDTH: i32 = 18;
const CELL_HEIGHT: i32 = 14;

const TEXT: [u8; 4] = [216, 224, 216, 255];
const DIM: [u8; 4] = [120, 136, 120, 255];
const PICKED: [u8; 4] = [255, 190, 70, 255];
const CURSOR: [u8; 4] = [40, 60, 48, 255];
const BACKGROUND: [u8; 4] = [10, 14, 12, 240];

/// Where the cursor is, and whether the board is up at all.
///
/// Down and at the top left by default: a board nobody raised is not up, and
/// one that is starts where a thumb expects it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Osk {
    pub open: bool,
    row: usize,
    column: usize,
}

impl Osk {
    /// Raise the board, cursor back at the top left.
    pub fn open(&mut self) {
        self.open = true;
        self.row = 0;
        self.column = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.column)
    }

    /// Move the cursor, wrapping at every edge.
    ///
    /// Wrapping rather than clamping because a thumbstick has no edges: on a
    /// ten-wide grid, clamping means the traverse from `A` to `.` is a
    /// thirteen-step crawl instead of two.
    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        let columns = COLUMNS as i32;
        let rows = ROWS as i32;
        self.column = (self.column as i32 + dx).rem_euclid(columns) as usize;
        self.row = (self.row as i32 + dy).rem_euclid(rows) as usize;
    }

    /// The character under the cursor.
    pub fn picked(&self) -> char {
        key_at(self.row, self.column)
    }
}

/// The character at a grid position.
pub fn key_at(row: usize, column: usize) -> char {
    KEYS[row.min(ROWS - 1)]
        .chars()
        .nth(column.min(COLUMNS - 1))
        .unwrap_or(' ')
}

/// Draw the board. Pure in the cursor, like every panel here.
pub fn render_osk(osk: &Osk) -> Vec<u8> {
    let mut pixels = vec![0u8; (OSK_WIDTH * OSK_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    font::draw_text(&mut pixels, OSK_WIDTH, margin, 3, 1, DIM, "A CONFIRM  X BACK  START SEND");

    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let x = margin + column as i32 * CELL_WIDTH;
            let y = margin + LINE_HEIGHT as i32 + row as i32 * CELL_HEIGHT;
            let here = (row, column) == osk.cursor();
            if here {
                fill(&mut pixels, x - 3, y - 3, CELL_WIDTH - 1, CELL_HEIGHT - 1, CURSOR);
            }
            let character = key_at(row, column);
            // Space has nothing to draw, so it is drawn as its name.
            let label = if character == ' ' { "SP".to_string() } else { character.to_string() };
            let ink = if here { PICKED } else { TEXT };
            font::draw_text(&mut pixels, OSK_WIDTH, x, y, 1, ink, &label);
        }
    }
    pixels
}

/// A filled rectangle, clipped to the panel.
fn fill(pixels: &mut [u8], x: i32, y: i32, width: i32, height: i32, colour: [u8; 4]) {
    for row in y..y + height {
        for column in x..x + width {
            if row < 0 || column < 0 || row >= OSK_HEIGHT as i32 || column >= OSK_WIDTH as i32 {
                continue;
            }
            let at = ((row as u32 * OSK_WIDTH + column as u32) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key the board offers is one the font draws and the terminal
    /// keeps. A key that types nothing is worse than a key that is not there.
    #[test]
    fn every_key_is_a_character_the_game_can_actually_type() {
        for row in KEYS {
            assert_eq!(row.chars().count(), COLUMNS, "a row is the wrong width: {row:?}");
            for character in row.chars() {
                assert!(font::knows(character), "the font cannot draw {character:?}");
                let mut terminal = crate::terminal::Terminal::default();
                terminal.type_char(character);
                assert_eq!(
                    terminal.typed(),
                    character.to_ascii_uppercase().to_string(),
                    "the terminal dropped {character:?}"
                );
            }
        }
    }

    #[test]
    fn the_whole_alphabet_and_every_digit_are_on_the_board() {
        let all: String = KEYS.concat();
        for character in ('A'..='Z').chain('0'..='9') {
            assert!(all.contains(character), "{character} is not on the board");
        }
        assert!(all.contains(' '), "there is no space key");
    }

    #[test]
    fn the_cursor_wraps_at_every_edge() {
        let mut osk = Osk::default();
        osk.open();
        assert_eq!(osk.cursor(), (0, 0));
        // Left from the first column lands on the last, not nowhere.
        osk.move_cursor(-1, 0);
        assert_eq!(osk.cursor(), (0, COLUMNS - 1));
        osk.move_cursor(0, -1);
        assert_eq!(osk.cursor(), (ROWS - 1, COLUMNS - 1));
        osk.move_cursor(1, 1);
        assert_eq!(osk.cursor(), (0, 0));
    }

    #[test]
    fn the_cursor_picks_the_key_it_is_over() {
        let mut osk = Osk::default();
        osk.open();
        assert_eq!(osk.picked(), 'A');
        osk.move_cursor(1, 0);
        assert_eq!(osk.picked(), 'B');
        osk.move_cursor(0, 1);
        assert_eq!(osk.picked(), 'L');
        // The bottom right is the space key.
        let mut corner = Osk::default();
        corner.open();
        corner.move_cursor(-1, -1);
        assert_eq!(corner.picked(), ' ');
    }

    /// The point of the round: a line typed entirely from the board is the
    /// same line a keyboard would have produced, and the terminal parses it.
    #[test]
    fn a_verb_can_be_typed_from_the_board_alone() {
        let mut osk = Osk::default();
        osk.open();
        let mut terminal = crate::terminal::Terminal::default();
        // Walk to each letter of "FOUND" and press it, exactly as a thumb
        // would: no keyboard anywhere in this test.
        for wanted in "FOUND".chars() {
            let mut steps = 0;
            while osk.picked() != wanted {
                osk.move_cursor(1, 0);
                if osk.cursor().1 == 0 {
                    osk.move_cursor(0, 1);
                }
                steps += 1;
                assert!(steps < ROWS * COLUMNS * 2, "{wanted} is unreachable on the board");
            }
            terminal.type_char(osk.picked());
        }
        assert_eq!(terminal.typed(), "FOUND");
    }

    #[test]
    fn the_board_is_drawable_and_deterministic() {
        let mut osk = Osk::default();
        osk.open();
        let first = render_osk(&osk);
        assert_eq!(first, render_osk(&osk), "the board is not pure in its cursor");
        assert_eq!(first.len(), (OSK_WIDTH * OSK_HEIGHT * 4) as usize);
        // Moving the cursor has to show.
        osk.move_cursor(3, 1);
        assert_ne!(first, render_osk(&osk), "the cursor does not draw");
        // Every row of keys fits inside the panel.
        let bottom = 6 + LINE_HEIGHT as i32 + ROWS as i32 * CELL_HEIGHT;
        assert!(bottom <= OSK_HEIGHT as i32, "the board overflows its panel");
        let right = 6 + COLUMNS as i32 * CELL_WIDTH;
        assert!(right <= OSK_WIDTH as i32, "the board is wider than its panel");
    }
}
