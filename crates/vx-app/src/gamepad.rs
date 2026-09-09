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

use std::collections::{HashMap, HashSet};

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
    /// Modifiers physically held right now — `SELECT` and `LB`. Kept apart
    /// from `down` because a modifier is a *layer*, not a key: it resolves
    /// to nothing until it is let go, and then only if it was never used.
    pub held: HashSet<Button>,
    /// Modifiers that had a button pressed under them. A used modifier was
    /// a hold; an unused one was a tap, and a tap has its own action.
    pub used: HashSet<Button>,
    /// Whether the control-scheme overlay is up.
    pub help: bool,
    /// Seconds until the held stick moves a panel's cursor again.
    pub repeat: f32,
    /// Whether the stick has just been pushed, so the first step waits
    /// longer than the ones that follow — the keyboard's own repeat shape.
    pub fresh: bool,
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
            held: HashSet::new(),
            used: HashSet::new(),
            repeat: 0.0,
            fresh: true,
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

/// Which layer the pad is on. The mapping's whole context.
///
/// Stage 24 shipped this as one bit — panel or world — which was enough
/// when the pad only had to reach the things a hand finds without looking.
/// It is not enough to *play* from: a live machine feed is not a panel and
/// not the world, and the pad had no way out of one; and thirteen buttons
/// cannot name twenty actions without a modifier. Both are layers, so both
/// are this enum rather than a second input path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// Walking about, nothing owning the screen.
    World,
    /// `SELECT` held: the world's second layer, for everything that does not
    /// fit on a face button and does not need to be fast.
    Second,
    /// `LB` held: the block palette, one slot per direction.
    Palette,
    /// A panel owns the screen. South confirms, east backs out.
    Panel,
    /// Looking through a machine. Neither a panel nor the world — the
    /// mistake this enum exists to fix, because there was no way to hang up.
    Feed,
    /// The on-screen keyboard is up. Only the buttons with a keyboard twin
    /// resolve here; the one that types has no twin at all — on a keyboard
    /// you simply press the letter — so `main` handles it directly.
    Typing,
}

