//! The controller: a pad driving the seams the keyboard and mouse built.
//!
//! # Synthesis, not a second input system
//!
//! Buttons resolve to the [`KeyCode`] the same action is already bound to
//! and go through the very same `handle_press` / `InputState` path the
//! keyboard uses; the left stick feeds the movement axes and the walk
//! sampler's direction bits, the right stick feeds the mouse-look
//! accumulator, and the triggers mirror the mouse buttons. Nothing
//! downstream knows a pad exists — which is why every
//! panel, the map, the shop and the handheld gained pad support the moment
//! this module compiled, and why there is exactly one implementation of
//! every rule input can reach.
//!
//! # Context is one bit
//!
//! A pad has fewer buttons than a keyboard has keys, so the face buttons
//! mean different things with a panel open — the console convention: south
//! confirms, east backs out. The keyboard already routes keys by which
//! panel is open, so the mapping only needs the one bit; everything finer
//! is downstream's business.
//!
//! # The pad is optional everywhere
//!
//! [`Pad::new`] failing (no udev, a headless test runner, a locked-down
//! container) leaves a `Pad` that polls nothing, forever. Input must never
//! be the reason the game cannot start.

use std::collections::HashMap;

use gilrs::{Axis, Button, EventType, Gilrs};
use winit::keyboard::KeyCode;

use vx_render::font::{self, LINE_HEIGHT};

/// Stick tilt below this is noise. Steam Deck sticks rest around 0.05;
/// worn pads drift further, and a drifting camera reads as a broken game.
pub const DEADZONE: f32 = 0.18;

/// Right-stick look speed at full tilt, in mouse-pixel-equivalents per
/// second. The mouse pipeline is the one consumer, so the unit is its unit.
pub const LOOK_SPEED: f32 = 640.0;

/// One thing the pad did since the last poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Press(Button),
    Release(Button),
    Connected,
    Disconnected,
}

/// The pad, its live stick state, and what each held button meant when it
/// went down.
pub struct Pad {
    session: Option<Gilrs>,
    left: (f32, f32),
    right: (f32, f32),
    /// The `KeyCode` each held button resolved to at press time. Releases
    /// look the answer up here rather than re-asking the mapping — the
    /// context can change mid-hold, and a remap between press and release
    /// would leak a stuck key.
    pub down: HashMap<Button, KeyCode>,
    /// Whether the control-scheme overlay is up.
    pub help: bool,
}

impl Pad {
    pub fn new() -> Self {
        let session = match Gilrs::new() {
            Ok(session) => Some(session),
            Err(error) => {
                // Headless, no udev, or no permission to /dev/input: the
                // game runs on, keyboard-only.
                log::warn!("no gamepad support: {error}");
                None
            }
        };
        Pad {
            session,
            left: (0.0, 0.0),
            right: (0.0, 0.0),
            down: HashMap::new(),
            help: false,
        }
    }

    /// Drain everything the pad did since last frame, updating stick state
    /// on the way through. Returned in arrival order.
    pub fn poll(&mut self) -> Vec<Change> {
        let Some(session) = &mut self.session else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        while let Some(event) = session.next_event() {
            match event.event {
                EventType::ButtonPressed(button, _) => changes.push(Change::Press(button)),
                EventType::ButtonReleased(button, _) => changes.push(Change::Release(button)),
                EventType::AxisChanged(axis, value, _) => match axis {
                    Axis::LeftStickX => self.left.0 = value,
                    Axis::LeftStickY => self.left.1 = value,
                    Axis::RightStickX => self.right.0 = value,
                    Axis::RightStickY => self.right.1 = value,
                    _ => {}
                },
                EventType::Connected => changes.push(Change::Connected),
                EventType::Disconnected => {
                    // A pad yanked mid-stride must not leave the stick
                    // wedged forward.
                    self.left = (0.0, 0.0);
                    self.right = (0.0, 0.0);
                    changes.push(Change::Disconnected);
                }
                _ => {}
            }
        }
        changes
    }

