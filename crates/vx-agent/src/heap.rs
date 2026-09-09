//! Where the spoil goes: a heap the crew stacks, rather than a hole it digs.
//!
//! # Why this is the mirror of [`crate::mine`], and why that matters
//!
//! `mine` turns a marked region into an ordered list of *removals*, ordered so
//! that a drone is never asked to stand on ground it has already taken away.
//! This turns a marked region into an ordered list of *placements*, ordered so
//! that a drone is never asked to build on air. Same problem, opposite sign,
//! and the two orderings are the two halves of one rule — see
//! [`crate::drone::PLACE_OFFSETS`], which is [`crate::drone::REACH_OFFSETS`]
//! read from the bottom.
//!
//! For fifty-five stages this crate could only ever remove blocks, and a
//! surprising amount rested on that. [`crate::flow`]'s "every route in is a
//! route out" holds because digging only ever *adds* routes; the argument that
//! a full drone can keep working until it finds somewhere to unload holds for
//! the same reason. Placement breaks both, so the safety has to move somewhere
//! else — and it moves **into the plan**:
//!
//! > Every cell in [`HeapPlan::cells`] rests either on the ground that was
//! > there before the heap started, or on a cell earlier in the same list.
//!
//! That is one sentence, it is arithmetic, it is tested
//! ([`a_plan_is_buildable_from_the_ground_up`]), and it is what makes the
//! whole thing safe without teaching the pathfinder anything new. A drone
//! working the list in order always has something under the cell it is filling
//! and always has somewhere to stand.
//!
//! # The three shapes
//!
//! The player named them: a square-base pyramid, a spiral tower, a straight
//! shaft. They are not decoration — they are three different answers to "how
//! tall can this footprint get", and the footprint is the only number the
//! player chooses.
//!
//! - [`HeapShape::Pyramid`] — a course per layer, each one ring narrower.
//!   Height falls out of the base: a 9x9 footprint is four courses. Stable by
//!   construction, and the only shape that is drivable all the way to the top
//!   without doing anything clever, because each course is a one-block step in
//!   from the one below and one block up — exactly [`crate::flow::STEP`].
//! - [`HeapShape::Spiral`] — a column with a helical ramp wound round it, so
//!   the thing reads as built rather than tipped, and so it can be walked up.
//!   The ramp rises one block per cell for the same reason.
//! - [`HeapShape::Shaft`] — a plain prism, the full footprint all the way up.
//!   `mine`'s module note says a vertical shaft is *"the cheapest volume of the
//!   lot and nothing that drives can climb out of one"*, and that is exactly
//!   why it is the constrained one here: a drone builds a shaft by standing
//!   beside it and reaching up, so its height is capped by how far a machine
//!   can reach above where it can stand. See [`SHAFT_COURSES`].
//!
//! Every shape is pure in `(footprint, ground)` — one world read for the floor
//! and nothing else — so they test as arithmetic rather than as simulation.

use vx_core::{BlockPos, CHUNK_HEIGHT};
use vx_world::World;

use crate::aabb::VoxelAabb;

/// What to stack the spoil into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeapShape {
    /// Square-base pyramid: a course per layer, each a ring narrower.
    Pyramid,
    /// A column with a helical ramp, so it can be climbed and so it reads as
    /// something that was built.
    Spiral,
    /// A plain prism, the full footprint all the way up.
    Shaft,
}

impl HeapShape {
    pub const ALL: [HeapShape; 3] = [HeapShape::Pyramid, HeapShape::Spiral, HeapShape::Shaft];

    pub fn name(self) -> &'static str {
        match self {
            HeapShape::Pyramid => "pyramid",
            HeapShape::Spiral => "spiral tower",
            HeapShape::Shaft => "shaft",
        }
    }
}

/// How many courses a shaft is allowed, over the ground it stands on.
///
/// A drone builds a shaft from beside it, reaching up — [`crate::drone::PLACE_OFFSETS`]
/// reaches one block above the drone's own level, and a drone standing on the
/// original ground can therefore fill two courses. Past that it would have to
/// stand *on* the shaft, and a prism has nothing to step up onto: that is the
/// whole reason `mine` refuses to plan a vertical shaft as an excavation.
///
/// So a shaft heap is short and wide rather than tall and thin, and that is
/// honest rather than a limitation worked around. A player who wants height
/// picks the pyramid or the spiral, which are the shapes that carry their own
/// staircase.
pub const SHAFT_COURSES: i32 = 2;

