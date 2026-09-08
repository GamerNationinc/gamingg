//! The player's movement: stance, stamina, ledges, and the weight of what you
//! are carrying.
//!
//! # Why this is a command and not a `dt`
//!
//! Everything else in this world runs on ticks. Mining quantises elapsed time
//! before it touches a block, villagers take the hour as an argument, the
//! economy catches its markets up in fixed steps. The player was the last actor
//! still integrated straight off the frame clock, which meant where you ended
//! up depended on how fast your machine drew — and house rule 2 says agents are
//! bit-identical given the same inputs. The player is an agent.
//!
//! So held keys become a [`MoveCommand`] once per frame and the simulation
//! consumes one per tick at [`MOVE_HZ`]. That is not a new idiom: `PilotCommand`
//! in `vx-agent` already does exactly this for a driven machine, for exactly
//! this reason.
//!
//! # Why 64 Hz and not 60
//!
//! Because the command journal speaks in mining ticks, and mining runs at 8 Hz.
//! Sixty over eight is seven and a half; sixty-four over eight is exactly eight.
//! An integer ratio means `Advance { ticks }` keeps the meaning it already has
//! and replay simply runs eight movement sub-ticks per journal tick. A
//! fractional one would have forced a clock change through the whole log format
//! to buy nothing.
//!
//! # What lives here and what does not
//!
//! The AABB sweep is `vx_world::step_aabb` and knows nothing about any of this.
//! Stance, stamina, ledges and carried mass are fiction, so they live here —
//! the same line that keeps `vx-agent` free of quests and economy.

use glam::{DVec3, Vec2, Vec3};
use vx_world::{
    collides, step_aabb, Aabb, MoveParams, PlayerBody, World, GRAVITY, JUMP_SPEED,
    TERMINAL_VELOCITY,
};

/// Movement sub-ticks per second.
pub const MOVE_HZ: u32 = 64;

/// One movement tick, in seconds.
pub const MOVE_TICK: f32 = 1.0 / MOVE_HZ as f32;

/// Movement sub-ticks inside one journal (mining) tick. Exact by construction —
/// see the module note.
pub const SUBTICKS: u32 = 8;

// ---------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------

pub const FWD: u16 = 1 << 0;
pub const BACK: u16 = 1 << 1;
pub const LEFT: u16 = 1 << 2;
pub const RIGHT: u16 = 1 << 3;
pub const JUMP: u16 = 1 << 4;
pub const SPRINT: u16 = 1 << 5;
pub const CROUCH: u16 = 1 << 6;
pub const PRONE: u16 = 1 << 7;

/// Buckets a full turn is divided into. About five arc-minutes, which is finer
/// than a mouse can be aimed and coarse enough to round away the last bits of a
/// float that would never survive a round trip through a save file.
pub const YAW_STEPS: i32 = 4096;

/// Buckets from straight down to straight up.
pub const PITCH_STEPS: i32 = 2048;

/// What the player is asking for on this tick.
///
/// Angles are quantised on the way in rather than carried as floats. A recorded
/// input sequence has to replay to the same position or the regression test is
/// worthless, and a raw mouse delta is exactly the kind of value that will not
/// survive being written to a file and read back.
/// Full tilt, in 255ths. A keyboard only ever writes this.
pub const FULL_THROTTLE: u8 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveCommand {
    pub bits: u16,
    pub yaw_q: i16,
    pub pitch_q: i16,
    /// How far the stick is pushed, in 255ths of full tilt.
    ///
    /// Quantised for the same reason the angles are: it crosses the journal's
    /// wire, and the simulation replays it. A key is all-or-nothing and
    /// writes [`FULL_THROTTLE`], so keyboard play is unchanged to the bit —
    /// the byte only ever means something when a stick sent it.
    pub throttle: u8,
    /// How full the pack is, in 255ths — see [`load_byte`].
    ///
    /// Carried in the command rather than looked up during replay because the
    /// pack is not in the journal, and a replay that assumed an empty one would
    /// reproduce a lighter, faster player than the one who was actually there.
    pub load: u8,
}

/// A command asking for nothing, at full tilt.
///
/// Written by hand rather than derived so `throttle` starts at
/// [`FULL_THROTTLE`]: a derived zero would mean "the stick is centred", and
/// every caller that builds a command from `..Default::default()` would ask
/// the body to stand still.
impl Default for MoveCommand {
    fn default() -> Self {
        MoveCommand {
            bits: 0,
            yaw_q: 0,
            pitch_q: 0,
            throttle: FULL_THROTTLE,
            load: 0,
        }
    }
}

impl MoveCommand {
    pub fn held(&self, bit: u16) -> bool {
        self.bits & bit != 0
    }

    /// Quantise a look angle into a command.
    pub fn looking(bits: u16, yaw: f32, pitch: f32) -> Self {
        let turn = std::f32::consts::TAU;
        let yaw_q = (yaw.rem_euclid(turn) / turn * YAW_STEPS as f32).round() as i32;
        let half = std::f32::consts::FRAC_PI_2;
        let pitch_q =
            (pitch.clamp(-half, half) / half * (PITCH_STEPS / 2) as f32).round() as i32;
        MoveCommand {
            bits,
            yaw_q: yaw_q.rem_euclid(YAW_STEPS) as i16,
            pitch_q: pitch_q as i16,
            throttle: FULL_THROTTLE,
            load: 0,
        }
    }

    /// The same command at a fraction of full tilt.
    pub fn throttled(mut self, throttle: u8) -> Self {
        self.throttle = throttle;
        self
    }

    /// How much of top speed this command asks for, `0..1`.
    ///
    /// A separate scalar from [`MoveCommand::wish_dir`] rather than a shorter
    /// wish vector, because everything else that reads the wish — the ledge
    /// probe, the landing, the slide launch — wants the *direction* and would
    /// quietly change behaviour if a gentle push shortened it.
    pub fn throttle_scale(&self) -> f32 {
        f32::from(self.throttle) / f32::from(FULL_THROTTLE)
    }

    /// The same command, carrying a load.
    pub fn laden(mut self, load: u8) -> Self {
        self.load = load;
        self
    }

    /// The speed multiplier this command's load implies.
    pub fn mass(&self) -> f32 {
        mass_from_byte(self.load)
    }

    pub fn yaw(&self) -> f32 {
        self.yaw_q as f32 / YAW_STEPS as f32 * std::f32::consts::TAU
    }


    /// Level movement direction in world space, unit length or zero.
    pub fn wish_dir(&self) -> Vec3 {
        let forward = f32::from(self.held(FWD)) - f32::from(self.held(BACK));
        let strafe = f32::from(self.held(RIGHT)) - f32::from(self.held(LEFT));
        if forward == 0.0 && strafe == 0.0 {
            return Vec3::ZERO;
        }

        // The camera's own basis, rebuilt from the quantised angle through a
        // table rather than called fresh every tick.
        let facing = yaw_vector(self.yaw_q);
        let ahead = Vec3::new(facing.x, 0.0, facing.y);
        let right = Vec3::new(-facing.y, 0.0, facing.x);

        let raw = ahead * forward + right * strafe;
        // Diagonals must not outrun straight lines.
        if raw.length_squared() > 1.0 {
            raw.normalize()
        } else {
            raw
        }
    }
}

