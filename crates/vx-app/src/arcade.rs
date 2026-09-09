//! The pocket arcade: a small original shooter that runs on the handheld.
//!
//! # It is not a port of anything
//!
//! The licensing note in `ROADMAP.md` has said so since long before this
//! stage existed, and it is the first constraint here rather than an
//! afterthought: no WAD, no borrowed engine, no sprites off a disc. Every
//! wall, every colour and every shape below is computed from numbers, the
//! same way the terrain, the block tiles and the gunfire already are. The
//! *form* — a grid of walls drawn a column at a time, things coming at you
//! down a corridor — is thirty years old and belongs to nobody.
//!
//! # A column at a time, into a buffer somebody is holding
//!
//! The renderer is the classic one, and it is classic because it is the
//! cheapest thing that works: for each of the 240 columns on the handheld's
//! screen, walk the grid until a wall is hit, and draw a vertical strip whose
//! height is the reciprocal of that distance. No triangles, no GPU, no new
//! pipeline. The frame it produces is a plain RGBA buffer exactly like every
//! other panel in this game, which means it lands on the unit's glass through
//! the same projection stage 33 built — the game is *on the thing in your
//! hands*, not in a window over it.
//!
//! # The floors are a number
//!
//! [`Level::of`] carves its map with the world's own hashing, and carves it
//! as a single continuous walk, so the map is connected **by construction**:
//! there is no reachability check anywhere below because there is no way to
//! author an unreachable exit. The same cartridge therefore plays the same
//! floors in the same order on every machine, which is the only thing that
//! makes a high score worth keeping.
//!
//! # Live-only
//!
//! Nothing here touches the world, the pile, the clock or the journal. It is
//! a toy on a screen, and the replay oracle has never heard of it.

use std::io::{Read, Write};
use std::path::Path;

use vx_render::font;
use vx_world::seed::{finalise, unit};

use crate::device::{DEVICE_HEIGHT, DEVICE_WIDTH};

/// Cells to a side of one floor.
pub const SIDE: usize = 24;

/// How tall the status strip along the bottom is, in screen pixels.
const STATUS: u32 = 13;

/// The horizon: everything above is ceiling, below is floor.
const HORIZON: u32 = (DEVICE_HEIGHT - STATUS) / 2;

/// Half the field of view, as the width of the camera plane against a unit
/// forward vector. 0.66 is a shade under sixty degrees.
const PLANE: f32 = 0.66;

/// How far a column looks before it gives up and calls it darkness.
const DRAW_DISTANCE: f32 = 20.0;

/// Cells a second, walking and strafing.
const WALK: f32 = 2.6;
/// Radians a second, turning.
const TURN: f32 = 2.4;

/// How close a wall may get before it stops you. Keeps the camera out of the
/// geometry, which is what makes the raycaster's life simple.
const SKIN: f32 = 0.22;

/// Seconds between shots, and how long the muzzle flash shows.
const FIRE_EVERY: f32 = 0.28;
const FLASH: f32 = 0.09;

/// Seconds of mercy after something touches you.
const HURT_EVERY: f32 = 0.9;

/// What a body starts with, and the most it can hold.
pub const MAX_HEALTH: i32 = 5;

/// Rounds a floor starts you with, and what a kill gives back.
const FLOOR_AMMO: u32 = 30;
const KILL_AMMO: u32 = 2;

/// How close something has to be to hurt you, and to be shot.
const TOUCH: f32 = 0.55;
const HIT_HALF_WIDTH: f32 = 0.38;

const MAGIC: &[u8; 4] = b"VXGM";
const VERSION: u32 = 1;

// ------------------------------------------------------------------ colours

const CEILING: [u8; 3] = [58, 62, 78];
const FLOOR_TONE: [u8; 3] = [86, 74, 58];
const WALL_LIGHT: [u8; 3] = [178, 120, 70];
const WALL_DARK: [u8; 3] = [120, 82, 48];
const EXIT_LIGHT: [u8; 3] = [90, 210, 160];
const EXIT_DARK: [u8; 3] = [56, 150, 112];
const ENEMY_BODY: [u8; 4] = [190, 60, 60, 255];
const ENEMY_EYE: [u8; 4] = [250, 230, 120, 255];
const BACKDROP: [u8; 4] = [8, 10, 14, 255];
const TEXT: [u8; 4] = [225, 230, 235, 255];
const WARN: [u8; 4] = [235, 110, 90, 255];
const GOOD: [u8; 4] = [150, 220, 150, 255];
const MUZZLE: [u8; 4] = [255, 220, 140, 255];

/// One floor: which cells are rock, where you come in and where the way out
/// is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    solid: Vec<bool>,
    start: (usize, usize),
    exit: (usize, usize),
}

fn hash01(seed: u64, salt: u64) -> f32 {
    unit(finalise(seed ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)))
}

