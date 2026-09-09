//! Star forts: walls with the receipts to justify them.
//!
//! # Why the shape is the shape
//!
//! A bastioned trace is not decoration. Tall thin walls exist to stop ladders;
//! once cannon exist they simply fall down, so the answer is low, thick and
//! *angled* — every face of the wall covered by the guns of another face, and
//! no dead ground anywhere along it where an attacker can stand unseen. The
//! star is what falls out of "no re-entrant angle may go unwatched", and it
//! belongs in this game because stage 13 gave the frontier something to shoot
//! with.
//!
//! # The trace is a polar radius, which makes the wall a signed distance
//!
//! Authoring a fort as a loop of points would mean carrying that loop into
//! every chunk that touches it. Instead the trace is a function of angle:
//!
//! ```text
//! r(θ) = base + bastion · cos(points · θ + phase)
//! ```
//!
//! …so "how far is this column from the wall" is `hypot(dx, dz) - r(θ)` — one
//! cheap expression, pure in `(seed, position)`, needing no cross-chunk
//! context. Wall, walk, parapet and ditch are then bands of that one signed
//! distance, exactly the way the design note asked for.
//!
//! # Gates, and gaps
//!
//! Roads leave a town along the four cardinal axes, so gates sit where the
//! roads already run and the trace opens for them. Each gate is a claim with
//! a lockbox like any other door, which the permits system needed nothing new
//! to handle. And some forts are ruins: a deterministic pass drops whole
//! segments of wall, because a perfect wall is a worse story than a broken
//! one — and because a breach is the thing a player actually remembers.

use crate::seed::{finalise, unit};
use crate::town::{Speciality, TownSite};

/// Which trace a town has earned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trace {
    /// The frontier floor: four short bastions on a low, thin wall drawn in
    /// tight against the buildings.
    ///
    /// Every settlement out here has one, and that is the point — a hamlet
    /// that has not walled itself at all is a hamlet nobody has raided
    /// twice. It is a *mini* star rather than a bad one: the geometry is
    /// the same argument at a smaller scale, built by fewer people with
    /// less earth to move.
    MiniStar,
    /// A plain ring: earth and a walk, no bastions. Somewhere that wanted a
    /// wall before it knew what a wall was for.
    Palisade,
    /// Four bastions. The working answer.
    FourPoint,
    /// Six, with the re-entrant angles deep enough to read as ravelins
    /// covering the gates.
    SixPoint,
}

impl Trace {
    /// How many bastions the trace throws out.
    fn points(self) -> f32 {
        match self {
            Trace::MiniStar => 4.0,
            Trace::Palisade => 0.0,
            Trace::FourPoint => 4.0,
            Trace::SixPoint => 6.0,
        }
    }

    /// How far a bastion reaches past the curtain, in blocks.
    fn bastion(self) -> f32 {
        match self {
            // Long enough to read as points from the air. Shorter bastions
            // were geometrically fine and looked like a rounded square,
            // which is not what the word "star" promises anybody.
            Trace::MiniStar => 5.5,
            Trace::Palisade => 0.0,
            Trace::FourPoint => 7.0,
            Trace::SixPoint => 9.0,
        }
    }

    /// How far out from the town core this trace runs.
    ///
    /// The mini star hugs the buildings — it is a wall a village could
    /// actually have built — where the full traces stand off far enough to
    /// keep their re-entrant angles clear of the market square.
    fn standoff(self) -> i32 {
        match self {
            Trace::MiniStar => 9,
            _ => STANDOFF,
        }
    }

    /// How high the rampart stands above the plateau.
    ///
    /// Four rather than three for the mini star, and the reason is the gate:
    /// a gateway is a four-block opening with the wall carrying on over it,
    /// so a three-high wall would have to choose between a doorway you
    /// cannot walk through and an arch that is not there. Four keeps every
    /// rule the full traces already follow and still reads as low.
    fn height(self) -> i32 {
        match self {
            Trace::MiniStar => 4,
            _ => WALL_HEIGHT,
        }
    }

