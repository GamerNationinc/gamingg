//! The welcome panel: hello, here is your house, and here is the whole story.
//!
//! # The changelog cannot rot
//!
//! The panel's changelog is parsed out of `ROADMAP.md` at compile time —
//! `include_str!`, not a hand-maintained copy. The roadmap is already the one
//! honest record of what shipped and what is next; duplicating it into a
//! second list would guarantee the two drift, and a welcome screen that lies
//! about the game is worse than none. A test pins that the parse yields real
//! lines and that every character of them is drawable.
//!
//! # Shown once
//!
//! A zero-byte marker file in the *config* directory — player metadata, not
//! world state, so it does not belong in any world's save and needs no format
//! to version. Presence is the whole protocol.

use std::path::PathBuf;

use vx_render::font::{self, LINE_HEIGHT};

/// The roadmap, compiled in. Three levels up from this file to the repo root.
const ROADMAP: &str = include_str!("../../../ROADMAP.md");

/// Panel size in texture pixels, displayed at the shop's scale.
pub const INTRO_WIDTH: u32 = 280;
pub const INTRO_HEIGHT: u32 = 220;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 240];

/// Lines the changelog window shows at once.
const WINDOW: usize = 9;

/// The authored greeting above the derived changelog.
const PREAMBLE: [&str; 5] = [
    "THIS IS YOUR HOUSE IN STONEHAVEN.",
    "THE CHEST KEEPS YOUR GOODS. THE MAILBOX",
    "OUTSIDE TAKES DELIVERIES YOU ORDER AT",
    "THE SHOP COUNTER UP THE PATH.",
    "PLACE A CONTAINER BEFORE YOU MINE.",
];

/// Every changelog line, derived from the roadmap.
///
/// Shipped stages come from the Shipped table (stage and summary, commit
/// hashes dropped); what is next comes from the `## In flight` and
/// `## Planned` headings. Uppercased for the bitmap font; anything the font
/// cannot draw becomes a space rather than a missing-glyph box.
pub fn changelog() -> Vec<String> {
    let mut lines = Vec::new();

    let mut in_shipped = false;
    for line in ROADMAP.lines() {
        if line.starts_with("## Shipped") && !in_shipped {
            // Only the table under the first Shipped heading; the prose
            // sections repeat the same stages at length.
            in_shipped = lines.is_empty();
            continue;
        }
        if in_shipped {
            if line.starts_with("---") {
                in_shipped = false;
                continue;
            }
            let mut cells = line.split('|').map(str::trim);
            let (Some(_), Some(stage), Some(_commit), Some(what)) =
                (cells.next(), cells.next(), cells.next(), cells.next())
            else {
                continue;
            };
            if stage.is_empty() || stage == "Stage" || stage.starts_with("---") {
                continue;
            }
            lines.push(drawable(&format!("{stage}  {what}")));
        }
        if let Some(title) = line.strip_prefix("## In flight — ") {
            lines.push(drawable(&format!("NEXT  {title}")));
        }
        if let Some(title) = line.strip_prefix("## Planned — ") {
            lines.push(drawable(&format!("LATER  {title}")));
        }
    }
    lines
}

/// Characters that fit one panel row. The window is 280 px less margins at a
/// 6 px advance; running off the edge reads worse than an ellipsis.
const ROW_CHARS: usize = 43;

/// Uppercase, clipped to the row, anything undrawable softened to a space.
fn drawable(line: &str) -> String {
    let mut cleaned: String = line
        .to_uppercase()
        .chars()
        .map(|character| if font::knows(character) { character } else { ' ' })
        .collect();
    if cleaned.chars().count() > ROW_CHARS {
        cleaned = cleaned.chars().take(ROW_CHARS - 2).collect();
        cleaned.push_str("..");
    }
    cleaned
}

/// Where the seen-marker lives: config, not any world's save.
fn marker_path() -> PathBuf {
    vx_platform::paths::config_dir().join("welcome-seen")
}

/// Has the welcome ever been dismissed on this machine?
pub fn seen() -> bool {
    marker_path().is_file()
}

/// Remember the dismissal. Failure to write is failure to remember — the
/// panel shows again next boot, which is annoying and harmless.
pub fn mark_seen() {
    let path = marker_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&path, b"") {
        log::warn!("could not write {}: {error}", path.display());
    }
}

/// The panel's state.
#[derive(Debug)]
pub struct Intro {
    pub open: bool,
    scroll: usize,
    lines: Vec<String>,
}

impl Intro {
    pub fn new() -> Self {
        Intro {
            open: false,
            scroll: 0,
            lines: changelog(),
        }
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let last = self.lines.len().saturating_sub(WINDOW);
        self.scroll = (self.scroll as i32 + delta).clamp(0, last as i32) as usize;
    }
}