    /// The movement stick, deadzoned. `(x right, y forward)`.
    pub fn left_stick(&self) -> (f32, f32) {
        deadzoned(self.left)
    }

    /// The look stick, deadzoned. `(x right, y up)`.
    pub fn right_stick(&self) -> (f32, f32) {
        deadzoned(self.right)
    }
}

/// Radial deadzone with rescale: dead centre is exactly zero, and the live
/// range re-spans 0..1 so slow drift dies but a slow *walk* is still
/// possible right above the threshold.
fn deadzoned(raw: (f32, f32)) -> (f32, f32) {
    let magnitude = (raw.0 * raw.0 + raw.1 * raw.1).sqrt();
    if magnitude < DEADZONE {
        return (0.0, 0.0);
    }
    let live = ((magnitude - DEADZONE) / (1.0 - DEADZONE)).min(1.0);
    let scale = live / magnitude;
    (raw.0 * scale, raw.1 * scale)
}

/// What a button means: the key it presses. `panel` is whether any panel
/// owns the screen — the one bit of context the mapping needs.
///
/// `Select` is absent on purpose (it toggles the help overlay in `main`,
/// not a key), and so are the analog triggers (they mirror the mouse
/// buttons, which have no `KeyCode` to resolve to).
pub fn key_for(button: Button, panel: bool) -> Option<KeyCode> {
    if panel {
        return match button {
            // Console convention: south confirms, east backs out. Every
            // panel already closes on Escape, which is what makes one
            // mapping serve eleven panels.
            Button::South => Some(KeyCode::Enter),
            Button::East => Some(KeyCode::Escape),
            Button::West => Some(KeyCode::KeyE),
            Button::North => Some(KeyCode::Tab),
            Button::DPadUp => Some(KeyCode::ArrowUp),
            Button::DPadDown => Some(KeyCode::ArrowDown),
            Button::DPadLeft => Some(KeyCode::ArrowLeft),
            Button::DPadRight => Some(KeyCode::ArrowRight),
            Button::Start => Some(KeyCode::Enter),
            _ => None,
        };
    }
    match button {
        Button::South => Some(KeyCode::Space),
        Button::East => Some(KeyCode::ShiftLeft),
        Button::West => Some(KeyCode::KeyE),
        Button::North => Some(KeyCode::KeyV),
        Button::LeftTrigger => Some(KeyCode::KeyC),
        Button::RightTrigger => Some(KeyCode::Tab),
        Button::LeftThumb => Some(KeyCode::ControlLeft),
        Button::RightThumb => Some(KeyCode::KeyL),
        Button::DPadUp => Some(KeyCode::KeyM),
        Button::DPadDown => Some(KeyCode::KeyN),
        Button::DPadLeft => Some(KeyCode::KeyG),
        Button::DPadRight => Some(KeyCode::KeyF),
        Button::Start => Some(KeyCode::Enter),
        _ => None,
    }
}

/// The help overlay's size in texture pixels.
pub const PAD_WIDTH: u32 = 300;
pub const PAD_HEIGHT: u32 = 232;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 240];

/// The control scheme, written for the player. One row per physical
/// control, in the order a hand finds them. Tested drawable.
pub const SCHEME: [(&str, &str); 16] = [
    ("LEFT STICK", "MOVE, CLICK TO SPRINT"),
    ("RIGHT STICK", "LOOK, CLICK FOR OPTICS"),
    ("RT", "DRILL, OR FIRE"),
    ("LT", "PLACE THE SELECTED BLOCK"),
    ("A", "JUMP - IN PANELS, CONFIRM"),
    ("B", "CROUCH - IN PANELS, BACK OUT"),
    ("X", "USE, TRADE, TALK"),
    ("Y", "THE HANDHELD UPLINK"),
    ("LB", "FIRST OR THIRD PERSON"),
    ("RB", "TURN THE PAGE, CYCLE THE METHOD"),
    ("D-PAD UP", "MARK AN ORE CORNER"),
    ("D-PAD DOWN", "THE MINIMAP"),
    ("D-PAD LEFT", "SCAN THIS SECTOR"),
    ("D-PAD RIGHT", "WALK OR FLY"),
    ("START", "CONFIRM, DISPATCH, PICK"),
    ("SELECT", "THIS PANEL"),
];