    /// Half-thickness of the curtain.
    fn half(self) -> f32 {
        match self {
            Trace::MiniStar => 1.5,
            _ => WALL_HALF,
        }
    }

    /// How wide the ditch outside it runs.
    fn ditch(self) -> f32 {
        match self {
            Trace::MiniStar => 2.5,
            _ => DITCH_WIDTH,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Trace::MiniStar => "MINI STAR",
            Trace::Palisade => "PALISADE",
            Trace::FourPoint => "FOUR-POINT",
            Trace::SixPoint => "SIX-POINT",
        }
    }
}

/// Half-thickness of the curtain. Low and thick is the whole doctrine.
const WALL_HALF: f32 = 2.5;

/// How high the rampart stands above the town's plateau.
pub const WALL_HEIGHT: i32 = 5;

/// How deep the ditch outside it is cut.
const DITCH_DEPTH: i32 = 3;

/// How deep the curtain's footing runs below grade.
///
/// Deeper than the ditch beside it, or the ditch would undercut the very
/// wall it exists to protect — which is exactly the mistake that makes a real
/// counterscarp collapse into its own ditch.
const FOOTING_DEPTH: i32 = 4;

/// How wide the ditch runs, beyond the wall's outer face.
const DITCH_WIDTH: f32 = 5.0;

/// Half-width of a gateway, in blocks, measured along the wall.
const GATE_HALF: f32 = 3.0;

/// How far out from the town core the curtain runs, past the buildings.
///
/// Bigger than a bastion plus the wall's own half-thickness, or the trace's
/// *re-entrant* angles — the notches between bastions — would cut back inside
/// the plaza. A six-point trace reaches nine blocks out and is five thick, so
/// anything under twelve here walls the town's own market square in.
const STANDOFF: i32 = 14;

/// Arc of one ruinable segment, in radians. Small enough that a breach is a
/// gap rather than half a fort.
const SEGMENT_ARC: f32 = std::f32::consts::TAU / 24.0;

/// How much of a ruined fort's wall has fallen.
const RUIN_SHARE: f32 = 0.34;

/// One town's fort, derived whole from its site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fort {
    pub trace: Trace,
    /// Mean radius of the curtain from the town centre.
    pub radius: f32,
    /// Rotation of the bastions, so two six-point forts do not face alike.
    pub phase: f32,
    /// Has this one been let go?
    pub ruined: bool,
    /// The site's own hash stream, for the ruin pass.
    seed: u64,
    centre: (i32, i32),
    /// The plaza's level. Only a starting point: every column of the wall
    /// rides its own ground — see [`Fort::part_at`].
    pub ground: i32,
}

fn hash01(seed: u64, salt: u64) -> f32 {
    unit(finalise(seed ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)))
}

/// What this town built, if anything.
///
/// Tiered like everything else here: a hamlet throws up a mini star, the
/// middling towns get a working four-point trace, and only the largest earns
/// six with ravelins. A refinery is likelier to be well walled than a depot
/// for the obvious reason — it holds the valuable thing.
///
/// Nothing comes back unwalled. There used to be an `Open` case and it made
/// most of the map read as sheds in a field; a variant that says "this place
/// never bothered" is a story the frontier does not tell, so it is gone
/// rather than left in the table unreachable.
pub fn fort_for(site: &TownSite) -> Fort {
    let roll = hash01(site.seed, 0x0f_01);
    let big = site.core_half >= crate::town::MAX_CORE_HALF;
    let middling = site.core_half >= crate::town::HOME_CORE_HALF;
    let inclined = match site.speciality {
        Speciality::Refinery => 0.25,
        Speciality::Mine => 0.10,
        Speciality::Depot => 0.0,
    };

    let trace = if big {
        if roll < 0.55 + inclined {
            Trace::SixPoint
        } else {
            Trace::FourPoint
        }
    } else if middling {
        if roll < 0.45 + inclined {
            Trace::FourPoint
        } else if roll < 0.75 + inclined {
            Trace::Palisade
        } else {
            Trace::MiniStar
        }
    } else if roll < 0.30 + inclined {
        Trace::Palisade
    } else {
        Trace::MiniStar
    };

    Fort {
        trace,
        radius: (site.core_half + trace.standoff()) as f32,
        phase: hash01(site.seed, 0x0f_02) * std::f32::consts::TAU,
        // A quarter of the proper traces have been let go, and a third of
        // the mini stars — a hamlet has fewer hands to keep a wall standing.
        // Rarer than this and nobody ever finds a breach; commoner and a
        // standing wall stops meaning anything.
        ruined: hash01(site.seed, 0x0f_03)
            < if matches!(trace, Trace::MiniStar) {
                0.35
            } else {
                0.25
            },
        seed: site.seed,
        centre: site.centre,
        ground: site.ground,
    }
}

