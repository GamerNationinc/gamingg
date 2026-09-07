//! Putting a tree on the ground.
//!
//! The numbers here are forestry rather than invention. Directional felling
//! cuts a **face notch** into the side the tree is meant to fall toward —
//! traditionally about a third of the trunk's diameter, less in modern
//! practice — and leaves a **hinge** of roughly a tenth of the diameter on
//! the far side. The notch aims the tree; the hinge steers it down. When the
//! cut takes the holding wood below the hinge the stem can no longer carry
//! its own lean and over it goes. Cut a hard leaner too fast and the trunk
//! splits upward instead — a **barber chair** — and it goes where it is heavy
//! rather than where it was aimed.
//!
//! All of that lands on machinery this engine already has. A trunk block's
//! cross-section *is* [`vx_world::micro`]'s sixty-four-cell mask, so "a third
//! of the way through" is a popcount, and [`micro::Shape::Notch`] is the cut.
//!
//! **The fall is kinematic, and that is a determinism decision, not a
//! shortcut.** Float rigid-body integration diverges across runs and machines;
//! the replay oracle would not survive it. A trunk rotating about its hinge
//! under the honest pendulum equation is a pure function of the tree, the
//! direction and the tick — cheap, replayable, and still collidable every
//! tick against everything in its way. The module is shaped exactly like
//! [`crate::arsenal`]: the world edits happen *inside* [`advance_falls`], and
//! the sweeps it returns carry the live-only half — who got hit, how loud it
//! was — for the caller to spend or drop.

use glam::DVec3;
use vx_core::{BlockId, BlockPos};
use vx_world::flora::{self, Species, Tree, TreePart};
use vx_world::micro;
use vx_world::town::TownSite;
use vx_world::World;

/// The player clock's step. The same one the arsenal integrates on.
const TICK_SECONDS: f32 = 1.0 / 64.0;

/// How far through the cross-section the notch has to reach before the stem
/// is aimed rather than merely wounded. The face notch is 15–33% of trunk
/// diameter in practice; a third is the traditional figure.
pub const NOTCH_AT: f32 = 0.30;

/// How much of the hinge is still holding when it lets go.
///
/// The forestry number is a hinge about a tenth of the trunk's diameter. Four
/// cells across a block makes a tenth of the diameter *a quarter of one cell*,
/// which the mask cannot say. What it can say is how much of the far layer —
/// the holding wood — is still there, and the moment that layer is more than
/// a third gone is the same moment in the same order. The note's own advice
/// about impact energy applies here: compress the span, keep the ordering.
pub const HOLDING_AT: f32 = 0.65;

/// How far up the trunk still counts as the stump. Cut inside this band and
/// you are felling; cut above it and you are taking a block off a tree, which
/// is what the drill has always done.
pub const STUMP_BAND: i32 = 2;

/// Wood, in kilograms per cubic metre. Softwoods run lighter than this and
/// dense hardwoods heavier; one figure is enough to order a sapling against a
/// giant, which is the only thing the number is for.
const DENSITY: f32 = 700.0;

const GRAVITY: f32 = 9.81;

/// The tilt a stem starts with the moment the hinge gives. Without it the
/// pendulum has no torque and the tree stands there for ever.
const START_TILT: f32 = 0.06;

/// A fall is over when the trunk is down.
const FLAT: f32 = std::f32::consts::FRAC_PI_2;

/// How hard a block has to be to stop a falling trunk dead. Soil, planks and
/// foliage give way; rock and steel do not, and a tree hung up on an outcrop
/// is a real morning in the woods.
const UNYIELDING: f32 = 2.5;

/// Trunk radius by species, in metres. The whole point of the table is the
/// ratio between the ends of it.
pub fn radius(species: Species) -> f32 {
    match species {
        Species::BogSpruce => 0.15,
        Species::Spruce => 0.22,
        Species::Hardwood => 0.25,
        Species::Giant => 0.50,
        Species::Ancient => 0.70,
        Species::Krummholz => 0.10,
    }
}

