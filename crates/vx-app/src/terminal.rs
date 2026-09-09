//! The terminal: the font's third user, and the game's first typed input.
//!
//! # Why a console in a game with panels
//!
//! Every readout so far answers one question in one place — the HUD says how
//! you are, the handheld says where your machines are, a shop panel says what
//! is on the shelf. None of them answer *"what just happened"*, because the
//! answer has been a toast that fades in three seconds. A terminal keeps a
//! scrollback, so the level-up you missed while looking at a wall is still
//! there when you open it.
//!
//! And it is the first place in this game a player types. That is worth its
//! own round: a caret, editing behind the caret, a command history on the
//! arrows, and a parser that refuses politely — none of which the bitmap font
//! had ever been asked for.
//!
//! # Orders go the long way round on purpose
//!
//! Nothing here writes to the world. A command that changes anything returns
//! an [`Order`] for `main` to carry out through the very same call the keys
//! and panels use, so the journal sees one kind of dispatch and not two. A
//! terminal that recorded its own orders would be a second implementation of
//! every rule in this game, and the first one to drift.

use std::collections::VecDeque;

use vx_render::font::{self, LINE_HEIGHT};

/// How many lines of scrollback to keep. Enough for a long session's worth of
/// notable events, bounded so a machine left running overnight cannot eat the
/// heap one toast at a time.
pub const SCROLLBACK: usize = 400;

/// Lines shown at once.
pub const WINDOW: usize = 14;

/// The longest line the panel draws before wrapping.
pub const COLUMNS: usize = 56;

/// How many past commands the arrows walk.
const HISTORY: usize = 64;

pub const TERM_WIDTH: u32 = 360;
pub const TERM_HEIGHT: u32 = 210;

const TEXT: [u8; 4] = [216, 224, 216, 255];
const DIM: [u8; 4] = [120, 136, 120, 255];
const ECHO: [u8; 4] = [120, 200, 240, 255];
const WARN: [u8; 4] = [235, 140, 90, 255];
const PROMPT: [u8; 4] = [150, 220, 150, 255];
const BACKGROUND: [u8; 4] = [8, 12, 10, 242];

/// What a line in the scrollback is, which is only ever how it is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something the game said.
    Note,
    /// Something the player typed, echoed back.
    Echo,
    /// A refusal, or something that went wrong.
    Warn,
}

/// An order the terminal wants carried out.
///
/// Deliberately data rather than an effect: the terminal decides *what*, and
/// `main` decides how, through the same paths the keys already use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Order {
    /// Close the terminal.
    Close,
    /// Send the fleet's scout somewhere.
    Scout(crate::journal::ScoutOrder),
    /// Start the marked dig.
    Dig,
    /// Drop the marked plan.
    Cancel,
    /// Sweep the sector the player stands in.
    Survey,
    /// Save the world.
    Save,
    /// Turn the optics dial.
    Lights,
}

/// One command the terminal understands.
struct Verb {
    name: &'static str,
    help: &'static str,
}