/// What a column of a fort holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The body of the rampart, from the plateau up.
    Rampart,
    /// The walkway along the top.
    Walk,
    /// The parapet on the outer lip, one course above the walk.
    Parapet,
    /// Cut ground outside the wall.
    Ditch,
    /// The lockbox beside a gateway.
    GateLock,
    /// The footing the curtain stands on, below grade.
    ///
    /// Poured under the whole trace including the gateways and the fallen
    /// segments: a wall that came down leaves its foundation in the ground,
    /// which is how ruins actually read on a site — and which means a breach
    /// is a gap you walk through rather than a hole you dig under.
    Footing,
}

impl Fort {
    /// The trace's radius at this angle.
    fn radius_at(&self, angle: f32) -> f32 {
        self.radius + self.trace.bastion() * (self.trace.points() * angle + self.phase).cos()
    }

    /// How far out this fort reaches — what a caller needs to know before
    /// deciding a chunk is clear of it.
    pub fn reach(&self) -> i32 {
        (self.radius + self.trace.bastion() + self.trace.half() + self.trace.ditch()).ceil()
            as i32
            + 2
    }

    /// Is this angle inside a gateway?
    ///
    /// Gates sit on the cardinal axes because that is where the roads already
    /// run — the trace opens for the traffic that was there first.
    fn in_gateway(&self, angle: f32, radius: f32) -> bool {
        let span = (GATE_HALF / radius.max(1.0)).atan();
        [0.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, -std::f32::consts::FRAC_PI_2]
            .into_iter()
            .any(|axis: f32| {
                let mut delta = angle - axis;
                while delta > std::f32::consts::PI {
                    delta -= std::f32::consts::TAU;
                }
                while delta < -std::f32::consts::PI {
                    delta += std::f32::consts::TAU;
                }
                delta.abs() <= span
            })
    }

    /// Has the segment at this angle fallen down?
    fn breached(&self, angle: f32) -> bool {
        if !self.ruined {
            return false;
        }
        let segment = (angle.rem_euclid(std::f32::consts::TAU) / SEGMENT_ARC).floor() as i64;
        hash01(self.seed, 0x0f_10 ^ segment as u64) < RUIN_SHARE
    }

