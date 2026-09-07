//! The arsenal: the compact slug launcher, and what firing it costs you.
//!
//! # A projectile steps on the journal clock
//!
//! A slug is spawned by a journalled `Fire` order and then advanced one step
//! per journal tick — the same clock the drones dig on — with a fixed step
//! length, so its whole flight is a pure function of the order. Hit detection
//! is `raycast_solid` over the segment the round swept that tick; no new
//! traversal, no per-frame physics, and a replayed journal carves the same
//! craters, which is what keeps the world hash honest after a firefight.
//!
//! # A slug does not ask permission
//!
//! Ballistic block damage goes through `World::set_block` directly (bedrock
//! and hardness guarded here) rather than through the cancellable break event.
//! The permits gate exists to *refuse* an edit before it happens; a fired slug
//! is past refusing. The consequence arrives as a bill instead: the caller
//! reads the [`Sweep`]s this module returns and charges bounty for whatever
//! was hit in view of witnesses.
//!
//! # What a slug can break
//!
//! [`SLUG_PUNCH`] sits between sheet metal and mast steel on the hardness
//! scale: a slug shatters planks, roofs, logs, sheet metal — and craters
//! soft rock, which sits at the bottom of the scale — but glances off ore
//! bodies, mast steel and every grade of lockbox. It is still no mining
//! tool: one block per slug at shop prices is the worst drill money can buy,
//! and the ore a mine exists for is exactly what it cannot touch.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;

use glam::DVec3;
use vx_core::BlockPos;
use vx_world::{PlayerBody, World};

use crate::movement::{self, Movement};
use crate::tuning::Tuning;

const MAGIC: &[u8; 4] = b"VXAR";
/// 2: a crash's column is `f64` on disk (stage 45). A version-1 file reads
/// its `f32` columns and widens them.
const VERSION: u32 = 2;
const OLDEST: u32 = 1;

/// Seconds per journal tick — the projectile integration step.
pub const TICK_SECONDS: f64 = 1.0 / crate::mining::TICK_RATE;

/// Muzzle speed, metres per second. Slow for a bullet on purpose: you can
/// watch the round go, and lead a moving caravan.
pub const SLUG_SPEED: f32 = 55.0;

/// Gravity on a slug, metres per second squared. Heavier than honest physics
/// so range is a skill rather than a given.
pub const SLUG_GRAVITY: f32 = 14.0;

/// The hardest block a slug breaks. Between sheet metal (1.6) and mast steel
/// (2.0): buildings are vulnerable; ore, masts and lockboxes are not.
pub const SLUG_PUNCH: f32 = 1.7;

/// Recoil speed handed to the shooter, metres per second, opposite the aim.
pub const SLUG_KICK: f32 = 4.5;

/// Journal ticks between shots.
pub const SLUG_RATE: f32 = 6.0;

/// Ticks a slug flies before it is spent. 32 ticks at 55 m/s is roughly 200 m
/// of reach, less what gravity takes.
pub const SLUG_TTL: u32 = 32;

/// Screen shake multiplier. Visual only — it never touches the simulation —
/// but it lives in the tuning table because "how scary" is a number a
/// designer drags.
pub const SHAKE_POWER: f32 = 1.0;

/// How far in front of the eye the round spawns, clear of the shooter's own
/// hull.
const MUZZLE_CLEARANCE: f32 = 0.4;

/// What the launcher costs at the shop counter.
pub const LAUNCHER_COST: u64 = 600;

/// What a box of slugs costs, and how many come in it.
pub const SLUG_COST: u64 = 40;
pub const SLUG_BATCH: u32 = 8;

/// How much the slung launcher weighs, in the same pack units the movement
/// load byte speaks. Folded into the load *before* the command is journalled,
/// so the weight replays without the oracle ever learning what a weapon is.
pub const LAUNCHER_HEFT: u64 = 15;

/// Base bounty for one block of somebody's property destroyed, before the
/// witness multiplier.
pub const BOUNTY_PROPERTY: u64 = 12;

/// Bounty for pointing the muzzle at a person in view of anyone — or of the
/// person themselves. Assault without a health bar.
pub const BOUNTY_MENACE: u64 = 10;