/// Direction table, indexed by quantised yaw.
///
/// This does not make `sin`/`cos` bit-identical across platforms — nothing
/// does. What it buys is that the transcendentals are evaluated once, from a
/// fixed input, instead of every tick on a value the whole simulation keys off.
/// A last-bit difference in a table entry is then a constant offset rather than
/// something that compounds over a thousand ticks.
fn yaw_vector(yaw_q: i16) -> Vec2 {
    static TABLE: std::sync::OnceLock<Vec<Vec2>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..YAW_STEPS)
            .map(|i| {
                let angle = i as f32 / YAW_STEPS as f32 * std::f32::consts::TAU;
                // Matches `Camera::forward_level`: yaw 0 looks down -Z.
                Vec2::new(angle.sin(), -angle.cos())
            })
            .collect()
    });
    table[(yaw_q as i32).rem_euclid(YAW_STEPS) as usize]
}

/// The full look direction a quantised aim describes, matching
/// `Camera::forward`: yaw 0 looks down -Z, positive pitch looks up.
///
/// Lives here rather than in the arsenal because it is the inverse of
/// [`MoveCommand::looking`]'s quantisation, and live fire and journal replay
/// must dequantise identically or a replayed shot flies a different line.
pub fn aim_vector(yaw_q: i16, pitch_q: i16) -> Vec3 {
    let yaw = yaw_q as f32 / YAW_STEPS as f32 * std::f32::consts::TAU;
    let pitch = pitch_q as f32 / (PITCH_STEPS / 2) as f32 * std::f32::consts::FRAC_PI_2;
    let (level, rise) = (pitch.cos(), pitch.sin());
    Vec3::new(level * yaw.sin(), rise, -(level * yaw.cos()))
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const WALK: f32 = 4.3;
pub const SPRINT_SPEED: f32 = 6.5;
pub const CROUCH_SPEED: f32 = 1.9;
pub const PRONE_SPEED: f32 = 0.7;

/// Ground acceleration.
///
/// Raised from the design note's 60 because of how friction and acceleration
/// meet in `step_with`: drag is exponential and acceleration is capped, so the
/// fastest a body can hold on the ground is `accel / friction`. At 60 that
/// ceiling is 6.0 m/s and [`SPRINT_SPEED`] at 6.5 would have been a number you
/// could never actually reach. Any change to [`FRICTION`] has to keep
/// `ACCEL_GROUND > SPRINT_SPEED * FRICTION`.
pub const ACCEL_GROUND: f32 = 100.0;
pub const ACCEL_AIR: f32 = 8.0;
pub const ACCEL_SLIDE: f32 = 2.0;

pub const FRICTION: f32 = 10.0;
pub const FRICTION_SLIDE: f32 = 1.6;
pub const FRICTION_AIR: f32 = 0.0;

pub const SLIDE_ENTRY: f32 = 5.0;
pub const SLIDE_BOOST: f32 = 1.4;
pub const SLIDE_CAP: f32 = 9.0;
pub const SLIDE_EXIT: f32 = 3.0;

/// How much of a drop's speed a slide converts into forward momentum on
/// landing.
///
/// This is the voxel world's answer to "project gravity onto the surface
/// normal". There are no ramps here — a decline the mine planner cut is a
/// staircase, and every surface normal is exactly `+Y`, so projecting onto it
/// yields nothing. What actually differs downhill is that you spend the run
/// falling off one-block steps. Converting a slice of that fall into forward
/// speed makes a slide down a decline carry and a slide up a bench die, which
/// is the behaviour the projection was there to produce.
pub const SLIDE_LANDING_TRANSFER: f32 = 0.35;

pub const MANTLE_MAX: f32 = 2.2;
pub const VAULT_MAX: f32 = 1.3;
/// The design note asks for a vault of about six ticks and a mantle of about
/// twelve. Those were twenty-hertz ticks — three tenths of a second and six
/// tenths. At [`MOVE_HZ`] the same *durations* are these numbers. Taking the
/// tick counts literally would have made vaulting a one-block bench faster than
/// sprinting across it, which turns a climb into a shortcut.
pub const VAULT_TICKS: u8 = 19;
pub const MANTLE_TICKS: u8 = 38;

pub const COYOTE: u8 = 2;
pub const JUMP_BUFFER: u8 = 3;

pub const STAM_MAX: f32 = 100.0;
pub const STAM_SPRINT: f32 = 8.0;
pub const STAM_SLIDE: f32 = 12.0;
pub const STAM_MANTLE: f32 = 15.0;
pub const STAM_REGEN: f32 = 20.0;
pub const STAM_REGEN_DELAY: f32 = 1.5;

/// Speed multiplier when out of breath. Slowed, never stopped — a hard stop in
/// a game about hauling ore across open ground punishes playing it as designed.
pub const WINDED: f32 = 0.6;

pub const STAND_HEIGHT: f32 = 1.8;
pub const CROUCH_HEIGHT: f32 = 1.25;
pub const PRONE_HEIGHT: f32 = 0.7;

// ---------------------------------------------------------------------------
// Stance
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stance {
    Grounded,
    Sprinting,
    Crouched,
    Prone,
    Swimming,
    Sliding { ticks: u16 },
    Airborne { coyote: u8 },
    Mantling { from: DVec3, to: DVec3, t: u8, span: u8 },
}

impl Stance {
    /// Eye height above the feet, in centimetres.
    ///
    /// An integer on purpose: this is the value the camera rides, and a whole
    /// number interpolates to the same place every run.
    pub fn eye_cm(&self) -> u16 {
        match self {
            Stance::Grounded | Stance::Sprinting | Stance::Airborne { .. } => 162,
            Stance::Mantling { .. } => 140,
            Stance::Crouched => 110,
            Stance::Sliding { .. } => 70,
            Stance::Swimming => 90,
            Stance::Prone => 35,
        }
    }

    /// A short label for the HUD.
    pub fn label(&self) -> &'static str {
        match self {
            Stance::Grounded => "STAND",
            Stance::Sprinting => "SPRINT",
            Stance::Crouched => "CROUCH",
            Stance::Prone => "PRONE",
            Stance::Swimming => "SWIM",
            Stance::Sliding { .. } => "SLIDE",
            Stance::Airborne { .. } => "AIR",
            Stance::Mantling { .. } => "CLIMB",
        }
    }

    /// How tall the collision hull stands.
    pub fn body_height(&self) -> f32 {
        match self {
            Stance::Crouched | Stance::Sliding { .. } => CROUCH_HEIGHT,
            Stance::Prone => PRONE_HEIGHT,
            _ => STAND_HEIGHT,
        }
    }

    /// Top speed on the flat, before mass and stamina take their cut.
    ///
    /// Reads the live tuning, not the constants: the constants are only the
    /// defaults, and a journal that set `sprint_speed` must move by it.
    pub fn speed(&self, tuning: &crate::tuning::Tuning) -> f32 {
        match self {
            Stance::Sprinting => tuning.sprint_speed,
            Stance::Crouched => tuning.crouch_speed,
            Stance::Prone => tuning.prone_speed,
            Stance::Sliding { .. } => 0.0,
            Stance::Swimming => tuning.crouch_speed,
            _ => tuning.walk,
        }
    }

    pub fn is_grounded(&self) -> bool {
        matches!(
            self,
            Stance::Grounded | Stance::Sprinting | Stance::Crouched | Stance::Prone
        ) || matches!(self, Stance::Sliding { .. })
    }
}

