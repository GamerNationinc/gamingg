//! What an NPC can see, and what it remembers.
//!
//! Two ideas, deliberately separated. *Sight* is a world question and lives in
//! [`vx_world::sight`]; this module is the *policy* on top — who is worth
//! looking at, how long a sighting is remembered once it is out of view, and
//! how the cost of asking is kept flat as the cast grows.
//!
//! **Memory is the feature, not an optimisation.** An observer that forgets
//! the instant you break line of sight snaps its head away the moment you step
//! behind a tree, which looks broken. One that keeps watching where you *were*
//! for a few seconds reads as a person who saw something — and it is exactly
//! the shape a hostile will need later to hunt rather than to twitch.
//!
//! The cost control falls out of the same mechanism: only one observer
//! re-casts per update, and between its turns it faces the remembered
//! position. Three villagers today and thirty later cost the same per frame.

use glam::{DVec3, Vec3};
use vx_core::BlockRegistry;
use vx_world::{sight, World};

/// How far an observer can see at all.
pub const SIGHT_RANGE: f32 = 18.0;
/// How close something must be before an observer bothers reacting to it.
pub const NOTICE_RANGE: f32 = 12.0;
/// How long a sighting is remembered after it goes out of view.
pub const MEMORY_SECONDS: f32 = 6.0;
/// Observers that re-cast their line of sight per update.
pub const RECHECK_PER_UPDATE: usize = 1;

/// Eye heights, so sight lines leave and arrive at faces rather than feet —
/// standing on a kerb should not break eye contact.
pub const VILLAGER_EYE: f32 = 1.55;
/// The player's eye *standing*. A crouching or prone player carries their own
/// height instead — see [`Surroundings::player_eye`], which is the whole of the
/// stealth system.
pub const PLAYER_EYE: f32 = 1.62;
pub const MACHINE_EYE: f32 = 0.8;

/// Half the width of an observer's sight cone, as a cosine.
///
/// Sight used to be a full circle, which made hiding impossible to reason
/// about: there was no behind. Roughly sixty degrees either side of where
/// somebody is actually looking is generous enough that they are not blind and
/// tight enough that getting behind them means something.
pub const CONE_COS: f32 = 0.5;

/// Inside this, the cone stops mattering.
///
/// You do not have to be looking at somebody to know they are at your elbow,
/// and a cone without this exception lets a player stand nose-to-nose with a
/// villager unnoticed, which reads as a bug rather than as stealth.
pub const CLOSE_RANGE: f32 = 3.5;

/// What kind of thing was seen. Kept as a plain enum rather than a trait: the
/// three cases have nothing in common but a position, and a shared entity
/// abstraction should be driven by a feature that actually needs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Player,
    Villager,
    Digger,
    Flier,
}

impl TargetKind {
    /// Where this kind's eyes (or camera) sit above its ground point.
    pub fn eye_height(self) -> f32 {
        match self {
            TargetKind::Player => PLAYER_EYE,
            TargetKind::Villager => VILLAGER_EYE,
            TargetKind::Digger | TargetKind::Flier => MACHINE_EYE,
        }
    }
}

/// Something an observer noticed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sighting {
    pub kind: TargetKind,
    /// Index within its own kind, so a caller can find it again.
    pub index: usize,
    /// Where it was when it was seen.
    pub position: DVec3,
    pub distance: f32,
    /// Where this target's own eyes sit above `position`.
    ///
    /// Carried per sighting rather than taken from the kind, because the
    /// player's changes: standing 1.62, crouched 1.10, prone 0.35. That single
    /// number is what makes going flat behind a one-block wall work — the
    /// existing raycast simply misses.
    pub eye: f32,
}

impl Sighting {
    /// A sighting at its kind's default eye height.
    pub fn new(kind: TargetKind, index: usize, position: DVec3, distance: f32) -> Self {
        Sighting {
            kind,
            index,
            position,
            distance,
            eye: kind.eye_height(),
        }
    }
}

/// Everything an observer could notice this update, gathered once for the
/// whole cast rather than per observer.
///
/// `world: None` means "clear air": no terrain, so nothing blocks. That is
/// what the determinism tests and the headless captures run with, and it keeps
/// the reaction logic testable without loading chunks.
#[derive(Default)]
pub struct Surroundings<'a> {
    pub world: Option<&'a World>,
    pub player: Option<DVec3>,
    /// How high the player's eyes are right now — their stance decides it.
    /// Zero means "use the standing default", so a caller that does not care
    /// about stealth can leave it alone.
    pub player_eye: f32,
    /// Ground points of the machines about: diggers and fliers.
    pub machines: &'a [(TargetKind, DVec3)],
}