/// Everything the terminal answers to, in the order `help` lists them.
const VERBS: &[Verb] = &[
    Verb { name: "help", help: "THIS LIST" },
    Verb { name: "status", help: "SKILLS, BOUNTY, FUEL, THE HOUR" },
    Verb { name: "fleet", help: "EVERY MACHINE AND WHAT IT IS DOING" },
    Verb { name: "where", help: "YOUR POSITION AND THE NEAREST TOWN" },
    Verb { name: "bank", help: "WHAT THIS TOWN HOLDS FOR YOU" },
    Verb { name: "pile", help: "WHAT IS ON THE BASE PILE" },
    Verb { name: "payroll", help: "WHAT THE CREW EARNS, AND WHERE TO SELL IT" },
    Verb { name: "kit", help: "EVERY UPGRADE LINE AND WHAT IS FITTED" },
    Verb { name: "repair", help: "MEND THE WORST MACHINE, OR REPAIR DIGGER 2" },
    Verb { name: "patch", help: "SPEND A MEDKIT ON YOURSELF, WHEREVER YOU ARE" },
    Verb { name: "wells", help: "EVERY HOLE YOU HAVE SUNK, AND WHAT IT IS DOING" },
    Verb { name: "weather", help: "THE SKY, THE WIND, AND HOW DRY THE WOODS ARE" },
    Verb { name: "law", help: "WHAT THE DEPUTIES ARE DOING, AND HOW THEIR NERVE IS" },
    Verb { name: "standing", help: "YOUR NAME WITH THE TOWNS AND THE SHELTERS" },
    Verb { name: "who", help: "THE TOWN ROSTER, AND WHERE EVERYBODY IS" },
    Verb { name: "town", help: "WHO RUNS THIS PLACE, WHAT IT EARNS, AND THE WARRANT" },
    Verb { name: "found", help: "FOUND IRON REACH: FILE A CHARTER ON OPEN GROUND" },
    Verb { name: "take", help: "SEIZE THIS TOWN'S CHAIRS, ONCE ITS DEPUTIES ARE DOWN" },
    Verb { name: "talk", help: "A WORD WITH THE NEAREST TOWNSFOLK" },
    Verb { name: "gift", help: "GIFT GOOD, HAND THE NEAREST PERSON ONE" },
    Verb { name: "scout", help: "ORBIT, DOCK, PERCH, VANGUARD, SORTIE X Z" },
    Verb { name: "dig", help: "SEND THE CREW AT THE MARKED PLAN" },
    Verb { name: "heap", help: "STACK THE SPOIL: PYRAMID, SPIRAL OR SHAFT" },
    Verb { name: "cancel", help: "DROP THE MARKED PLAN" },
    Verb { name: "survey", help: "SWEEP THE SECTOR YOU STAND IN" },
    Verb { name: "lights", help: "TURN THE OPTICS DIAL" },
    Verb { name: "drill", help: "THE DRILL MOD: DRILL HOLO, DRILL PING" },
    Verb { name: "pack", help: "WHAT YOU ARE CARRYING, AND WHAT IT WEIGHS" },
    Verb { name: "save", help: "WRITE THE WORLD OUT" },
    Verb { name: "clear", help: "EMPTY THE SCROLLBACK" },
    Verb { name: "exit", help: "CLOSE THE TERMINAL" },
];

/// What the parser made of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Run this order, and print these lines first.
    Run(Order),
    /// Answer a question `main` has to look up: the verb, and its arguments.
    Ask(String, Vec<String>),
    /// Print these lines and do nothing.
    Say(Vec<String>),
    /// Nothing was typed.
    Empty,
    /// Say this, in the warning colour.
    Refuse(String),
}