/// What a trunk brings down with it, in joules.
///
/// A stem pivoting about its base drops its centre of mass by half its
/// height, so `E = m·g·h/2` with `m = π·r²·h·ρ`. A sapling comes out at a
/// couple of kilojoules and an old-growth giant at megajoules — a
/// thousandfold span whose *ordering* is the design, however the damage curve
/// compresses it.
pub fn energy(species: Species, height: i32) -> f32 {
    let h = height as f32;
    let r = radius(species);
    let mass = std::f32::consts::PI * r * r * h * DENSITY;
    mass * GRAVITY * h * 0.5
}

/// How much of the cross-section the cut has taken.
pub fn severed(mask: micro::Mask) -> f32 {
    1.0 - micro::remaining(mask) as f32 / micro::CELLS as f32
}

/// How much of the holding wood is left, as a fraction of the layer it
/// started as.
pub fn hinge(mask: micro::Mask, face: usize) -> f32 {
    micro::hinge_left(mask, face) as f32 / 16.0
}

/// Is this stem cut through? Past the notch, and down to the hinge.
///
/// Two conditions, and they measure different things — which is the whole
/// point of the pair. `severed` is how much of the whole section has gone:
/// below the notch depth the stem is wounded rather than aimed. `hinge` is
/// what is left on the far side, and it is what actually holds the tree up.
pub fn ready(mask: micro::Mask, face: usize) -> bool {
    severed(mask) >= NOTCH_AT && hinge(mask, face) <= HOLDING_AT
}

/// Which way a stem goes, and whether it went there on purpose.
///
/// The notch faces the way the tree falls, so a stem falls toward the side it
/// was cut from — toward whoever cut it, which is both the forestry and the
/// danger. Lean overrides a badly-cut stem: if the ground falls away hard
/// enough against the notch, the trunk splits and goes downhill instead.
pub fn aim(face: usize, lean: DVec3) -> (DVec3, bool) {
    let notch = face_normal(face);
    let downhill = DVec3::new(lean.x, 0.0, lean.z);
    let pull = downhill.length();
    if pull > 0.35 && downhill.normalize().dot(notch) < -0.25 {
        // A hard leaner, cut against its lean: barber chair. It goes where it
        // is heavy.
        return (downhill / pull, true);
    }
    // A gentle lean only nudges the aim.
    let aimed = notch + downhill * 0.5;
    let flat = DVec3::new(aimed.x, 0.0, aimed.z);
    if flat.length() < 1.0e-3 {
        (notch, false)
    } else {
        (flat.normalize(), false)
    }
}

/// The outward normal of a block face, by the index the raycast reports.
pub fn face_normal(face: usize) -> DVec3 {
    match face {
        0 => DVec3::NEG_X,
        1 => DVec3::X,
        2 => DVec3::NEG_Y,
        3 => DVec3::Y,
        4 => DVec3::NEG_Z,
        _ => DVec3::Z,
    }
}

/// Which way the ground falls away under a column, as a vector whose length
/// is the slope. A stem leans downhill and so does its fall.
pub fn lean_at(world: &World, x: i32, z: i32) -> DVec3 {
    let height = |dx: i32, dz: i32| {
        world
            .surface_y(x + dx, z + dz)
            .unwrap_or_else(|| world.generator().natural_height_at(x + dx, z + dz))
            as f64
    };
    let span = 8.0;
    DVec3::new(
        (height(4, 0) - height(-4, 0)) / span,
        0.0,
        (height(0, 4) - height(0, -4)) / span,
    ) * -1.0
}

/// A stem on its way down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Falling {
    /// The block the trunk stands on. The hinge is one above it.
    pub base: BlockPos,
    pub species: Species,
    pub height: i32,
    /// Level, and pointing the way it is going.
    pub direction: DVec3,
    /// Radians from upright.
    pub angle: f32,
    /// Radians per second, growing as it goes over.
    pub rate: f32,
    /// Was this one aimed, or did it split and go where it was heavy?
    pub barber_chair: bool,
    /// It came down because something else landed on it.
    pub chained: bool,
    /// How many other stems it has brought down so far. Counted across the
    /// whole fall rather than read off the last tick, so a neighbour taken
    /// early in the arc is still on the bill when the stem lands.
    pub took: u32,
}

impl Falling {
    /// Where the hinge is: the top of the stump.
    pub fn hinge_point(&self) -> DVec3 {
        DVec3::new(
            self.base.x as f64 + 0.5,
            self.base.y as f64 + 1.0,
            self.base.z as f64 + 0.5,
        )
    }