/// Bounty added when a panicked villager reaches the security office and
/// reports you. The office is its own witness.
pub const BOUNTY_REPORTED: u64 = 20;

/// One slug in flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub position: DVec3,
    pub velocity: DVec3,
    pub age: u32,
}

/// Where one shot went this tick, and what it met.
#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    pub from: DVec3,
    pub to: DVec3,
    pub hit: Option<Impact>,
}

/// The end of a flight.
#[derive(Debug, Clone, PartialEq)]
pub struct Impact {
    /// Where the round stopped.
    pub at: DVec3,
    /// The block it struck.
    pub block: BlockPos,
    /// Whether the block gave way. A glance off rock is still an impact —
    /// still loud, still a crime if the rock was somebody's wall's footing —
    /// but only a broken block owes property bounty.
    pub broke: bool,
}

/// Spawn a slug from a `Fire` order and stagger the shooter.
///
/// Called by live fire and by journal replay with the same arguments — the
/// recorded muzzle and the quantised aim — which is what makes a firefight
/// replay: the two sides run on different clocks, so the muzzle position is
/// part of the order rather than derived from the body.
pub fn launch(
    shots: &mut Vec<Shot>,
    movement: &mut Movement,
    muzzle: DVec3,
    yaw_q: i16,
    pitch_q: i16,
) {
    // The aim is a direction and stays `f32`; it is widened once to scale
    // the muzzle, which is a place.
    let aim = movement::aim_vector(yaw_q, pitch_q);
    let along = aim.as_dvec3();
    shots.push(Shot {
        position: muzzle + along * f64::from(MUZZLE_CLEARANCE),
        velocity: along * f64::from(movement.tuning.slug_speed),
        age: 0,
    });
    movement.kick(-aim * movement.tuning.slug_kick);
}

/// Where the muzzle sits for a body about to fire: the eye, exactly. The
/// muzzle crosses the journal's wire at this width.
pub fn muzzle_of(player: &PlayerBody) -> DVec3 {
    player.eye_position()
}

/// Step every shot one journal tick, breaking what they break.
///
/// Returns each shot's sweep so the live game can bill the damage and test
/// the segment against caravans and bystanders; replay drops the return value
/// — the world edits are the part the oracle checks.
pub fn advance_shots(shots: &mut Vec<Shot>, world: &mut World, tuning: &Tuning) -> Vec<Sweep> {
    let mut sweeps = Vec::with_capacity(shots.len());
    shots.retain_mut(|shot| {
        let from = shot.position;
        shot.velocity.y -= f64::from(tuning.slug_gravity) * TICK_SECONDS;
        let travel = shot.velocity * TICK_SECONDS;
        let length = travel.length();
        // The origin is exact; the direction is scale-free and the ray takes
        // it narrow, as `sight` does.
        let hit = if length > 1.0e-6 {
            vx_world::raycast_solid(
                &*world,
                world.registry(),
                from,
                (travel / length).as_vec3(),
                length as f32,
            )
        } else {
            None
        };

        let (alive, to, impact) = match hit {
            Some(found) => {
                let stopped = from + travel / length * f64::from(found.distance);
                let breakable = world
                    .registry()
                    .get(found.id)
                    .and_then(|def| def.hardness)
                    .is_some_and(|hardness| hardness <= tuning.slug_punch);
                // A slug bites rather than deletes. Which cell it bit comes
                // from where the round actually stopped, so a round that
                // clipped a corner chews the corner — and a block only goes
                // once it has been chewed past holding together. What a slug
                // may damage at all is unchanged: rock too hard to punch
                // still turns the round away without a mark.
                let breakable = if breakable {
                    // The fraction inside the cell, from the exact corner.
                    let cell = |along: f64, block: i32| {
                        (((along - f64::from(block)) * f64::from(vx_world::micro::SIDE)).floor()
                            as i32)
                            .clamp(0, vx_world::micro::SIDE - 1)
                    };
                    let face = vx_core::Face::ALL
                        .iter()
                        .position(|other| *other == found.face)
                        .unwrap_or(0);
                    let bite = vx_world::micro::Shape::SlugBite.cells(
                        cell(stopped.x, found.block.x),
                        cell(stopped.y, found.block.y),
                        cell(stopped.z, found.block.z),
                        face,
                    );
                    matches!(world.carve(found.block, bite), vx_world::Carved::Broke(_))
                } else {
                    false
                };
                (
                    false,
                    stopped,
                    Some(Impact {
                        at: stopped,
                        block: found.block,
                        broke: breakable,
                    }),
                )
            }
            None => {
                shot.position = from + travel;
                shot.age += 1;
                (shot.age < SLUG_TTL, from + travel, None)
            }
        };
        sweeps.push(Sweep {
            from,
            to,
            hit: impact,
        });
        alive
    });
    sweeps
}