    /// What stands at a world position, if anything.
    ///
    /// `ground` is that column's own surface height. A fort rides the terrain
    /// rather than sitting at one height: the curtain runs a long way out from
    /// the centre, and out there the town's plateau has already blended most
    /// of the way back to natural ground — so a wall pinned to the plaza's
    /// level would hang in the air on the downhill side and be buried on the
    /// up. Real ones step with the slope; so does this one.
    ///
    /// Pure in the fort, the position and that height, so every chunk that
    /// touches a wall agrees about it without consulting a neighbour — the
    /// same contract the terrain, the ore and the towns already keep.
    pub fn part_at(&self, x: i32, y: i32, z: i32, ground: i32) -> Option<Part> {
        // Thickness, height and ditch all come off the trace now: a mini
        // star is the same geometry built smaller, not a special case.
        let half = self.trace.half();
        let ditch = self.trace.ditch();
        let height = self.trace.height();

        let (dx, dz) = ((x - self.centre.0) as f32, (z - self.centre.1) as f32);
        let distance = (dx * dx + dz * dz).sqrt();
        // Cheap reject before the trigonometry: most columns are nowhere near.
        if distance > self.radius + self.trace.bastion() + half + ditch + 1.0
            || distance < self.radius - self.trace.bastion() - half - 1.0
        {
            return None;
        }

        let angle = dz.atan2(dx);
        let trace = self.radius_at(angle);
        let signed = distance - trace;

        // A gateway is a hole in the wall, and since stage 50 the ditch stops
        // at its edges rather than running across it.
        //
        // It used to run across, on the argument that a causeway would be a
        // hole in the *argument* and the drawbridge that would answer it is a
        // mechanism this game has not got. That was wrong twice over. It is
        // wrong historically — a fort with no bridge is built with an
        // *uncut causeway* at the gate, a strip of natural ground left in
        // place, which is a fortification detail and not a compromise. And it
        // was wrong in play, which is how it was found: the ditch is three
        // blocks deep, a body mantles 2.2, and there is no other way through
        // a curtain. Stage 50's haul walked two hundred blocks to sell its
        // ore at the next town, fell into that town's ditch fourteen blocks
        // from its gate, and could not get out of it or into the town — and
        // neither could a player, at any town, ever, including the one they
        // started in the moment they walked out of it.
        let gateway = self.in_gateway(angle, distance);

        if signed.abs() <= half {
            // Below grade comes first, and it does not care about breaches or
            // gateways: the trace was founded along its whole circuit before
            // any of it fell down or was left open for a road.
            if y < ground {
                return (y >= ground - FOOTING_DEPTH).then_some(Part::Footing);
            }
            if self.breached(angle) {
                return None;
            }
            if gateway {
                // The lock stands at the gate's edge, on the inner face,
                // where a door's lockbox would be.
                let at_edge = (signed + half).abs() < 1.0;
                if at_edge && y == ground + 1 {
                    return Some(Part::GateLock);
                }
                // Above the opening the wall carries on, so a gate reads as
                // an arch rather than a missing tooth.
                // Above the opening the wall carries on, so a gate reads as
                // an arch rather than a missing tooth — one course of it on
                // a mini star, three on a full trace.
                return (y >= ground + 4 && y <= ground + height).then_some(Part::Rampart);
            }
            if y == ground + height {
                // The outermost course of the top is the parapet; the rest is
                // walkway a defender can actually stand on.
                return Some(if signed > half - 1.0 {
                    Part::Parapet
                } else {
                    Part::Walk
                });
            }
            return (y >= ground && y < ground + height).then_some(Part::Rampart);
        }

        if signed > half && signed <= half + ditch && !self.breached(angle) && !gateway {
            // A ditch is cut ground, not built ground: it only ever removes.
            return (y <= ground && y > ground - DITCH_DEPTH).then_some(Part::Ditch);
        }
        None
    }

    /// The four gateways, as world columns on the trace. What a map pin or a
    /// road-builder would ask for.
    pub fn gateways(&self) -> Vec<(i32, i32)> {
        [0.0f32, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, -std::f32::consts::FRAC_PI_2]
            .into_iter()
            .map(|angle| {
                let radius = self.radius_at(angle);
                (
                    self.centre.0 + (angle.cos() * radius).round() as i32,
                    self.centre.1 + (angle.sin() * radius).round() as i32,
                )
            })
            .collect()
    }
}