/// How full the pack is, in 255ths.
///
/// Quantised for the same reason the look angles are: this number rides in the
/// journal, and a replay that reconstructed it from a float would drift away
/// from the run it is supposed to reproduce. Going through a byte makes the
/// live game and the replay agree by construction rather than by luck.
pub fn load_byte(carried: u64, capacity: u64) -> u8 {
    let fraction = (carried as f32 / capacity.max(1) as f32).clamp(0.0, 1.0);
    (fraction * 255.0).round() as u8
}

/// How much of your speed a load costs you.
///
/// 1.0 empty, floors at 0.55 fully laden. `Logistics` raises capacity, which
/// makes the same rock lighter — so the skill is a movement upgrade without a
/// single new item, and every cargo upgrade the shop sells is also a tax on
/// using it. That tension is the point; a straight buff would not be worth a
/// number.
pub fn mass_from_byte(load: u8) -> f32 {
    (1.0 - 0.45 * (load as f32 / 255.0)).max(0.55)
}

// ---------------------------------------------------------------------------
// Ledges
// ---------------------------------------------------------------------------

/// What is directly ahead, by how far up its top surface sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ledge {
    /// Nothing worth climbing, or nothing climbable.
    None,
    /// Low enough that the sweep walks it without being asked.
    Step,
    /// Waist high. Taken automatically on contact.
    Vault { top: f64, across: DVec3 },
    /// Chest high. Costs stamina and has to be wanted.
    Mantle { top: f64, across: DVec3 },
}

/// Classify what the body is pressed against, using two casts through the
/// existing DDA.
///
/// One forward at chest height to find the wall, one down from above the
/// contact to find its top. A destination that is not clear to full standing
/// height classifies as nothing, because pulling yourself into a ceiling is
/// worse than being stopped by a wall.
pub fn classify_ledge(world: &World, body: &PlayerBody, direction: Vec3) -> Ledge {
    let level = Vec3::new(direction.x, 0.0, direction.z);
    if level.length_squared() < 1.0e-6 {
        return Ledge::None;
    }
    let level = level.normalize();

    let reach = (body.width * 0.5 + 0.35) as f32;
    let chest = body.position + DVec3::Y * 0.9;
    let wall = vx_world::raycast_solid(world, world.registry(), chest, level, reach);
    let Some(wall) = wall else {
        return Ledge::None;
    };

    // Stand over the block that was struck and look straight down for its top.
    let over = DVec3::new(
        wall.block.x as f64 + 0.5,
        body.position.y + MANTLE_MAX as f64 + 0.5,
        wall.block.z as f64 + 0.5,
    );
    let down = vx_world::raycast_solid(world, world.registry(), over, -Vec3::Y, MANTLE_MAX + 1.5);
    let Some(top_hit) = down else {
        return Ledge::None;
    };
    let top = top_hit.block.y as f64 + 1.0;
    let rise = top - body.position.y;

    if rise <= 0.0 {
        return Ledge::None;
    }
    if rise < vx_world::STEP_HEIGHT {
        return Ledge::Step;
    }
    if rise > MANTLE_MAX as f64 {
        return Ledge::None;
    }

    // Where you would end up: one step past the wall face, standing on its top.
    let across = DVec3::new(
        wall.block.x as f64 + 0.5,
        top,
        wall.block.z as f64 + 0.5,
    ) + level.as_dvec3() * 0.15;

    let landing = Aabb::standing_on(across, body.width, STAND_HEIGHT as f64);
    if collides(world, &landing) {
        return Ledge::None;
    }

    if rise <= VAULT_MAX as f64 {
        Ledge::Vault { top, across }
    } else {
        Ledge::Mantle { top, across }
    }
}

/// Is the body's waist in water?
///
/// Water is registered non-solid, so the collision sweep swims straight through
/// it — this is the only thing that notices. Swimming is barely a stance yet:
/// it slows you and stops you sprinting. There is nothing to do underwater and
/// no aquatic machine to do it with, so it stays a stub on purpose rather than
/// a variant nothing can ever reach.
pub fn submerged(world: &World, body: &PlayerBody) -> bool {
    let Some(water) = world.registry().id_of("engine:water") else {
        return false;
    };
    let waist = body.position + DVec3::Y * (body.height * 0.5);
    world.block(vx_core::BlockPos::new(
        waist.x.floor() as i32,
        waist.y.floor() as i32,
        waist.z.floor() as i32,
    )) == water
}

// ---------------------------------------------------------------------------
// The state machine
// ---------------------------------------------------------------------------

/// Everything about how the player is moving that is not the body itself.
#[derive(Debug, Clone, Copy)]
pub struct Movement {
    /// The numbers this body moves by. Defaults to the shipped constants;
    /// the gold panel's `SetTuning` orders are what change it, and because it
    /// rides here it is part of what a journal replay reconstructs.
    pub tuning: crate::tuning::Tuning,
    pub stance: Stance,
    pub stamina: f32,
    /// Seconds since stamina was last spent.
    rested: f32,
    /// Ticks a jump press is still honoured for after it was made.
    buffered_jump: u8,
    /// Set while the previous tick was airborne, so a landing can be detected.
    was_airborne: bool,
    /// Speed a freshly entered slide should be kicked to, applied once.
    ///
    /// Held rather than written straight into the body because the stance is
    /// chosen before the tick's integration, and an impulse applied before
    /// friction runs is an impulse partly thrown away.
    entry_impulse: Option<f32>,
    /// A shove from outside — the launcher's recoil. Same one-shot shape as
    /// `entry_impulse` and consumed at the same slot in the tick, for the same
    /// reason: applied before friction runs, it is partly thrown away.
    recoil: Option<Vec3>,
}

impl Default for Movement {
    fn default() -> Self {
        Movement {
            stance: Stance::Airborne { coyote: 0 },
            stamina: STAM_MAX,
            rested: 0.0,
            buffered_jump: 0,
            was_airborne: true,
            entry_impulse: None,
            recoil: None,
            tuning: crate::tuning::Tuning::default(),
        }
    }
}

impl Movement {
    /// Fraction of the stamina bar remaining, for the HUD.
    pub fn stamina_fraction(&self) -> f32 {
        (self.stamina / self.tuning.stam_max).clamp(0.0, 1.0)
    }

    pub fn winded(&self) -> bool {
        self.stamina < self.tuning.stam_slide
    }