    /// Where the tip is at the angle it has reached.
    pub fn tip(&self) -> DVec3 {
        // The angle is an angle and stays `f32`; the reach is a place. Down
        // is level: `cos` of a single-precision right angle is not zero, and
        // a tip a millionth of a block under the hinge reads a block lower.
        let reach = f64::from(self.height);
        let (along, rise) = if self.angle >= FLAT {
            (1.0, 0.0)
        } else {
            (f64::from(self.angle.sin()), f64::from(self.angle.cos()))
        };
        self.hinge_point() + self.direction * (reach * along) + DVec3::Y * (reach * rise)
    }

    /// Is it down?
    pub fn down(&self) -> bool {
        self.angle >= FLAT
    }

    /// What it is carrying.
    pub fn energy(&self) -> f32 {
        energy(self.species, self.height)
    }
}

/// What one stem did in one tick, for the caller to spend.
///
/// The world edits are already done by the time this comes back. What is left
/// is the live-only half: who was standing there, and how loud it was.
#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    /// The trunk's centre line this tick, hinge first.
    pub from: DVec3,
    pub to: DVec3,
    pub species: Species,
    pub energy: f32,
    /// Set on the tick it finishes: where the trunk came to rest, how many
    /// log blocks it laid down, and whether it was hung up short.
    pub landing: Option<Landing>,
}

/// The end of a fall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landing {
    pub at: DVec3,
    pub logs: u32,
    /// Stopped by something it could not go through.
    pub hung_up: bool,
    pub chained: u32,
}

/// Start a stem falling, and take the standing tree out of the world.
///
/// The stump stays — cut, and with whatever the notch left of it — because a
/// stump is the mark of a tree somebody felled rather than one that was never
/// there.
pub fn start(world: &mut World, tree: &Tree, direction: DVec3, barber_chair: bool) -> Falling {
    clear_standing(world, tree);
    Falling {
        base: tree.base,
        species: tree.species,
        height: tree.height,
        direction: level(direction),
        angle: START_TILT,
        rate: 0.0,
        barber_chair,
        chained: false,
        took: 0,
    }
}

fn level(direction: DVec3) -> DVec3 {
    let flat = DVec3::new(direction.x, 0.0, direction.z);
    if flat.length() < 1.0e-3 {
        DVec3::X
    } else {
        flat.normalize()
    }
}

/// Take the standing tree's trunk and crown out of the world, keeping the
/// stump.
fn clear_standing(world: &mut World, tree: &Tree) {
    let reach = flora::CANOPY_REACH;
    let top = tree.base.y + tree.height + 4;
    for y in tree.base.y + 1..=top {
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                let x = tree.base.x + dx;
                let z = tree.base.z + dz;
                let Some(part) = flora::tree_part_at(tree, x, y, z) else {
                    continue;
                };
                // The stump is the one block that stays.
                if part == TreePart::Trunk && y == tree.base.y + 1 {
                    continue;
                }
                let pos = BlockPos::new(x, y, z);
                if world.block(pos) != BlockId::AIR {
                    world.set_block(pos, BlockId::AIR);
                }
            }
        }
    }
}

/// Step every stem on the ground's own clock.
///
/// Everything that changes the world happens in here — what the trunk smashes
/// through, what it knocks over, and the logs it leaves — so the live game
/// and a replay of it edit the same blocks in the same order.
/// The towns a felling lookup has to know about: a superset of the ones whose
/// plateau could reach the columns the tree's own lattice cell asks about.
pub const TOWN_REACH: i32 = 64;

