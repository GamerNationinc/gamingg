//! Turning input into camera motion.
//!
//! Separated from the event loop so it can be tested without a window: the
//! logic is a pure function of (camera, input, elapsed time).
//!
//! Controllers own *orientation* and, when walking, the body. They do not own
//! where the camera sits — `view::camera_placement` decides that once per
//! frame, which is what lets third person exist without a second controller.

use glam::Vec3;
use vx_platform::InputState;
use vx_render::Camera;
use winit::keyboard::KeyCode;

use crate::movement::{self, MoveCommand};

/// How the player moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementMode {
    /// Free flight, ignoring terrain. Useful for building and debugging.
    Fly,
    /// Walking, subject to gravity and collision.
    Walk,
}

impl MovementMode {
    pub fn toggled(self) -> Self {
        match self {
            MovementMode::Fly => MovementMode::Walk,
            MovementMode::Walk => MovementMode::Fly,
        }
    }
}

/// Apply accumulated mouse movement to the camera's orientation.
///
/// Shared by both controllers so look behaviour cannot drift between them. The
/// delta is consumed even when the mouse is not captured, so motion that
/// arrived while the pointer was free is discarded rather than replayed as a
/// snap the moment capture resumes.
pub fn apply_mouse_look(camera: &mut Camera, input: &mut InputState, sensitivity: f32) {
    let (dx, dy) = input.take_mouse_delta();
    if !input.mouse_captured {
        return;
    }
    camera.yaw += dx * sensitivity;
    // Screen y grows downward, so pushing the mouse down looks down.
    camera.pitch -= dy * sensitivity;
    camera.clamp_pitch();
}

/// Free-flying camera controls.
#[derive(Debug, Clone, Copy)]
pub struct FlyController {
    /// Blocks per second.
    pub speed: f32,
    /// Multiplier applied while sprinting.
    pub sprint_multiplier: f32,
    /// Radians of rotation per pixel of mouse movement.
    pub sensitivity: f32,
}

impl Default for FlyController {
    fn default() -> Self {
        FlyController {
            speed: 24.0,
            sprint_multiplier: 4.0,
            sensitivity: 0.0022,
        }
    }
}

impl FlyController {
    /// Advance the camera by `dt` seconds of input.
    pub fn apply(&self, camera: &mut Camera, input: &mut InputState, dt: f32) {
        apply_mouse_look(camera, input, self.sensitivity);

        let axes = input.movement_axes();
        if axes == Vec3::ZERO {
            return;
        }

        let speed = if input.is_sprinting() {
            self.speed * self.sprint_multiplier
        } else {
            self.speed
        };

        // Horizontal movement follows where you are looking, but stays level:
        // flying should not sink because you glanced down.
        let motion = camera.right() * axes.x + Vec3::Y * axes.y + camera.forward_level() * axes.z;
        camera.position += (motion * speed * dt).as_dvec3();
    }
}

/// Walking controls.
///
/// This is a *sampler*, not an integrator. It reads the held keys and the
/// pad's stick once per frame and hands back a [`MoveCommand`], which is a
/// bitfield either can set; the simulation then consumes one
/// command per movement tick. Nothing here touches the body, and nothing here
/// sees a `dt` — that is the whole point of the movement round. See
/// [`crate::movement`] for why.
#[derive(Debug, Clone, Copy)]
pub struct WalkController {
    pub sensitivity: f32,
}

impl Default for WalkController {
    fn default() -> Self {
        WalkController { sensitivity: 0.0022 }
    }
}