/// The tallest a heap may get, whatever its shape.
///
/// Big enough for a landmark you can see from the next ridge, small enough
/// that a runaway footprint cannot ask the crew for a mountain.
pub const MAX_COURSES: i32 = 24;

/// Whether a good is spoil — waste rock, worth stacking rather than selling.
///
/// The mirror of [`crate::prospect::is_ore`], and deliberately the same one
/// rule read the other way: ore is anything whose name ends `_ore`, so spoil
/// is everything else that came out of a hole. One predicate, not two tables
/// that can disagree — and it means a block added by a later stage is spoil by
/// default, which is the safe way round: the worst case is that the crew
/// stacks something you would rather have sold, not that they sell something
/// into a heap.
pub fn is_spoil(name: &str) -> bool {
    !name.ends_with("_ore")
}

/// What a heap will be, cell by cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeapPlan {
    pub shape: HeapShape,
    /// The footprint the player marked, flattened onto the ground.
    pub footprint: VoxelAabb,
    /// Every cell to fill, **strictly bottom-up**. This ordering is the safety
    /// property the module note describes: work it in order and no block is
    /// ever placed on air.
    pub cells: Vec<BlockPos>,
    /// How many blocks of spoil the heap wants. What the player is shown when
    /// choosing between shapes, exactly as [`crate::mine::MinePlan::volume`] is.
    pub volume: u64,
}

impl HeapPlan {
    /// How many courses tall it finishes.
    pub fn courses(&self) -> i32 {
        self.cells
            .last()
            .zip(self.cells.first())
            .map_or(0, |(top, floor)| top.y - floor.y + 1)
    }

    /// Everything it will occupy, for pinning the ground while it is built.
    pub fn span(&self) -> VoxelAabb {
        VoxelAabb::containing(self.cells.iter().copied()).unwrap_or(self.footprint)
    }
}

/// Plan a heap on the marked footprint, or `None` if the ground will not take
/// that shape.
///
/// `None` rather than a shrunken plan, and for the same reason
/// [`crate::mine::plan`] returns `None` for a method that will not fit: a
/// player choosing between three shapes wants to be told which ones the ground
/// can actually carry, not handed a quiet substitute.
///
/// The one world read is the floor: [`crate::flow::settle`] would drop a single
/// cell, but a heap needs one height for the whole base or it is built on a
/// slope and half of it hangs. The highest ground under the footprint is the
/// answer — build up from the top of the slope and the low side fills in.
pub fn plan(world: &World, footprint: VoxelAabb, shape: HeapShape) -> Option<HeapPlan> {
    let footprint = footprint.clamped_to_world();
    let [width, _, depth] = footprint.size();
    if width < 1 || depth < 1 {
        return None;
    }

    let courses = match shape {
        // A pyramid is as tall as its narrowest side allows: each course is a
        // ring in from the one below, so a side of `n` gives `(n + 1) / 2`.
        HeapShape::Pyramid => ((width.min(depth) as i32) + 1) / 2,
        // A spiral needs a cell to wind round, so a 1-wide footprint is not a
        // spiral, it is a shaft with extra steps. As tall as its ramp is long:
        // the flight rises one block a cell, so a wider base buys a taller
        // tower and the helix closes on itself.
        HeapShape::Spiral => {
            if width < 3 || depth < 3 {
                return None;
            }
            (2 * (width + depth) as i32 - 4).min(MAX_COURSES)
        }
        HeapShape::Shaft => SHAFT_COURSES,
    }
    .clamp(0, MAX_COURSES);
    if courses < 1 {
        return None;
    }

    // **Every column starts at its own ground.**
    //
    // The first version took the *highest* ground under the whole footprint
    // and laid a flat first course at that height, on the theory that the low
    // side would fill in from above. It does not: the cells on the high side
    // are then inside the hill, and the ones on the low side are not in the
    // plan at all — so a heap ordered on any slope came out as a handful of
    // cells hanging mid-air with nowhere to stand and build them from. Found
    // by the played test, which is the only place real ground turns up.
    //
    // A tipped heap follows the ground it is tipped onto, so each column runs
    // from its own surface up to whatever height the shape wants there.
    let mut cells = Vec::new();
    for x in footprint.min.x..=footprint.max.x {
        for z in footprint.min.z..=footprint.max.z {
            let Some(ground) = world.surface_y(x, z) else {
                continue;
            };
            let height = height_at(shape, footprint, courses, x, z);
            for step in 0..height {
                let y = ground + step;
                if y >= CHUNK_HEIGHT {
                    break;
                }
                cells.push(BlockPos::new(x, y, z));
            }
        }
    }
    if cells.is_empty() {
        return None;
    }
    // Bottom-up globally, stable within a course. Each column is contiguous
    // from its own ground, so sorting by height keeps the invariant: every
    // cell rests on ground or on a cell earlier in the list.
    cells.sort_by_key(|cell| (cell.y, cell.x, cell.z));

    Some(HeapPlan {
        shape,
        footprint,
        volume: cells.len() as u64,
        cells,
    })
}