pub fn advance_falls(falls: &mut Vec<Falling>, world: &mut World) -> Vec<Sweep> {
    let mut sweeps = Vec::with_capacity(falls.len());
    let mut started: Vec<Falling> = Vec::new();
    let mut finished = Vec::with_capacity(falls.len());

    for fall in falls.iter_mut() {
        let from = fall.hinge_point();
        // The honest pendulum: a rod pivoting about one end accelerates as
        // `3g·sinθ / 2L`, so a tall stem goes over slowly and a sapling
        // snaps down. One line, and it is the whole of the physics.
        let acceleration = 3.0 * GRAVITY * fall.angle.sin() / (2.0 * fall.height as f32);
        fall.rate += acceleration * TICK_SECONDS;
        fall.angle = (fall.angle + fall.rate * TICK_SECONDS).min(FLAT);
        let to = fall.tip();

        let cleared = clear_path(world, fall, from, to, &mut started);
        fall.took += cleared.chained;
        let done = fall.down() || cleared.blocked;
        let landing = done.then(|| {
            let logs = lay_logs(world, fall, cleared.reach as f32);
            Landing {
                at: to,
                logs,
                hung_up: cleared.blocked && !fall.down(),
                chained: fall.took,
            }
        });
        finished.push(done);

        sweeps.push(Sweep {
            from,
            to,
            species: fall.species,
            energy: fall.energy(),
            landing,
        });
    }

    // Retire the ones that are down or hung up, in one pass keyed by the
    // flags gathered above: two retains over a shrinking vector would read
    // the wrong sweep for the wrong stem.
    let mut index = 0;
    falls.retain(|_| {
        let keep = !finished[index];
        index += 1;
        keep
    });
    falls.extend(started);
    sweeps
}

struct Cleared {
    /// How far along the trunk the sweep got before something stopped it.
    reach: f64,
    blocked: bool,
    chained: u32,
}

/// Take out everything the trunk passed through this tick.
fn clear_path(
    world: &mut World,
    fall: &Falling,
    from: DVec3,
    to: DVec3,
    started: &mut Vec<Falling>,
) -> Cleared {
    let span = (to - from).length();
    let along = if span > 1.0e-4 {
        (to - from) / span
    } else {
        DVec3::Y
    };
    let steps = (span * 2.0).ceil() as i32;
    let mut chained = 0;
    for step in 1..=steps.max(1) {
        let distance = span * step as f64 / steps.max(1) as f64;
        let at = from + along * distance;
        let pos = BlockPos::new(
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        );
        let block = world.block(pos);
        if block == BlockId::AIR {
            continue;
        }
        // Its own stump is not in its way.
        if pos.x == fall.base.x && pos.z == fall.base.z && pos.y <= fall.base.y + 1 {
            continue;
        }
        let hardness = world.registry().get_or_air(block).hardness;
        let Some(hardness) = hardness else {
            // Unbreakable: bedrock, a shop counter. The stem hangs up on it.
            return Cleared {
                reach: distance,
                blocked: true,
                chained,
            };
        };
        // Another stem in the way comes down too, if this one is carrying
        // enough to push it over. The domino happens in here, on the same
        // tick, so a replay sees it the same way.
        // The towns are gathered here rather than passed in, so the live
        // game and a replay of it cannot disagree about which list was used.
        // A stump is the mark of a tree already down: hitting one topples
        // nothing, however the worldgen's tree list reads it.
        let sites = world.towns_near((pos.x, pos.z), TOWN_REACH);
        if let Some(neighbour) = standing_tree(world, pos, &sites).filter(|tree| is_standing(world, tree)) {
            if neighbour.base != fall.base && fall.energy() > energy(neighbour.species, 3) {
                started.push(Falling {
                    chained: true,
                    ..start(world, &neighbour, fall.direction, false)
                });
                chained += 1;
                continue;
            }
        }
        if hardness > UNYIELDING {
            return Cleared {
                reach: distance,
                blocked: true,
                chained,
            };
        }
        world.set_block(pos, BlockId::AIR);
    }
    Cleared {
        reach: span,
        blocked: false,
        chained,
    }
}

/// Lay the trunk down as log blocks along the line it fell.
///
/// They follow the ground rather than floating where the arc ended: a felled
/// stem lies on the country, and the country is not flat.
fn lay_logs(world: &mut World, fall: &Falling, reach: f32) -> u32 {
    let Some(timber) = timber_block(world, fall.species) else {
        return 0;
    };
    let length = (reach.floor() as i32).clamp(1, fall.height);
    let mut laid = 0;
    for step in 1..=length {
        let at = fall.hinge_point() + fall.direction * step as f64;
        let (x, z) = (at.x.floor() as i32, at.z.floor() as i32);
        let Some(ground) = world.surface_y(x, z) else {
            continue;
        };
        let pos = BlockPos::new(x, ground + 1, z);
        if world.block(pos) != BlockId::AIR {
            continue;
        }
        world.set_block(pos, timber);
        laid += 1;
    }
    laid
}