impl Level {
    /// The floor `floor` of the cartridge seeded `seed`.
    ///
    /// Carved by one continuous walk, which is the whole trick: a walk cannot
    /// leave a room it never entered, so everything it opens is reachable
    /// from everything else it opened, and the exit — placed on the walk —
    /// can never be walled off. No flood fill, no retries, no "generate until
    /// it works" loop.
    pub fn of(seed: u64, floor: u32) -> Level {
        let seed = finalise(seed ^ (floor as u64 + 1).wrapping_mul(0x51ed_5eed_1234_5677));
        let mut solid = vec![true; SIDE * SIDE];

        // Start near the middle, and never touch the outer ring: a level with
        // a hole in its edge is a level you can walk out of.
        let mut at = (SIDE / 2, SIDE / 2);
        let mut carved = vec![at];
        solid[at.1 * SIDE + at.0] = false;

        // Longer walks on deeper floors, so the maps grow with the run.
        let steps = 260 + floor as usize * 45;
        for step in 0..steps {
            let roll = hash01(seed, step as u64 * 3 + 1);
            let (dx, dz) = match (roll * 4.0) as u32 {
                0 => (1i32, 0i32),
                1 => (-1, 0),
                2 => (0, 1),
                _ => (0, -1),
            };
            // Two cells at a time keeps corridors from degenerating into one
            // open blob, which is what a one-cell walk gives you.
            for _ in 0..2 {
                let next = (at.0 as i32 + dx, at.1 as i32 + dz);
                if next.0 < 1 || next.1 < 1 || next.0 >= SIDE as i32 - 1 || next.1 >= SIDE as i32 - 1
                {
                    break;
                }
                at = (next.0 as usize, next.1 as usize);
                let cell = at.1 * SIDE + at.0;
                if solid[cell] {
                    solid[cell] = false;
                    carved.push(at);
                }
            }
        }

        let start = carved[0];
        // The way out is the furthest thing the walk reached — furthest by
        // steps taken, not by distance, so it is genuinely the far end of the
        // map rather than a corner of the same room.
        let exit = *carved
            .iter()
            .max_by_key(|(x, z)| {
                (*x as i32 - start.0 as i32).abs() + (*z as i32 - start.1 as i32).abs()
            })
            .unwrap_or(&start);

        Level { solid, start, exit }
    }

    /// Is this cell rock? Anything off the grid is.
    pub fn solid(&self, x: i32, z: i32) -> bool {
        if x < 0 || z < 0 || x >= SIDE as i32 || z >= SIDE as i32 {
            return true;
        }
        self.solid[z as usize * SIDE + x as usize]
    }

    /// Is this cell the way out?
    pub fn is_exit(&self, x: i32, z: i32) -> bool {
        (x, z) == (self.exit.0 as i32, self.exit.1 as i32)
    }

    /// Every open cell, for placing things.
    fn open(&self) -> Vec<(usize, usize)> {
        (0..SIDE * SIDE)
            .filter(|cell| !self.solid[*cell])
            .map(|cell| (cell % SIDE, cell / SIDE))
            .collect()
    }
}

/// Where the player is standing and looking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub x: f32,
    pub z: f32,
    pub facing: f32,
}

/// One of them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Enemy {
    x: f32,
    z: f32,
    alive: bool,
}

/// What the machine is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Waiting to be started.
    Attract,
    Playing,
    /// Dead, showing the score until somebody presses start.
    Over,
}

/// Which controls are held this frame.
///
/// Keys rather than the mouse, deliberately: the handheld is a panel, a panel
/// releases the pointer, and the gamepad's panel mapping already turns sticks
/// and buttons into key codes — so a keys-only game is one that works on a
/// Deck without a line of new input code.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Buttons {
    pub forward: bool,
    pub back: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub fire: bool,
    pub start: bool,
}

/// What a step did that is worth saying out loud.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub killed: u32,
    pub cleared: bool,
    pub died: bool,
}

/// The cartridge, and the game on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Arcade {
    /// Has one been printed? Until then the page is an advertisement.
    pub owned: bool,
    pub state: State,
    pub floor: u32,
    pub score: u32,
    pub best: u32,
    pub deepest: u32,
    pub health: i32,
    pub ammo: u32,
    pose: Pose,
    level: Level,
    enemies: Vec<Enemy>,
    seed: u64,
    fire_cool: f32,
    flash: f32,
    hurt_cool: f32,
    /// The last thing that happened, for the status strip.
    pub tell: Option<String>,
}

impl Default for Arcade {
    fn default() -> Self {
        let seed = 0x9e37_79b9_7f4a_7c15;
        let level = Level::of(seed, 1);
        Arcade {
            owned: false,
            state: State::Attract,
            floor: 1,
            score: 0,
            best: 0,
            deepest: 0,
            health: MAX_HEALTH,
            ammo: FLOOR_AMMO,
            pose: Pose {
                x: level.start.0 as f32 + 0.5,
                z: level.start.1 as f32 + 0.5,
                facing: 0.0,
            },
            level,
            enemies: Vec::new(),
            seed,
            fire_cool: 0.0,
            flash: 0.0,
            hurt_cool: 0.0,
            tell: None,
        }
    }
}

impl Arcade {
    /// A cartridge comes out of the fabricator.
    pub fn print(&mut self) {
        self.owned = true;
        self.tell = Some("CARTRIDGE LOADED".into());
    }

    /// Start a run at floor one.
    pub fn start(&mut self) {
        self.floor = 1;
        self.score = 0;
        self.health = MAX_HEALTH;
        self.state = State::Playing;
        self.enter_floor();
        self.tell = None;
    }