/// Read one typed line.
///
/// Pure, and the whole reason the terminal is testable without a window: a
/// parser that reached into the world would need one to test at all.
pub fn parse(line: &str) -> Parsed {
    let mut words = line.split_whitespace();
    let Some(verb) = words.next() else {
        return Parsed::Empty;
    };
    let verb = verb.to_ascii_lowercase();
    let rest: Vec<String> = words.map(|word| word.to_ascii_lowercase()).collect();

    match verb.as_str() {
        "help" | "?" => Parsed::Say(
            VERBS
                .iter()
                .map(|entry| format!("{:<8} {}", entry.name.to_uppercase(), entry.help))
                .collect(),
        ),
        "clear" => Parsed::Say(Vec::new()),
        "exit" | "quit" | "close" => Parsed::Run(Order::Close),
        "dig" => Parsed::Run(Order::Dig),
        "cancel" => Parsed::Run(Order::Cancel),
        "survey" | "sweep" => Parsed::Run(Order::Survey),
        "lights" | "lamp" => Parsed::Run(Order::Lights),
        "save" => Parsed::Run(Order::Save),
        "status" | "fleet" | "where" | "bank" | "pile" | "who" | "talk" | "gift"
        | "kit" | "repair" | "law" | "standing" | "wells" | "patch" => Parsed::Ask(verb, rest),
        // The books, in one place: what the crew cuts an hour, what the pile
        // is worth here, and which counter would pay more — and can.
        "payroll" | "books" | "wages" => Parsed::Ask("payroll".into(), rest),
        // The drill mod. Bare, it reports both switches and the last ping;
        // with an argument it flips one. It is here as well as on `H` and
        // `P` because a player on a pad who never finds SELECT and a stick
        // click should not be locked out of a feature that is free.
        "drill" | "holo" | "sonar" | "ping" => Parsed::Ask("drill".into(), rest),
        "pack" | "carrying" | "inventory" => Parsed::Ask("pack".into(), rest),
        "heap" | "spoil" | "stack" => Parsed::Ask("heap".into(), rest),
        "weather" | "sky" | "forecast" => Parsed::Ask("weather".into(), rest),
        "town" | "hall" | "council" => Parsed::Ask("town".into(), rest),
        "found" | "charter" => Parsed::Ask("found".into(), rest),
        "take" | "seize" => Parsed::Ask("take".into(), rest),
        "scout" => match rest.first().map(String::as_str) {
            Some("orbit") => Parsed::Run(Order::Scout(crate::journal::ScoutOrder::Orbit)),
            Some("dock") | Some("home") => {
                Parsed::Run(Order::Scout(crate::journal::ScoutOrder::Dock))
            }
            Some("vanguard") | Some("ahead") => {
                Parsed::Run(Order::Scout(crate::journal::ScoutOrder::Vanguard))
            }
            Some("perch") => Parsed::Ask("scout-perch".into(), rest),
            Some("sortie") => match (rest.get(1), rest.get(2)) {
                (Some(x), Some(z)) => match (x.parse::<i32>(), z.parse::<i32>()) {
                    (Ok(x), Ok(z)) => {
                        Parsed::Run(Order::Scout(crate::journal::ScoutOrder::Sortie { x, z }))
                    }
                    _ => Parsed::Refuse("SORTIE WANTS TWO WHOLE NUMBERS".into()),
                },
                _ => Parsed::Refuse("SORTIE WANTS X AND Z".into()),
            },
            Some(other) => Parsed::Refuse(format!("NO SUCH ORDER: {}", other.to_uppercase())),
            None => Parsed::Refuse("SCOUT WANTS AN ORDER. TRY HELP".into()),
        },
        other => Parsed::Refuse(format!("NO SUCH COMMAND: {}", other.to_uppercase())),
    }
}

/// The console's whole state.
#[derive(Debug, Default)]
pub struct Terminal {
    pub open: bool,
    /// What is being typed, and where the caret sits *in characters*.
    input: String,
    caret: usize,
    lines: VecDeque<(Kind, String)>,
    history: Vec<String>,
    /// How far back the arrows have walked, counting from the newest.
    recalled: Option<usize>,
    /// Lines scrolled back from the bottom.
    scroll: usize,
}

impl Terminal {
    /// What is on the input line right now.
    ///
    /// Test-only: the game reads the line through `render_terminal`, and the
    /// on-screen keyboard's tests need to see that a key actually landed.
    #[cfg(test)]
    pub fn typed(&self) -> &str {
        &self.input
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open && self.lines.is_empty() {
            self.say(Kind::Note, "TERMINAL READY. TYPE HELP.");
        }
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Everything in the scrollback, oldest first.
    ///
    /// The renderer walks the deque directly for the window it draws; this is
    /// how a test reads the whole log without one.
    #[cfg(test)]
    pub fn lines(&self) -> impl Iterator<Item = &(Kind, String)> {
        self.lines.iter()
    }

    /// Add a line, wrapping it to the panel's width and dropping the oldest
    /// once the scrollback is full.
    pub fn say(&mut self, kind: Kind, line: impl AsRef<str>) {
        for chunk in wrap(line.as_ref(), COLUMNS) {
            self.lines.push_back((kind, chunk));
            while self.lines.len() > SCROLLBACK {
                self.lines.pop_front();
            }
        }
        // A new line pins the view to the bottom: a console that kept your
        // scroll position while the world talked would hide the thing you
        // opened it to read.
        self.scroll = 0;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll = 0;
    }

    /// Type one character. Anything the font cannot draw is refused rather
    /// than stored, so a line can never contain something the panel would
    /// have to render as a hole.
    pub fn type_char(&mut self, character: char) {
        let character = character.to_ascii_uppercase();
        if !font::knows(character) || self.input.chars().count() >= COLUMNS - 2 {
            return;
        }
        let at = byte_at(&self.input, self.caret);
        self.input.insert(at, character);
        self.caret += 1;
    }

    pub fn backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        let from = byte_at(&self.input, self.caret - 1);
        let to = byte_at(&self.input, self.caret);
        self.input.replace_range(from..to, "");
        self.caret -= 1;
    }