    fn spend(&mut self, amount: f32) {
        self.stamina = (self.stamina - amount).max(0.0);
        self.rested = 0.0;
    }

    /// Queue a shove for the next tick's integration. Firing calls this on
    /// both sides of the oracle — live and replay — so the stagger replays.
    pub fn kick(&mut self, shove: Vec3) {
        self.recoil = Some(self.recoil.map_or(shove, |held| held + shove));
    }

    /// Advance one movement tick.
    ///
    /// `mass` is the multiplier from [`mass_multiplier`]; this function never
    /// learns what a stockpile is.
    pub fn advance(
        &mut self,
        body: &mut PlayerBody,
        world: &World,
        command: MoveCommand,
        mass: f32,
        dt: f32,
    ) {
        if self.advance_mantle(body, world, dt) {
            return;
        }

        self.buffered_jump = self.buffered_jump.saturating_sub(1);
        if command.held(JUMP) {
            self.buffered_jump = JUMP_BUFFER;
        }

        let wish = command.wish_dir();
        let grounded = body.on_ground;

        if submerged(world, body) {
            self.stance = Stance::Swimming;
        } else {
            self.retake_stance(body, world, command, grounded);
        }
        self.try_ledge(body, world, wish, command);
        if matches!(self.stance, Stance::Mantling { .. }) {
            return;
        }

        // Landing is what carries a slide down a staircase. Read it before the
        // sweep overwrites the vertical velocity.
        if grounded && self.was_airborne {
            self.on_landing(body, wish);
        }
        self.was_airborne = !grounded;

        self.jump(body, grounded);
        self.launch_slide(body, wish);
        if let Some(shove) = self.recoil.take() {
            body.velocity += shove.as_dvec3();
        }
        self.drain(mass, dt);

        // After the jump, not before: a tick that leaves the ground must be
        // integrated with air rules, or ground friction eats the impulse on the
        // way out and a slide-jump lands at walking pace.
        let params = self.params(body.on_ground);
        let speed = self.top_speed(mass);
        body.height = self.stance.body_height() as f64;
        body.eye_height = self.stance.eye_cm() as f64 / 100.0;
        body.step_with(
            world,
            (wish * speed * command.throttle_scale()).as_dvec3(),
            dt as f64,
            params,
        );

        if body.on_ground {
            self.settle_stance(world, body, command);
        } else if self.stance.is_grounded() && !matches!(self.stance, Stance::Sliding { .. }) {
            self.stance = Stance::Airborne { coyote: COYOTE };
        }
        body.height = self.stance.body_height() as f64;
        body.eye_height = self.stance.eye_cm() as f64 / 100.0;
    }

    /// Adopt the stance the keys ask for, but never one the ceiling forbids.
    ///
    /// The hull grows when you stand up. Doing that without checking is how a
    /// body ends up inside rock, and the check has to happen *after* the sweep
    /// as well as before it — the sweep is what moved you under the overhang.
    fn settle_stance(&mut self, world: &World, body: &PlayerBody, command: MoveCommand) {
        let wanted = self.grounded_stance(command, body);
        self.stance = if self.headroom(world, body, wanted.body_height()) {
            wanted
        } else if self.headroom(world, body, CROUCH_HEIGHT) {
            Stance::Crouched
        } else {
            Stance::Prone
        };
    }

    /// A mantle is the one state that is not ordinary physics: the integrator is
    /// suspended and the box is walked along a fixed arc. Keeping it the only
    /// exception is what stops the rest from becoming special cases.
    fn advance_mantle(&mut self, body: &mut PlayerBody, world: &World, dt: f32) -> bool {
        let Stance::Mantling { from, to, t, span } = self.stance else {
            return false;
        };
        let _ = dt;
        let next = t + 1;
        let progress = next as f64 / span as f64;
        // Up first, then across: the shape of pulling yourself over an edge,
        // and now literally that rather than nearly it. The two phases used
        // to overlap between 0.4 and 0.6 — rising while already moving in —
        // and that overlap is exactly the window where the hull cut the
        // corner of the block being climbed. Separating them costs nothing
        // (the same span, the same endpoints) and is what makes the arc
        // survive the collision check below.
        let lift = (progress / 0.5).min(1.0);
        let reach = ((progress - 0.5).max(0.0)) / 0.5;
        let step = DVec3::new(
            from.x + (to.x - from.x) * reach,
            from.y + (to.y - from.y) * lift,
            from.z + (to.z - from.z) * reach,
        );

        // The arc is validated at every step, not only at its destination.
        //
        // This is the one state that is not ordinary physics — the integrator
        // is suspended and the box is walked along a fixed curve — and until
        // now that meant *no collision query at all*. `classify_ledge` checks
        // where a mantle lands and nothing checks where it goes, so between
        // the lift and the reach the hull passed straight through the block
        // it was climbing: about six tenths of a second with the eye inside
        // solid rock. The exception was meant to be the integrator, not the
        // geometry.
        if collides(world, &Aabb::standing_on(step, body.width, body.height)) {
            // Blocked mid-arc. Stop where the last clear step left the body
            // rather than pressing on through: it reads as a hand slipping,
            // and it is the same thing the sweep would have done.
            self.stance = Stance::Airborne { coyote: 0 };
            body.velocity = DVec3::ZERO;
            return true;
        }
        body.position = step;
        body.velocity = DVec3::ZERO;

        if next >= span {
            body.position = to;
            body.on_ground = true;
            self.stance = Stance::Grounded;
            self.was_airborne = false;
        } else {
            self.stance = Stance::Mantling {
                from,
                to,
                t: next,
                span,
            };
        }
        true
    }

    /// Pick the stance the held keys are asking for, honouring headroom.
    fn retake_stance(
        &mut self,
        body: &PlayerBody,
        world: &World,
        command: MoveCommand,
        grounded: bool,
    ) {
        let speed = DVec3::new(body.velocity.x, 0.0, body.velocity.z).length() as f32;

        // Sprint into crouch at pace is a slide, and only from a standing start
        // of real speed — jogging into it would make the verb free.
        if grounded
            && command.held(CROUCH)
            && speed >= self.tuning.slide_entry
            && self.stamina >= self.tuning.stam_slide
            && !matches!(self.stance, Stance::Sliding { .. })
        {
            self.spend(self.tuning.stam_slide);
            self.stance = Stance::Sliding { ticks: 0 };
            self.entry_impulse = Some((speed * self.tuning.slide_boost).min(self.tuning.slide_cap));
            return;
        }

        if let Stance::Sliding { ticks } = self.stance {
            // Leaving the ground does not end a slide. Going downhill *is*
            // leaving the ground — a decline the mine planner cut is a
            // staircase, so a slide down one spends most of its life in the air
            // between steps. Ending it on the first drop would make the one
            // case the verb exists for the one case it cannot do.
            let done = grounded && (speed < self.tuning.slide_exit || !command.held(CROUCH));
            self.stance = if done {
                if command.held(CROUCH) {
                    Stance::Crouched
                } else {
                    Stance::Grounded
                }
            } else {
                Stance::Sliding {
                    ticks: ticks.saturating_add(1),
                }
            };
            return;
        }

        if !grounded {
            return;
        }
        self.settle_stance(world, body, command);
    }