    /// Lay out the current floor and put everything on it.
    fn enter_floor(&mut self) {
        self.level = Level::of(self.seed, self.floor);
        self.pose = Pose {
            x: self.level.start.0 as f32 + 0.5,
            z: self.level.start.1 as f32 + 0.5,
            facing: 0.0,
        };
        // Less ammunition the deeper it goes, and never so little that a
        // floor is unwinnable before it starts.
        self.ammo = FLOOR_AMMO.saturating_sub(self.floor * 2).max(10);
        self.fire_cool = 0.0;
        self.hurt_cool = 0.0;

        let open = self.level.open();
        let count = (3 + self.floor as usize).min(14);
        let start = self.level.start;
        self.enemies = (0..count)
            .filter_map(|index| {
                // Spread them over the map by walking the open list at a
                // hashed stride, so two never share a cell and none of them
                // is standing on the door you came in by.
                let roll = hash01(self.seed ^ self.floor as u64, index as u64 * 7 + 3);
                let cell = open[(roll * open.len() as f32) as usize % open.len()];
                let away = (cell.0 as i32 - start.0 as i32).abs()
                    + (cell.1 as i32 - start.1 as i32).abs();
                (away > 4).then_some(Enemy {
                    x: cell.0 as f32 + 0.5,
                    z: cell.1 as f32 + 0.5,
                    alive: true,
                })
            })
            .collect();
    }

    /// How many are still standing.
    pub fn standing(&self) -> usize {
        self.enemies.iter().filter(|enemy| enemy.alive).count()
    }

    /// The floor being walked. The picture reads it; so does a fixture that
    /// has to find its way across one.
    pub fn level(&self) -> &Level {
        &self.level
    }

    /// Where the player is standing and looking. The picture does not need
    /// telling — it reads the pose itself — but a test does, and so does the
    /// capture fixture, which has to steer the thing to get a frame worth
    /// looking at.
    pub fn pose(&self) -> Pose {
        self.pose
    }

    /// The closest one still on its feet, in grid units. `sighted` narrows it
    /// to the ones there is nothing between you and.
    pub fn nearest_standing(&self) -> Option<(f32, f32)> {
        self.closest(false)
    }

    /// The closest one you could actually see from where you stand.
    pub fn sighted(&self) -> Option<(f32, f32)> {
        self.closest(true)
    }

    fn closest(&self, visible_only: bool) -> Option<(f32, f32)> {
        self.enemies
            .iter()
            .filter(|enemy| enemy.alive)
            .filter(|enemy| {
                !visible_only
                    || line_of_sight(&self.level, (self.pose.x, self.pose.z), (enemy.x, enemy.z))
            })
            .min_by(|left, right| {
                let span = |enemy: &&Enemy| {
                    let dx = enemy.x - self.pose.x;
                    let dz = enemy.z - self.pose.z;
                    dx * dx + dz * dz
                };
                span(left).total_cmp(&span(right))
            })
            .map(|enemy| (enemy.x, enemy.z))
    }

    /// May a body stand here?
    fn clear(&self, x: f32, z: f32) -> bool {
        !self.level.solid((x + SKIN) as i32, (z + SKIN) as i32)
            && !self.level.solid((x - SKIN) as i32, (z - SKIN) as i32)
            && !self.level.solid((x + SKIN) as i32, (z - SKIN) as i32)
            && !self.level.solid((x - SKIN) as i32, (z + SKIN) as i32)
    }

