//! What the kestrel sees: contact marks, and how they go stale.
//!
//! A mark is a *report*, not a tracking beacon. It holds the position a
//! contact was seen at, from the moment of sighting, and fades after
//! [`MARK_DECAY`] ticks unless re-sighted. Stale intelligence looking
//! different from fresh intelligence is the same honesty the market page
//! practices with prices — and it is what makes breaking line of sight
//! mean something.
//!
//! Marks are live-side intelligence, like the town books: journal replay
//! never sees them, because they never touch the ground the hash covers.

use glam::DVec3;
use vx_world::World;

/// Ticks a mark survives unsighted (30 s at the 8 Hz journal clock).
pub const MARK_DECAY: u64 = 240;

/// How close a fresh sighting must be to an old mark to refresh it rather
/// than spawn a second one.
const SAME_CONTACT: f32 = 2.5;

/// What kind of thing was seen. Deliberately binary — friend, crew,
/// villager and stranger read the same until factions land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Person,
    Machine,
}

/// One sighting: what, where, when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub position: DVec3,
    /// The journal tick it was (last) seen at.
    pub seen: u64,
}

impl Mark {
    /// Ticks since the sighting.
    pub fn age(&self, now: u64) -> u64 {
        now.saturating_sub(self.seen)
    }

    /// Whether the report is still worth showing.
    pub fn live(&self, now: u64) -> bool {
        self.age(now) < MARK_DECAY
    }
}

/// The scout's collected intelligence.
#[derive(Debug, Default)]
pub struct Marks {
    marks: Vec<Mark>,
}

impl Marks {
    /// One scan from an airborne eye: every contact within `radius` with an
    /// unbroken line of sight gets marked or refreshed. The raycast is the
    /// existing sight query, which is what makes a roof — or tree canopy —
    /// real cover from above without any new rule.
    pub fn scan(
        &mut self,
        world: &World,
        eye: DVec3,
        radius: f32,
        contacts: &[(MarkKind, DVec3)],
        now: u64,
    ) {
        for &(kind, at) in contacts {
            let target = at + DVec3::Y * 1.0;
            if (target - eye).length() > f64::from(radius) {
                continue;
            }
            if !vx_world::sight::sees(
                world,
                world.registry(),
                eye,
                target,
                radius + 2.0,
            ) {
                continue;
            }
            match self
                .marks
                .iter_mut()
                .find(|mark| mark.kind == kind && (mark.position - at).length() < f64::from(SAME_CONTACT))
            {
                Some(mark) => {
                    mark.position = at;
                    mark.seen = now;
                }
                None => self.marks.push(Mark {
                    kind,
                    position: at,
                    seen: now,
                }),
            }
        }
    }

    /// Forget everything past its decay.
    pub fn cull(&mut self, now: u64) {
        self.marks.retain(|mark| mark.live(now));
    }

    /// Every report still worth showing.
    pub fn live(&self, now: u64) -> impl Iterator<Item = &Mark> {
        self.marks.iter().filter(move |mark| mark.live(now))
    }

    /// Test convenience; the live game asks `live()` instead.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockPos, ChunkPos};

    fn open_world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 2);
        world
    }

    /// High-air fixture: eye and contacts well above any terrain, so only
    /// the rules under test decide.
    const SKY: f32 = 200.0;

    #[test]
    fn a_mark_decays_exactly_on_schedule() {
        let world = open_world();
        let mut marks = Marks::default();
        let eye = DVec3::new(0.5, f64::from(SKY) + 8.0, 0.5);
        let seen_at = 100;
        marks.scan(
            &world,
            eye,
            24.0,
            &[(MarkKind::Person, DVec3::new(4.5, f64::from(SKY), 4.5))],
            seen_at,
        );
        assert_eq!(marks.live(seen_at).count(), 1, "the contact was not marked");

        let last_tick = seen_at + MARK_DECAY - 1;
        assert_eq!(marks.live(last_tick).count(), 1, "faded a tick early");
        assert_eq!(
            marks.live(seen_at + MARK_DECAY).count(),
            0,
            "held past its decay"
        );

        marks.cull(seen_at + MARK_DECAY);
        assert!(marks.is_empty(), "cull left a stale report behind");
    }

    #[test]
    fn resighting_refreshes_instead_of_duplicating() {
        let world = open_world();
        let mut marks = Marks::default();
        let eye = DVec3::new(0.5, f64::from(SKY) + 8.0, 0.5);
        let walker = |t: f64| DVec3::new(4.5 + t, f64::from(SKY), 4.5);
        marks.scan(&world, eye, 24.0, &[(MarkKind::Person, walker(0.0))], 100);
        marks.scan(&world, eye, 24.0, &[(MarkKind::Person, walker(1.0))], 110);
        assert_eq!(
            marks.live(110).count(),
            1,
            "a moving contact left a trail of marks"
        );
        assert_eq!(marks.live(110).next().unwrap().seen, 110);
    }

    #[test]
    fn under_a_roof_is_unseen() {
        let mut world = open_world();
        let stone = world.registry().id_of("engine:stone").unwrap();
        // A slab of roof between the sky eye and the contact under it.
        for x in 2..8 {
            for z in 2..8 {
                world.set_block(BlockPos::new(x, SKY as i32 + 4, z), stone);
            }
        }
        let mut marks = Marks::default();
        let eye = DVec3::new(4.5, f64::from(SKY) + 8.0, 4.5);
        let sheltered = DVec3::new(4.5, f64::from(SKY), 4.5);
        let exposed = DVec3::new(20.5, f64::from(SKY), 4.5);
        marks.scan(
            &world,
            eye,
            24.0,
            &[
                (MarkKind::Person, sheltered),
                (MarkKind::Person, exposed),
            ],
            50,
        );
        let seen: Vec<DVec3> = marks.live(50).map(|mark| mark.position).collect();
        assert_eq!(seen, vec![exposed], "cover from above did not count: {seen:?}");
    }

    #[test]
    fn out_of_radius_is_out_of_the_report() {
        let world = open_world();
        let mut marks = Marks::default();
        let eye = DVec3::new(0.5, f64::from(SKY), 0.5);
        marks.scan(
            &world,
            eye,
            24.0,
            &[(MarkKind::Machine, DVec3::new(60.5, f64::from(SKY), 0.5))],
            10,
        );
        assert!(marks.is_empty(), "marked something beyond the scanner");
    }
}