    pub fn delete(&mut self) {
        if self.caret >= self.input.chars().count() {
            return;
        }
        let from = byte_at(&self.input, self.caret);
        let to = byte_at(&self.input, self.caret + 1);
        self.input.replace_range(from..to, "");
    }

    pub fn move_caret(&mut self, delta: i32) {
        let last = self.input.chars().count() as i32;
        self.caret = (self.caret as i32 + delta).clamp(0, last) as usize;
    }

    pub fn caret_home(&mut self) {
        self.caret = 0;
    }

    pub fn caret_end(&mut self) {
        self.caret = self.input.chars().count();
    }

    /// Walk the command history. `-1` is older, `+1` newer.
    pub fn recall(&mut self, delta: i32) {
        if self.history.is_empty() {
            return;
        }
        let newest = self.history.len() - 1;
        self.recalled = match (self.recalled, delta) {
            (None, -1) => Some(newest),
            (Some(0), -1) => Some(0),
            (Some(at), -1) => Some(at - 1),
            (Some(at), _) if at >= newest => None,
            (Some(at), _) => Some(at + 1),
            (None, _) => None,
        };
        self.input = self
            .recalled
            .and_then(|at| self.history.get(at))
            .cloned()
            .unwrap_or_default();
        self.caret_end();
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let deepest = self.lines.len().saturating_sub(WINDOW) as i32;
        self.scroll = (self.scroll as i32 + delta).clamp(0, deepest.max(0)) as usize;
    }

    /// Take the typed line, echo it, remember it, and hand back what it meant.
    pub fn submit(&mut self) -> Parsed {
        let line = std::mem::take(&mut self.input);
        self.caret = 0;
        self.recalled = None;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Parsed::Empty;
        }
        self.say(Kind::Echo, format!("> {trimmed}"));
        // Consecutive repeats are not worth a history slot: holding the same
        // command down would otherwise push everything else out of reach.
        if self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_string());
            while self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        let parsed = parse(trimmed);
        match &parsed {
            Parsed::Say(lines) => {
                if lines.is_empty() {
                    self.clear();
                } else {
                    for line in lines.clone() {
                        self.say(Kind::Note, line);
                    }
                }
            }
            Parsed::Refuse(reason) => {
                let reason = reason.clone();
                self.say(Kind::Warn, reason);
            }
            _ => {}
        }
        parsed
    }
}

/// The byte offset of the nth character.
fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(at, _)| at)
}