    fn grounded_stance(&self, command: MoveCommand, body: &PlayerBody) -> Stance {
        if let Stance::Sliding { ticks } = self.stance {
            let speed = DVec3::new(body.velocity.x, 0.0, body.velocity.z).length() as f32;
            if speed >= self.tuning.slide_exit {
                return Stance::Sliding { ticks };
            }
        }
        if command.held(PRONE) {
            Stance::Prone
        } else if command.held(CROUCH) {
            Stance::Crouched
        } else if command.held(SPRINT) && command.wish_dir() != Vec3::ZERO && !self.winded() {
            Stance::Sprinting
        } else {
            Stance::Grounded
        }
    }

    fn headroom(&self, world: &World, body: &PlayerBody, height: f32) -> bool {
        // The skin, like everywhere else the physics touches a plane. Without
        // it a body could stand up with its head *exactly* on the ceiling
        // plane — not colliding, thanks to the inset in the block query, but
        // with none of the millimetre the sweep leaves against every other
        // surface. That is the one place in the system where the margin was
        // zero rather than `SKIN`.
        !collides(
            world,
            &Aabb::standing_on(body.position, body.width, height as f64 + vx_world::SKIN),
        )
    }

    /// Vault on contact, mantle when it is wanted and affordable.
    fn try_ledge(
        &mut self,
        body: &mut PlayerBody,
        world: &World,
        wish: Vec3,
        command: MoveCommand,
    ) {
        if wish == Vec3::ZERO || matches!(self.stance, Stance::Mantling { .. }) {
            return;
        }
        // Only when actually stopped by something — otherwise every wall you
        // walk past would grab you.
        let pressing = DVec3::new(body.velocity.x, 0.0, body.velocity.z).length()
            < self.tuning.walk as f64 * 0.5;
        if !pressing {
            return;
        }

        match classify_ledge(world, body, wish) {
            Ledge::Vault { across, .. } => {
                self.begin_climb(body, across, VAULT_TICKS);
            }
            Ledge::Mantle { across, .. } => {
                if command.held(JUMP) || self.buffered_jump > 0 {
                    if self.stamina < self.tuning.stam_mantle {
                        return;
                    }
                    self.spend(self.tuning.stam_mantle);
                    self.begin_climb(body, across, MANTLE_TICKS);
                }
            }
            Ledge::Step | Ledge::None => {}
        }
    }

    fn begin_climb(&mut self, body: &mut PlayerBody, to: DVec3, span: u8) {
        self.stance = Stance::Mantling {
            from: body.position,
            to,
            t: 0,
            span,
        };
        body.velocity = DVec3::ZERO;
        self.buffered_jump = 0;
    }

    /// Kick a freshly entered slide up to its boosted speed.
    ///
    /// Once, on entry. A slide that only ever decayed from walking pace would
    /// never be worth pressing; the boost is what makes it a decision rather
    /// than a slower crouch.
    fn launch_slide(&mut self, body: &mut PlayerBody, wish: Vec3) {
        let Some(target) = self.entry_impulse.take() else {
            return;
        };
        let along = DVec3::new(body.velocity.x, 0.0, body.velocity.z);
        let direction = if along.length_squared() > 1.0e-6 {
            along.normalize()
        } else if wish != Vec3::ZERO {
            wish.normalize().as_dvec3()
        } else {
            return;
        };
        let kicked = direction * target as f64;
        body.velocity.x = kicked.x;
        body.velocity.z = kicked.z;
    }

    /// Turn a slice of a drop into forward speed, so a slide carries downhill.
    fn on_landing(&mut self, body: &mut PlayerBody, wish: Vec3) {
        if !matches!(self.stance, Stance::Sliding { .. }) {
            return;
        }
        let drop = (-body.velocity.y).max(0.0).min(TERMINAL_VELOCITY.abs());
        if drop <= 0.0 {
            return;
        }
        let along = DVec3::new(body.velocity.x, 0.0, body.velocity.z);
        let direction = if along.length_squared() > 1.0e-6 {
            along.normalize()
        } else if wish != Vec3::ZERO {
            wish.normalize().as_dvec3()
        } else {
            return;
        };
        let gained = along.length() + drop * self.tuning.slide_landing_transfer as f64;
        let capped = direction * gained.min(self.tuning.slide_cap as f64);
        body.velocity.x = capped.x;
        body.velocity.z = capped.z;
    }

    fn jump(&mut self, body: &mut PlayerBody, grounded: bool) {
        if self.buffered_jump == 0 {
            return;
        }
        let coyote = matches!(self.stance, Stance::Airborne { coyote } if coyote > 0);
        if !grounded && !coyote {
            return;
        }
        // Slide-jumping keeps the horizontal velocity it built. That is the
        // whole reason the verb is worth having; take it away and a slide is a
        // strictly worse crouch.
        let carried = DVec3::new(body.velocity.x, 0.0, body.velocity.z);
        body.velocity.y = JUMP_SPEED;
        body.velocity.x = carried.x;
        body.velocity.z = carried.z;
        body.on_ground = false;
        self.buffered_jump = 0;
        self.stance = Stance::Airborne { coyote: 0 };
        self.was_airborne = true;
    }

    fn drain(&mut self, mass: f32, dt: f32) {
        match self.stance {
            Stance::Sprinting => {
                // Heavier means winded sooner. This is where the weight system
                // gets its teeth: the cargo upgrade you bought is also the
                // reason you are out of breath.
                self.spend(self.tuning.stam_sprint * dt / mass.max(0.01));
            }
            _ => {
                self.rested += dt;
                if self.rested >= self.tuning.stam_regen_delay {
                    self.stamina = (self.stamina + self.tuning.stam_regen * dt).min(self.tuning.stam_max);
                }
            }
        }
    }

    fn top_speed(&self, mass: f32) -> f32 {
        let base = self.stance.speed(&self.tuning) * mass;
        if self.winded() {
            base * self.tuning.winded
        } else {
            base
        }
    }

    fn params(&self, grounded: bool) -> MoveParams {
        let step_height = match self.stance {
            Stance::Prone => 0.0,
            _ => vx_world::STEP_HEIGHT,
        };
        match self.stance {
            Stance::Sliding { .. } => MoveParams {
                accel: self.tuning.accel_slide as f64,
                friction: if grounded {
                    self.tuning.friction_slide as f64
                } else {
                    self.tuning.friction_air as f64
                },
                step_height,
                gravity: true,
            },
            _ if !grounded => MoveParams {
                accel: self.tuning.accel_air as f64,
                friction: self.tuning.friction_air as f64,
                step_height,
                gravity: true,
            },
            _ => MoveParams {
                accel: self.tuning.accel_ground as f64,
                friction: self.tuning.friction as f64,
                step_height,
                gravity: true,
            },
        }
    }
}