/// Draw the control scheme. Pure, like every panel here.
pub fn render_pad_help() -> Vec<u8> {
    let mut pixels = vec![0u8; (PAD_WIDTH * PAD_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 8i32;
    let mut y = margin;
    font::draw_text(&mut pixels, PAD_WIDTH, margin, y, 1, ACCENT, "CONTROLLER");
    y += LINE_HEIGHT as i32 + 4;

    for (control, does) in SCHEME {
        font::draw_text(&mut pixels, PAD_WIDTH, margin, y, 1, DIM, control);
        font::draw_text(&mut pixels, PAD_WIDTH, margin + 76, y, 1, TEXT, does);
        y += LINE_HEIGHT as i32;
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deadzone_kills_drift_but_not_a_slow_walk() {
        assert_eq!(deadzoned((0.05, -0.1)), (0.0, 0.0), "drift got through");
        let (x, y) = deadzoned((0.0, DEADZONE + 0.02));
        assert_eq!(x, 0.0);
        assert!(y > 0.0 && y < 0.1, "just past the deadzone should be a creep, got {y}");
        let (_, forward) = deadzoned((0.0, 1.0));
        assert!((forward - 1.0).abs() < 1.0e-5, "full tilt should be full speed");
    }

    #[test]
    fn the_deadzone_is_monotonic() {
        let mut last = -1.0f32;
        for step in 0..=20 {
            let tilt = step as f32 / 20.0;
            let (_, y) = deadzoned((0.0, tilt));
            assert!(y >= last, "speed fell as the stick tilted further");
            last = y;
        }
    }

    #[test]
    fn confirm_and_back_out_swap_when_a_panel_opens() {
        // The console convention, pinned: in the world south is jump; with
        // a panel up it is confirm, and east is the way out.
        assert_eq!(key_for(Button::South, false), Some(KeyCode::Space));
        assert_eq!(key_for(Button::South, true), Some(KeyCode::Enter));
        assert_eq!(key_for(Button::East, true), Some(KeyCode::Escape));
        // The d-pad turns into the arrows every panel lists with.
        assert_eq!(key_for(Button::DPadUp, true), Some(KeyCode::ArrowUp));
    }

    #[test]
    fn every_mapped_button_resolves_in_both_contexts_or_neither_deliberately() {
        // Buttons that act in the world keep acting in panels (possibly as
        // something else) except the deliberate exceptions — a button that
        // silently dies when a panel opens reads as a broken pad.
        let world_only = [
            Button::LeftTrigger,
            Button::RightTrigger,
            Button::LeftThumb,
            Button::RightThumb,
        ];
        for button in [
            Button::South,
            Button::East,
            Button::West,
            Button::North,
            Button::DPadUp,
            Button::DPadDown,
            Button::DPadLeft,
            Button::DPadRight,
            Button::Start,
        ] {
            assert!(key_for(button, false).is_some(), "{button:?} dead in the world");
            assert!(key_for(button, true).is_some(), "{button:?} dead in panels");
        }
        for button in world_only {
            assert!(key_for(button, false).is_some());
        }
        // Select is main's own: the help toggle, never a key.
        assert_eq!(key_for(Button::Select, false), None);
        assert_eq!(key_for(Button::Select, true), None);
    }

    #[test]
    fn the_help_panel_is_drawable_and_deterministic() {
        for (control, does) in SCHEME {
            for character in control.chars().chain(does.chars()) {
                assert!(font::knows(character), "undrawable {character:?}");
            }
        }
        assert_eq!(render_pad_help(), render_pad_help());
        // Every row fits the panel: the last row's baseline stays inside.
        let rows = SCHEME.len() as u32 + 1;
        assert!(rows * LINE_HEIGHT + 12 + 4 <= PAD_HEIGHT, "the scheme overflows the panel");
    }
}