/// A downed caravan's load, on the ground where it fell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crash {
    pub x: f64,
    pub z: f64,
    pub good: usize,
    pub amount: u64,
}

/// What the player owns and has done with it. Live state, saved beside the
/// wallet; the shots themselves are transient and the orders are the
/// journal's.
#[derive(Debug, Default)]
pub struct Arsenal {
    /// Whether the launcher has been bought.
    pub owned: bool,
    /// Slugs in the satchel.
    pub ammo: u32,
    /// Whether the launcher is in hand rather than the drill.
    pub equipped: bool,
    /// Journal ticks until the next shot is allowed.
    pub cooldown: f32,
    /// Towns that have already spent their one warning, by name.
    warned: BTreeSet<String>,
    /// Cargo on the ground where caravans came down.
    pub crashes: Vec<Crash>,
}

impl Arsenal {
    /// Whether a shot may be fired right now.
    pub fn ready(&self) -> bool {
        self.owned && self.equipped && self.ammo > 0 && self.cooldown <= 0.0
    }

    /// Spend one slug and start the cooldown.
    pub fn spend(&mut self, tuning: &Tuning) {
        self.ammo = self.ammo.saturating_sub(1);
        self.cooldown = tuning.slug_rate;
    }

    /// Run the cooldown down by some journal ticks.
    pub fn cool(&mut self, ticks: u32) {
        self.cooldown = (self.cooldown - ticks as f32).max(0.0);
    }

    /// Spend a town's one warning. True when this was it — the caller shows
    /// the warning; every later shot in that town is past warnings.
    pub fn warn_once(&mut self, town: &str) -> bool {
        self.warned.insert(town.to_string())
    }

    /// Write the arsenal beside the world save.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file =
            std::io::BufWriter::new(std::fs::File::create(directory.join("arsenal.dat"))?);
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&[u8::from(self.owned)])?;
        file.write_all(&self.ammo.to_le_bytes())?;
        file.write_all(&(self.warned.len() as u32).to_le_bytes())?;
        for town in &self.warned {
            let bytes = town.as_bytes();
            file.write_all(&(bytes.len() as u32).to_le_bytes())?;
            file.write_all(bytes)?;
        }
        file.write_all(&(self.crashes.len() as u32).to_le_bytes())?;
        for crash in &self.crashes {
            file.write_all(&crash.x.to_bits().to_le_bytes())?;
            file.write_all(&crash.z.to_bits().to_le_bytes())?;
            file.write_all(&(crash.good as u32).to_le_bytes())?;
            file.write_all(&crash.amount.to_le_bytes())?;
        }
        file.flush()
    }

    /// Load the arsenal, tolerating absence and damage.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("arsenal.dat");
        match read_arsenal(&path) {
            Ok(Some((owned, ammo, warned, crashes))) => {
                self.owned = owned;
                self.ammo = ammo;
                self.warned = warned;
                self.crashes = crashes;
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting fresh", path.display());
                *self = Arsenal::default();
            }
        }
    }
}

type ArsenalData = (bool, u32, BTreeSet<String>, Vec<Crash>);