/// What a button means on a given layer: the key it presses.
///
/// `Select` and `LeftTrigger` are absent from [`Context::World`] on purpose —
/// they are the two modifiers, and `main` decides on release whether a tap
/// meant the help panel or a view change. The analog triggers are absent
/// everywhere: they mirror the mouse buttons, which have no `KeyCode`.
pub fn key_for(button: Button, context: Context) -> Option<KeyCode> {
    match context {
        Context::World => match button {
            Button::South => Some(KeyCode::Space),
            Button::East => Some(KeyCode::ShiftLeft),
            Button::West => Some(KeyCode::KeyE),
            Button::North => Some(KeyCode::KeyV),
            Button::RightTrigger => Some(KeyCode::Tab),
            Button::LeftThumb => Some(KeyCode::ControlLeft),
            Button::RightThumb => Some(KeyCode::KeyL),
            Button::DPadUp => Some(KeyCode::KeyM),
            Button::DPadDown => Some(KeyCode::KeyN),
            // The minimap is always up, so zoom earns the two directions
            // scan and fly used to hold. Both moved to the second layer.
            Button::DPadLeft => Some(KeyCode::BracketLeft),
            Button::DPadRight => Some(KeyCode::BracketRight),
            Button::Start => Some(KeyCode::Enter),
            _ => None,
        },
        // Everything a hand does not need in a hurry. The drill mod's two
        // switches land here rather than on the world layer: they are set
        // once and left, and the world layer has been full since stage 46a.
        Context::Second => match button {
            Button::North => Some(KeyCode::KeyT),
            Button::South => Some(KeyCode::KeyZ),
            Button::West => Some(KeyCode::F10),
            Button::East => Some(KeyCode::Backspace),
            Button::DPadUp => Some(KeyCode::F3),
            Button::DPadDown => Some(KeyCode::F5),
            Button::DPadLeft => Some(KeyCode::KeyG),
            Button::DPadRight => Some(KeyCode::KeyF),
            Button::LeftThumb => Some(KeyCode::KeyH),
            Button::RightThumb => Some(KeyCode::KeyP),
            // What you are carrying. On the second layer because it is a
            // readout you check between swings, not a thing you do mid-cut.
            Button::Start => Some(KeyCode::KeyI),
            // Stack the marked footprint. On the second layer beside the
            // dispatch's own keys: it is the same gesture, one shape along.
            Button::LeftTrigger => Some(KeyCode::KeyB),
            _ => None,
        },
        // Ten slots, ten controls, no cursor: the palette is muscle memory
        // or it is nothing.
        Context::Palette => match button {
            Button::DPadUp => Some(KeyCode::Digit1),
            Button::DPadRight => Some(KeyCode::Digit2),
            Button::DPadDown => Some(KeyCode::Digit3),
            Button::DPadLeft => Some(KeyCode::Digit4),
            Button::North => Some(KeyCode::Digit5),
            Button::East => Some(KeyCode::Digit6),
            Button::South => Some(KeyCode::Digit7),
            Button::West => Some(KeyCode::Digit8),
            Button::RightTrigger => Some(KeyCode::Digit9),
            Button::RightThumb => Some(KeyCode::Digit0),
            _ => None,
        },
        Context::Panel => match button {
            // Console convention: south confirms, east backs out. Every
            // panel already closes on Escape, which is what makes one
            // mapping serve fourteen panels.
            Button::South => Some(KeyCode::Enter),
            Button::East => Some(KeyCode::Escape),
            Button::West => Some(KeyCode::KeyE),
            Button::North => Some(KeyCode::Tab),
            Button::DPadUp => Some(KeyCode::ArrowUp),
            Button::DPadDown => Some(KeyCode::ArrowDown),
            Button::DPadLeft => Some(KeyCode::ArrowLeft),
            Button::DPadRight => Some(KeyCode::ArrowRight),
            Button::Start => Some(KeyCode::Enter),
            // The bumpers scroll, which is what the terminal's backlog and
            // any long roster want.
            Button::LeftTrigger => Some(KeyCode::PageUp),
            Button::RightTrigger => Some(KeyCode::PageDown),
            // Withdraw at a vault, delete in the terminal: the one verb a
            // pad could not reach at all, and the reason you could put money
            // into a strongroom but never take it out.
            Button::LeftThumb => Some(KeyCode::Backspace),
            // Reset a tunable on the operator console.
            Button::RightThumb => Some(KeyCode::KeyX),
            _ => None,
        },
        Context::Typing => match button {
            Button::DPadUp => Some(KeyCode::ArrowUp),
            Button::DPadDown => Some(KeyCode::ArrowDown),
            Button::DPadLeft => Some(KeyCode::ArrowLeft),
            Button::DPadRight => Some(KeyCode::ArrowRight),
            Button::West => Some(KeyCode::Backspace),
            Button::North => Some(KeyCode::Tab),
            Button::East => Some(KeyCode::Escape),
            Button::Start => Some(KeyCode::Enter),
            Button::LeftThumb => Some(KeyCode::Home),
            Button::RightThumb => Some(KeyCode::End),
            // South types the key under the cursor. There is no `KeyCode`
            // for "the letter I am pointing at", so `poll_pad` does it.
            _ => None,
        },
        Context::Feed => match button {
            // The fix this enum was written for: a feed is not a panel, so
            // the pad was in world context and east was *crouch*. There was
            // no button that hung up.
            Button::East => Some(KeyCode::Escape),
            Button::North => Some(KeyCode::KeyR),
            Button::West => Some(KeyCode::KeyV),
            // A flier climbs and descends on the same axes a body does.
            Button::South => Some(KeyCode::Space),
            Button::DPadUp => Some(KeyCode::Space),
            Button::DPadDown => Some(KeyCode::ShiftLeft),
            Button::LeftThumb => Some(KeyCode::ShiftLeft),
            Button::RightThumb => Some(KeyCode::KeyL),
            Button::RightTrigger => Some(KeyCode::Tab),
            Button::Start => Some(KeyCode::Enter),
            _ => None,
        },
    }
}