/// What a stem of this species lies down as.
fn timber_block(world: &World, species: Species) -> Option<BlockId> {
    let name = match species {
        Species::Giant | Species::Ancient => "engine:prime_timber",
        Species::Spruce => "engine:spruce_log",
        Species::BogSpruce => "engine:bog_log",
        _ => "engine:log",
    };
    world.registry().id_of(name)
}

/// The standing tree whose trunk covers this block, if there is one.
///
/// Worldgen is the authority: the forest is a pure function of the seed, so a
/// trunk block standing where the generator says a trunk stands belongs to
/// that tree. A stack of logs somebody placed is not a tree, and cannot be
/// felled — which is the honest answer rather than a limitation.
pub fn standing_tree(world: &World, pos: BlockPos, sites: &[TownSite]) -> Option<Tree> {
    let generator = world.generator();
    let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, sites);
    let natural_at = |x: i32, z: i32| generator.natural_height_at(x, z);
    let reach = flora::CANOPY_REACH;
    let trees = flora::trees_overlapping(
        world.seed(),
        (pos.x - reach, pos.z - reach),
        (pos.x + reach, pos.z + reach),
        &height_at,
        &natural_at,
        sites,
    );
    trees.into_iter().find(|tree| {
        flora::tree_part_at(tree, pos.x, pos.y, pos.z) == Some(TreePart::Trunk)
    })
}

/// Is the tree still up, or only its stump? The first trunk block above
/// the stump band is the tell: felling keeps the stump and takes the rest.
pub fn is_standing(world: &World, tree: &Tree) -> bool {
    world.block(BlockPos::new(tree.base.x, tree.base.y + STUMP_BAND + 1, tree.base.z)) != BlockId::AIR
}

/// Is this block low enough on its trunk to be a stump?
pub fn is_stump(tree: &Tree, pos: BlockPos) -> bool {
    pos.y > tree.base.y && pos.y <= tree.base.y + STUMP_BAND
}

#[cfg(test)]
mod tests {
    use super::*;

    use vx_core::ChunkPos;

    /// A patch of real generated country with a forest on it, well away from
    /// the home town — a town keeps its lawns mowed, and a test that wants a
    /// tree has to go where the trees are.
    const WOODS: (i32, i32) = (96, 96);