impl Default for Intro {
    fn default() -> Self {
        Intro::new()
    }
}

/// Draw the welcome panel. Pure in its inputs.
pub fn render_intro(intro: &Intro) -> Vec<u8> {
    let mut pixels = vec![0u8; (INTRO_WIDTH * INTRO_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 8i32;
    let mut y = margin;
    font::draw_text(&mut pixels, INTRO_WIDTH, margin, y, 1, ACCENT, "WELCOME TO THE FRONTIER");
    y += LINE_HEIGHT as i32 + 4;

    for line in PREAMBLE {
        font::draw_text(&mut pixels, INTRO_WIDTH, margin, y, 1, TEXT, line);
        y += LINE_HEIGHT as i32;
    }
    y += 4;

    font::draw_text(&mut pixels, INTRO_WIDTH, margin, y, 1, DIM, "THE STORY SO FAR, AND WHAT IS NEXT:");
    y += LINE_HEIGHT as i32 + 2;

    for line in intro.lines.iter().skip(intro.scroll).take(WINDOW) {
        font::draw_text(&mut pixels, INTRO_WIDTH, margin, y, 1, TEXT, line);
        y += LINE_HEIGHT as i32;
    }

    if intro.lines.len() > WINDOW {
        let note = format!(
            "{}-{} OF {}",
            intro.scroll + 1,
            (intro.scroll + WINDOW).min(intro.lines.len()),
            intro.lines.len()
        );
        font::draw_text(
            &mut pixels,
            INTRO_WIDTH,
            INTRO_WIDTH as i32 - margin - font::text_width(&note, 1) as i32,
            INTRO_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
            1,
            DIM,
            &note,
        );
    }
    font::draw_text(
        &mut pixels,
        INTRO_WIDTH,
        margin,
        INTRO_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
        1,
        DIM,
        "ARROWS SCROLL. E CLOSES.",
    );
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_changelog_parses_into_real_lines() {
        let lines = changelog();
        assert!(
            lines.len() >= 12,
            "only {} lines parsed out of the roadmap",
            lines.len()
        );
    }

    #[test]
    fn the_panel_mentions_the_stage_that_built_it() {
        // Self-referential truthfulness: the newest shipped stage has to
        // appear in the panel that stage built. If it does not, the parse is
        // reading the wrong part of the file.
        let lines = changelog();
        assert!(
            lines.iter().any(|line| line.starts_with("59A ")),
            "the newest stage is missing from its own welcome panel: {lines:#?}"
        );
    }

    #[test]
    fn every_changelog_character_is_drawable() {
        for line in changelog() {
            for character in line.chars() {
                assert!(
                    font::knows(character),
                    "undrawable {character:?} in {line:?}"
                );
            }
        }
        for line in PREAMBLE {
            for character in line.chars() {
                assert!(font::knows(character), "undrawable {character:?} in the preamble");
            }
        }
    }

    /// The preamble grew a line in stage 48 — a new player was never told
    /// that mining without a container on the ground throws the ore away —
    /// so the panel it lives in has to be checked for room rather than
    /// assumed to have it.
    #[test]
    fn the_greeting_and_the_changelog_both_fit_the_panel() {
        let margin = 8i32;
        let usable = INTRO_WIDTH as i32 - margin * 2;
        for line in PREAMBLE {
            assert!(
                font::text_width(line, 1) as i32 <= usable,
                "the preamble line {line:?} is wider than the panel"
            );
        }

        // The same walk down the panel `render_intro` makes.
        let mut y = margin;
        y += LINE_HEIGHT as i32 + 4;
        y += PREAMBLE.len() as i32 * LINE_HEIGHT as i32;
        y += 4;
        y += LINE_HEIGHT as i32 + 2;
        y += WINDOW as i32 * LINE_HEIGHT as i32;
        assert!(
            y <= INTRO_HEIGHT as i32,
            "the panel overflows: the changelog ends at {y} of {INTRO_HEIGHT}"
        );
    }

    #[test]
    fn the_render_is_deterministic_and_reacts_to_scroll() {
        let intro = Intro::new();
        assert_eq!(render_intro(&intro), render_intro(&intro));

        let mut scrolled = Intro::new();
        scrolled.scroll_by(3);
        if scrolled.lines.len() > WINDOW {
            assert_ne!(
                render_intro(&intro),
                render_intro(&scrolled),
                "scrolling did not change the picture"
            );
        }
    }

    #[test]
    fn scrolling_clamps_at_both_ends() {
        let mut intro = Intro::new();
        intro.scroll_by(-100);
        assert_eq!(intro.scroll, 0);
        intro.scroll_by(10_000);
        assert!(intro.scroll <= intro.lines.len());
        let bottom = intro.scroll;
        intro.scroll_by(1);
        assert_eq!(intro.scroll, bottom, "scrolled past the end");
    }
}