/// Run one journal tick's worth of movement: [`SUBTICKS`] sub-ticks of the same
/// held command.
///
/// This is the entry point replay uses, which is what makes the regression test
/// the existing determinism oracle rather than a parallel one.
pub fn advance_journal_tick(
    movement: &mut Movement,
    body: &mut PlayerBody,
    world: &World,
    command: MoveCommand,
) {
    for _ in 0..SUBTICKS {
        movement.advance(body, world, command, command.mass(), MOVE_TICK);
    }
}

/// Keep the unused-import checker honest about what this module leans on.
const _: fn(&World, Aabb, DVec3, f64, f64) -> vx_world::StepResult = step_aabb;
const _: f64 = GRAVITY;

/// Turns elapsed wall-clock time into whole movement ticks.
///
/// The remainder is carried rather than dropped, so a run of short frames adds
/// up to the same number of ticks a run of long ones does. A very long stall is
/// clamped rather than replayed all at once: catching up ten seconds of
/// movement in one frame would fire the player through a wall, and the honest
/// answer to a stall is that the time was lost.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ticker {
    carry: f32,
}

/// Longest stall that is caught up rather than discarded.
pub const MAX_CATCHUP: f32 = 0.25;

impl Ticker {
    pub fn take(&mut self, elapsed: f32) -> u32 {
        self.carry = (self.carry + elapsed.max(0.0)).min(MAX_CATCHUP);
        let ticks = (self.carry / MOVE_TICK) as u32;
        self.carry -= ticks as f32 * MOVE_TICK;
        ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockId, BlockPos, ChunkPos};

    /// Solid floor at y=40, nothing above it.
    fn flat_world() -> World {
        let mut world = World::new(7);
        world.load_around(ChunkPos::new(0, 0), 1);
        let stone = world.registry().id_of("engine:stone").unwrap();
        for x in -16..32 {
            for z in -16..32 {
                for y in 0..80 {
                    let fill = if y == 40 { stone } else { BlockId::AIR };
                    world.set_block(BlockPos::new(x, y, z), fill);
                }
            }
        }
        world
    }

    fn stone_of(world: &World) -> BlockId {
        world.registry().id_of("engine:stone").unwrap()
    }

    /// The speed multiplier for a pack holding `carried` of `capacity`, going
    /// through the same byte the journal carries — so a test and a replay agree
    /// by construction.
    fn mass_of(carried: u64, capacity: u64) -> f32 {
        mass_from_byte(load_byte(carried, capacity))
    }

    fn standing(x: f64, z: f64) -> PlayerBody {
        PlayerBody {
            position: DVec3::new(x, 41.0, z),
            ..PlayerBody::default()
        }
    }

    /// Yaw that points along +X, matching `yaw_vector`'s convention.
    fn facing_plus_x() -> f32 {
        std::f32::consts::FRAC_PI_2
    }

    fn run(
        movement: &mut Movement,
        body: &mut PlayerBody,
        world: &World,
        command: MoveCommand,
        ticks: usize,
    ) {
        for _ in 0..ticks {
            movement.advance(body, world, command, 1.0, MOVE_TICK);
        }
    }

    #[test]
    fn a_command_round_trips_through_quantisation() {
        // The replay oracle is only worth having if the angle that went in is
        // the angle that comes back out.
        for step in 0..YAW_STEPS {
            let angle = step as f32 / YAW_STEPS as f32 * std::f32::consts::TAU;
            let command = MoveCommand::looking(FWD, angle, 0.0);
            let back = MoveCommand::looking(FWD, command.yaw(), 0.0);
            assert_eq!(command.yaw_q, back.yaw_q, "yaw bucket {step} drifted");
        }
    }

    #[test]
    fn walking_direction_follows_the_quantised_yaw() {
        let forward = MoveCommand::looking(FWD, facing_plus_x(), 0.0).wish_dir();
        assert!(forward.x > 0.99, "forward was {forward:?}");
        assert!(forward.y.abs() < 1e-6, "walking gained altitude");

        // Default yaw looks down -Z, same as the camera.
        let north = MoveCommand::looking(FWD, 0.0, 0.0).wish_dir();
        assert!(north.z < -0.99, "default facing was {north:?}");
    }

    #[test]
    fn diagonals_do_not_outrun_straight_lines() {
        let straight = MoveCommand::looking(FWD, 0.0, 0.0).wish_dir().length();
        let diagonal = MoveCommand::looking(FWD | RIGHT, 0.0, 0.0).wish_dir().length();
        assert!((straight - diagonal).abs() < 1e-5, "{straight} vs {diagonal}");
    }

    #[test]
    fn movement_is_frame_rate_independent() {
        // The point of the whole round. The same ticks, batched differently,
        // must land in exactly the same place — bit for bit, not nearly.
        let world = flat_world();
        let command = MoveCommand::looking(FWD | SPRINT, facing_plus_x(), 0.0);

        let play = |batches: &[usize]| {
            let mut body = standing(4.5, 4.5);
            let mut movement = Movement::default();
            for &count in batches {
                run(&mut movement, &mut body, &world, command, count);
            }
            body.position
        };

        let all_at_once = play(&[192]);
        let sixty = play(&[1; 192]);
        let thirty = play(&[2; 96]);
        let ragged = play(&[7, 1, 13, 40, 2, 129]);

        assert_eq!(all_at_once, sixty, "one batch differed from many");
        assert_eq!(all_at_once, thirty, "half rate differed");
        assert_eq!(all_at_once, ragged, "an uneven frame pattern differed");
        assert!(all_at_once.x > 6.0, "never actually moved: {all_at_once:?}");
    }

    #[test]
    fn the_ticker_carries_its_remainder_rather_than_dropping_it() {
        // Frames that do not divide evenly into ticks must still add up.
        let mut ticker = Ticker::default();
        let mut total = 0;
        for _ in 0..144 {
            total += ticker.take(1.0 / 144.0);
        }
        assert!(
            (total as i32 - MOVE_HZ as i32).abs() <= 1,
            "a second at 144 fps produced {total} ticks, not {MOVE_HZ}"
        );
    }

    #[test]
    fn a_long_stall_is_dropped_rather_than_fired_through_a_wall() {
        let mut ticker = Ticker::default();
        let ticks = ticker.take(30.0);
        assert!(
            ticks as f32 * MOVE_TICK <= MAX_CATCHUP + MOVE_TICK,
            "caught up {ticks} ticks of a thirty second stall"
        );
    }

