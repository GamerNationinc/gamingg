//! The heads-up display: a bottom-left panel of words and bars.
//!
//! First real user of the bitmap font. Everything that used to go to the log
//! — mining status, ferry status — now lands where the player is looking,
//! along with the skill sheet and the drill's progress. The panel is a plain
//! RGBA buffer stamped with text and rectangles, shipped through overlay slot
//! 1; the compositor knows nothing about what the pixels mean, exactly like
//! the minimap in slot 0.

use vx_render::font::{self, LINE_HEIGHT};

use crate::clock::TimeOfDay;
use crate::skills::{self, Skills};

/// Panel size in texture pixels. Displayed 2x, so text is comfortably
/// readable without a big texture.
pub const HUD_WIDTH: u32 = 220;
pub const HUD_HEIGHT: u32 = 94;

/// On-screen scale factor for the panel.
pub const HUD_SCALE: f32 = 2.0;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const BAR_BACK: [u8; 4] = [45, 48, 55, 255];
const BACKGROUND: [u8; 4] = [12, 14, 18, 165];
const NIGHT: [u8; 4] = [130, 150, 200, 255];
/// Wind left. Goes red when there is not enough for a slide or a mantle, which
/// is the only moment the number changes what you can do.
const WIND: [u8; 4] = [110, 200, 255, 255];
const WIND_LOW: [u8; 4] = [235, 90, 70, 255];
/// The load bar. Warms as the pack fills, because a full pack is a decision.
const LOAD: [u8; 4] = [200, 170, 110, 255];
/// A bounty on your head, and the eye that says somebody can see you.
const WANTED: [u8; 4] = [235, 90, 70, 255];
const EYE: [u8; 4] = [255, 210, 120, 255];

/// Everything the HUD shows this frame.
pub struct HudContent<'a> {
    pub skills: &'a Skills,
    /// The hour, passed in rather than read: the HUD is a pure function of
    /// its inputs, and a wall-clock read here would make captures wobble.
    pub time: TimeOfDay,
    /// The one-line activity readout (mining/ferrying/scanning), if any.
    pub status: Option<String>,
    /// Drill progress through the current block, while digging.
    pub drilling: Option<f32>,
    /// A recent level-up to celebrate: (skill, new level).
    pub level_up: Option<(String, u32)>,
    /// A villager's line, freshly spoken.
    pub greeting: Option<String>,
    /// True while the body waits for its own ground to stream back in after a
    /// feed. Worth saying out loud: the controls are briefly dead on purpose.
    pub reconnecting: bool,
    /// Bounty points the town has written against you. Zero draws nothing.
    pub bounty: u64,
    /// True while at least one villager can actually see you.
    ///
    /// Without this the stealth rules are invisible mechanics: a player has no
    /// way to learn that crouching behind a wall worked except by not being
    /// arrested, which is far too slow a teacher.
    pub watched: bool,
    /// Stance, wind left, and how full the pack is.
    ///
    /// Passed in rather than read, like the hour above: the HUD stays a pure
    /// function of its inputs so a capture cannot wobble.
    pub movement: MovementReadout,
    /// Slugs left, shown while the launcher is in hand. `None` = slung or
    /// unowned, and no line is drawn.
    pub ammo: Option<u32>,
    /// Townsfolk running scared right now. Zero draws nothing; anything else
    /// tells you the street knows.
    pub panicking: usize,
    /// The scout's one-line status ("KESTREL ORBITING 32S"), if owned.
    pub kestrel: Option<String>,
    /// The optics dial, when it is not off: LAMP / HIGH BEAM / NIGHT VISION /
    /// THERMAL.
    pub optic: Option<&'static str>,
    /// What the fleet has left to burn, when there is a fleet to fuel.
    pub fuel: Option<String>,
    /// What the crew is cutting, while a dispatch is running. The number that
    /// says whether the last drone was worth what it cost — and the one thing
    /// a player buying machines had no way at all to see before stage 52.
    pub payroll: Option<String>,
    /// Hits left, once anything has landed. Whole draws nothing: a bar that
    /// is always there is a bar nobody reads.
    pub condition: Option<String>,
    /// Deputies still coming for you. Zero draws nothing.
    pub deputies: usize,
    /// What the deep ore is doing to you, once anything is showing. Ranked
    /// with the condition line rather than the kit lines for the same
    /// reason: it is a thing to act on now.
    pub dose: Option<String>,
    /// What is out there in the dark, when something is. Top of the urgent
    /// block: nothing else on this panel outranks it.
    pub dark: Option<String>,
}