/// Every shape the marked ground will take, cheapest first.
///
/// The twin of [`crate::mine::options`], and the same contract: a list the
/// player cycles with the same key they already use to cycle mining methods.
pub fn options(world: &World, footprint: VoxelAabb) -> Vec<HeapPlan> {
    let mut plans: Vec<HeapPlan> = HeapShape::ALL
        .into_iter()
        .filter_map(|shape| plan(world, footprint, shape))
        .collect();
    plans.sort_by_key(|plan| plan.volume);
    plans
}

/// How many courses the shape wants over the column at `(x, z)`.
///
/// The whole geometry of the three shapes, in one function of position — the
/// `*_part_at` idiom this codebase uses for every other solid it generates,
/// from bunker rooms to tree crowns.
fn height_at(shape: HeapShape, footprint: VoxelAabb, courses: i32, x: i32, z: i32) -> i32 {
    match shape {
        // Distance to the nearest edge: one course at the rim, `courses` in
        // the middle, one ring in per step. That is a square-base pyramid.
        HeapShape::Pyramid => {
            let inset = (x - footprint.min.x)
                .min(footprint.max.x - x)
                .min(z - footprint.min.z)
                .min(footprint.max.z - z);
            (inset + 1).min(courses)
        }
        HeapShape::Shaft => courses,
        // The core is the tower, full height. The ring is the ramp: one block
        // taller per cell as you walk round, then full height once the flight
        // has topped out — see the module note.
        HeapShape::Spiral => {
            let on_ring = x == footprint.min.x
                || x == footprint.max.x
                || z == footprint.min.z
                || z == footprint.max.z;
            if !on_ring {
                return courses;
            }
            let ring = ring_walk(footprint);
            let index = ring
                .iter()
                .position(|(rx, rz)| *rx == x && *rz == z)
                .unwrap_or(0) as i32;
            (index + 1).min(courses)
        }
    }
}

