//! Input state.
//!
//! Kept free of any camera or rendering type so it can be unit-tested without
//! a window: the event loop pushes raw events in, and the game loop reads
//! movement intent out.

use std::collections::HashSet;

use glam::Vec3;
use winit::keyboard::KeyCode;

/// Which keys are down, and how far the mouse has moved since last read.
#[derive(Debug, Default, Clone)]
pub struct InputState {
    pressed: HashSet<KeyCode>,
    /// Accumulated since the last [`InputState::take_mouse_delta`], in pixels.
    mouse_delta: (f32, f32),
    /// Whether the mouse is captured for looking around.
    pub mouse_captured: bool,
    /// Analog movement from a gamepad's stick, in the same camera-local
    /// axes as [`InputState::movement_axes`]: `x` right, `z` forward.
    /// Summed with the keys, so pad and keyboard cannot fight — whoever
    /// pushes, the player moves.
    pad_axes: Vec3,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn press(&mut self, key: KeyCode) {
        self.pressed.insert(key);
    }

    pub fn release(&mut self, key: KeyCode) {
        self.pressed.remove(&key);
    }

    pub fn is_down(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// Drop every held key. Called when the window loses focus, so keys held
    /// at that moment do not stick down forever.
    pub fn clear_keys(&mut self) {
        self.pressed.clear();
    }

    /// The pad's stick, written once per frame by whoever polls the pad.
    /// Zero when no pad is tilted, which restores keyboard-only behaviour
    /// exactly.
    pub fn set_pad_axes(&mut self, axes: Vec3) {
        self.pad_axes = axes;
    }

    /// The pad's stick as last written, undeadzoned and unmerged.
    ///
    /// [`InputState::movement_axes`] is the wrong thing for the walk
    /// sampler: it merges the keys in and normalises, and the sampler needs
    /// the stick on its own to turn into bits.
    pub fn pad_axes(&self) -> Vec3 {
        self.pad_axes
    }

    pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Read and reset the accumulated mouse movement.
    pub fn take_mouse_delta(&mut self) -> (f32, f32) {
        std::mem::take(&mut self.mouse_delta)
    }

    /// Movement intent in camera-local axes: `x` right, `y` up, `z` forward.
    ///
    /// Normalised, so moving diagonally is not faster than moving straight —
    /// the classic bug where holding W+D outruns W alone.
    pub fn movement_axes(&self) -> Vec3 {
        let axis = |negative: KeyCode, positive: KeyCode| -> f32 {
            f32::from(self.is_down(positive)) - f32::from(self.is_down(negative))
        };

        let raw = Vec3::new(
            axis(KeyCode::KeyA, KeyCode::KeyD),
            axis(KeyCode::ShiftLeft, KeyCode::Space),
            axis(KeyCode::KeyS, KeyCode::KeyW),
        ) + self.pad_axes;

        if raw.length_squared() > 1.0 {
            raw.normalize()
        } else {
            raw
        }
    }

    /// True while the sprint modifier is held.
    pub fn is_sprinting(&self) -> bool {
        self.is_down(KeyCode::ControlLeft)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn pad_axes_merge_with_keys_and_cap_at_full_speed() {
        use super::*;
        let mut input = InputState::new();
        // Stick alone: half tilt is half speed.
        input.set_pad_axes(Vec3::new(0.0, 0.0, 0.5));
        assert_eq!(input.movement_axes(), Vec3::new(0.0, 0.0, 0.5));
        // Stick and key together never outrun a full press.
        input.press(KeyCode::KeyW);
        assert!((input.movement_axes().length() - 1.0).abs() < 1.0e-5);
        // Stick released: keyboard-only behaviour is restored exactly.
        input.set_pad_axes(Vec3::ZERO);
        assert_eq!(input.movement_axes(), Vec3::new(0.0, 0.0, 1.0));
    }

    use super::*;

    #[test]
    fn keys_track_press_and_release() {
        let mut input = InputState::new();
        assert!(!input.is_down(KeyCode::KeyW));

        input.press(KeyCode::KeyW);
        assert!(input.is_down(KeyCode::KeyW));

        input.release(KeyCode::KeyW);
        assert!(!input.is_down(KeyCode::KeyW));
    }

    #[test]
    fn pressing_twice_and_releasing_once_leaves_the_key_up() {
        // Key repeat fires press events repeatedly; release must still work.
        let mut input = InputState::new();
        input.press(KeyCode::KeyW);
        input.press(KeyCode::KeyW);
        input.release(KeyCode::KeyW);
        assert!(!input.is_down(KeyCode::KeyW));
    }

    #[test]
    fn losing_focus_clears_held_keys() {
        let mut input = InputState::new();
        input.press(KeyCode::KeyW);
        input.press(KeyCode::KeyA);

        input.clear_keys();

        assert!(!input.is_down(KeyCode::KeyW));
        assert!(!input.is_down(KeyCode::KeyA));
        assert_eq!(input.movement_axes(), Vec3::ZERO);
    }

    #[test]
    fn no_input_means_no_movement() {
        assert_eq!(InputState::new().movement_axes(), Vec3::ZERO);
    }

    #[test]
    fn forward_and_back_map_to_the_z_axis() {
        let mut input = InputState::new();
        input.press(KeyCode::KeyW);
        assert_eq!(input.movement_axes(), Vec3::new(0.0, 0.0, 1.0));

        input.release(KeyCode::KeyW);
        input.press(KeyCode::KeyS);
        assert_eq!(input.movement_axes(), Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn opposing_keys_cancel_out() {
        let mut input = InputState::new();
        input.press(KeyCode::KeyW);
        input.press(KeyCode::KeyS);
        input.press(KeyCode::KeyA);
        input.press(KeyCode::KeyD);
        assert_eq!(input.movement_axes(), Vec3::ZERO);
    }

    #[test]
    fn diagonal_movement_is_not_faster_than_straight() {
        let mut input = InputState::new();
        input.press(KeyCode::KeyW);
        let straight = input.movement_axes().length();

        input.press(KeyCode::KeyD);
        let diagonal = input.movement_axes().length();

        assert!((straight - 1.0).abs() < 1e-6);
        assert!(
            (diagonal - 1.0).abs() < 1e-6,
            "diagonal speed {diagonal} should match straight speed"
        );
    }

    #[test]
    fn vertical_movement_uses_space_and_shift() {
        let mut input = InputState::new();
        input.press(KeyCode::Space);
        assert_eq!(input.movement_axes(), Vec3::new(0.0, 1.0, 0.0));

        input.release(KeyCode::Space);
        input.press(KeyCode::ShiftLeft);
        assert_eq!(input.movement_axes(), Vec3::new(0.0, -1.0, 0.0));
    }

    #[test]
    fn mouse_delta_accumulates_then_resets_when_read() {
        let mut input = InputState::new();
        input.add_mouse_delta(3.0, -2.0);
        input.add_mouse_delta(1.0, 0.5);

        assert_eq!(input.take_mouse_delta(), (4.0, -1.5));
        // Reading consumes it, so movement is never applied twice.
        assert_eq!(input.take_mouse_delta(), (0.0, 0.0));
    }
}