/// Break a line at the panel's width, on spaces where there is one.
pub fn wrap(line: &str, columns: usize) -> Vec<String> {
    let cleaned: String = line
        .to_uppercase()
        .chars()
        .map(|character| if font::knows(character) { character } else { ' ' })
        .collect();
    if cleaned.chars().count() <= columns {
        return vec![cleaned];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in cleaned.split(' ') {
        // A word longer than the whole line is cut rather than dropped.
        if word.chars().count() > columns {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            let mut rest = word;
            while rest.chars().count() > columns {
                let split = byte_at(rest, columns);
                out.push(rest[..split].to_string());
                rest = &rest[split..];
            }
            current = rest.to_string();
            continue;
        }
        let would = current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
        if would > columns {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Draw the console. Pure in its state, like every other panel here.
///
/// `blink` is passed in rather than read from a clock: a panel that consulted
/// wall time would make two captures of the same state differ.
pub fn render_terminal(terminal: &Terminal, blink: bool) -> Vec<u8> {
    let mut pixels = vec![0u8; (TERM_WIDTH * TERM_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    font::draw_text(&mut pixels, TERM_WIDTH, margin, y, 1, PROMPT, "TERMINAL");
    if terminal.scroll > 0 {
        let note = format!("-{} LINES", terminal.scroll);
        font::draw_text(
            &mut pixels,
            TERM_WIDTH,
            TERM_WIDTH as i32 - margin - font::text_width(&note, 1) as i32,
            y,
            1,
            DIM,
            &note,
        );
    }
    y += LINE_HEIGHT as i32 + 3;

    // The window sits `scroll` lines back from the newest.
    let total = terminal.lines.len();
    let end = total.saturating_sub(terminal.scroll);
    let start = end.saturating_sub(WINDOW);
    for (kind, line) in terminal.lines.iter().take(end).skip(start) {
        let colour = match kind {
            Kind::Note => TEXT,
            Kind::Echo => ECHO,
            Kind::Warn => WARN,
        };
        font::draw_text(&mut pixels, TERM_WIDTH, margin, y, 1, colour, line);
        y += LINE_HEIGHT as i32;
    }

    // The prompt sits on the bottom rule whatever the scrollback is doing.
    let prompt_y = TERM_HEIGHT as i32 - LINE_HEIGHT as i32 - margin;
    font::draw_text(&mut pixels, TERM_WIDTH, margin, prompt_y, 1, PROMPT, ">");
    font::draw_text(
        &mut pixels,
        TERM_WIDTH,
        margin + 10,
        prompt_y,
        1,
        TEXT,
        terminal.input(),
    );
    if blink {
        let ahead: String = terminal.input().chars().take(terminal.caret()).collect();
        let caret_x = margin + 10 + font::text_width(&ahead, 1) as i32;
        for row in 0..LINE_HEIGHT as i32 {
            for column in 0..2 {
                let px = ((prompt_y + row) * TERM_WIDTH as i32 + caret_x + column) as usize * 4;
                if px + 4 <= pixels.len() {
                    pixels[px..px + 4].copy_from_slice(&PROMPT);
                }
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_editing_and_the_caret_agree() {
        let mut terminal = Terminal::default();
        for character in "status".chars() {
            terminal.type_char(character);
        }
        assert_eq!(terminal.input(), "STATUS");
        assert_eq!(terminal.caret(), 6);

        // Edit behind the caret, which is the part a naive push-only line
        // gets wrong.
        terminal.move_caret(-2);
        terminal.backspace();
        assert_eq!(terminal.input(), "STAUS");
        assert_eq!(terminal.caret(), 3);
        terminal.type_char('t');
        assert_eq!(terminal.input(), "STATUS");

        terminal.caret_home();
        terminal.delete();
        assert_eq!(terminal.input(), "TATUS");
        terminal.caret_end();
        assert_eq!(terminal.caret(), 5);
    }

    #[test]
    fn undrawable_characters_never_enter_a_line() {
        // The font is the contract: a line that held something it cannot draw
        // would render as a hole, and there is nowhere good to discover that.
        let mut terminal = Terminal::default();
        for character in ['é', '\u{1}', '☃'] {
            terminal.type_char(character);
        }
        assert_eq!(terminal.input(), "");
        terminal.say(Kind::Note, "SNOWMAN ☃ HERE");
        for (_, line) in terminal.lines() {
            for character in line.chars() {
                assert!(font::knows(character), "undrawable {character:?}");
            }
        }
    }

    #[test]
    fn the_history_walks_both_ways_and_skips_repeats() {
        let mut terminal = Terminal::default();
        for line in ["status", "fleet", "fleet", "where"] {
            for character in line.chars() {
                terminal.type_char(character);
            }
            terminal.submit();
        }
        terminal.recall(-1);
        assert_eq!(terminal.input(), "WHERE");
        terminal.recall(-1);
        assert_eq!(terminal.input(), "FLEET");
        terminal.recall(-1);
        assert_eq!(terminal.input(), "STATUS", "a repeat took a history slot");
        terminal.recall(-1);
        assert_eq!(terminal.input(), "STATUS", "walking past the oldest");
        terminal.recall(1);
        assert_eq!(terminal.input(), "FLEET");
        terminal.recall(1);
        terminal.recall(1);
        assert_eq!(terminal.input(), "", "walking past the newest clears the line");
    }

    #[test]
    fn the_parser_reads_orders_and_refuses_politely() {
        assert_eq!(parse(""), Parsed::Empty);
        assert_eq!(parse("dig"), Parsed::Run(Order::Dig));
        assert_eq!(parse("  CANCEL  "), Parsed::Run(Order::Cancel));
        assert_eq!(
            parse("scout sortie -40 120"),
            Parsed::Run(Order::Scout(crate::journal::ScoutOrder::Sortie {
                x: -40,
                z: 120
            }))
        );
        assert!(matches!(parse("scout sortie 4"), Parsed::Refuse(_)));
        assert!(matches!(parse("scout sortie a b"), Parsed::Refuse(_)));
        assert!(matches!(parse("frobnicate"), Parsed::Refuse(_)));
        assert!(matches!(parse("status"), Parsed::Ask(_, _)));
        // Every refusal is drawable, which is the only way a player reads it.
        for line in ["scout nowhere", "frobnicate", "scout sortie x y"] {
            if let Parsed::Refuse(reason) = parse(line) {
                for character in reason.chars() {
                    assert!(font::knows(character), "undrawable {character:?} in {reason}");
                }
            }
        }
    }

    #[test]
    fn help_names_every_verb_and_draws() {
        let Parsed::Say(lines) = parse("help") else {
            panic!("help said nothing");
        };
        assert_eq!(lines.len(), VERBS.len());
        for line in &lines {
            for character in line.chars() {
                assert!(font::knows(character), "undrawable {character:?} in {line}");
            }
        }
        // And every verb the help lists is one the parser recognises. A bare
        // verb that needs arguments may still refuse — but it must refuse by
        // asking for them, not by claiming never to have heard of itself.
        for verb in VERBS {
            if let Parsed::Refuse(reason) = parse(verb.name) {
                assert!(
                    !reason.starts_with("NO SUCH COMMAND"),
                    "help lists {} but the parser does not know it",
                    verb.name
                );
            }
        }
    }

    #[test]
    fn the_scrollback_is_capped_and_wraps() {
        let mut terminal = Terminal::default();
        for step in 0..SCROLLBACK + 50 {
            terminal.say(Kind::Note, format!("LINE {step}"));
        }
        assert_eq!(terminal.lines().count(), SCROLLBACK);
        assert_eq!(
            terminal.lines().next().map(|(_, line)| line.as_str()),
            Some("LINE 50"),
            "the wrong end was dropped"
        );

        let long = "A".repeat(COLUMNS * 2 + 5);
        let wrapped = wrap(&long, COLUMNS);
        assert!(wrapped.len() >= 3);
        for chunk in &wrapped {
            assert!(chunk.chars().count() <= COLUMNS);
        }
        let sentence = wrap(
            "THE FLEET IS DRY AND THE NEAREST ELECTROLYSER IS A LONG WALK SOUTH OF HERE",
            24,
        );
        assert!(sentence.len() > 1);
        for chunk in &sentence {
            assert!(chunk.chars().count() <= 24, "{chunk:?} runs past the edge");
        }
    }

    #[test]
    fn submitting_echoes_clears_and_answers() {
        let mut terminal = Terminal::default();
        for character in "help".chars() {
            terminal.type_char(character);
        }
        let parsed = terminal.submit();
        assert!(matches!(parsed, Parsed::Say(_)));
        assert_eq!(terminal.input(), "", "the line was not taken");
        assert_eq!(
            terminal.lines().next().map(|(kind, _)| *kind),
            Some(Kind::Echo),
            "the command was not echoed"
        );

        for character in "clear".chars() {
            terminal.type_char(character);
        }
        terminal.submit();
        assert_eq!(terminal.lines().count(), 0, "clear left something behind");
    }

    #[test]
    fn the_panel_is_deterministic_and_shows_the_caret() {
        let mut terminal = Terminal::default();
        terminal.toggle();
        for character in "scout orbit".chars() {
            terminal.type_char(character);
        }
        assert_eq!(
            render_terminal(&terminal, true),
            render_terminal(&terminal, true)
        );
        assert_ne!(
            render_terminal(&terminal, true),
            render_terminal(&terminal, false),
            "the caret never blinked"
        );
        // Scrolling back changes the picture once there is anything to scroll.
        for step in 0..40 {
            terminal.say(Kind::Note, format!("LINE {step}"));
        }
        let bottom = render_terminal(&terminal, false);
        terminal.scroll_by(6);
        assert_ne!(bottom, render_terminal(&terminal, false), "scrolling did nothing");
    }
}