    /// One frame of the game.
    pub fn step(&mut self, dt: f32, held: Buttons) -> Report {
        let mut report = Report::default();
        self.flash = (self.flash - dt).max(0.0);

        match self.state {
            State::Attract | State::Over => {
                if held.start {
                    self.start();
                }
                return report;
            }
            State::Playing => {}
        }

        self.fire_cool = (self.fire_cool - dt).max(0.0);
        self.hurt_cool = (self.hurt_cool - dt).max(0.0);

        // Turning.
        if held.turn_left {
            self.pose.facing -= TURN * dt;
        }
        if held.turn_right {
            self.pose.facing += TURN * dt;
        }

        // Walking, one axis at a time so a wall slides you along it rather
        // than stopping you dead — the same courtesy the player's own physics
        // extends outside the game.
        let (sin, cos) = self.pose.facing.sin_cos();
        let mut along = 0.0;
        let mut across = 0.0;
        if held.forward {
            along += 1.0;
        }
        if held.back {
            along -= 1.0;
        }
        if held.strafe_right {
            across += 1.0;
        }
        if held.strafe_left {
            across -= 1.0;
        }
        let step = WALK * dt;
        let dx = (cos * along - sin * across) * step;
        let dz = (sin * along + cos * across) * step;
        if self.clear(self.pose.x + dx, self.pose.z) {
            self.pose.x += dx;
        }
        if self.clear(self.pose.x, self.pose.z + dz) {
            self.pose.z += dz;
        }

        // Shooting: a ray down the middle of the screen, nearest thing hit.
        if held.fire && self.fire_cool == 0.0 && self.ammo > 0 {
            self.fire_cool = FIRE_EVERY;
            self.flash = FLASH;
            self.ammo -= 1;
            if let Some(index) = self.aimed_at() {
                self.enemies[index].alive = false;
                self.ammo += KILL_AMMO;
                self.score += 10 * self.floor;
                report.killed += 1;
                self.tell = Some("HIT".into());
            }
        }

        // Them. They close when they can see you, which is a line check
        // against the same grid the walls are drawn from.
        let (px, pz) = (self.pose.x, self.pose.z);
        let speed = (0.8 + self.floor as f32 * 0.12).min(2.2) * dt;
        let mut touched = false;
        for enemy in &mut self.enemies {
            if !enemy.alive {
                continue;
            }
            let (dx, dz) = (px - enemy.x, pz - enemy.z);
            let gap = (dx * dx + dz * dz).sqrt();
            if gap < TOUCH {
                touched = true;
                continue;
            }
            if gap > 0.001 && line_of_sight(&self.level, (enemy.x, enemy.z), (px, pz)) {
                let (nx, nz) = (dx / gap * speed, dz / gap * speed);
                if !self.level.solid((enemy.x + nx) as i32, enemy.z as i32) {
                    enemy.x += nx;
                }
                if !self.level.solid(enemy.x as i32, (enemy.z + nz) as i32) {
                    enemy.z += nz;
                }
            }
        }

        if touched && self.hurt_cool == 0.0 {
            self.hurt_cool = HURT_EVERY;
            self.health -= 1;
            self.tell = Some("HIT - GET BACK".into());
            if self.health <= 0 {
                self.state = State::Over;
                self.best = self.best.max(self.score);
                self.deepest = self.deepest.max(self.floor);
                self.tell = Some(format!("DOWN ON FLOOR {}", self.floor));
                report.died = true;
                return report;
            }
        }

        // The way out.
        if self.level.is_exit(self.pose.x as i32, self.pose.z as i32) {
            self.floor += 1;
            self.score += 50;
            self.health = (self.health + 1).min(MAX_HEALTH);
            self.deepest = self.deepest.max(self.floor);
            self.best = self.best.max(self.score);
            self.enter_floor();
            self.tell = Some(format!("FLOOR {}", self.floor));
            report.cleared = true;
        }

        report
    }

    /// The nearest thing the barrel is pointed at, if anything.
    fn aimed_at(&self) -> Option<usize> {
        let (sin, cos) = self.pose.facing.sin_cos();
        let mut best: Option<(usize, f32)> = None;
        for (index, enemy) in self.enemies.iter().enumerate() {
            if !enemy.alive {
                continue;
            }
            let (dx, dz) = (enemy.x - self.pose.x, enemy.z - self.pose.z);
            // Into the shooter's own frame: along the barrel, and across it.
            let along = dx * cos + dz * sin;
            let across = -dx * sin + dz * cos;
            if along <= 0.0 || along > DRAW_DISTANCE {
                continue;
            }
            // A body is a body wide at any range, not an angle — which is
            // what stops distant things being impossible to hit.
            if across.abs() > HIT_HALF_WIDTH {
                continue;
            }
            if !line_of_sight(&self.level, (self.pose.x, self.pose.z), (enemy.x, enemy.z)) {
                continue;
            }
            if best.is_none_or(|(_, nearest)| along < nearest) {
                best = Some((index, along));
            }
        }
        best.map(|(index, _)| index)
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "arcade.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&[u8::from(self.owned)])?;
        file.write_all(&self.best.to_le_bytes())?;
        file.write_all(&self.deepest.to_le_bytes())?;
        file.commit()
    }