impl WalkController {
    /// Turn this frame's input into one command.
    ///
    /// Mouse look still applies at frame rate — the camera's orientation is a
    /// presentation concern and nobody replays it — but the angle that reaches
    /// the simulation is the quantised one carried in the command.
    pub fn sample(&self, camera: &mut Camera, input: &mut InputState) -> MoveCommand {
        apply_mouse_look(camera, input, self.sensitivity);

        let mut bits = 0u16;
        let mut set = |key: KeyCode, bit: u16| {
            if input.is_down(key) {
                bits |= bit;
            }
        };
        set(KeyCode::KeyW, movement::FWD);
        set(KeyCode::KeyS, movement::BACK);
        set(KeyCode::KeyA, movement::LEFT);
        set(KeyCode::KeyD, movement::RIGHT);
        set(KeyCode::Space, movement::JUMP);
        set(KeyCode::ControlLeft, movement::SPRINT);
        set(KeyCode::ShiftLeft, movement::CROUCH);
        set(KeyCode::KeyZ, movement::PRONE);

        // The pad walks too. `MoveCommand` is a bitfield, so a tilt becomes
        // the same bits a key press would — eight directions — and the *speed*
        // rides beside them in the throttle byte. The stick is already
        // radially deadzoned and rescaled in `gamepad::deadzoned`, so this
        // threshold is dead travel on top of that rather than a second
        // deadzone. Sign convention is the one `poll_pad` packs: `x` right,
        // `z` forward, matching W/S and A/D.
        const TILT: f32 = 0.35;
        let pad = input.pad_axes();
        if pad.z > TILT {
            bits |= movement::FWD;
        }
        if pad.z < -TILT {
            bits |= movement::BACK;
        }
        if pad.x > TILT {
            bits |= movement::RIGHT;
        }
        if pad.x < -TILT {
            bits |= movement::LEFT;
        }

        // How hard the stick is pushed, in 255ths. A key is all-or-nothing
        // and wins any tie: holding W while nudging the stick walks at full
        // pace, which is the only reading that is not a surprise.
        let keyed = [KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD]
            .into_iter()
            .any(|key| input.is_down(key));
        let lean = glam::Vec2::new(pad.x, pad.z).length().min(1.0);
        let throttle = if keyed || lean <= 0.0 {
            movement::FULL_THROTTLE
        } else {
            (lean * f32::from(movement::FULL_THROTTLE)).round() as u8
        };

        MoveCommand::looking(bits, camera.yaw, camera.pitch).throttled(throttle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    fn setup() -> (Camera, InputState, FlyController) {
        (
            Camera {
                position: glam::DVec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
                ..Camera::default()
            },
            InputState::new(),
            FlyController::default(),
        )
    }

    #[test]
    fn with_no_input_the_camera_does_not_move() {
        let (mut camera, mut input, controller) = setup();
        controller.apply(&mut camera, &mut input, 0.016);
        assert_eq!(camera.position, glam::DVec3::ZERO);
        assert_eq!(camera.yaw, 0.0);
    }

    #[test]
    fn pressing_forward_moves_along_the_view_direction() {
        let (mut camera, mut input, controller) = setup();
        input.press(KeyCode::KeyW);

        controller.apply(&mut camera, &mut input, 1.0);

        // Default orientation looks down -Z.
        assert!(camera.position.z < 0.0, "moved {:?}", camera.position);
        assert!((camera.position.length() as f32 - controller.speed).abs() < 1e-3);
    }

    #[test]
    fn movement_scales_with_elapsed_time() {
        let (mut camera, mut input, controller) = setup();
        input.press(KeyCode::KeyW);

        controller.apply(&mut camera, &mut input, 0.5);
        let half = camera.position.length();

        camera.position = glam::DVec3::ZERO;
        input.press(KeyCode::KeyW);
        controller.apply(&mut camera, &mut input, 1.0);
        let full = camera.position.length();

        assert!((full - half * 2.0).abs() < 1e-3, "{half} then {full}");
    }

    #[test]
    fn sprinting_is_faster_than_walking() {
        let (mut camera, mut input, controller) = setup();
        input.press(KeyCode::KeyW);
        controller.apply(&mut camera, &mut input, 1.0);
        let walk = camera.position.length() as f32;

        camera.position = glam::DVec3::ZERO;
        input.press(KeyCode::ControlLeft);
        controller.apply(&mut camera, &mut input, 1.0);
        let sprint = camera.position.length() as f32;

        assert!((sprint / walk - controller.sprint_multiplier).abs() < 1e-3);
    }

    #[test]
    fn looking_up_does_not_change_where_forward_takes_you() {
        // Fly movement is level: pitching up then pressing forward should not
        // gain altitude, or the camera drifts skyward whenever you look up.
        let (mut camera, mut input, controller) = setup();
        camera.pitch = 1.0;
        input.press(KeyCode::KeyW);

        controller.apply(&mut camera, &mut input, 1.0);

        assert!(
            camera.position.y.abs() < 1e-5,
            "forward movement gained {} altitude",
            camera.position.y
        );
    }

    #[test]
    fn space_and_shift_move_straight_up_and_down() {
        let (mut camera, mut input, controller) = setup();
        input.press(KeyCode::Space);
        controller.apply(&mut camera, &mut input, 1.0);
        assert!(camera.position.y > 0.0);
        assert!(camera.position.x.abs() < 1e-6 && camera.position.z.abs() < 1e-6);

        input.release(KeyCode::Space);
        input.press(KeyCode::ShiftLeft);
        controller.apply(&mut camera, &mut input, 1.0);
        assert!(camera.position.y.abs() < 1e-5, "up then down should cancel");
    }

    #[test]
    fn mouse_look_only_applies_while_captured() {
        let (mut camera, mut input, controller) = setup();

        input.mouse_captured = false;
        input.add_mouse_delta(100.0, 50.0);
        controller.apply(&mut camera, &mut input, 0.016);
        assert_eq!(camera.yaw, 0.0, "view moved while the mouse was free");

        input.mouse_captured = true;
        input.add_mouse_delta(100.0, 0.0);
        controller.apply(&mut camera, &mut input, 0.016);
        assert!(camera.yaw > 0.0, "view did not turn while captured");
    }

    #[test]
    fn releasing_the_mouse_does_not_bank_up_motion_for_later() {
        // Motion arriving while uncaptured must be discarded, not replayed as
        // a snap the moment capture is re-enabled.
        let (mut camera, mut input, controller) = setup();
        input.mouse_captured = false;
        input.add_mouse_delta(500.0, 500.0);
        controller.apply(&mut camera, &mut input, 0.016);

        input.mouse_captured = true;
        controller.apply(&mut camera, &mut input, 0.016);

        assert_eq!(camera.yaw, 0.0);
        assert_eq!(camera.pitch, 0.0);
    }

    #[test]
    fn moving_the_mouse_down_pitches_the_view_down() {
        let (mut camera, mut input, controller) = setup();
        input.mouse_captured = true;
        input.add_mouse_delta(0.0, 100.0);

        controller.apply(&mut camera, &mut input, 0.016);

        assert!(camera.pitch < 0.0, "inverted vertical look");
    }

    #[test]
    fn pitch_cannot_pass_straight_up() {
        let (mut camera, mut input, controller) = setup();
        input.mouse_captured = true;
        for _ in 0..200 {
            input.add_mouse_delta(0.0, -500.0);
            controller.apply(&mut camera, &mut input, 0.016);
        }
        assert!(camera.pitch < std::f32::consts::FRAC_PI_2);
        assert!(camera.forward().is_finite());
    }

    fn sampler() -> (Camera, InputState, WalkController) {
        (
            Camera { yaw: 0.0, pitch: 0.0, ..Camera::default() },
            InputState::new(),
            WalkController::default(),
        )
    }

    #[test]
    fn movement_mode_toggles_between_the_two_states() {
        assert_eq!(MovementMode::Fly.toggled(), MovementMode::Walk);
        assert_eq!(MovementMode::Walk.toggled(), MovementMode::Fly);
        assert_eq!(MovementMode::Fly.toggled().toggled(), MovementMode::Fly);
    }

    #[test]
    fn held_keys_become_bits_in_a_command() {
        let (mut camera, mut input, controller) = sampler();
        input.press(KeyCode::KeyW);
        input.press(KeyCode::ControlLeft);

        let command = controller.sample(&mut camera, &mut input);

        assert!(command.held(movement::FWD));
        assert!(command.held(movement::SPRINT));
        assert!(!command.held(movement::BACK));
        assert!(!command.held(movement::CROUCH));
    }

    #[test]
    fn every_movement_key_reaches_the_command() {
        // A key that samples into nothing is a control that silently does not
        // exist, which is exactly the kind of gap a test should hold shut.
        let cases = [
            (KeyCode::KeyW, movement::FWD),
            (KeyCode::KeyS, movement::BACK),
            (KeyCode::KeyA, movement::LEFT),
            (KeyCode::KeyD, movement::RIGHT),
            (KeyCode::Space, movement::JUMP),
            (KeyCode::ControlLeft, movement::SPRINT),
            (KeyCode::ShiftLeft, movement::CROUCH),
            (KeyCode::KeyZ, movement::PRONE),
        ];
        for (key, bit) in cases {
            let (mut camera, mut input, controller) = sampler();
            input.press(key);
            let command = controller.sample(&mut camera, &mut input);
            assert_eq!(command.bits, bit, "{key:?} did not sample to its own bit");
        }
    }

    /// The whole of the pad bug in one test: a tilted stick has to sample
    /// into the same bit the key would, or the pad can look around and open
    /// every panel while the player stands still.
    #[test]
    fn the_pads_stick_walks() {
        let (mut camera, mut input, controller) = sampler();
        input.set_pad_axes(glam::Vec3::new(0.0, 0.0, 1.0));

        let command = controller.sample(&mut camera, &mut input);

        assert_eq!(command.bits, movement::FWD, "the stick did not walk");
    }

    #[test]
    fn a_diagonal_tilt_sets_both_of_its_bits() {
        let (mut camera, mut input, controller) = sampler();
        input.set_pad_axes(glam::Vec3::new(-0.8, 0.0, -0.8));

        let command = controller.sample(&mut camera, &mut input);

        assert_eq!(command.bits, movement::BACK | movement::LEFT);
    }

    #[test]
    fn a_stick_at_rest_leaves_the_keys_exactly_as_they_were() {
        let (mut camera, mut input, controller) = sampler();
        input.press(KeyCode::KeyW);
        input.set_pad_axes(glam::Vec3::ZERO);

        let command = controller.sample(&mut camera, &mut input);

        assert_eq!(command.bits, movement::FWD, "the stick added a bit at rest");
    }

    #[test]
    fn a_key_and_the_stick_together_set_one_bit_not_two() {
        // `bits` is a bitfield, so agreeing twice about forward is still
        // forward — there is no double count to be had, and this is the
        // test that says so.
        let (mut camera, mut input, controller) = sampler();
        input.press(KeyCode::KeyW);
        input.set_pad_axes(glam::Vec3::new(0.0, 0.0, 1.0));

        let command = controller.sample(&mut camera, &mut input);

        assert_eq!(command.bits, movement::FWD);
    }

    /// **A gentle push is a gentle walk.** The stick's magnitude rides the
    /// command as a quantised byte, so the *speed* reaches the simulation
    /// and the simulation replays it.
    #[test]
    fn the_sticks_lean_becomes_the_commands_throttle() {
        let (mut camera, mut input, controller) = sampler();
        input.set_pad_axes(glam::Vec3::new(0.0, 0.0, 1.0));
        let full = controller.sample(&mut camera, &mut input);
        assert_eq!(full.throttle, movement::FULL_THROTTLE, "full tilt is not full pace");

        let (mut camera, mut input, controller) = sampler();
        input.set_pad_axes(glam::Vec3::new(0.0, 0.0, 0.5));
        let half = controller.sample(&mut camera, &mut input);
        assert!(half.held(movement::FWD), "a half tilt should still walk");
        let scale = half.throttle_scale();
        assert!((scale - 0.5).abs() < 0.01, "a half tilt asked for {scale} of top speed");
    }

    /// A key is all-or-nothing, and keyboard play is untouched to the bit.
    #[test]
    fn a_key_always_walks_at_full_pace_whatever_the_stick_is_doing() {
        let (mut camera, mut input, controller) = sampler();
        input.press(KeyCode::KeyW);
        let keyed = controller.sample(&mut camera, &mut input);
        assert_eq!(keyed.throttle, movement::FULL_THROTTLE);

        // A key held while the stick rests half way still walks fully: the
        // key wins the tie, which is the only reading that is not a surprise.
        let (mut camera, mut input, controller) = sampler();
        input.press(KeyCode::KeyW);
        input.set_pad_axes(glam::Vec3::new(0.0, 0.0, 0.5));
        let both = controller.sample(&mut camera, &mut input);
        assert_eq!(both.throttle, movement::FULL_THROTTLE);
    }

    #[test]
    fn a_tilt_under_the_threshold_walks_nowhere() {
        // Dead travel above `gamepad::deadzoned`'s own rescale, so a stick
        // barely off centre does not creep the player across the room.
        let (mut camera, mut input, controller) = sampler();
        input.set_pad_axes(glam::Vec3::new(0.34, 0.0, 0.34));

        let command = controller.sample(&mut camera, &mut input);

        assert_eq!(command.bits, 0, "a resting stick walked");
    }

    #[test]
    fn the_sampler_does_not_move_anything() {
        // It reads input and returns a value. The body is advanced on the tick
        // clock, somewhere else entirely — that separation is the round.
        let (mut camera, mut input, controller) = sampler();
        let parked = camera.position;
        input.press(KeyCode::KeyW);

        for _ in 0..30 {
            controller.sample(&mut camera, &mut input);
        }

        assert_eq!(camera.position, parked, "the sampler moved the camera");
    }

    #[test]
    fn looking_around_still_works_while_sampling() {
        let (mut camera, mut input, controller) = sampler();
        input.mouse_captured = true;
        input.add_mouse_delta(100.0, 0.0);

        let command = controller.sample(&mut camera, &mut input);

        assert!(camera.yaw > 0.0, "the view did not turn");
        assert_ne!(command.yaw_q, 0, "the command did not carry the new heading");
    }

    #[test]
    fn the_command_carries_the_camera_heading_to_within_a_bucket() {
        let (mut camera, mut input, controller) = sampler();
        camera.yaw = 1.234;
        let command = controller.sample(&mut camera, &mut input);
        let bucket = std::f32::consts::TAU / movement::YAW_STEPS as f32;
        assert!(
            (command.yaw() - 1.234).abs() <= bucket,
            "heading {} did not survive quantisation",
            command.yaw()
        );
    }

    #[test]
    fn turning_around_reverses_which_way_forward_goes() {
        let (mut camera, mut input, controller) = setup();
        camera.yaw = std::f32::consts::PI;
        input.press(KeyCode::KeyW);

        controller.apply(&mut camera, &mut input, 1.0);

        assert!(camera.position.z > 0.0, "yaw is not applied to movement");
    }
}