    fn woods() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(WOODS.0 / 16, WOODS.1 / 16), 3);
        world
    }

    fn sites_here(world: &World) -> Vec<TownSite> {
        world.towns_overlapping(
            (WOODS.0 - 80, WOODS.1 - 80),
            (WOODS.0 + 80, WOODS.1 + 80),
        )
    }

    /// The first standing tree in the loaded patch, and the trunk block to
    /// cut it at.
    fn a_tree(world: &World) -> (Tree, BlockPos) {
        let sites = sites_here(world);
        let generator = world.generator();
        let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, &sites);
        let natural_at = |x: i32, z: i32| generator.natural_height_at(x, z);
        let trees = flora::trees_overlapping(
            world.seed(),
            (WOODS.0 - 24, WOODS.1 - 24),
            (WOODS.0 + 24, WOODS.1 + 24),
            &height_at,
            &natural_at,
            &sites,
        );
        // Actually standing: the lattice hands back trees whose cells reach
        // the search box from outside it, and a tree in an unloaded chunk is
        // a tree nobody stamped.
        let tree = trees
            .into_iter()
            .find(|tree| {
                tree.height >= 5
                    && tree.species != Species::Krummholz
                    && (tree.base.x - WOODS.0).abs() <= 24
                    && (tree.base.z - WOODS.1).abs() <= 24
                    && world.block(BlockPos::new(tree.base.x, tree.base.y + 1, tree.base.z))
                        != BlockId::AIR
            })
            .expect("no standing tree in the loaded square");
        let stump = BlockPos::new(tree.base.x, tree.base.y + 1, tree.base.z);
        (tree, stump)
    }

    #[test]
    fn a_trunk_block_knows_which_tree_it_belongs_to() {
        let world = woods();
        let sites = sites_here(&world);
        let (tree, stump) = a_tree(&world);

        let found = standing_tree(&world, stump, &sites).expect("the trunk had no tree");
        assert_eq!(found, tree);
        assert!(is_stump(&found, stump), "the bottom of the trunk is not a stump");
        // High up the same trunk is a block on a tree, not a stump: cut there
        // and the drill does what it has always done.
        let high = BlockPos::new(tree.base.x, tree.base.y + tree.height, tree.base.z);
        assert!(!is_stump(&found, high));
        // And open ground belongs to nobody.
        assert_eq!(
            standing_tree(
                &world,
                BlockPos::new(tree.base.x + 6, tree.base.y + 3, tree.base.z + 6),
                &sites
            ),
            None
        );
    }

    #[test]
    fn felling_takes_the_tree_down_and_leaves_the_stump() {
        let mut world = woods();
        let (tree, stump) = a_tree(&world);
        let crown = BlockPos::new(tree.base.x, tree.base.y + tree.height, tree.base.z);
        assert_ne!(world.block(crown), BlockId::AIR, "the tree was not there");

        let mut falls = vec![start(&mut world, &tree, DVec3::X, false)];
        assert_ne!(world.block(stump), BlockId::AIR, "the stump went with it");
        assert_eq!(world.block(crown), BlockId::AIR, "the trunk is still standing");

        let mut landing = None;
        for _ in 0..64 * 8 {
            for sweep in advance_falls(&mut falls, &mut world) {
                if let Some(end) = sweep.landing {
                    landing = Some(end);
                }
            }
            if falls.is_empty() {
                break;
            }
        }
        let landing = landing.expect("the stem never came down");
        assert!(landing.logs > 0, "it landed and left nothing to pick up");
        assert!(falls.is_empty(), "the stem is still falling");
    }

    #[test]
    fn two_identical_fells_agree_block_for_block() {
        let fell = || {
            let mut world = woods();
            let (tree, _) = a_tree(&world);
            let mut falls = vec![start(&mut world, &tree, DVec3::X, false)];
            for _ in 0..64 * 8 {
                advance_falls(&mut falls, &mut world);
                if falls.is_empty() {
                    break;
                }
            }
            // Every block in the box the fall could have touched.
            let mut after = Vec::new();
            for y in tree.base.y - 2..=tree.base.y + tree.height + 4 {
                for x in tree.base.x - 20..=tree.base.x + 20 {
                    for z in tree.base.z - 20..=tree.base.z + 20 {
                        after.push(world.block(BlockPos::new(x, y, z)));
                    }
                }
            }
            after
        };
        assert_eq!(fell(), fell(), "the same tree fell two different ways");
    }

    #[test]
    fn a_stem_lays_its_logs_along_the_line_it_fell() {
        let mut world = woods();
        let (tree, _) = a_tree(&world);
        let timber = timber_block(&world, tree.species).expect("no timber for this species");

        let mut falls = vec![start(&mut world, &tree, DVec3::X, false)];
        for _ in 0..64 * 8 {
            advance_falls(&mut falls, &mut world);
            if falls.is_empty() {
                break;
            }
        }

        // Logs lie out along +X from the stump, on the ground, and nowhere
        // off to the sides.
        let mut found = 0;
        for step in 1..=tree.height {
            let x = tree.base.x + step;
            let z = tree.base.z;
            let Some(ground) = world.surface_y(x, z) else {
                continue;
            };
            for y in ground - 1..=ground + 2 {
                if world.block(BlockPos::new(x, y, z)) == timber {
                    found += 1;
                    break;
                }
            }
        }
        assert!(found >= 2, "only {found} logs along the fall line");
    }

    #[test]
    fn a_falling_stem_is_stopped_by_what_it_cannot_go_through() {
        let mut world = woods();
        let (tree, _) = a_tree(&world);
        // A wall of bedrock two blocks out, the whole height of the arc.
        let bedrock = world.registry().id_of("engine:bedrock").expect("no bedrock");
        for y in tree.base.y..=tree.base.y + tree.height + 2 {
            for dz in -2..=2 {
                world.set_block(BlockPos::new(tree.base.x + 2, y, tree.base.z + dz), bedrock);
            }
        }

        let mut falls = vec![start(&mut world, &tree, DVec3::X, false)];
        let mut hung = false;
        for _ in 0..64 * 8 {
            for sweep in advance_falls(&mut falls, &mut world) {
                if let Some(end) = sweep.landing {
                    hung |= end.hung_up;
                }
            }
            if falls.is_empty() {
                break;
            }
        }
        assert!(hung, "the stem went straight through a wall of bedrock");
        assert_eq!(
            world.block(BlockPos::new(tree.base.x + 2, tree.base.y + 2, tree.base.z)),
            bedrock,
            "the stem ate the bedrock"
        );
    }

    #[test]
    fn the_cut_fires_at_the_numbers_it_claims() {
        // An intact trunk is not going anywhere.
        assert!(!ready(micro::FULL, 0));
        assert_eq!(severed(micro::FULL), 0.0);
        assert_eq!(hinge(micro::FULL, 0), 1.0);

        // A notch on its own aims the tree but does not drop it: there is
        // still a hinge holding it.
        let notched = micro::carve(micro::FULL, micro::notch(0, 22));
        assert!(severed(notched) >= NOTCH_AT, "{}", severed(notched));
        assert!(!ready(notched, 0), "a notch alone put the tree over");

        // Cutting on until the holding wood is down to the corners does.
        let through = micro::carve(micro::FULL, micro::notch(0, 40));
        assert!(hinge(through, 0) <= HOLDING_AT, "{}", hinge(through, 0));
        assert!(ready(through, 0));

        // The two conditions are not the same condition wearing a hat: a cut
        // driven straight through the middle can spend the hinge before it
        // has taken a third of the section, and that stem is not aimed.
        let slot = micro::carve(micro::FULL, micro::notch(0, 12));
        assert!(severed(slot) < NOTCH_AT);
        assert!(!ready(slot, 0), "an unaimed slot dropped the tree");

        // And a cut that has barely started does neither.
        let scratch = micro::carve(micro::FULL, micro::notch(0, 4));
        assert!(severed(scratch) < NOTCH_AT);
        assert!(!ready(scratch, 0));
    }

    #[test]
    fn a_stem_falls_toward_the_side_it_was_cut_from() {
        // Cutting the +X face aims it +X: toward whoever is standing there.
        let (direction, chair) = aim(1, DVec3::ZERO);
        assert!(!chair);
        assert!(direction.x > 0.9, "{direction:?}");

        let (direction, _) = aim(4, DVec3::ZERO);
        assert!(direction.z < -0.9, "{direction:?}");

        // A gentle lean only nudges it.
        let (direction, chair) = aim(1, DVec3::new(0.0, 0.0, 0.2));
        assert!(!chair);
        assert!(direction.x > 0.8 && direction.z > 0.0, "{direction:?}");
    }

    #[test]
    fn a_hard_leaner_cut_against_its_lean_barber_chairs() {
        // The ground falls away hard to -X and the notch says +X. The tree
        // does not care what the notch says.
        let (direction, chair) = aim(1, DVec3::new(-0.9, 0.0, 0.0));
        assert!(chair, "the stem obeyed a notch it had no business obeying");
        assert!(direction.x < -0.9, "{direction:?}");
    }

    #[test]
    fn the_arc_is_the_same_arc_every_time() {
        let make = || Falling {
            base: BlockPos::new(4, 80, -7),
            species: Species::Hardwood,
            height: 7,
            direction: DVec3::X,
            angle: START_TILT,
            rate: 0.0,
            barber_chair: false,
            chained: false,
            took: 0,
        };
        let sweep = |mut fall: Falling| {
            let mut path = Vec::new();
            while !fall.down() {
                let acceleration =
                    3.0 * GRAVITY * fall.angle.sin() / (2.0 * fall.height as f32);
                fall.rate += acceleration * TICK_SECONDS;
                fall.angle = (fall.angle + fall.rate * TICK_SECONDS).min(FLAT);
                path.push(fall.tip());
            }
            path
        };
        let first = sweep(make());
        assert_eq!(first, sweep(make()), "two identical stems fell differently");
        assert!(!first.is_empty(), "the stem never went over");
        // It ends flat, a trunk's length out along the way it was aimed, and
        // level with its own hinge.
        let last = *first.last().unwrap();
        let hinge = make().hinge_point();
        assert!((last.y - hinge.y).abs() < 0.01, "{last:?}");
        assert!((last.x - hinge.x - 7.0).abs() < 0.01, "{last:?}");
    }

    #[test]
    fn a_taller_stem_takes_longer_to_go_over() {
        let ticks = |height: i32| {
            let mut fall = Falling {
                base: BlockPos::new(0, 80, 0),
                species: Species::Hardwood,
                height,
                direction: DVec3::X,
                angle: START_TILT,
                rate: 0.0,
                barber_chair: false,
                chained: false,
                took: 0,
            };
            let mut count = 0;
            while !fall.down() && count < 10_000 {
                let acceleration =
                    3.0 * GRAVITY * fall.angle.sin() / (2.0 * fall.height as f32);
                fall.rate += acceleration * TICK_SECONDS;
                fall.angle = (fall.angle + fall.rate * TICK_SECONDS).min(FLAT);
                count += 1;
            }
            count
        };
        assert!(ticks(14) > ticks(5), "a giant went over as fast as a sapling");
        // And nothing takes so long that the player walks away bored.
        assert!(ticks(14) < 64 * 6);
    }

    #[test]
    fn energy_runs_from_a_bonk_to_a_catastrophe() {
        let sapling = energy(Species::BogSpruce, 4);
        let mature = energy(Species::Hardwood, 15);
        let giant = energy(Species::Giant, 25);
        assert!(sapling < mature && mature < giant);
        // The note's own arithmetic, to an order of magnitude: kilojoules for
        // a young stem, hundreds of kilojoules for a mature one, megajoules
        // for old growth.
        assert!((1_000.0..10_000.0).contains(&sapling), "{sapling}");
        assert!((50_000.0..500_000.0).contains(&mature), "{mature}");
        assert!(giant > 1_000_000.0, "{giant}");
    }

    #[test]
    fn a_stem_takes_its_neighbour_with_it() {
        // Two trees close enough for one to reach the other. The lattice
        // jitters, so a pair like this exists in any decent patch of woods —
        // and finding one rather than planting one keeps the test honest
        // about what worldgen actually produces.
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(WOODS.0 / 16, WOODS.1 / 16), 5);
        let sites = sites_here(&world);
        let generator = world.generator();
        let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, &sites);
        let natural_at = |x: i32, z: i32| generator.natural_height_at(x, z);
        let trees: Vec<Tree> = flora::trees_overlapping(
            world.seed(),
            (WOODS.0 - 56, WOODS.1 - 56),
            (WOODS.0 + 56, WOODS.1 + 56),
            &height_at,
            &natural_at,
            &sites,
        )
        .into_iter()
        .filter(|tree| {
            tree.species != Species::Krummholz
                && world.block(BlockPos::new(tree.base.x, tree.base.y + 1, tree.base.z))
                    != BlockId::AIR
        })
        .collect();

        let pair = trees
            .iter()
            .flat_map(|tree| trees.iter().map(move |other| (tree, other)))
            .find(|(tree, other)| {
                if tree.base == other.base || tree.height < 6 {
                    return false;
                }
                let (dx, dz) = (
                    (other.base.x - tree.base.x) as f64,
                    (other.base.z - tree.base.z) as f64,
                );
                let span = (dx * dx + dz * dz).sqrt();
                // Close enough to reach, and standing at about the same
                // height so the arc passes through its trunk rather than
                // over it.
                span > 1.0
                    && span < tree.height as f64 - 1.5
                    && (other.base.y - tree.base.y).abs() <= 2
            });
        let Some((cutter, victim)) = pair else {
            panic!("no two trees within reach of each other in a 112-block square");
        };

        let direction = DVec3::new(
            (victim.base.x - cutter.base.x) as f64,
            0.0,
            (victim.base.z - cutter.base.z) as f64,
        )
        .normalize();
        let mut falls = vec![start(&mut world, cutter, direction, false)];
        let mut chained = 0;
        for _ in 0..64 * 10 {
            for sweep in advance_falls(&mut falls, &mut world) {
                if let Some(end) = sweep.landing {
                    chained += end.chained;
                }
            }
            if falls.is_empty() {
                break;
            }
        }
        assert!(chained > 0, "the stem went through its neighbour without touching it");
        // And the neighbour is no longer standing.
        let top = BlockPos::new(victim.base.x, victim.base.y + victim.height, victim.base.z);
        assert_eq!(world.block(top), BlockId::AIR, "the neighbour is still up");
    }
}