fn read_arsenal(path: &Path) -> std::io::Result<Option<ArsenalData>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("bad magic"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    let version = u32::from_le_bytes(word);
    if !(OLDEST..=VERSION).contains(&version) {
        return Err(std::io::Error::other("unknown version"));
    }
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte)?;
    let owned = byte[0] != 0;
    file.read_exact(&mut word)?;
    let ammo = u32::from_le_bytes(word);

    file.read_exact(&mut word)?;
    let towns = u32::from_le_bytes(word);
    let mut warned = BTreeSet::new();
    for _ in 0..towns {
        file.read_exact(&mut word)?;
        let length = u32::from_le_bytes(word) as usize;
        if length > 256 {
            return Err(std::io::Error::other("town name implausibly long"));
        }
        let mut bytes = vec![0u8; length];
        file.read_exact(&mut bytes)?;
        let name = String::from_utf8(bytes)
            .map_err(|_| std::io::Error::other("town name not utf-8"))?;
        warned.insert(name);
    }

    file.read_exact(&mut word)?;
    let count = u32::from_le_bytes(word);
    let mut crashes = Vec::new();
    for _ in 0..count {
        let mut long = [0u8; 8];
        let mut column = |file: &mut std::io::BufReader<std::fs::File>| -> std::io::Result<f64> {
            if version < 2 {
                file.read_exact(&mut word)?;
                Ok(f64::from(f32::from_bits(u32::from_le_bytes(word))))
            } else {
                file.read_exact(&mut long)?;
                Ok(f64::from_bits(u64::from_le_bytes(long)))
            }
        };
        let x = column(&mut file)?;
        let z = column(&mut file)?;
        file.read_exact(&mut word)?;
        let good = u32::from_le_bytes(word) as usize;
        file.read_exact(&mut long)?;
        let amount = u64::from_le_bytes(long);
        crashes.push(Crash { x, z, good, amount });
    }

    Ok(Some((owned, ammo, warned, crashes)))
}

/// The bounty a piece of witnessed damage costs: the damage, plus half again
/// for every witness beyond the first. Zero witnesses is zero bounty — seen
/// is the rule, for gunfire the same as for lockpicks.
pub fn witnessed_bounty(damage: u64, witnesses: usize) -> u64 {
    if witnesses == 0 {
        return 0;
    }
    let scale = 1.0 + 0.5 * (witnesses as f64 - 1.0);
    (damage as f64 * scale).round() as u64
}