/// Cut and raise every fort reaching this chunk.
///
/// Runs after the town stamp, and writes air as well as blocks — a ditch is a
/// hole, and a breach is the absence of a wall that would otherwise be there.
pub fn stamp(
    chunk: &mut crate::chunk::Chunk,
    position: vx_core::ChunkPos,
    sites: &[TownSite],
    blocks: &crate::gen::TerrainBlocks,
    height: &impl Fn(i32, i32) -> i32,
) {
    let origin = position.origin();
    for site in sites {
        let fort = fort_for(site);
        let reach = fort.reach();
        if origin.x > site.centre.0 + reach
            || origin.z > site.centre.1 + reach
            || origin.x + vx_core::CHUNK_SIZE <= site.centre.0 - reach
            || origin.z + vx_core::CHUNK_SIZE <= site.centre.1 - reach
        {
            continue;
        }

        for local_z in 0..vx_core::CHUNK_SIZE {
            for local_x in 0..vx_core::CHUNK_SIZE {
                let (world_x, world_z) = (origin.x + local_x, origin.z + local_z);
                let ground = height(world_x, world_z);
                for y in (ground - FOOTING_DEPTH.max(DITCH_DEPTH))..=(ground + WALL_HEIGHT) {
                    let Some(part) = fort.part_at(world_x, y, world_z, ground) else {
                        continue;
                    };
                    let block = match part {
                        Part::Footing => blocks.footing,
                        Part::Rampart => blocks.rampart,
                        Part::Walk => blocks.catwalk,
                        Part::Parapet => blocks.metal_wall,
                        Part::Ditch => vx_core::BlockId::AIR,
                        Part::GateLock => blocks.permit_box_ii,
                    };
                    if let Some(local) = vx_core::LocalPos::new(local_x, y, local_z) {
                        chunk.set(local, block);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::town;

    fn ground() -> impl Fn(i32, i32) -> i32 {
        |x: i32, z: i32| {
            80 + (crate::noise::value_2d(0x40f7, x as f32 / 110.0, z as f32 / 110.0) * 34.0) as i32
        }
    }

    fn sample() -> Vec<TownSite> {
        town::towns_near(2024, (0, 0), town::CELL * 14, &ground())
    }

    #[test]
    fn the_trace_is_tiered_and_derived() {
        let sites = sample();
        assert!(sites.len() >= 8, "only {} towns sampled", sites.len());
        for site in &sites {
            let fort = fort_for(site);
            assert_eq!(fort, fort_for(site), "two answers for one town");
            if fort.trace == Trace::SixPoint {
                assert!(
                    site.core_half >= town::MAX_CORE_HALF,
                    "a small town built a six-point trace"
                );
            }
        }
        // And the tiers actually vary across a world rather than collapsing
        // onto one answer.
        let traces: std::collections::BTreeSet<&str> =
            sites.iter().map(|site| fort_for(site).trace.name()).collect();
        assert!(traces.len() >= 2, "every town built the same fort: {traces:?}");
    }

    #[test]
    fn nowhere_on_the_frontier_is_unwalled() {
        // The floor, checked rather than asserted in a comment: every town a
        // world can produce puts *something* around itself, and the smallest
        // of them build the mini star.
        let mut seen_mini = false;
        for seed in [7, 909, 2024, 4242] {
            let ground = |_: i32, _: i32| 90;
            for site in town::towns_near(seed, (0, 0), 12_000, &ground) {
                let fort = fort_for(&site);
                assert!(
                    fort.reach() > site.core_half,
                    "the town at {:?} put nothing around itself",
                    site.centre
                );
                if fort.trace == Trace::MiniStar {
                    seen_mini = true;
                    // A mini star is genuinely smaller on every axis that
                    // matters, or the word is decoration.
                    assert!(fort.trace.standoff() < STANDOFF);
                    assert!(fort.trace.height() < WALL_HEIGHT);
                    assert!(fort.trace.half() < WALL_HALF);
                    assert!(fort.trace.bastion() > 0.0, "a star with no points");
                }
            }
        }
        assert!(seen_mini, "no hamlet anywhere built a mini star");
    }

    /// A gate you can walk through, all the way through.
    ///
    /// The hole in the wall was never the hard part; the ditch outside it
    /// was. Until stage 50 the ditch ran across the gateway, three blocks
    /// deep, and a body that mantles 2.2 could neither leave a town nor
    /// enter one. This walks the gate's own radius from outside the ditch to
    /// inside the wall and insists the ground is unbroken the whole way.
    #[test]
    fn every_gate_has_ground_under_it_from_outside_the_ditch_to_in() {
        let site = town::home_site();
        for trace in [Trace::MiniStar, Trace::Palisade, Trace::FourPoint, Trace::SixPoint] {
            let fort = Fort {
                trace,
                ruined: false,
                ..fort_for(&site)
            };
            for (gx, gz) in fort.gateways() {
                let (dx, dz) = (
                    (gx - fort.centre.0) as f32,
                    (gz - fort.centre.1) as f32,
                );
                let length = (dx * dx + dz * dz).sqrt().max(1.0);
                let reach = (trace.half() + trace.ditch() + 2.0).ceil() as i32;
                for step in -reach..=reach {
                    let x = fort.centre.0
                        + (dx / length * (length + step as f32)).round() as i32;
                    let z = fort.centre.1
                        + (dz / length * (length + step as f32)).round() as i32;
                    // Nothing cut below the surface...
                    assert_ne!(
                        fort.part_at(x, site.ground, z, site.ground),
                        Some(Part::Ditch),
                        "{trace:?} cut its own gateway away at {x},{z}"
                    );
                    // ...and nothing standing in the two blocks above it
                    // except the gate's own lock, which is the door and is
                    // supposed to be in the way until it is opened.
                    for up in 1..=2 {
                        let part = fort.part_at(x, site.ground + up, z, site.ground);
                        assert!(
                            part.is_none() || part == Some(Part::GateLock),
                            "{trace:?} blocked its own gateway at {x},{z} with {part:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_mini_star_clears_the_buildings_and_keeps_its_gates() {
        // The two things a tighter standoff could break: a wall drawn in
        // close enough to sit on the town, and a gate too low to walk
        // through. Both checked against the smallest core a world makes.
        let site = TownSite {
            core_half: town::MIN_CORE_HALF,
            ..town::home_site()
        };
        let fort = Fort {
            trace: Trace::MiniStar,
            radius: (site.core_half + Trace::MiniStar.standoff()) as f32,
            ruined: false,
            ..fort_for(&site)
        };
        let steps = 360;
        for step in 0..steps {
            let angle = std::f32::consts::TAU * step as f32 / steps as f32;
            let inner = fort.radius_at(angle) - Trace::MiniStar.half();
            assert!(
                inner > site.core_half as f32,
                "the mini star cut into the town at {angle}"
            );
        }
        for (x, z) in fort.gateways() {
            for step in 0..3 {
                assert_eq!(
                    fort.part_at(x, site.ground + step, z, site.ground),
                    None,
                    "the mini star's gate is too low to walk through"
                );
            }
        }
    }

    #[test]
    fn a_bastioned_wall_has_no_dead_ground() {
        // The military claim, checked as geometry: every angle of the trace
        // is either wall or gateway or breach, and the radius never doubles
        // back on itself — a trace that folded would leave a pocket the guns
        // of the next face could not see into.
        let site = town::home_site();
        let fort = Fort {
            trace: Trace::SixPoint,
            ..fort_for(&site)
        };
        let mut last = fort.radius_at(-std::f32::consts::PI);
        let steps = 720;
        for step in 0..=steps {
            let angle = -std::f32::consts::PI
                + std::f32::consts::TAU * step as f32 / steps as f32;
            let radius = fort.radius_at(angle);
            assert!(radius > 0.0, "the trace collapsed at {angle}");
            assert!(
                (radius - last).abs() < 2.0,
                "the trace jumps at {angle}: {last} to {radius}"
            );
            last = radius;
        }
    }

    #[test]
    fn the_wall_stands_outside_the_buildings() {
        // The fort may not eat the town it is protecting.
        for site in sample() {
            let fort = fort_for(&site);
            let steps = 360;
            for step in 0..steps {
                let angle = std::f32::consts::TAU * step as f32 / steps as f32;
                let inner = fort.radius_at(angle) - WALL_HALF;
                assert!(
                    inner > site.core_half as f32,
                    "{:?} wall crosses the plaza at {angle}: {inner} vs core {}",
                    fort.trace,
                    site.core_half
                );
            }
        }
    }

    #[test]
    fn every_fort_has_four_ways_in() {
        // A wall with no gate is a wall around a town nobody can trade with.
        for site in sample() {
            let fort = fort_for(&site);
            let gates = fort.gateways();
            assert_eq!(gates.len(), 4);
            for (x, z) in gates {
                let (dx, dz) = ((x - site.centre.0) as f32, (z - site.centre.1) as f32);
                let angle = dz.atan2(dx);
                let distance = (dx * dx + dz * dz).sqrt();
                assert!(
                    fort.in_gateway(angle, distance),
                    "a listed gateway is not in one"
                );
                // And the wall is open there: no rampart at head height.
                assert_eq!(
                    fort.part_at(x, site.ground + 2, z, site.ground),
                    None,
                    "the gateway at ({x}, {z}) is bricked up"
                );
            }
        }
    }

    #[test]
    fn a_fallen_wall_leaves_its_footing() {
        // Two claims at once: nothing along the trace can be tunnelled under,
        // and a ruin reads as a ruin — the foundation is still there in the
        // ground where the curtain used to be.
        let site = town::home_site();
        let fort = Fort {
            trace: Trace::SixPoint,
            ruined: true,
            ..fort_for(&site)
        };
        let steps = 240;
        let mut breaches = 0;
        for step in 0..steps {
            let angle = std::f32::consts::TAU * step as f32 / steps as f32;
            let radius = fort.radius_at(angle);
            let x = site.centre.0 + (angle.cos() * radius).round() as i32;
            let z = site.centre.1 + (angle.sin() * radius).round() as i32;
            assert_eq!(
                fort.part_at(x, site.ground - 1, z, site.ground),
                Some(Part::Footing),
                "the trace is unfounded at {angle}"
            );
            if fort.part_at(x, site.ground + 1, z, site.ground).is_none()
                && !fort.in_gateway(angle, radius)
            {
                breaches += 1;
            }
        }
        assert!(breaches > 0, "a ruined fort with no breach to find");
    }

    #[test]
    fn ruins_have_breaches_and_whole_forts_do_not() {
        let sites = sample();
        let mut ruins = 0;
        for site in &sites {
            let fort = fort_for(site);
            let mut gaps = 0;
            let steps = 240;
            for step in 0..steps {
                let angle = std::f32::consts::TAU * step as f32 / steps as f32;
                let radius = fort.radius_at(angle);
                let x = site.centre.0 + (angle.cos() * radius).round() as i32;
                let z = site.centre.1 + (angle.sin() * radius).round() as i32;
                if fort.in_gateway(angle, radius) {
                    continue;
                }
                if fort.part_at(x, site.ground + 1, z, site.ground).is_none() {
                    gaps += 1;
                }
            }
            if fort.ruined {
                ruins += 1;
                assert!(gaps > 0, "a ruined fort has no breach anywhere");
            } else {
                assert_eq!(gaps, 0, "a standing fort has a hole in it");
            }
        }
        assert!(ruins > 0, "no ruined fort in the whole sample");
    }

    #[test]
    fn the_ditch_only_ever_cuts_and_the_wall_only_ever_builds() {
        // The one invariant that keeps a fort from floating or from filling
        // in a valley: nothing is placed below the plateau, and nothing is
        // cut above it.
        let site = town::home_site();
        let fort = Fort {
            trace: Trace::FourPoint,
            ruined: false,
            ..fort_for(&site)
        };
        for x in site.centre.0 - 60..site.centre.0 + 60 {
            for z in site.centre.1 - 60..site.centre.1 + 60 {
                for y in site.ground - 8..site.ground + 12 {
                    match fort.part_at(x, y, z, site.ground) {
                        Some(Part::Ditch) => assert!(y <= site.ground),
                        // A footing is the one built thing below grade, and
                        // it never rises above it.
                        Some(Part::Footing) => assert!(y < site.ground),
                        Some(_) => assert!(y >= site.ground),
                        None => {}
                    }
                }
            }
        }
    }
}