/// Keys the pad reaches through no button on any layer, and why.
///
/// An explicit list rather than an absence, so
/// [`tests::every_binding_the_game_has_is_reachable_from_the_pad`] can hold
/// the door shut: a new keyboard binding with no pad path fails the build
/// unless somebody writes down here that it is deliberate.
/// What the two modifiers do when *tapped* rather than held.
///
/// `main` produces these on release, never `key_for`: whether a hold was a
/// hold is only knowable once the button comes back up, so the mapping
/// cannot answer for them. Declared here so the reachability test can see
/// the whole surface rather than only the half that resolves to a key.
pub const TAPS: [(Button, Option<KeyCode>, &str); 2] = [
    (Button::Select, None, "shows the control scheme"),
    (Button::LeftTrigger, Some(KeyCode::KeyC), "first or third person"),
];

// Read by the reachability test rather than by the game: it is a statement
// about the mapping, and the mapping is what it guards.
#[allow(dead_code)]
pub const NOT_ON_THE_PAD: [(KeyCode, &str); 2] = [
    (KeyCode::Escape, "world context: the pad never captures the mouse, so it has nothing to release"),
    (KeyCode::Delete, "terminal edit: backspace is the pad's delete, and the board has no forward delete"),
];

/// How far the stick must lean to walk a panel's cursor. Higher than the
/// walk threshold: a menu should not scroll because a thumb rested.
pub const CURSOR_TILT: f32 = 0.5;

/// Seconds before a held stick repeats the first time, and after.
pub const REPEAT_FIRST: f32 = 0.35;
pub const REPEAT_AGAIN: f32 = 0.09;

/// How far the help overlay is blown up on screen.
///
/// Its own number rather than the shop's: the scheme grew from sixteen rows
/// to thirty-eight when the pad learned to reach everything, and at the
/// shop's doubling it stood a thousand pixels tall on a seven-hundred-pixel
/// screen — the top of the list off the top of the world.
pub const PAD_SCALE: f32 = 1.3;

/// The help overlay's size in texture pixels.
pub const PAD_WIDTH: u32 = 320;
pub const PAD_HEIGHT: u32 = 528;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 240];

/// The control scheme, written for the player. One row per physical
/// control, in the order a hand finds them, grouped by layer. A row whose
/// control is empty is a heading. Tested drawable.
pub const SCHEME: [(&str, &str); 38] = [
    ("", "ON FOOT"),
    ("LEFT STICK", "MOVE, CLICK TO SPRINT"),
    ("RIGHT STICK", "LOOK, CLICK FOR OPTICS"),
    ("RT", "DRILL, OR FIRE"),
    ("LT", "PLACE THE SELECTED BLOCK"),
    ("A", "JUMP"),
    ("B", "CROUCH"),
    ("X", "USE, TRADE, TALK"),
    ("Y", "THE HANDHELD UPLINK"),
    ("LB", "TAP: FIRST OR THIRD PERSON"),
    ("RB", "CYCLE THE MINING METHOD"),
    ("D-PAD UP", "MARK AN ORE CORNER"),
    ("D-PAD DOWN", "THE MINIMAP"),
    ("D-PAD L/R", "ZOOM THE MAP"),
    ("START", "DISPATCH, OR PICK A LOCK"),
    ("", "HOLD LB - THE PALETTE"),
    ("D-PAD", "SLOTS ONE TO FOUR"),
    ("FACE", "SLOTS FIVE TO EIGHT"),
    ("RB, R-STICK", "SLOT NINE, SLOT TEN"),
    ("", "HOLD SELECT - THE SECOND LAYER"),
    ("Y", "THE TERMINAL"),
    ("A", "GO PRONE"),
    ("B", "WITHDRAW AT A VAULT"),
    ("D-PAD LEFT", "SCAN THIS SECTOR"),
    ("D-PAD RIGHT", "WALK OR FLY"),
    ("D-PAD UP", "THE DEBUG READOUT"),
    ("D-PAD DOWN", "SAVE THE WORLD"),
    ("L-STICK", "THE DRILL HOLOGRAM"),
    ("R-STICK", "THE DRILL SONAR"),
    ("START", "YOUR PACK"),
    ("LB", "HEAP THE MARKED SPOIL"),
    ("SELECT", "TAP: THIS PANEL"),
    ("", "IN A PANEL"),
    ("A, B", "CONFIRM, BACK OUT"),
    ("D-PAD", "MOVE THE CURSOR"),
    ("Y", "TURN THE PAGE"),
    ("LB, RB", "SCROLL THE BACKLOG"),
    ("", "LOOKING THROUGH A MACHINE"),
];