    #[test]
    fn a_full_block_ledge_is_vaulted_without_pressing_anything() {
        // STEP_HEIGHT is 0.6 now, so this cannot be walked. The mine planner
        // cuts one-block benches and the player walks through them constantly;
        // if this needed a keypress, every excavation would be a chore.
        let mut world = flat_world();
        let stone = stone_of(&world);
        for x in 8..32 {
            for z in -5..15 {
                world.set_block(BlockPos::new(x, 41, z), stone);
            }
        }

        let mut body = standing(5.0, 4.5);
        let mut movement = Movement::default();
        let walk = MoveCommand::looking(FWD, facing_plus_x(), 0.0);
        // Long enough to reach the bench and get over it, short enough not to
        // stroll off the far end of the fixture.
        run(&mut movement, &mut body, &world, walk, 128);

        assert!(
            body.position.x > 9.0,
            "never got over the bench: stopped at x={}",
            body.position.x
        );
        assert!(
            (body.position.y - 42.0).abs() < 0.1,
            "ended at y={} rather than on top of the bench",
            body.position.y
        );
        assert!(!collides(&world, &body.aabb()));
    }

    /// The mantle is the one state that is not ordinary physics, and until
    /// stage 47 that meant it ran with *no collision query at all*: the box
    /// was walked along a fixed arc and only the destination had ever been
    /// checked. For about six tenths of a second per ledge the hull — and the
    /// eye inside it — passed through the block being climbed. This is the
    /// test that would have caught it: not "did it end up somewhere legal",
    /// but "was it ever anywhere illegal".
    #[test]
    fn a_mantle_never_puts_the_hull_inside_the_ledge_it_climbs() {
        let mut world = flat_world();
        let stone = stone_of(&world);
        // A two-block bench: too tall to walk or vault, exactly what a mantle
        // is for.
        for x in 8..32 {
            for z in -5..15 {
                for y in 41..43 {
                    world.set_block(BlockPos::new(x, y, z), stone);
                }
            }
        }

        let mut body = standing(5.0, 4.5);
        let mut movement = Movement::default();
        let climb = MoveCommand::looking(FWD | JUMP, facing_plus_x(), 0.0);
        let mut mantled = false;
        for tick in 0..192 {
            movement.advance(&mut body, &world, climb, 1.0, MOVE_TICK);
            mantled |= matches!(movement.stance, Stance::Mantling { .. });
            assert!(
                !collides(&world, &body.aabb()),
                "tick {tick}: hull inside the ledge at {:?} as {:?}",
                body.position,
                movement.stance
            );
        }

        assert!(mantled, "never mantled, so the arc was never exercised");
        assert!(
            body.position.y > 42.5,
            "never got up the bench: ended at y={}",
            body.position.y
        );
    }

    /// Standing up is the other place the margin was zero. `headroom` asked
    /// whether the taller hull *collides*, and thanks to the inset in the
    /// block query a head resting exactly on the ceiling plane does not — so
    /// the one gesture in the game that grows the hull was also the one that
    /// left it no room at all. The boundary is now `SKIN` below the plane,
    /// like every other surface the body meets.
    #[test]
    fn standing_up_under_a_ceiling_keeps_the_skin() {
        let mut world = flat_world();
        let stone = stone_of(&world);
        for x in 0..10 {
            for z in 0..10 {
                world.set_block(BlockPos::new(x, 43, z), stone);
            }
        }

        let body = standing(4.5, 4.5);
        let movement = Movement::default();
        // Feet at 41, the ceiling's underside at 43: two blocks of gap.
        let gap = 43.0 - body.position.y;
        assert!(
            !movement.headroom(&world, &body, gap as f32),
            "a hull exactly filling the gap was allowed to stand"
        );
        assert!(
            movement.headroom(&world, &body, (gap - vx_world::SKIN) as f32),
            "a hull a skin short of the gap was refused"
        );
    }

    #[test]
    fn a_three_block_wall_is_still_a_wall() {
        // Above MANTLE_MAX there is nothing to do but go around.
        let mut world = flat_world();
        let stone = stone_of(&world);
        for y in 41..45 {
            for z in -5..15 {
                world.set_block(BlockPos::new(8, y, z), stone);
            }
        }

        let mut body = standing(5.0, 4.5);
        let mut movement = Movement::default();
        let push = MoveCommand::looking(FWD | JUMP, facing_plus_x(), 0.0);
        // Checked every tick, not just at the end: a body that clips through a
        // wall and comes back out the far side would pass a final-position test.
        for _ in 0..400 {
            movement.advance(&mut body, &world, push, 1.0, MOVE_TICK);
            assert!(
                !collides(&world, &body.aabb()),
                "clipped into the wall at {:?}",
                body.position
            );
        }

        assert!(
            body.position.x < 8.0,
            "climbed a four-block wall to x={}",
            body.position.x
        );
    }

    #[test]
    fn you_cannot_stand_up_under_a_ceiling() {
        // The classic way to end up inside geometry. Blocks are unit cubes, so
        // the headroom over a crouching body is a whole number: one block of
        // clearance is not enough even to crouch in and drops you to prone, two
        // is enough to stand. Both cases matter, and neither may leave the hull
        // inside rock.
        let ceiling_at = |y: i32| {
            let mut world = flat_world();
            let stone = stone_of(&world);
            for x in 0..10 {
                for z in 0..10 {
                    world.set_block(BlockPos::new(x, y, z), stone);
                }
            }
            world
        };

        let settled = |world: &World| {
            let mut body = standing(4.5, 4.5);
            let mut movement = Movement::default();
            let crouch = MoveCommand::looking(CROUCH, 0.0, 0.0);
            run(&mut movement, &mut body, world, crouch, 32);
            // Let go of crouch: standing is only allowed if there is room.
            let idle = MoveCommand::looking(0, 0.0, 0.0);
            run(&mut movement, &mut body, world, idle, 32);
            assert!(
                !collides(world, &body.aabb()),
                "hull ended inside rock at {:?} as {:?}",
                body.position,
                movement.stance
            );
            movement.stance
        };

        // One block of headroom: too low even to crouch in.
        assert_eq!(settled(&ceiling_at(42)), Stance::Prone);
        // Two: room to stand.
        assert_eq!(settled(&ceiling_at(43)), Stance::Grounded);
    }

    #[test]
    fn a_slide_carries_downhill_and_dies_going_up() {
        // The behaviour the design note wanted from projecting gravity onto the
        // surface normal. There are no ramps in a voxel world — a decline is a
        // staircase and every normal is +Y — so it comes instead from what a
        // slide does with the drop off each step.
        let stairs = |descending: bool| {
            let mut world = flat_world();
            let stone = stone_of(&world);
            for step in 0..12 {
                let top = if descending { 40 - step } else { 40 + step };
                for x in (8 + step * 2)..(10 + step * 2) {
                    for z in -6..16 {
                        // Clear the flat floor first, or a "descent" is buried
                        // under it and both runs are the same run.
                        for y in 0..80 {
                            world.set_block(BlockPos::new(x, y, z), BlockId::AIR);
                        }
                        for y in 0..=top {
                            world.set_block(BlockPos::new(x, y, z), stone);
                        }
                    }
                }
            }
            world
        };

        let distance = |world: &World| {
            let mut body = standing(5.0, 4.5);
            let mut movement = Movement::default();
            let sprint = MoveCommand::looking(FWD | SPRINT, facing_plus_x(), 0.0);
            run(&mut movement, &mut body, world, sprint, 96);
            let launched = body.position.x;
            let slide = MoveCommand::looking(FWD | SPRINT | CROUCH, facing_plus_x(), 0.0);
            run(&mut movement, &mut body, world, slide, 128);
            body.position.x - launched
        };

        let down = distance(&stairs(true));
        let up = distance(&stairs(false));

        assert!(
            down > up,
            "a slide downhill ({down}) did not outrun one uphill ({up})"
        );
    }