impl Surroundings<'_> {
    /// Nothing to see: an empty world with nobody in it.
    pub fn empty() -> Self {
        Surroundings::default()
    }

    /// The player's eye height, falling back to standing.
    pub fn eye(&self) -> f32 {
        if self.player_eye > 0.0 {
            self.player_eye
        } else {
            PLAYER_EYE
        }
    }
}

/// Is this target inside the observer's cone — or simply too close to miss?
pub fn in_cone(from: DVec3, facing: Option<Vec3>, target: DVec3, distance: f32) -> bool {
    let Some(facing) = facing else {
        return true;
    };
    if distance <= CLOSE_RANGE {
        return true;
    }
    // The difference is taken wide and narrowed: a cone is a direction test.
    let to = Vec3::new((target.x - from.x) as f32, 0.0, (target.z - from.z) as f32);
    let facing = Vec3::new(facing.x, 0.0, facing.z);
    if to.length_squared() < 1.0e-6 || facing.length_squared() < 1.0e-6 {
        return true;
    }
    to.normalize().dot(facing.normalize()) >= CONE_COS
}

/// One observer's senses.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Perception {
    /// Seen right now, as of this observer's last re-cast.
    pub visible: Option<Sighting>,
    /// The last thing seen, kept for a while after it goes out of view.
    pub last_seen: Option<Sighting>,
    /// Seconds since `last_seen` was refreshed.
    pub age: f32,
}

impl Perception {
    /// Re-cast: the nearest target with an unobstructed line becomes
    /// `visible`, and refreshes the memory.
    ///
    /// Pure in `(world, from, targets)` — no time, no randomness — which is
    /// what keeps the villagers' bit-identical determinism intact.
    /// `facing` is a level direction. `None` looks in every direction at
    /// once, which is what the headless tests and anything without a heading
    /// want.
    pub fn observe(
        &mut self,
        world: Option<&World>,
        registry: Option<&BlockRegistry>,
        from: DVec3,
        facing: Option<Vec3>,
        targets: &[Sighting],
        range: f32,
    ) {
        let mut best: Option<Sighting> = None;
        for candidate in targets {
            if candidate.distance > range {
                continue;
            }
            if !in_cone(from, facing, candidate.position, candidate.distance) {
                continue;
            }
            let clear = match (world, registry) {
                (Some(world), Some(registry)) => sight::sees(
                    world,
                    registry,
                    from,
                    candidate.position + DVec3::Y * f64::from(candidate.eye),
                    range,
                ),
                // No terrain to get in the way.
                _ => true,
            };
            if !clear {
                continue;
            }
            if best.is_none_or(|found| candidate.distance < found.distance) {
                best = Some(*candidate);
            }
        }

        self.visible = best;
        if let Some(seen) = best {
            self.last_seen = Some(seen);
            self.age = 0.0;
        }
    }

    /// Age the memory, dropping it once it is stale.
    pub fn forget(&mut self, dt: f32) {
        if self.last_seen.is_none() {
            return;
        }
        self.age += dt;
        if self.age > MEMORY_SECONDS {
            self.last_seen = None;
            self.visible = None;
            self.age = 0.0;
        }
    }

    /// What the observer is attending to: what it can see, or failing that
    /// where it last saw something.
    pub fn watching(&self) -> Option<Sighting> {
        self.visible.or(self.last_seen)
    }

    /// The player, if that is who is currently in view.
    pub fn sees_player(&self) -> Option<Sighting> {
        self.visible
            .filter(|seen| seen.kind == TargetKind::Player)
    }
}