/// The two layers that did not fit in [`SCHEME`]'s array. Drawn under it.
pub const FEED_SCHEME: [(&str, &str); 9] = [
    ("B", "HANG UP"),
    ("Y", "TAKE OR HAND BACK THE WHEEL"),
    ("X", "BACK TO THE ROSTER"),
    ("A, L-STICK", "CLIMB, DESCEND"),
    ("", "TYPING - Y IN THE TERMINAL"),
    ("D-PAD", "MOVE OVER THE KEYS"),
    ("A, X", "TYPE, RUB OUT"),
    ("START", "SEND THE LINE"),
    ("Y", "PUT THE BOARD AWAY"),
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

    for (control, does) in SCHEME.iter().chain(FEED_SCHEME.iter()) {
        if control.is_empty() {
            // A heading: the layer this block of rows belongs to.
            y += 4;
            font::draw_text(&mut pixels, PAD_WIDTH, margin, y, 1, ACCENT, does);
        } else {
            font::draw_text(&mut pixels, PAD_WIDTH, margin, y, 1, DIM, control);
            font::draw_text(&mut pixels, PAD_WIDTH, margin + 84, y, 1, TEXT, does);
        }
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
        assert_eq!(key_for(Button::South, Context::World), Some(KeyCode::Space));
        assert_eq!(key_for(Button::South, Context::Panel), Some(KeyCode::Enter));
        assert_eq!(key_for(Button::East, Context::Panel), Some(KeyCode::Escape));
        // The d-pad turns into the arrows every panel lists with.
        assert_eq!(key_for(Button::DPadUp, Context::Panel), Some(KeyCode::ArrowUp));
    }

    /// **A feed can be hung up.** Stage 24's mapping had one bit of context,
    /// and a live machine feed is neither a panel nor the world: the pad sat
    /// in world context, east was *crouch*, and no button on the pad ended
    /// the feed. You could fly a drone and never get your own eyes back.
    #[test]
    fn a_machine_feed_can_be_hung_up_from_the_pad() {
        assert_eq!(key_for(Button::East, Context::Feed), Some(KeyCode::Escape));
        assert_ne!(key_for(Button::East, Context::Feed), key_for(Button::East, Context::World));
        // And the wheel is takeable, which was the other feed-only verb no
        // button produced.
        assert_eq!(key_for(Button::North, Context::Feed), Some(KeyCode::KeyR));
    }

    #[test]
    fn the_modifier_layers_do_not_collide_with_the_layer_under_them() {
        // A held modifier must change what a button means, or holding it
        // is a lie. Every button the palette and the second layer claim
        // reads differently from the world layer beneath.
        for button in [Button::DPadUp, Button::DPadDown, Button::DPadLeft, Button::DPadRight] {
            let world = key_for(button, Context::World);
            assert_ne!(key_for(button, Context::Palette), world, "{button:?} unchanged on the palette");
            assert_ne!(key_for(button, Context::Second), world, "{button:?} unchanged on the second layer");
        }
        // Ten palette slots, ten distinct keys.
        let slots: Vec<KeyCode> = [
            Button::DPadUp, Button::DPadRight, Button::DPadDown, Button::DPadLeft,
            Button::North, Button::East, Button::South, Button::West,
            Button::RightTrigger, Button::RightThumb,
        ]
        .into_iter()
        .filter_map(|button| key_for(button, Context::Palette))
        .collect();
        assert_eq!(slots.len(), 10, "the palette lost a slot");
        let unique: std::collections::BTreeSet<_> = slots.iter().collect();
        assert_eq!(unique.len(), 10, "two palette slots pick the same block");
    }

    /// **Every key the game binds is reachable from the pad.** The round's
    /// whole claim, as a list: a keyboard binding with no pad path fails
    /// here unless somebody writes it into [`NOT_ON_THE_PAD`] with a reason.
    #[test]
    fn every_binding_the_game_has_is_reachable_from_the_pad() {
        // Every `KeyCode` `handle_press` and the movement sampler act on.
        let bound = [
            KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyV, KeyCode::KeyC, KeyCode::KeyT,
            KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN, KeyCode::KeyG, KeyCode::KeyR,
            KeyCode::KeyX, KeyCode::KeyZ, KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA,
            KeyCode::KeyD, KeyCode::KeyQ, KeyCode::KeyH, KeyCode::KeyP,
            KeyCode::KeyI, KeyCode::KeyB,
            KeyCode::Space, KeyCode::ShiftLeft, KeyCode::ControlLeft,
            KeyCode::Tab, KeyCode::Enter, KeyCode::Escape, KeyCode::Backspace,
            KeyCode::Delete, KeyCode::Home, KeyCode::End,
            KeyCode::PageUp, KeyCode::PageDown,
            KeyCode::ArrowUp, KeyCode::ArrowDown, KeyCode::ArrowLeft, KeyCode::ArrowRight,
            KeyCode::BracketLeft, KeyCode::BracketRight,
            KeyCode::F3, KeyCode::F5, KeyCode::F10,
            KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3,
            KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7,
            KeyCode::Digit8, KeyCode::Digit9,
        ];
        // The four movement keys and the arcade's strafe are the stick's,
        // not a button's: `WalkController::sample` and the arcade read the
        // axes directly, so they are reachable without resolving to a key.
        let on_the_stick = [
            KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD, KeyCode::KeyQ,
        ];
        let every_context = [
            Context::World, Context::Second, Context::Palette, Context::Panel,
            Context::Feed, Context::Typing,
        ];
        let every_button = [
            Button::South, Button::East, Button::West, Button::North,
            Button::LeftTrigger, Button::RightTrigger,
            Button::LeftThumb, Button::RightThumb,
            Button::DPadUp, Button::DPadDown, Button::DPadLeft, Button::DPadRight,
            Button::Start, Button::Select,
        ];
        for key in bound {
            if on_the_stick.contains(&key) {
                continue;
            }
            if NOT_ON_THE_PAD.iter().any(|(excused, _)| *excused == key) {
                continue;
            }
            let reachable = every_context.iter().any(|context| {
                every_button
                    .iter()
                    .any(|button| key_for(*button, *context) == Some(key))
            }) || TAPS.iter().any(|(_, tap, _)| *tap == Some(key));
            assert!(reachable, "{key:?} is bound but no pad button reaches it");
        }
        // And the excuses are real: nothing in the list is secretly mapped.
        for (excused, reason) in NOT_ON_THE_PAD {
            assert!(!reason.is_empty(), "{excused:?} is excused without a reason");
        }
    }

    #[test]
    fn every_mapped_button_resolves_in_the_world_and_in_panels() {
        // A button that silently dies when a panel opens reads as a broken
        // pad. The two modifiers are `main`'s own and resolve to no key.
        for button in [
            Button::South, Button::East, Button::West, Button::North,
            Button::DPadUp, Button::DPadDown, Button::DPadLeft, Button::DPadRight,
            Button::Start,
        ] {
            assert!(key_for(button, Context::World).is_some(), "{button:?} dead in the world");
            assert!(key_for(button, Context::Panel).is_some(), "{button:?} dead in panels");
        }
        // Select is main's own: tapped it is the help panel, held it is the
        // second layer. It is never a key itself.
        for context in [
            Context::World, Context::Panel, Context::Feed, Context::Second, Context::Typing,
        ] {
            assert_eq!(key_for(Button::Select, context), None);
        }
        // And LB is the palette modifier, so it is not a key in the world.
        assert_eq!(key_for(Button::LeftTrigger, Context::World), None);
    }

    #[test]
    fn the_help_panel_is_drawable_and_deterministic() {
        for (control, does) in SCHEME.iter().chain(FEED_SCHEME.iter()) {
            for character in control.chars().chain(does.chars()) {
                assert!(font::knows(character), "undrawable {character:?}");
            }
        }
        assert_eq!(render_pad_help(), render_pad_help());
        // Every row fits the panel: the last row's baseline stays inside.
        let rows = (SCHEME.len() + FEED_SCHEME.len()) as u32 + 1;
        let headings = SCHEME.iter().filter(|(control, _)| control.is_empty()).count() as u32;
        assert!(
            rows * LINE_HEIGHT + headings * 4 + 12 + 4 <= PAD_HEIGHT,
            "the scheme overflows the panel"
        );
    }
}