/// The outer ring of a footprint, walked once round so that consecutive
/// entries are neighbours.
fn ring_walk(area: VoxelAabb) -> Vec<(i32, i32)> {
    let mut ring = Vec::new();
    for x in area.min.x..=area.max.x {
        ring.push((x, area.min.z));
    }
    for z in (area.min.z + 1)..=area.max.z {
        ring.push((area.max.x, z));
    }
    for x in (area.min.x..area.max.x).rev() {
        ring.push((x, area.max.z));
    }
    for z in ((area.min.z + 1)..area.max.z).rev() {
        ring.push((area.min.x, z));
    }
    ring
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;

    fn footprint(min: (i32, i32), max: (i32, i32)) -> VoxelAabb {
        VoxelAabb::new(
            BlockPos::new(min.0, 0, min.1),
            BlockPos::new(max.0, 0, max.1),
        )
    }

    /// **The claim the whole round rests on.**
    ///
    /// For fifty-five stages this crate could only take blocks away, and a
    /// surprising amount of the machinery quietly assumed it. `flow`'s "every
    /// route in is a route out" holds because digging only ever *adds* routes.
    /// Placement breaks that, so the safety has to live somewhere, and it lives
    /// here: work the list in order and every cell you fill has something
    /// solid under it — either ground that was already there, or a cell you
    /// filled earlier.
    ///
    /// It is one sentence and it is arithmetic. No world, no drones, no ticks.
    #[test]
    fn a_plan_is_buildable_from_the_ground_up() {
        // Flat *and* sloping: the slope is where the first version broke, and
        // a rule proved only on a table is not proved.
        for world in [fixture::flat(40, 64), fixture::slope(40, 70, 0, 4)] {
        for shape in HeapShape::ALL {
            for (min, max) in [((-4, -4), (4, 4)), ((0, 0), (6, 8)), ((2, 2), (10, 10))] {
                let Some(plan) = plan(&world, footprint(min, max), shape) else {
                    continue;
                };
                let mut filled: std::collections::HashSet<BlockPos> =
                    std::collections::HashSet::new();
                for cell in &plan.cells {
                    let under = cell.offset([0, -1, 0]);
                    let on_ground = world.is_solid(under);
                    assert!(
                        on_ground || filled.contains(&under),
                        "{} would put a block at {cell:?} on air: {under:?} is not \
                         ground and has not been filled yet",
                        shape.name()
                    );
                    filled.insert(*cell);
                }
                // And bottom-up overall, which is what makes the check above
                // meaningful rather than accidental.
                assert!(
                    plan.cells.windows(2).all(|pair| pair[0].y <= pair[1].y),
                    "{} is not ordered bottom-up",
                    shape.name()
                );
            }
        }
        }
    }

    /// No cell twice: a heap that listed a cell again would ask a drone to
    /// place a block into rock, which is a refusal, which is a stall.
    #[test]
    fn no_cell_is_asked_for_twice() {
        let world = fixture::flat(40, 64);
        for shape in HeapShape::ALL {
            let Some(plan) = plan(&world, footprint((-5, -5), (5, 5)), shape) else {
                continue;
            };
            let unique: std::collections::HashSet<BlockPos> =
                plan.cells.iter().copied().collect();
            assert_eq!(
                unique.len(),
                plan.cells.len(),
                "{} lists a cell more than once",
                shape.name()
            );
            assert_eq!(plan.volume, plan.cells.len() as u64);
        }
    }

    /// A pyramid is square at every course and narrows by exactly one ring.
    #[test]
    fn a_pyramid_narrows_one_ring_a_course() {
        let world = fixture::flat(40, 64);
        let plan = plan(&world, footprint((-4, -4), (4, 4)), HeapShape::Pyramid)
            .expect("a nine-wide pyramid should fit anywhere flat");
        assert_eq!(plan.courses(), 5, "a 9x9 base is five courses");

        let floor = plan.cells[0].y;
        let mut last = None;
        for step in 0..plan.courses() {
            let y = floor + step;
            let width = plan.cells.iter().filter(|cell| cell.y == y).count();
            let side = (width as f64).sqrt() as usize;
            assert_eq!(side * side, width, "course {step} is not square");
            if let Some(below) = last {
                assert_eq!(side + 2, below, "course {step} did not step in by one ring");
            }
            last = Some(side);
        }
        assert_eq!(last, Some(1), "a pyramid should finish on a single block");
    }

    /// A spiral's ramp climbs one block per cell, which is `flow::STEP` —
    /// which is what makes it a staircase a drone can drive up rather than a
    /// wall it stands next to.
    #[test]
    fn a_spiral_is_a_staircase_a_drone_can_climb() {
        let world = fixture::flat(40, 64);
        let plan = plan(&world, footprint((-4, -4), (4, 4)), HeapShape::Spiral)
            .expect("a nine-wide spiral should fit anywhere flat");

        // The top of each ring cell, walked once round: consecutive cells
        // never differ by more than a drone can step.
        let ring = ring_walk(plan.footprint);
        let mut tops = Vec::new();
        for (x, z) in &ring {
            let top = plan
                .cells
                .iter()
                .filter(|cell| cell.x == *x && cell.z == *z)
                .map(|cell| cell.y)
                .max();
            if let Some(top) = top {
                tops.push(top);
            }
        }
        assert!(tops.len() > 8, "the ramp barely exists: {} cells", tops.len());
        for pair in tops.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= crate::flow::STEP,
                "the ramp steps {} at once; a drone can manage {}",
                (pair[1] - pair[0]).abs(),
                crate::flow::STEP
            );
        }
        // And it really does climb, or it is a ring rather than a ramp.
        assert!(tops.iter().max() > tops.iter().min());
    }

    /// A shaft is a prism, and a *short* one — the shape `mine` refuses to
    /// plan as an excavation, for the same reason it is capped here.
    #[test]
    fn a_shaft_is_a_prism_no_taller_than_a_drone_can_reach() {
        let world = fixture::flat(40, 64);
        let plan = plan(&world, footprint((0, 0), (3, 3)), HeapShape::Shaft)
            .expect("a shaft should fit anywhere flat");
        assert_eq!(plan.courses(), SHAFT_COURSES);
        let floor = plan.cells[0].y;
        for step in 0..SHAFT_COURSES {
            let width = plan.cells.iter().filter(|cell| cell.y == floor + step).count();
            assert_eq!(width, 16, "a 4x4 shaft course is sixteen blocks");
        }
    }

    /// Ground the shape cannot carry gets a refusal, not a quiet substitute.
    #[test]
    fn a_footprint_too_small_for_a_spiral_says_so() {
        let world = fixture::flat(40, 64);
        // Two wide: there is no cell to wind a ramp around.
        assert!(plan(&world, footprint((0, 0), (1, 1)), HeapShape::Spiral).is_none());
        // But a shaft and a pyramid will both stand on it.
        assert!(plan(&world, footprint((0, 0), (1, 1)), HeapShape::Shaft).is_some());
        assert!(plan(&world, footprint((0, 0), (1, 1)), HeapShape::Pyramid).is_some());

        // And the offer list says which, cheapest first.
        let offered = options(&world, footprint((0, 0), (1, 1)));
        assert_eq!(offered.len(), 2);
        assert!(offered.windows(2).all(|pair| pair[0].volume <= pair[1].volume));
    }

    /// **A heap on a slope follows the ground it is tipped onto.**
    ///
    /// The bug this replaces was found by the played test rather than by any
    /// unit test, because it needed real terrain to show up. The first version
    /// took the *highest* ground under the whole footprint and laid a flat
    /// first course at that height, on the theory that the low side would fill
    /// in from above. It does not: the high side's cells end up buried inside
    /// the hill, the low side's are not in the plan at all, and what is left
    /// is a handful of cells hanging in mid-air with nowhere to stand and
    /// build them from. The crew stood beside it holding a full load, for a
    /// thousand ticks.
    #[test]
    fn every_column_of_a_heap_starts_on_its_own_ground() {
        let world = fixture::slope(40, 70, 0, 4);
        let area = footprint((-4, -4), (4, 4));
        for shape in HeapShape::ALL {
            let Some(plan) = plan(&world, area, shape) else {
                continue;
            };
            for cell in &plan.cells {
                let ground = world
                    .surface_y(cell.x, cell.z)
                    .expect("no ground under a planned cell");
                assert!(
                    cell.y >= ground,
                    "{} put a cell at {cell:?} below the ground at {ground}",
                    shape.name()
                );
            }
            // And the lowest cell of each column really is that column's own
            // ground, so nothing is left floating over a dip.
            let mut columns: std::collections::HashMap<(i32, i32), i32> =
                std::collections::HashMap::new();
            for cell in &plan.cells {
                let bottom = columns.entry((cell.x, cell.z)).or_insert(cell.y);
                *bottom = (*bottom).min(cell.y);
            }
            for ((x, z), bottom) in columns {
                assert_eq!(
                    Some(bottom),
                    world.surface_y(x, z),
                    "{} floats the column at ({x}, {z})",
                    shape.name()
                );
            }
        }
    }

    /// Same footprint, same plan — twice. A heap is replayed rather than
    /// recorded, so a shape that wandered would be a hole in the oracle.
    #[test]
    fn the_same_footprint_plans_the_same_heap() {
        let world = fixture::flat(40, 64);
        for shape in HeapShape::ALL {
            let once = plan(&world, footprint((-3, -3), (5, 5)), shape);
            let twice = plan(&world, footprint((-3, -3), (5, 5)), shape);
            assert_eq!(once, twice, "{} is not deterministic", shape.name());
        }
    }

    /// Every shape names itself in something the game can draw.
    #[test]
    fn every_shape_is_named() {
        for shape in HeapShape::ALL {
            assert!(!shape.name().is_empty());
            assert!(shape.name().chars().all(|c| c.is_ascii_lowercase() || c == ' '));
        }
    }
}