/// Whether a swept segment passes through an axis-aligned box.
///
/// The caravan test: the box is the machine's hull, the segment is where a
/// slug went this tick. Standard slab test, degenerate axes handled by the
/// containment check.
pub fn segment_hits_box(from: DVec3, to: DVec3, centre: DVec3, half: DVec3) -> bool {
    let direction = to - from;
    let mut enter = 0.0f64;
    let mut exit = 1.0f64;
    for axis in 0..3 {
        let start = from[axis] - centre[axis];
        let step = direction[axis];
        if step.abs() < 1.0e-6 {
            if start.abs() > half[axis] {
                return false;
            }
            continue;
        }
        let low = (-half[axis] - start) / step;
        let high = (half[axis] - start) / step;
        let (near, far) = if low < high { (low, high) } else { (high, low) };
        enter = enter.max(near);
        exit = exit.min(far);
        if enter > exit {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;

    fn open_sky() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 2);
        world
    }

    /// A shot fired level from high above the ground: it must fall, and its
    /// path must be identical every time it is refought.
    #[test]
    fn a_slug_drops_and_two_flights_match() {
        let mut world = open_sky();
        let tuning = Tuning::default();
        let fly = |world: &mut World| {
            let mut shots = vec![Shot {
                position: DVec3::new(0.5, 200.0, 0.5),
                velocity: DVec3::new(f64::from(tuning.slug_speed), 0.0, 0.0),
                age: 0,
            }];
            let mut path = Vec::new();
            for _ in 0..8 {
                advance_shots(&mut shots, world, &tuning);
                if let Some(shot) = shots.first() {
                    path.push(shot.position);
                }
            }
            path
        };
        let first = fly(&mut world);
        let second = fly(&mut world);
        assert_eq!(first, second, "a flight is not deterministic");
        let start = 200.0;
        let end = first.last().expect("shot died in open sky").y;
        assert!(end < start - 3.0, "no gravity: still at {end}");
    }

    #[test]
    fn a_slug_breaks_planks_and_glances_off_ore() {
        let mut world = open_sky();
        let tuning = Tuning::default();
        let plank = world.registry().id_of("engine:plank").expect("no plank");
        let ore = world
            .registry()
            .id_of("engine:copper_ore")
            .expect("no copper ore");
        let air = vx_core::BlockId::AIR;

        // A single slug into each, to see what one round does.
        for (target, expect_wound) in [(plank, true), (ore, false)] {
            let at = BlockPos::new(10, 180, 0);
            world.set_block(at, target);
            let impact =
                fire_one(&mut world, &tuning, 180.5, 0.5).expect("shot missed the wall");
            assert_eq!(impact.block, at);
            // Nothing dies to one round any more: a slug bites. What a slug
            // may damage at all is unchanged, so ore still turns it away
            // without a mark on it.
            assert!(!impact.broke, "one slug destroyed a whole block");
            assert_ne!(world.block(at), air, "one slug destroyed a whole block");
            assert_eq!(
                world.mask(at).is_some(),
                expect_wound,
                "the wrong thing was marked by a slug"
            );
            world.set_block(at, air);
        }

        // Firing twice down the *same* line drills a peephole and then
        // shoots through it. This is the whole promise of micro-on-damage
        // arriving through the arsenal by itself: nobody wrote "make a
        // hole", the cells simply stopped being there and the ray noticed.
        let at = BlockPos::new(10, 180, 0);
        world.set_block(at, plank);
        assert!(fire_one(&mut world, &tuning, 180.5, 0.5).is_some());
        let mut through = false;
        for _ in 0..8 {
            if fire_one(&mut world, &tuning, 180.5, 0.5).is_none() {
                through = true;
                break;
            }
        }
        assert!(through, "a line of fire never opened a hole it could shoot through");
        assert_ne!(world.block(at), air, "a peephole took the whole block");

        // And spread fire does finish it: violence converges, which is what
        // keeps the launcher a weapon rather than a chisel.
        world.set_block(at, air);
        world.set_block(at, plank);
        let mut rounds = 0;
        'demolition: for _ in 0..8 {
            for cell in 0..4 {
                for column in 0..4 {
                    let y = 180.125 + cell as f32 * 0.25;
                    let z = 0.125 + column as f32 * 0.25;
                    fire_one(&mut world, &tuning, y, z);
                    rounds += 1;
                    if world.block(at) == air {
                        break 'demolition;
                    }
                }
            }
        }
        assert_eq!(world.block(at), air, "a plank outlasted {rounds} slugs");
        assert!(rounds > 1, "the plank still died to a single round");
    }

    /// One slug down the +x line at the wall, returning what it struck.
    fn fire_one(world: &mut World, tuning: &Tuning, y: f32, z: f32) -> Option<Impact> {
        let mut shots = vec![Shot {
            position: DVec3::new(0.5, f64::from(y), f64::from(z)),
            velocity: DVec3::new(f64::from(tuning.slug_speed), 0.0, 0.0),
            age: 0,
        }];
        // A tick sweeps ~7 m; the wall at x = 10 takes two.
        for _ in 0..4 {
            let sweeps = advance_shots(&mut shots, world, tuning);
            if let Some(sweep) = sweeps.into_iter().find(|sweep| sweep.hit.is_some()) {
                return sweep.hit;
            }
        }
        None
    }

    #[test]
    fn a_slug_expires_in_open_sky() {
        let mut world = open_sky();
        // Aimed up, so gravity brings it back slowly enough to time out.
        let mut shots = vec![Shot {
            position: DVec3::new(0.5, 220.0, 0.5),
            velocity: DVec3::new(0.0, 30.0, 0.0),
            age: 0,
        }];
        for _ in 0..SLUG_TTL {
            advance_shots(&mut shots, &mut world, &Tuning::default());
        }
        assert!(shots.is_empty(), "a slug should be spent after {SLUG_TTL} ticks");
    }

    #[test]
    fn recoil_shoves_opposite_the_aim() {
        let mut shots = Vec::new();
        let mut movement = Movement::default();
        let mut body = PlayerBody::default();
        // Aim straight down -Z (yaw_q = 0, pitch_q = 0).
        launch(&mut shots, &mut movement, body.eye_position(), 0, 0);
        assert_eq!(shots.len(), 1);
        assert!(shots[0].velocity.z < 0.0, "yaw 0 must fire down -Z");

        // The queued kick lands on the body at the next tick.
        let world = open_sky();
        let idle = crate::movement::MoveCommand::default();
        movement.advance(&mut body, &world, idle, 1.0, crate::movement::MOVE_TICK);
        assert!(
            body.velocity.z > 0.0,
            "recoil should shove the shooter back, got {:?}",
            body.velocity
        );
    }

    #[test]
    fn the_witness_multiplier_matches_the_agreed_curve() {
        assert_eq!(witnessed_bounty(12, 0), 0);
        assert_eq!(witnessed_bounty(12, 1), 12);
        assert_eq!(witnessed_bounty(12, 2), 18);
        assert_eq!(witnessed_bounty(12, 3), 24);
        assert_eq!(witnessed_bounty(BOUNTY_MENACE, 1), BOUNTY_MENACE);
    }

    #[test]
    fn the_segment_box_test_hits_and_misses() {
        let centre = DVec3::new(10.0, 26.0, 0.0);
        let half = DVec3::new(2.0, 1.5, 2.0);
        assert!(segment_hits_box(
            DVec3::new(0.0, 26.0, 0.0),
            DVec3::new(20.0, 26.0, 0.0),
            centre,
            half
        ));
        assert!(!segment_hits_box(
            DVec3::new(0.0, 40.0, 0.0),
            DVec3::new(20.0, 40.0, 0.0),
            centre,
            half
        ));
        // Degenerate axis: a segment running inside the slab on x only.
        assert!(!segment_hits_box(
            DVec3::new(9.0, 0.0, 30.0),
            DVec3::new(11.0, 0.0, 30.0),
            centre,
            half
        ));
    }

    #[test]
    fn the_satchel_survives_the_trip_to_disk() {
        let directory = std::env::temp_dir().join(format!(
            "gamingg-arsenal-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");

        let mut kept = Arsenal {
            owned: true,
            ammo: 13,
            ..Arsenal::default()
        };
        assert!(kept.warn_once("DUSTPAN"));
        assert!(!kept.warn_once("DUSTPAN"), "one warning per town");
        kept.crashes.push(Crash {
            x: 120.5,
            z: -44.25,
            good: 2,
            amount: 90,
        });
        kept.save(&directory).expect("save");

        let mut back = Arsenal::default();
        back.load(&directory);
        assert!(back.owned);
        assert_eq!(back.ammo, 13);
        assert!(!back.warn_once("DUSTPAN"), "the warning ledger must persist");
        assert_eq!(back.crashes, kept.crashes);

        let _ = std::fs::remove_dir_all(&directory);
    }
    /// A version-1 `arsenal.dat` carried `f32` crash columns; it reads back
    /// widened, and a version-2 file round-trips a column no `f32` could
    /// hold.
    #[test]
    fn a_crash_column_survives_the_disk_at_both_widths() {
        let directory = std::env::temp_dir().join(format!("vx-arsenal-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        // Version 1, by hand: one crash at (12.25, -3.5) of good 2, amount 7.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&12.25f32.to_bits().to_le_bytes());
        bytes.extend_from_slice(&(-3.5f32).to_bits().to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&7u64.to_le_bytes());
        std::fs::write(directory.join("arsenal.dat"), bytes).unwrap();
        let mut old = Arsenal::default();
        old.load(&directory);
        assert!(old.owned);
        assert_eq!(old.ammo, 40);
        assert_eq!(
            old.crashes,
            vec![Crash { x: 12.25, z: -3.5, good: 2, amount: 7 }],
            "the old columns did not widen"
        );

        // Version 2: a column three thousand kilometres out, to the eighth.
        let mut far = Arsenal {
            owned: true,
            ..Arsenal::default()
        };
        far.crashes.push(Crash { x: 3_000_000.125, z: -3_000_000.875, good: 1, amount: 3 });
        far.save(&directory).unwrap();
        let mut read = Arsenal::default();
        read.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(read.crashes, far.crashes, "the far column did not survive the disk");
        assert_ne!(f64::from(3_000_000.125f32), 3_000_000.125, "the test proves nothing at f32");
    }

}