    #[test]
    fn a_full_load_slows_you_and_never_stops_you() {
        assert_eq!(mass_of(0, 100), 1.0);
        assert!((mass_of(100, 100) - 0.55).abs() < 1e-6);
        // Overfull is still a floor, not a negative.
        assert!((mass_of(400, 100) - 0.55).abs() < 1e-6);
        // Monotonic all the way down.
        let mut last = f32::MAX;
        for carried in 0..=100 {
            let now = mass_of(carried, 100);
            assert!(now <= last + 1e-6, "not monotonic at {carried}");
            assert!(now >= 0.55, "floored below the floor at {carried}");
            last = now;
        }
        // A bigger hold makes the same rock lighter — Logistics as a movement
        // upgrade, with no new item.
        assert!(mass_of(50, 200) > mass_of(50, 100));
    }

    #[test]
    fn a_mantle_only_starts_where_there_is_room_to_stand() {
        // Pulling yourself into a ceiling is worse than being stopped.
        let mut world = flat_world();
        let stone = stone_of(&world);
        for x in 8..14 {
            for z in -5..15 {
                for y in 41..43 {
                    world.set_block(BlockPos::new(x, y, z), stone);
                }
                // A lid one block above the ledge top: nowhere to land.
                world.set_block(BlockPos::new(x, 44, z), stone);
            }
        }

        let body = standing(7.6, 4.5);
        let ahead = MoveCommand::looking(FWD, facing_plus_x(), 0.0).wish_dir();
        assert_eq!(
            classify_ledge(&world, &body, ahead),
            Ledge::None,
            "offered a mantle into a ceiling"
        );
    }

    #[test]
    fn every_classified_ledge_has_a_clear_column_to_stand_in() {
        let mut world = flat_world();
        let stone = stone_of(&world);
        for height in 1..=3 {
            let x = 8 + height * 4;
            for dx in 0..3 {
                for z in -5..15 {
                    for y in 41..(41 + height) {
                        world.set_block(BlockPos::new(x + dx, y, z), stone);
                    }
                }
            }
        }

        let ahead = MoveCommand::looking(FWD, facing_plus_x(), 0.0).wish_dir();
        for height in 1..=3 {
            let x = (8 + height * 4) as f64 - 0.4;
            let body = standing(x, 4.5);
            match classify_ledge(&world, &body, ahead) {
                Ledge::Vault { across, .. } | Ledge::Mantle { across, .. } => {
                    let column = Aabb::standing_on(across, body.width, STAND_HEIGHT as f64);
                    assert!(
                        !collides(&world, &column),
                        "climb of {height} blocks lands inside rock at {across:?}"
                    );
                }
                other => {
                    assert!(height >= 3, "a {height}-block ledge classified as {other:?}");
                }
            }
        }
    }

    #[test]
    fn sprinting_costs_wind_and_standing_still_gets_it_back() {
        let world = flat_world();
        let mut body = standing(4.5, 4.5);
        let mut movement = Movement::default();

        let sprint = MoveCommand::looking(FWD | SPRINT, facing_plus_x(), 0.0);
        run(&mut movement, &mut body, &world, sprint, 256);
        let spent = movement.stamina;
        assert!(spent < STAM_MAX, "sprinting was free");

        let idle = MoveCommand::looking(0, facing_plus_x(), 0.0);
        run(&mut movement, &mut body, &world, idle, 256);
        assert!(movement.stamina > spent, "never got the wind back");
    }

    #[test]
    fn a_heavier_load_runs_you_out_of_breath_sooner() {
        // Where the weight system gets its teeth.
        let world = flat_world();
        let wind_left = |mass: f32| {
            let mut body = standing(4.5, 4.5);
            let mut movement = Movement::default();
            let sprint = MoveCommand::looking(FWD | SPRINT, facing_plus_x(), 0.0);
            for _ in 0..256 {
                movement.advance(&mut body, &world, sprint, mass, MOVE_TICK);
            }
            movement.stamina
        };
        assert!(
            wind_left(mass_of(100, 100)) < wind_left(mass_of(0, 100)),
            "a full load did not cost extra wind"
        );
    }

    #[test]
    fn walking_is_free_forever() {
        // Stamina gates the exceptional, not the ordinary.
        let world = flat_world();
        let mut body = standing(4.5, 4.5);
        let mut movement = Movement::default();
        let walk = MoveCommand::looking(FWD, facing_plus_x(), 0.0);
        run(&mut movement, &mut body, &world, walk, 2_000);
        assert_eq!(movement.stamina, STAM_MAX, "walking drew on the bar");
    }

    #[test]
    fn sprinting_outruns_walking_by_the_margin_the_constants_promise() {
        let world = flat_world();
        let covered = |bits: u16| {
            let mut body = standing(4.5, 4.5);
            let mut movement = Movement::default();
            let command = MoveCommand::looking(bits, facing_plus_x(), 0.0);
            let start = body.position;
            run(&mut movement, &mut body, &world, command, MOVE_HZ as usize);
            (body.position - start).length()
        };

        let walked = covered(FWD);
        let sprinted = covered(FWD | SPRINT);
        // Stated against the constants rather than a magic number, so retuning
        // one of them cannot quietly invalidate the test.
        let expected = (SPRINT_SPEED / WALK) as f64;
        assert!(
            (sprinted / walked - expected).abs() < 0.1,
            "walk {walked}, sprint {sprinted}, ratio {} wanted {expected}",
            sprinted / walked
        );
    }

    #[test]
    fn a_slide_jump_keeps_the_speed_the_slide_built() {
        // Take this away and sliding is a strictly worse crouch.
        let world = flat_world();
        let mut body = standing(4.5, 4.5);
        let mut movement = Movement::default();

        let sprint = MoveCommand::looking(FWD | SPRINT, facing_plus_x(), 0.0);
        run(&mut movement, &mut body, &world, sprint, 64);
        let slide = MoveCommand::looking(FWD | SPRINT | CROUCH, facing_plus_x(), 0.0);
        run(&mut movement, &mut body, &world, slide, 4);
        let carried = DVec3::new(body.velocity.x, 0.0, body.velocity.z).length();

        let hop = MoveCommand::looking(FWD | SPRINT | CROUCH | JUMP, facing_plus_x(), 0.0);
        movement.advance(&mut body, &world, hop, 1.0, MOVE_TICK);

        let after = DVec3::new(body.velocity.x, 0.0, body.velocity.z).length();
        assert!(body.velocity.y > 0.0, "the slide jump did not leave the ground");
        assert!(
            after >= carried - 0.5,
            "the jump threw away the slide's speed: {carried} became {after}"
        );
    }
}