/// What the HUD says about how the player is moving.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MovementReadout {
    /// Short stance label: STAND, SPRINT, CROUCH, PRONE, SLIDE, AIR, CLIMB.
    pub stance: &'static str,
    /// Wind remaining, 0..1.
    pub stamina: f32,
    /// Pack fullness, 0..1.
    pub load: f32,
}

/// Paint a horizontal bar with a filled fraction.
fn draw_bar(pixels: &mut [u8], x: u32, y: u32, width: u32, height: u32, fraction: f32, colour: [u8; 4]) {
    let filled = (width as f32 * fraction.clamp(0.0, 1.0)) as u32;
    for py in y..y + height {
        for px in x..x + width {
            let at = ((py * HUD_WIDTH + px) * 4) as usize;
            let texel = if px < x + filled { colour } else { BAR_BACK };
            pixels[at..at + 4].copy_from_slice(&texel);
        }
    }
}

/// Render the panel. Deterministic in its inputs, which is what the tests
/// lean on.
pub fn render_hud(content: &HudContent) -> Vec<u8> {
    let mut pixels = vec![0u8; (HUD_WIDTH * HUD_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 5i32;
    let mut y = margin;

    // Line 1: the skill sheet.
    let sheet = format!(
        "MIN {}  PRO {}  LOG {}",
        content.skills.level(skills::MINING),
        content.skills.level(skills::PROSPECTING),
        content.skills.level(skills::LOGISTICS),
    );
    font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, TEXT, &sheet);

    // The hour, right-aligned on the same line, tinted by whether the town is
    // open for business.
    let (hours, minutes) = content.time.hhmm();
    let clock = format!("{hours:02}:{minutes:02}");
    let tint = if content.time.is_daylight() { ACCENT } else { NIGHT };
    font::draw_text(
        &mut pixels,
        HUD_WIDTH,
        HUD_WIDTH as i32 - margin - font::text_width(&clock, 1) as i32,
        y,
        1,
        tint,
        &clock,
    );
    y += LINE_HEIGHT as i32;

    // Line 2: the XP bar for whatever skill last moved.
    if let Some(recent) = content.skills.recent() {
        let xp = content.skills.xp(recent);
        let label: String = recent.chars().take(3).collect();
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, DIM, &label);
        draw_bar(
            &mut pixels,
            (margin + 24) as u32,
            y as u32 + 1,
            HUD_WIDTH - 34,
            5,
            skills::progress_to_next(xp),
            ACCENT,
        );
    }
    y += LINE_HEIGHT as i32;

    // Line 3: reconnecting outranks everything — the player's controls are
    // dead for a moment and they should know why — then a level-up shout, a
    // villager's word, and finally the activity readout.
    if content.reconnecting {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, ACCENT, "RECONNECTING");
    } else if let Some((skill, level)) = &content.level_up {
        let line = format!("{} LEVEL {level}!", skill.to_uppercase());
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, ACCENT, &line);
    } else if let Some(greeting) = &content.greeting {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, ACCENT, greeting);
    } else if let Some(status) = &content.status {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, TEXT, status);
    }
    y += LINE_HEIGHT as i32;

    // Line 4: how the body is moving. Two bars, because both numbers change
    // what you can do right now: wind gates the slide and the mantle, and load
    // is the tax on every upgrade that let you carry it.
    let readout = content.movement;
    if !readout.stance.is_empty() {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, DIM, readout.stance);
        let bars = margin + 40;
        let width = (HUD_WIDTH as i32 - bars - margin - 6) as u32 / 2;
        draw_bar(
            &mut pixels,
            bars as u32,
            y as u32 + 1,
            width,
            5,
            readout.stamina,
            if readout.stamina < 0.15 { WIND_LOW } else { WIND },
        );
        draw_bar(
            &mut pixels,
            (bars as u32) + width + 6,
            y as u32 + 1,
            width,
            5,
            readout.load,
            LOAD,
        );
    }
    y += LINE_HEIGHT as i32;

    // Line 4b: the launcher, while it is in hand — slugs left, and whether
    // the town is in a state about it.
    if content.ammo.is_some() || content.panicking > 0 {
        if let Some(ammo) = content.ammo {
            let line = format!("SLUGS {ammo}");
            let tint = if ammo == 0 { WANTED } else { TEXT };
            font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, tint, &line);
        }
        if content.panicking > 0 {
            let cry = "PANIC";
            font::draw_text(
                &mut pixels,
                HUD_WIDTH,
                HUD_WIDTH as i32 - margin - font::text_width(cry, 1) as i32,
                y,
                1,
                WANTED,
                cry,
            );
        }
        y += LINE_HEIGHT as i32;
    }

    // Line 4d: what the crew is earning, while it is working.
    if let Some(line) = &content.payroll {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, DIM, line);
        y += LINE_HEIGHT as i32;
    }
    // Line 4c: the scout, while one is owned.
    if let Some(line) = &content.fuel {
        // Red when the crew has stopped: a fleet that quietly does nothing is
        // the one failure a player will otherwise blame on the pathfinder.
        let colour = if line.contains("DRY") { WANTED } else { DIM };
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, colour, line);
        y += LINE_HEIGHT as i32;
    }
    // Your own condition and the law's attention outrank the kit lines:
    // both are things you must act on now.
    if let Some(dark) = &content.dark {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, WANTED, dark);
        y += LINE_HEIGHT as i32;
    }
    if let Some(condition) = &content.condition {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, WANTED, condition);
        y += LINE_HEIGHT as i32;
    }
    if let Some(dose) = &content.dose {
        let colour = if dose.contains("GET OUT") { WANTED } else { ACCENT };
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, colour, dose);
        y += LINE_HEIGHT as i32;
    }
    if content.deputies > 0 {
        let line = if content.deputies == 1 {
            "ONE DEPUTY ON YOU".to_string()
        } else {
            format!("{} DEPUTIES ON YOU", content.deputies)
        };
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, WANTED, &line);
        y += LINE_HEIGHT as i32;
    }

    if let Some(optic) = content.optic {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, ACCENT, optic);
        y += LINE_HEIGHT as i32;
    }
    if let Some(kestrel) = &content.kestrel {
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, DIM, kestrel);
        y += LINE_HEIGHT as i32;
    }

    // Line 5: what the town thinks of you, and whether anyone is looking.
    if content.bounty > 0 || content.watched {
        if content.bounty > 0 {
            let line = format!("WANTED {}", content.bounty);
            font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, WANTED, &line);
        }
        if content.watched {
            let seen = "SEEN";
            font::draw_text(
                &mut pixels,
                HUD_WIDTH,
                HUD_WIDTH as i32 - margin - font::text_width(seen, 1) as i32,
                y,
                1,
                EYE,
                seen,
            );
        }
        y += LINE_HEIGHT as i32;
    }

    // Line 6: the drill.
    if let Some(progress) = content.drilling {
        let line = format!("DRILLING {:.0}%", progress.clamp(0.0, 1.0) * 100.0);
        font::draw_text(&mut pixels, HUD_WIDTH, margin, y, 1, TEXT, &line);
        draw_bar(
            &mut pixels,
            (margin + font::text_width(&line, 1) as i32 + 6) as u32,
            y as u32 + 1,
            60,
            5,
            progress,
            ACCENT,
        );
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_content(skills: &Skills) -> HudContent<'_> {
        HudContent {
            skills,
            time: TimeOfDay::NOON,
            status: None,
            payroll: None,
            drilling: None,
            level_up: None,
            greeting: None,
            reconnecting: false,
            bounty: 0,
            watched: false,
            movement: MovementReadout::default(),
            ammo: None,
            panicking: 0,
            kestrel: None,
            optic: None,
            condition: None,
            dose: None,
            dark: None,
            deputies: 0,
            fuel: None,
        }
    }

    #[test]
    fn the_optic_line_appears_only_when_the_dial_is_on() {
        let skills = Skills::new();
        let plain = base_content(&skills);
        let mut lit = base_content(&skills);
        lit.optic = Some("NIGHT VISION");
        assert_ne!(render_hud(&plain), render_hud(&lit), "no optic line drawn");
    }

    #[test]
    fn the_kestrel_line_appears_only_when_owned() {
        let skills = Skills::new();
        let plain = base_content(&skills);
        let mut scouted = base_content(&skills);
        scouted.kestrel = Some("KESTREL ORBITING 32S".into());
        assert_ne!(render_hud(&plain), render_hud(&scouted), "no kestrel line drawn");
    }

    #[test]
    fn the_ammo_line_appears_only_with_the_launcher_out() {
        let skills = Skills::new();
        let plain = base_content(&skills);
        let mut armed = base_content(&skills);
        armed.ammo = Some(12);
        assert_ne!(render_hud(&plain), render_hud(&armed), "no ammo line drawn");

        let mut spooked = base_content(&skills);
        spooked.panicking = 2;
        assert_ne!(
            render_hud(&plain),
            render_hud(&spooked),
            "no panic cry drawn"
        );
    }

    #[test]
    fn reconnecting_outranks_every_other_line() {
        let skills = Skills::new();
        let mut busy = base_content(&skills);
        busy.status = Some("MINING 4 JOBS LEFT".into());
        busy.greeting = Some("MORNIN".into());
        busy.level_up = Some(("mining".into(), 9));

        let mut waiting = base_content(&skills);
        waiting.reconnecting = true;

        let mut waiting_busy = busy;
        waiting_busy.reconnecting = true;
        assert_eq!(
            render_hud(&waiting_busy),
            render_hud(&waiting),
            "something drew over the reconnecting notice"
        );
    }

    #[test]
    fn a_greeting_shows_and_yields_to_a_level_up() {
        let skills = Skills::new();
        let plain = base_content(&skills);
        let mut greeted = base_content(&skills);
        greeted.greeting = Some("MORNIN. FINE DAY FOR DIGGIN.".into());
        assert_ne!(render_hud(&plain), render_hud(&greeted));

        let mut both = base_content(&skills);
        both.greeting = Some("MORNIN. FINE DAY FOR DIGGIN.".into());
        both.level_up = Some(("mining".into(), 3));
        let mut level_only = base_content(&skills);
        level_only.level_up = Some(("mining".into(), 3));
        assert_eq!(render_hud(&both), render_hud(&level_only));
    }

    #[test]
    fn the_panel_is_the_declared_size_and_deterministic() {
        let skills = Skills::new();
        let a = render_hud(&base_content(&skills));
        let b = render_hud(&base_content(&skills));
        assert_eq!(a.len(), (HUD_WIDTH * HUD_HEIGHT * 4) as usize);
        assert_eq!(a, b);
    }

    #[test]
    fn levels_change_the_pixels() {
        let fresh = Skills::new();
        let mut veteran = Skills::new();
        veteran.add_xp(crate::skills::MINING, 50_000);

        let a = render_hud(&base_content(&fresh));
        let b = render_hud(&base_content(&veteran));
        assert_ne!(a, b, "a different level drew the same panel");
    }

    #[test]
    fn drilling_progress_shows_and_moves() {
        let skills = Skills::new();
        let mut early = base_content(&skills);
        early.drilling = Some(0.2);
        let mut late = base_content(&skills);
        late.drilling = Some(0.9);

        let a = render_hud(&early);
        let b = render_hud(&late);
        assert_ne!(a, b, "the drill bar did not move");
    }

    #[test]
    fn a_level_up_outranks_the_status_line() {
        let skills = Skills::new();
        let mut with_status = base_content(&skills);
        with_status.status = Some("FERRYING 40 ABOARD".into());
        let mut with_both = base_content(&skills);
        with_both.status = Some("FERRYING 40 ABOARD".into());
        with_both.level_up = Some(("mining".into(), 7));

        assert_ne!(render_hud(&with_status), render_hud(&with_both));
    }

    #[test]
    fn the_hud_shows_the_hour_and_tells_day_from_night() {
        let skills = Skills::new();
        let mut noon = base_content(&skills);
        noon.time = TimeOfDay::NOON;
        let mut midnight = base_content(&skills);
        midnight.time = TimeOfDay::MIDNIGHT;

        let day = render_hud(&noon);
        let night = render_hud(&midnight);
        assert_ne!(day, night, "the clock does not show on the panel");
        // And the panel stays a pure function of its inputs.
        assert_eq!(day, render_hud(&noon));
    }

    #[test]
    fn the_movement_line_draws_both_bars() {
        // Two numbers that change what you can do right now: wind gates the
        // slide and the mantle, load taxes every cargo upgrade you bought.
        let skills = Skills::new();
        let mut empty = base_content(&skills);
        empty.movement = MovementReadout {
            stance: "SPRINT",
            stamina: 0.0,
            load: 0.0,
        };
        let mut full = base_content(&skills);
        full.movement = MovementReadout {
            stance: "SPRINT",
            stamina: 1.0,
            load: 1.0,
        };

        assert_ne!(
            render_hud(&empty),
            render_hud(&full),
            "the bars do not respond to their own numbers"
        );
    }

    #[test]
    fn no_stance_means_no_movement_line() {
        // Fly mode and the capture path have nothing to say here, and a blank
        // label must leave the row empty rather than draw an empty bar.
        let skills = Skills::new();
        let blank = base_content(&skills);
        let mut loud = base_content(&skills);
        loud.movement = MovementReadout {
            stance: "PRONE",
            stamina: 0.5,
            load: 0.5,
        };

        assert_ne!(render_hud(&blank), render_hud(&loud));
    }

    #[test]
    fn the_wanted_line_and_the_eye_only_show_when_they_mean_something() {
        let skills = Skills::new();
        let clean = base_content(&skills);

        let mut wanted = base_content(&skills);
        wanted.bounty = 40;

        let mut watched = base_content(&skills);
        watched.watched = true;

        assert_ne!(render_hud(&clean), render_hud(&wanted), "bounty does not show");
        assert_ne!(render_hud(&clean), render_hud(&watched), "the eye does not show");
        assert_ne!(
            render_hud(&wanted),
            render_hud(&watched),
            "bounty and being seen render the same"
        );
    }

    #[test]
    fn a_clean_sheet_draws_no_accusation() {
        // A player who has done nothing should never see the word WANTED.
        let skills = Skills::new();
        let clean = base_content(&skills);
        let mut also_clean = base_content(&skills);
        also_clean.bounty = 0;
        assert_eq!(render_hud(&clean), render_hud(&also_clean));
    }

    #[test]
    fn every_crime_readout_is_drawable() {
        for line in ["WANTED 40", "SEEN"] {
            for character in line.chars() {
                assert!(font::knows(character), "undrawable {character:?} in {line}");
            }
        }
    }

    #[test]
    fn every_stance_label_is_drawable() {
        // A label the font cannot draw renders as a row of missing-glyph boxes.
        use crate::movement::Stance;
        let labels = [
            Stance::Grounded,
            Stance::Sprinting,
            Stance::Crouched,
            Stance::Prone,
            Stance::Swimming,
            Stance::Sliding { ticks: 0 },
            Stance::Airborne { coyote: 0 },
            Stance::Mantling {
                from: glam::DVec3::ZERO,
                to: glam::DVec3::ZERO,
                t: 0,
                span: 1,
            },
        ];
        for stance in labels {
            for character in stance.label().chars() {
                assert!(
                    font::knows(character),
                    "{:?} label uses an undrawable {character:?}",
                    stance
                );
            }
        }
    }
}