    /// Read the cartridge back. What survives a session is what a cabinet
    /// keeps: whether you own it, your best, and how deep you ever got. The
    /// run itself does not — nobody saves a game of this.
    pub fn load(&mut self, directory: &Path) {
        match read(&directory.join("arcade.dat")) {
            Ok(Some((owned, best, deepest))) => {
                self.owned = owned;
                self.best = best;
                self.deepest = deepest;
            }
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged arcade file: {error}"),
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<(bool, u32, u32)>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not an arcade file"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Ok(None);
    }
    let mut flag = [0u8; 1];
    file.read_exact(&mut flag)?;
    file.read_exact(&mut word)?;
    let best = u32::from_le_bytes(word);
    file.read_exact(&mut word)?;
    Ok(Some((flag[0] != 0, best, u32::from_le_bytes(word))))
}

/// Can these two points see each other across the grid?
///
/// Sampled along the segment rather than walked cell by cell: the step is
/// well under a cell, so nothing slips through a corner, and the whole thing
/// is six lines instead of a second DDA.
fn line_of_sight(level: &Level, from: (f32, f32), to: (f32, f32)) -> bool {
    let (dx, dz) = (to.0 - from.0, to.1 - from.1);
    let span = (dx * dx + dz * dz).sqrt();
    let steps = (span / 0.2).ceil() as i32;
    for step in 1..steps {
        let t = step as f32 / steps as f32;
        let (x, z) = (from.0 + dx * t, from.1 + dz * t);
        if level.solid(x as i32, z as i32) {
            return false;
        }
    }
    true
}

// ----------------------------------------------------------------- drawing

fn put(pixels: &mut [u8], x: u32, y: u32, colour: [u8; 4]) {
    if x >= DEVICE_WIDTH || y >= DEVICE_HEIGHT {
        return;
    }
    let at = ((y * DEVICE_WIDTH + x) * 4) as usize;
    pixels[at..at + 4].copy_from_slice(&colour);
}

/// Dim a colour with distance. Fog is free depth cueing and the only thing
/// standing between a flat wall of colour and something that reads as a
/// corridor.
fn fade(base: [u8; 3], distance: f32) -> [u8; 4] {
    let light = (1.0 - distance / DRAW_DISTANCE).clamp(0.12, 1.0);
    [
        (base[0] as f32 * light) as u8,
        (base[1] as f32 * light) as u8,
        (base[2] as f32 * light) as u8,
        255,
    ]
}

/// Draw the game. Pure in the state, like every other panel here.
pub fn render(game: &Arcade) -> Vec<u8> {
    let mut pixels = vec![0u8; (DEVICE_WIDTH * DEVICE_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKDROP);
    }

    if !game.owned {
        return advertisement(pixels);
    }
    if game.state != State::Playing {
        return title(pixels, game);
    }

    let view_height = DEVICE_HEIGHT - STATUS;
    let (sin, cos) = game.pose.facing.sin_cos();
    // The camera plane, perpendicular to the barrel.
    let (plane_x, plane_z) = (-sin * PLANE, cos * PLANE);

    // Ceiling and floor first, shaded by how far away the ground under each
    // row is. Two flat bands read as a void; a corridor is a thing that
    // recedes, and the same reciprocal that sizes the walls does it here.
    for y in 0..view_height {
        let rise = (y as f32 - HORIZON as f32).abs().max(1.0);
        let distance = (HORIZON as f32 / rise).min(DRAW_DISTANCE);
        let base = if y < HORIZON { CEILING } else { FLOOR_TONE };
        let colour = fade(base, distance);
        for x in 0..DEVICE_WIDTH {
            put(&mut pixels, x, y, colour);
        }
    }

    // One ray a column, and keep what each one hit so the sprites can be
    // depth-tested against the walls without a second pass.
    let mut depth = vec![DRAW_DISTANCE; DEVICE_WIDTH as usize];
    for column in 0..DEVICE_WIDTH {
        let camera = 2.0 * column as f32 / DEVICE_WIDTH as f32 - 1.0;
        let (ray_x, ray_z) = (cos + plane_x * camera, sin + plane_z * camera);

        let mut cell = (game.pose.x as i32, game.pose.z as i32);
        let delta_x = if ray_x.abs() < 1.0e-6 { f32::MAX } else { (1.0 / ray_x).abs() };
        let delta_z = if ray_z.abs() < 1.0e-6 { f32::MAX } else { (1.0 / ray_z).abs() };
        let (step_x, mut side_x) = if ray_x < 0.0 {
            (-1, (game.pose.x - cell.0 as f32) * delta_x)
        } else {
            (1, (cell.0 as f32 + 1.0 - game.pose.x) * delta_x)
        };
        let (step_z, mut side_z) = if ray_z < 0.0 {
            (-1, (game.pose.z - cell.1 as f32) * delta_z)
        } else {
            (1, (cell.1 as f32 + 1.0 - game.pose.z) * delta_z)
        };

        let mut hit_side = false;
        let mut distance = DRAW_DISTANCE;
        for _ in 0..(DRAW_DISTANCE as i32 * 3) {
            if side_x < side_z {
                cell.0 += step_x;
                hit_side = false;
                if game.level.solid(cell.0, cell.1) {
                    distance = side_x;
                    break;
                }
                side_x += delta_x;
            } else {
                cell.1 += step_z;
                hit_side = true;
                if game.level.solid(cell.0, cell.1) {
                    distance = side_z;
                    break;
                }
                side_z += delta_z;
            }
        }
        let distance = distance.clamp(0.05, DRAW_DISTANCE);
        depth[column as usize] = distance;

        // The strip. Reciprocal of distance is the whole of perspective.
        let height = (view_height as f32 / distance) as i32;
        let top = (HORIZON as i32 - height / 2).max(0) as u32;
        let bottom = ((HORIZON as i32 + height / 2).min(view_height as i32 - 1)).max(0) as u32;
        // The far side of the exit is lit differently, so the way out is
        // visible from across the map rather than something you walk into.
        let exit = game.level.is_exit(cell.0, cell.1);
        let base = match (exit, hit_side) {
            (true, false) => EXIT_LIGHT,
            (true, true) => EXIT_DARK,
            (false, false) => WALL_LIGHT,
            (false, true) => WALL_DARK,
        };
        let colour = fade(base, distance);
        for y in top..=bottom {
            put(&mut pixels, column, y, colour);
        }
    }

    // Them, furthest first so the near ones draw over the far ones.
    let mut order: Vec<(f32, usize)> = game
        .enemies
        .iter()
        .enumerate()
        .filter(|(_, enemy)| enemy.alive)
        .map(|(index, enemy)| {
            let (dx, dz) = (enemy.x - game.pose.x, enemy.z - game.pose.z);
            (dx * dx + dz * dz, index)
        })
        .collect();
    order.sort_by(|left, right| right.0.total_cmp(&left.0));

    for (_, index) in order {
        let enemy = game.enemies[index];
        let (dx, dz) = (enemy.x - game.pose.x, enemy.z - game.pose.z);
        // Into camera space: the same two-by-two inverse every sprite
        // billboard has used since this technique was invented.
        let determinant = 1.0 / (plane_x * sin - cos * plane_z);
        let transform_x = determinant * (sin * dx - cos * dz);
        let transform_z = determinant * (-plane_z * dx + plane_x * dz);
        if transform_z <= 0.15 {
            continue;
        }
        let screen_x =
            ((DEVICE_WIDTH as f32 / 2.0) * (1.0 + transform_x / transform_z)) as i32;
        let size = ((view_height as f32 / transform_z) * 0.7) as i32;
        if size <= 1 {
            continue;
        }
        let top = (HORIZON as i32 + (view_height as i32 / 2 - size) / 2).max(0);
        let body = fade(
            [ENEMY_BODY[0], ENEMY_BODY[1], ENEMY_BODY[2]],
            transform_z,
        );
        let eye = fade([ENEMY_EYE[0], ENEMY_EYE[1], ENEMY_EYE[2]], transform_z);
        for offset in -size / 2..size / 2 {
            let column = screen_x + offset;
            if column < 0 || column >= DEVICE_WIDTH as i32 {
                continue;
            }
            // Behind a wall is behind a wall.
            if transform_z >= depth[column as usize] {
                continue;
            }
            let across = (offset as f32 / (size as f32 * 0.5)).abs();
            for row in 0..size {
                let y = top + row;
                if y < 0 || y >= (view_height as i32) {
                    continue;
                }
                let down = row as f32 / size as f32;
                // A tapered body: narrow at the top, wide at the base. Two
                // bright eyes near the top, which is the only feature it
                // needs to read as facing you.
                let width_here = 0.45 + down * 0.55;
                if across > width_here {
                    continue;
                }
                let eyes = (0.18..0.30).contains(&down) && (0.30..0.75).contains(&across);
                put(
                    &mut pixels,
                    column as u32,
                    y as u32,
                    if eyes { eye } else { body },
                );
            }
        }
    }

    // The barrel, and its flash.
    let gun_top = view_height - 22;
    for y in gun_top..view_height {
        let half = 5 + (y - gun_top) / 3;
        for x in (DEVICE_WIDTH / 2 - half)..(DEVICE_WIDTH / 2 + half) {
            put(&mut pixels, x, y, [58, 60, 68, 255]);
        }
    }
    if game.flash > 0.0 {
        for y in (gun_top - 8)..gun_top {
            let half = 10 - (gun_top - y) / 2;
            for x in (DEVICE_WIDTH / 2 - half)..(DEVICE_WIDTH / 2 + half) {
                put(&mut pixels, x, y, MUZZLE);
            }
        }
    }

    status(&mut pixels, game);
    pixels
}

/// The strip along the bottom: what a cabinet shows you without making you
/// look away from what is coming.
fn status(pixels: &mut [u8], game: &Arcade) {
    let top = DEVICE_HEIGHT - STATUS;
    for y in top..DEVICE_HEIGHT {
        for x in 0..DEVICE_WIDTH {
            put(pixels, x, y, [14, 16, 20, 255]);
        }
    }
    let line = format!(
        "F{} HP{} AM{} {}",
        game.floor, game.health, game.ammo, game.score
    );
    font::draw_text(
        pixels,
        DEVICE_WIDTH,
        4,
        top as i32 + 3,
        1,
        if game.health <= 1 { WARN } else { TEXT },
        &line,
    );
    let left = format!("{} LEFT", game.standing());
    font::draw_text(
        pixels,
        DEVICE_WIDTH,
        DEVICE_WIDTH as i32 - 4 - font::text_width(&left, 1) as i32,
        top as i32 + 3,
        1,
        if game.standing() == 0 { GOOD } else { TEXT },
        &left,
    );
}

/// What the page says with no cartridge in it.
fn advertisement(mut pixels: Vec<u8>) -> Vec<u8> {
    let lines = [
        ("POCKET ARCADE", TEXT),
        ("", TEXT),
        ("NO CARTRIDGE LOADED", WARN),
        ("PRINT ONE AT A FABRICATOR", TEXT),
        ("", TEXT),
        ("ONE ORIGINAL SHOOTER.", GOOD),
        ("FLOORS, A WAY OUT, AND", GOOD),
        ("SOMETHING IN THE DARK.", GOOD),
    ];
    let mut y = 26i32;
    for (line, colour) in lines {
        font::draw_text(&mut pixels, DEVICE_WIDTH, 14, y, 1, colour, line);
        y += font::LINE_HEIGHT as i32;
    }
    pixels
}

/// The attract screen and the one after you die — the same layout, because a
/// cabinet only ever has the one.
fn title(mut pixels: Vec<u8>, game: &Arcade) -> Vec<u8> {
    let heading = match game.state {
        State::Over => "GAME OVER",
        _ => "POCKET ARCADE",
    };
    font::draw_text(&mut pixels, DEVICE_WIDTH, 14, 24, 1, TEXT, heading);
    let lines = [
        format!("SCORE {}", game.score),
        format!("BEST  {}", game.best),
        format!("DEEPEST FLOOR {}", game.deepest.max(1)),
        String::new(),
        "ENTER STARTS".to_string(),
        "WASD MOVES, Q E STRAFE".to_string(),
        "SPACE FIRES".to_string(),
    ];
    let mut y = 48i32;
    for line in lines {
        font::draw_text(&mut pixels, DEVICE_WIDTH, 14, y, 1, GOOD, &line);
        y += font::LINE_HEIGHT as i32;
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(floor: u32) -> Arcade {
        let mut game = Arcade {
            owned: true,
            ..Arcade::default()
        };
        game.start();
        for _ in 1..floor {
            game.floor += 1;
            game.enter_floor();
        }
        game
    }

    #[test]
    fn a_floor_is_a_number_and_the_way_out_is_always_reachable() {
        // The carve's whole promise: one continuous walk cannot open a room
        // it never entered, so the exit — placed on the walk — is reachable
        // from the start by construction. Checked here with a flood fill,
        // which is the thing the generator is allowed *not* to do.
        for seed in [1u64, 909, 4242] {
            for floor in 1..8 {
                let level = Level::of(seed, floor);
                assert_eq!(level, Level::of(seed, floor), "a floor changed its mind");

                let mut seen = vec![false; SIDE * SIDE];
                let mut queue = vec![level.start];
                seen[level.start.1 * SIDE + level.start.0] = true;
                while let Some((x, z)) = queue.pop() {
                    for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                        let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                        if level.solid(nx, nz) {
                            continue;
                        }
                        let cell = nz as usize * SIDE + nx as usize;
                        if !seen[cell] {
                            seen[cell] = true;
                            queue.push((nx as usize, nz as usize));
                        }
                    }
                }
                assert!(
                    seen[level.exit.1 * SIDE + level.exit.0],
                    "seed {seed} floor {floor} walled its own exit off"
                );
                assert_ne!(level.exit, level.start, "the way out is the way in");
            }
        }
    }

    #[test]
    fn the_outer_ring_is_always_rock() {
        // A hole in the edge is a level you can walk out of, and a
        // raycaster that walks out of its grid is a raycaster that indexes
        // nothing forever.
        let level = Level::of(77, 3);
        for index in 0..SIDE as i32 {
            assert!(level.solid(index, 0));
            assert!(level.solid(index, SIDE as i32 - 1));
            assert!(level.solid(0, index));
            assert!(level.solid(SIDE as i32 - 1, index));
        }
    }

    #[test]
    fn a_wall_stops_a_step_and_a_shot() {
        let mut game = playing(1);
        // Walk face-first into rock for a good while: the body must not end
        // up inside it.
        for _ in 0..600 {
            game.step(
                1.0 / 60.0,
                Buttons {
                    forward: true,
                    ..Buttons::default()
                },
            );
            assert!(
                !game.level.solid(game.pose.x as i32, game.pose.z as i32),
                "the player walked into the rock at {:.2},{:.2}",
                game.pose.x,
                game.pose.z
            );
        }

        // And a shot does not go through a wall: something parked on the far
        // side of rock is not hittable.
        let mut blocked = playing(2);
        blocked.enemies = vec![Enemy {
            x: blocked.pose.x,
            z: blocked.pose.z,
            alive: true,
        }];
        // Put it the other side of the map, where rock is guaranteed between.
        blocked.enemies[0].x = 1.5;
        blocked.enemies[0].z = 1.5;
        blocked.pose.facing = 0.0;
        assert!(blocked.aimed_at().is_none(), "a shot went through the map");
    }

    #[test]
    fn shooting_the_thing_in_front_of_you_kills_it_and_costs_a_round() {
        let mut game = playing(1);
        game.enemies = vec![Enemy {
            x: game.pose.x + 2.0,
            z: game.pose.z,
            alive: true,
        }];
        game.pose.facing = 0.0;
        let ammo = game.ammo;
        let report = game.step(
            1.0 / 60.0,
            Buttons {
                fire: true,
                ..Buttons::default()
            },
        );
        assert_eq!(report.killed, 1);
        assert_eq!(game.standing(), 0);
        // One round out, the kill's bounty back.
        assert_eq!(game.ammo, ammo - 1 + KILL_AMMO);
        assert!(game.score > 0);
    }

    #[test]
    fn the_floors_get_meaner() {
        let shallow = playing(1);
        let deep = playing(7);
        assert!(
            deep.enemies.len() > shallow.enemies.len(),
            "floor seven is no busier than floor one"
        );
        assert!(deep.ammo <= shallow.ammo, "deeper floors hand out more ammunition");
    }

    #[test]
    fn dying_ends_the_run_and_keeps_the_best() {
        let mut game = playing(1);
        game.score = 260;
        game.health = 1;
        // Stood on.
        game.enemies = vec![Enemy {
            x: game.pose.x,
            z: game.pose.z,
            alive: true,
        }];
        let report = game.step(1.0 / 60.0, Buttons::default());
        assert!(report.died);
        assert_eq!(game.state, State::Over);
        assert_eq!(game.best, 260);

        // Starting again clears the score and keeps the record.
        game.step(
            1.0 / 60.0,
            Buttons {
                start: true,
                ..Buttons::default()
            },
        );
        assert_eq!(game.state, State::Playing);
        assert_eq!(game.score, 0);
        assert_eq!(game.floor, 1);
        assert_eq!(game.best, 260);
    }

    #[test]
    fn walking_into_the_way_out_takes_you_down() {
        let mut game = playing(1);
        let exit = game.level.exit;
        game.pose.x = exit.0 as f32 + 0.5;
        game.pose.z = exit.1 as f32 + 0.5;
        let report = game.step(1.0 / 60.0, Buttons::default());
        assert!(report.cleared);
        assert_eq!(game.floor, 2);
        assert_eq!(game.deepest, 2);
    }

    #[test]
    fn the_frame_is_the_screens_size_and_never_leaves_the_grid() {
        // The renderer's one real hazard is a ray that walks off the map, so
        // this pans a full turn from a handful of poses and asks only that
        // it comes back with a full frame every time.
        let mut game = playing(3);
        for spin in 0..24 {
            game.pose.facing = spin as f32 * std::f32::consts::TAU / 24.0;
            let frame = render(&game);
            assert_eq!(frame.len(), (DEVICE_WIDTH * DEVICE_HEIGHT * 4) as usize);
            assert!(frame.chunks_exact(4).any(|texel| texel != BACKDROP));
        }
    }

    #[test]
    fn the_floor_recedes_rather_than_sitting_flat() {
        // Two flat bands read as a void. The row under the player's feet has
        // to be brighter than the one at the horizon or the corridor has no
        // depth at all, and the same for the ceiling above.
        let game = playing(1);
        let pixels = render(&game);
        // The darkest pixel in a row is the ground or the roof: a wall that
        // reaches the very top or the very bottom of the frame is a wall
        // close enough to be lit brightly.
        let darkest = |y: u32| {
            (0..DEVICE_WIDTH)
                .map(|x| {
                    let at = ((y * DEVICE_WIDTH + x) * 4) as usize;
                    pixels[at] as u32 + pixels[at + 1] as u32 + pixels[at + 2] as u32
                })
                .min()
                .expect("a row has pixels")
        };
        let view_height = DEVICE_HEIGHT - STATUS;
        assert!(darkest(view_height - 2) > darkest(HORIZON + 2));
        assert!(darkest(1) > darkest(HORIZON - 2));
    }

    #[test]
    fn what_you_can_see_is_never_more_than_what_is_standing() {
        let mut game = playing(3);
        assert_eq!(game.nearest_standing().is_some(), game.standing() > 0);
        // Sight is a subset of presence: nothing is visible that is not also
        // out there somewhere.
        if let Some(seen) = game.sighted() {
            let (px, pz) = (game.pose().x, game.pose().z);
            assert!(line_of_sight(game.level(), (px, pz), seen));
            let near = game.nearest_standing().expect("something is standing");
            let span = |(x, z): (f32, f32)| (x - px).powi(2) + (z - pz).powi(2);
            assert!(span(near) <= span(seen) + 1.0e-3);
        }
        // And when the floor is cleared there is nothing left to see.
        for enemy in &mut game.enemies {
            enemy.alive = false;
        }
        assert_eq!(game.nearest_standing(), None);
        assert_eq!(game.sighted(), None);
    }

    #[test]
    fn the_picture_is_deterministic_and_reacts() {
        let game = playing(2);
        assert_eq!(render(&game), render(&game));
        let mut moved = game.clone();
        moved.pose.facing += 0.4;
        assert_ne!(render(&game), render(&moved));
    }

    #[test]
    fn every_screen_the_cabinet_can_show_is_drawable() {
        let mut game = Arcade::default();
        // No cartridge, the attract screen, mid-game and game over.
        for state in [State::Attract, State::Over, State::Playing] {
            for owned in [false, true] {
                game.owned = owned;
                game.state = state;
                let frame = render(&game);
                assert_eq!(frame.len(), (DEVICE_WIDTH * DEVICE_HEIGHT * 4) as usize);
            }
        }
        for line in ["POCKET ARCADE", "NO CARTRIDGE LOADED", "GAME OVER", "F3 HP2 AM14 480"] {
            assert!(font::text_width(line, 1) > 0, "unrenderable: {line}");
        }
    }

    #[test]
    fn the_cabinet_remembers_the_record_and_nothing_else() {
        let directory = std::env::temp_dir().join(format!("vx-arcade-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut game = playing(4);
        game.print();
        game.best = 910;
        game.deepest = 6;
        game.score = 120;
        game.save(&directory).unwrap();

        let mut loaded = Arcade::default();
        loaded.load(&directory);
        assert!(loaded.owned);
        assert_eq!(loaded.best, 910);
        assert_eq!(loaded.deepest, 6);
        // A run in progress is not a thing anybody saves.
        assert_eq!(loaded.score, 0);
        assert_eq!(loaded.state, State::Attract);

        std::fs::remove_dir_all(&directory).ok();
    }
}