/// Which observers re-cast on this update: a round robin, so the per-update
/// cost does not grow with the size of the cast.
pub fn due(tick: u64, observers: usize) -> impl Iterator<Item = usize> {
    let count = if observers == 0 {
        0
    } else {
        RECHECK_PER_UPDATE.min(observers)
    };
    (0..count).map(move |offset| {
        ((tick as usize).wrapping_mul(RECHECK_PER_UPDATE) + offset) % observers.max(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockPos, ChunkPos};

    fn sighting(kind: TargetKind, index: usize, position: DVec3, from: DVec3) -> Sighting {
        Sighting::new(kind, index, position, (position - from).length() as f32)
    }

    #[test]
    fn an_observer_sees_the_nearest_visible_target() {
        let eye = DVec3::new(0.0, 1.5, 0.0);
        let near = sighting(TargetKind::Villager, 1, DVec3::new(3.0, 0.0, 0.0), eye);
        let far = sighting(TargetKind::Player, 0, DVec3::new(9.0, 0.0, 0.0), eye);

        let mut perception = Perception::default();
        perception.observe(None, None, eye, None, &[far, near], SIGHT_RANGE);
        assert_eq!(perception.visible.unwrap().index, 1);
        assert_eq!(perception.visible.unwrap().kind, TargetKind::Villager);
    }

    #[test]
    fn range_is_honoured() {
        let eye = DVec3::ZERO;
        let distant = sighting(TargetKind::Player, 0, DVec3::new(500.0, 0.0, 0.0), eye);
        let mut perception = Perception::default();
        perception.observe(None, None, eye, None, &[distant], SIGHT_RANGE);
        assert!(perception.visible.is_none());
    }

    #[test]
    fn a_target_behind_a_wall_is_not_visible() {
        // Real terrain, with a slab dropped between the two.
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 1);
        let stone = world.registry().id_of("engine:stone").unwrap();
        let ground = world.surface_y(0, 0).unwrap();

        let eye = DVec3::new(0.5, ground as f64 + 1.5, 0.5);
        let target_at = DVec3::new(0.5, ground as f64, 6.5);
        let target = sighting(TargetKind::Player, 0, target_at, eye);

        let mut perception = Perception::default();
        perception.observe(Some(&world), Some(world.registry()), eye, None, &[target], SIGHT_RANGE);
        assert!(perception.visible.is_some(), "clear line was not seen");

        for dy in 0..4 {
            for dx in -2..=2 {
                world.set_block(BlockPos::new(dx, ground + dy, 3), stone);
            }
        }
        let mut blocked = Perception::default();
        blocked.observe(
            Some(&world),
            Some(world.registry()),
            eye,
            None,
            &[target],
            SIGHT_RANGE,
        );
        assert!(blocked.visible.is_none(), "saw straight through a wall");
    }

    #[test]
    fn memory_holds_the_last_seen_position_and_then_expires() {
        let eye = DVec3::ZERO;
        let target = sighting(TargetKind::Player, 0, DVec3::new(4.0, 0.0, 0.0), eye);
        let mut perception = Perception::default();
        perception.observe(None, None, eye, None, &[target], SIGHT_RANGE);

        // Out of sight: still remembered, and still worth facing.
        perception.observe(None, None, eye, None, &[], SIGHT_RANGE);
        assert!(perception.visible.is_none());
        assert_eq!(perception.last_seen.map(|seen| seen.position), Some(target.position));
        assert_eq!(perception.watching().map(|seen| seen.position), Some(target.position));

        perception.forget(MEMORY_SECONDS * 0.5);
        assert!(perception.watching().is_some(), "forgot far too quickly");

        perception.forget(MEMORY_SECONDS);
        assert!(perception.watching().is_none(), "never forgot");
    }

    #[test]
    fn sees_player_only_reports_a_player_in_view() {
        let eye = DVec3::ZERO;
        let villager = sighting(TargetKind::Villager, 2, DVec3::new(2.0, 0.0, 0.0), eye);
        let mut perception = Perception::default();
        perception.observe(None, None, eye, None, &[villager], SIGHT_RANGE);
        assert!(perception.sees_player().is_none());

        let player = sighting(TargetKind::Player, 0, DVec3::new(1.0, 0.0, 0.0), eye);
        perception.observe(None, None, eye, None, &[villager, player], SIGHT_RANGE);
        assert!(perception.sees_player().is_some());
    }

    #[test]
    fn the_recheck_schedule_covers_every_observer_in_a_full_round() {
        let observers = 5;
        let mut seen = vec![false; observers];
        for tick in 0..observers as u64 {
            for index in due(tick, observers) {
                seen[index] = true;
            }
        }
        assert!(seen.iter().all(|&hit| hit), "some observer never re-casts");
        // And it stays inside bounds with nobody about.
        assert_eq!(due(7, 0).count(), 0);
    }

    #[test]
    fn perception_is_a_pure_function_of_world_and_positions() {
        let eye = DVec3::new(0.0, 1.5, 0.0);
        let targets = [
            sighting(TargetKind::Player, 0, DVec3::new(5.0, 0.0, 0.0), eye),
            sighting(TargetKind::Villager, 1, DVec3::new(2.0, 0.0, 3.0), eye),
        ];
        let mut a = Perception::default();
        let mut b = Perception::default();
        a.observe(None, None, eye, None, &targets, SIGHT_RANGE);
        b.observe(None, None, eye, None, &targets, SIGHT_RANGE);
        assert_eq!(a, b);
    }
}
