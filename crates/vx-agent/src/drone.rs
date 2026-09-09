//! One ground drone: where it is, what it is doing, what it is carrying.
//!
//! # Whole blocks per tick
//!
//! The drone moves a block at a time rather than carrying a velocity. The
//! simulation is a discrete voxel problem — which cell, which block to dig —
//! and pretending otherwise would mean floating-point movement that then has to
//! be rounded back to cells anyway, with the rounding as a new source of bugs.
//! [`Drone::previous_position`] is kept so the renderer can interpolate between
//! ticks and the drone still looks like it glides.
//!
//! # It cuts ahead and above, and descends only through its own floor
//!
//! Reach covers the cells level with the drone and the layer above it — never
//! diagonally below. That one restriction is what keeps an excavation
//! self-consistent, and it took three wrong answers to find:
//!
//! - Cutting a block leaves anything resting on it hanging in the air, since
//!   blocks do not fall. Taking the layer above first, always, means nothing is
//!   ever left floating.
//! - Cutting diagonally *down* carves a full-height notch beside the drone and
//!   destroys the floor of every cell in it — including the cells the drone
//!   needed to stand in to advance. It then sees the next block along, cannot
//!   get to it, and hands the job back forever.
//! - Working the whole region top-down instead fixes both but cannot cut a
//!   level tunnel at all: the roof is two blocks up, and nothing can stand
//!   inside solid rock to reach it.
//!
//! Descending is by undermining — cutting the block directly underfoot, which
//! is allowed only when there is solid ground under *that*, so the drop is
//! exactly one block and the step back up exists. Every hole it makes for
//! itself, it can climb out of.

use vx_core::BlockPos;
use vx_world::World;

use crate::flow::FlowField;
use crate::job::{DroneId, JobId};
use crate::stockpile::Stockpile;

/// What a drone is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DroneState {
    /// Nothing to do: the board is empty or nothing is reachable.
    Idle,
    /// On the way to a job.
    Travelling(JobId),
    /// At the face, cutting.
    Digging(JobId),
    /// Full, heading for the stockpile.
    Hauling,
    /// At a spoil heap, stacking what it is carrying into it.
    ///
    /// Its own state rather than a destination bolted onto [`Hauling`],
    /// because the state is snapshotted: where a loaded drone is headed has to
    /// be *in* it or a save taken mid-carry cannot restore what it was doing.
    Stacking(JobId),
    /// Cannot reach its work and has given it back to the board.
    Stuck,
    /// Under the player's direct control; the tick loop leaves it alone.
    Manual,
}

/// A single ground drone.
#[derive(Debug, Clone)]
pub struct Drone {
    pub id: DroneId,
    pub position: BlockPos,
    /// Where it was last tick, for the renderer to interpolate from.
    pub previous_position: BlockPos,
    pub state: DroneState,
    /// What is aboard, by block name.
    ///
    /// A [`Stockpile`] rather than a bare count, so waste rock and ore stay
    /// distinguishable all the way from the face to the pile — and so nothing
    /// keyed on a [`vx_core::BlockId`] can survive a mod shifting the ids.
    pub cargo: Stockpile,
    /// Blocks it can carry before it has to run them home.
    pub capacity: u64,
    /// Blocks of run per block of rise it needs on a ramp. A drone stat, and
    /// the number the mine planners generate excavations for.
    pub grade: i32,
    pub(crate) job: Option<JobId>,
    /// Positions in the current job the drone tried to break and was refused —
    /// a mod veto, in practice. Filtered out of everything so the drone moves
    /// on instead of chewing the same block forever; kept per job (cleared when
    /// a *different* job is claimed) so re-claiming the same job does not reset
    /// the lesson.
    pub(crate) denied: std::collections::HashSet<BlockPos>,
    /// Which job `denied` belongs to.
    pub(crate) denied_job: Option<JobId>,
    /// The flow field this drone is currently following, and what it was built
    /// against. Reused while the key and the world's edit count both match —
    /// a travelling drone changes nothing, so one field serves the whole leg
    /// instead of being rebuilt from scratch every single step.
    pub(crate) route: Option<CachedRoute>,
}

/// A drone's whole persistent state, as plain data.
///
/// Public fields and no behaviour: this exists so the app can serialise a
/// crew, and every field on it is one the drone would otherwise keep private.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroneSnapshot {
    pub id: DroneId,
    pub position: BlockPos,
    pub previous_position: BlockPos,
    pub state: DroneState,
    pub cargo: Stockpile,
    pub capacity: u64,
    pub grade: i32,
    pub job: Option<JobId>,
    /// Sorted, so the same crew writes the same bytes twice.
    pub denied: Vec<BlockPos>,
    pub denied_job: Option<JobId>,
}

/// What a cached route leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteKey {
    /// Somewhere the given job can be worked from.
    Work(JobId),
    /// The stockpile drop-off.
    Home,
}

/// A flow field plus the state it was valid for.
#[derive(Debug, Clone)]
pub(crate) struct CachedRoute {
    pub key: RouteKey,
    /// [`vx_world::World::edit_count`] at build time. Any edit anywhere
    /// invalidates — coarser than strictly needed, but an edit that matters
    /// and one that does not are indistinguishable without tracking regions,
    /// and rebuilding once per edit is already the win over once per tick.
    pub edits: u64,
    pub field: FlowField,
}

/// Offsets from a drone to the blocks it can cut, highest first.
///
/// The layer above first, then the drone's own layer. Nothing below: see the
/// module note for why that restriction is load-bearing. The block directly
/// underfoot is handled separately by [`Drone::may_undermine`], because whether
/// it is safe depends on the world rather than on geometry.
///
/// Above before level matters within the list too: cutting a block at the
/// drone's own level would leave whatever sits on it hanging, so the thing
/// sitting on it goes first.
///
/// This list is the single source of truth for reach **when cutting**.
/// [`Drone::reach`] applies it forwards to find blocks; [`stations_for`]
/// applies it backwards to find the cells a given block can be cut *from*. Two
/// separate versions of the same rule would drift, and the symptom would be a
/// drone standing next to a block it was sent to dig and refusing to.
///
/// Building is the other half and wants the opposite order — see
/// [`PLACE_OFFSETS`]. The *neighbourhood* is the same, which is why
/// [`stations_for`] serves both; the *order* is not, and cannot be, because
/// the reason cutting works top-down is exactly the reason stacking works
/// bottom-up.
///
/// A `const` rather than a builder: it was a `Vec`-returning function once,
/// which allocated afresh for every remaining block of every travel tick —
/// thousands of heap allocations per frame to enumerate seventeen numbers
/// known at compile time.
pub const REACH_OFFSETS: [[i32; 3]; 17] = [
    [-1, 1, -1], [0, 1, -1], [1, 1, -1],
    [-1, 1, 0],  [0, 1, 0],  [1, 1, 0],
    [-1, 1, 1],  [0, 1, 1],  [1, 1, 1],
    [-1, 0, -1], [0, 0, -1], [1, 0, -1],
    [-1, 0, 0],              [1, 0, 0],
    [-1, 0, 1],  [0, 0, 1],  [1, 0, 1],
];

/// Offsets from a drone to the cells it can build into, **lowest first**.
///
/// The mirror image of [`REACH_OFFSETS`], and the mirroring is the whole
/// point. Cutting is ordered highest-first because taking a block at your own
/// level leaves whatever rests on it hanging; stacking is ordered lowest-first
/// because putting a block above an empty cell leaves it hanging in the air.
/// Same neighbourhood, opposite argument, so the two lists are two orderings
/// of the same seventeen offsets and a test re-derives that from the rule.
///
/// Nothing below, for a different reason than cutting has: a drone cannot fill
/// the cell it is standing on without lifting itself, and `flow::is_standable`
/// wants the cell under a drone solid and the cell it occupies clear. Filling
/// underfoot would make it both.
///
/// The cell the drone occupies is absent from both lists, which is what stops
/// a drone walling itself in — `place_at`'s obstruction predicate is the
/// belt to this braces.
pub const PLACE_OFFSETS: [[i32; 3]; 17] = [
    [-1, 0, -1], [0, 0, -1], [1, 0, -1],
    [-1, 0, 0],              [1, 0, 0],
    [-1, 0, 1],  [0, 0, 1],  [1, 0, 1],
    [-1, 1, -1], [0, 1, -1], [1, 1, -1],
    [-1, 1, 0],  [0, 1, 0],  [1, 1, 0],
    [-1, 1, 1],  [0, 1, 1],  [1, 1, 1],
];

/// Carry capacity of a starting drone. Small enough that the haul back matters,
/// large enough that a body is not a hundred round trips.
pub const DEFAULT_CAPACITY: u64 = 64;

/// Grade a starting drone needs: three blocks of run per block of drop.
pub const DEFAULT_GRADE: i32 = 3;

impl Drone {
    pub fn new(id: DroneId, position: BlockPos) -> Self {
        Drone {
            id,
            position,
            previous_position: position,
            state: DroneState::Idle,
            cargo: Stockpile::new(),
            capacity: DEFAULT_CAPACITY,
            grade: DEFAULT_GRADE,
            job: None,
            denied: std::collections::HashSet::new(),
            denied_job: None,
            route: None,
        }
    }

    /// Everything about this drone that outlives a session.
    ///
    /// A plain-data twin with public fields, so `vx-app` can write a drone to
    /// disk without `vx-agent` ever hearing about a save directory — the same
    /// line [`crate::flow`] holds by knowing nothing about the app that drives
    /// it. Until stage 52 a crew simply evaporated at a save: `Mining` kept
    /// its operation in a private field and no file anywhere named it, so you
    /// could buy drones, set them cutting, quit, and come back to a half-dug
    /// hole with nothing in it.
    ///
    /// **`route` is deliberately absent.** It caches a whole [`FlowField`]
    /// keyed on the world's edit count, and any edit anywhere invalidates it —
    /// so it is megabytes to save something the next block broken would throw
    /// away regardless. A restored drone rebuilds it on its first step.
    ///
    /// `denied` comes out **sorted**: it is a `HashSet` in the drone and its
    /// iteration order is not stable, and a save that wrote different bytes
    /// for the same crew would make every quit a fresh write.
    pub fn snapshot(&self) -> DroneSnapshot {
        let mut denied: Vec<BlockPos> = self.denied.iter().copied().collect();
        denied.sort_by_key(|pos| (pos.x, pos.y, pos.z));
        DroneSnapshot {
            id: self.id,
            position: self.position,
            previous_position: self.previous_position,
            state: self.state,
            cargo: self.cargo.clone(),
            capacity: self.capacity,
            grade: self.grade,
            job: self.job,
            denied,
            denied_job: self.denied_job,
        }
    }

    /// Build a drone back from one.
    pub fn restore(snapshot: DroneSnapshot) -> Self {
        Drone {
            id: snapshot.id,
            position: snapshot.position,
            previous_position: snapshot.previous_position,
            state: snapshot.state,
            cargo: snapshot.cargo,
            capacity: snapshot.capacity,
            grade: snapshot.grade,
            job: snapshot.job,
            denied: snapshot.denied.into_iter().collect(),
            denied_job: snapshot.denied_job,
            route: None,
        }
    }

    /// Blocks aboard, of every kind.
    pub fn carrying(&self) -> u64 {
        self.cargo.total()
    }

    pub fn is_full(&self) -> bool {
        self.carrying() >= self.capacity
    }

    pub fn job(&self) -> Option<JobId> {
        self.job
    }

    /// Move to `to`, remembering where it came from.
    pub(crate) fn move_to(&mut self, to: BlockPos) {
        self.previous_position = self.position;
        self.position = to;
    }

    /// May the block directly underfoot be cut?    /// May the block directly underfoot be cut?
    ///
    /// Only when there is solid ground immediately below it. Then removing it
    /// drops the drone exactly one block, which is a step it can climb back up.
    /// Undermining into open space is a fall it may not be able to climb out
    /// of, and that is how a drone is lost at the bottom of its own hole.
    pub fn may_undermine(&self, world: &World) -> bool {
        world.is_solid(self.position.offset([0, -2, 0]))
    }

    /// Blocks this drone can cut from where it stands, in cutting order.
    pub fn reach(&self, world: &World) -> Vec<BlockPos> {
        let mut cells: Vec<BlockPos> = REACH_OFFSETS
            .into_iter()
            .map(|offset| self.position.offset(offset))
            .collect();
        // Last resort, after everything above and beside it.
        if self.may_undermine(world) {
            cells.push(self.position.offset([0, -1, 0]));
        }
        cells
    }

    /// The first block in reach that `wanted` accepts, if any.
    pub fn target_in_reach(
        &self,
        world: &World,
        wanted: impl Fn(BlockPos) -> bool,
    ) -> Option<BlockPos> {
        self.reach(world)
            .into_iter()
            .find(|pos| world.is_solid(*pos) && wanted(*pos))
    }

    /// The next block to cut inside `region` from where the drone stands.
    ///
    /// Everything level with it and above first, skipping anything unbreakable
    /// (bedrock — see [`is_breakable`]) and anything in its `denied` set (a
    /// veto it has already run into). Undermining is the last resort and
    /// carries one extra condition: **nothing in the region may still be left
    /// above the drone's own level.**
    ///
    /// That condition is the difference between an excavation that finishes and
    /// one that strands itself. Dropping a level removes the floor of the level
    /// above, so anything still up there loses every cell a drone could stand
    /// in to reach it — a block in plain sight that nothing can ever touch. The
    /// scan it costs is a whole-region sweep, but it only runs on the tick where
    /// the drone has genuinely run out of things to cut in front of it.
    pub fn next_cut(&self, world: &World, region: &crate::aabb::VoxelAabb) -> Option<BlockPos> {
        let wanted =
            |pos: &BlockPos| is_breakable(world, *pos) && !self.denied.contains(pos);

        let ahead = REACH_OFFSETS
            .into_iter()
            .map(|offset| self.position.offset(offset))
            .find(|pos| region.contains(*pos) && wanted(pos));
        if ahead.is_some() {
            return ahead;
        }

        let below = self.position.offset([0, -1, 0]);
        if !region.contains(below) || !wanted(&below) || !self.may_undermine(world) {
            return None;
        }
        let work_above = region
            .clamped_to_world()
            .blocks()
            .any(|pos| pos.y > self.position.y && is_breakable(world, pos));
        (!work_above).then_some(below)
    }
}

/// Is there a block here a drone could actually remove?
///
/// Solid, and with a hardness — blocks without one (bedrock) are unbreakable by
/// definition, and treating them as work is how a drone ends up chewing the
/// world floor until the end of time. Every scan that counts outstanding work
/// uses this, so a job whose region overlaps bedrock completes once everything
/// *breakable* in it is gone.
pub fn is_breakable(world: &World, pos: BlockPos) -> bool {
    let block = world.block(pos);
    world
        .registry()
        .get(block)
        .is_some_and(|def| def.solid && def.hardness.is_some())
}

/// Cells a drone could stand in and cut `block` from.
///
/// The inverse of [`REACH_OFFSETS`], and the reason it is shared: this
/// is what tells a drone *where to drive to* in order to dig something, so if
/// the two disagreed a drone would arrive somewhere it cannot work from and
/// hand the job straight back.
pub fn stations_for(world: &World, block: BlockPos) -> Vec<BlockPos> {
    let mut cells: Vec<BlockPos> = REACH_OFFSETS
        .into_iter()
        .map(|[dx, dy, dz]| block.offset([-dx, -dy, -dz]))
        .collect();
    // Standing on top of it counts only where undermining would be safe, which
    // is the one case the offset list cannot decide by itself.
    if world.is_solid(block.offset([0, -1, 0])) {
        cells.push(block.offset([0, 1, 0]));
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drone() -> Drone {
        Drone::new(DroneId(0), BlockPos::new(10, 64, -3))
    }

    #[test]
    fn a_new_drone_is_idle_and_empty() {
        let drone = drone();
        assert_eq!(drone.state, DroneState::Idle);
        assert_eq!(drone.carrying(), 0);
        assert!(!drone.is_full());
        assert!(drone.job().is_none());
        assert_eq!(drone.previous_position, drone.position);
    }

    #[test]
    fn moving_records_where_it_came_from() {
        // What the renderer interpolates along. Losing it makes drones teleport
        // between ticks.
        let mut drone = drone();
        let from = drone.position;
        let to = from.offset([1, 0, 0]);
        drone.move_to(to);

        assert_eq!(drone.position, to);
        assert_eq!(drone.previous_position, from);
    }

    /// Solid stone below `floor`, air above, so undermining is safe.
    fn bedrock_world(floor: i32) -> World {
        crate::fixture::flat(1, floor)
    }

    /// One solid block at `shelf` with nothing under it: the case where
    /// undermining is a fall rather than a step.
    fn shelf_world(floor: i32, shelf: i32) -> World {
        let mut world = bedrock_world(floor);
        let stone = world.registry().id_of("engine:stone").unwrap();
        world.set_block(BlockPos::new(0, shelf, 0), stone);
        world
    }

    #[test]
    fn reach_covers_the_drones_own_layer_and_the_one_above_it() {
        // Two 3x3 layers less the drone's own cell. Nothing below at all: that
        // restriction is what keeps excavations self-consistent. Written out
        // by hand as a const, so re-derive it here and make sure no offset was
        // fat-fingered.
        let offsets = REACH_OFFSETS;
        assert_eq!(offsets.len(), 17);
        let mut expected = Vec::new();
        for dy in [1, 0] {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if [dx, dy, dz] != [0, 0, 0] {
                        expected.push([dx, dy, dz]);
                    }
                }
            }
        }
        assert_eq!(offsets.to_vec(), expected, "the const drifted from its rule");
        assert!(!offsets.contains(&[0, 0, 0]), "it can dig its own cell");
        assert!(
            offsets.iter().all(|offset| offset[1] >= 0),
            "reach dips below the drone: {offsets:?}"
        );
        assert!(offsets.contains(&[0, 1, 0]), "it cannot dig straight up");
    }

    #[test]
    fn reach_excludes_the_diagonal_below_which_is_what_carves_notches() {
        // Cutting down-and-across looks like the natural way to descend, and it
        // is exactly what strands a drone: it removes the floor of every cell in
        // the notch, including the ones needed to advance the face.
        let offsets = REACH_OFFSETS;
        assert!(!offsets.contains(&[1, -1, 0]));
        assert!(!offsets.contains(&[0, -1, 1]));
    }

    #[test]
    fn reach_is_ordered_from_the_top_down() {
        let heights: Vec<i32> = REACH_OFFSETS.iter().map(|offset| offset[1]).collect();
        assert!(
            heights.windows(2).all(|pair| pair[0] >= pair[1]),
            "reach is not ordered highest first: a face would be cut from underneath"
        );
    }

    /// **The two lists are one neighbourhood read two ways.**
    ///
    /// Stage 56 added building, and building is not cutting run backwards —
    /// it is cutting with one argument inverted. Cutting takes the highest
    /// block first so nothing is left hanging; stacking fills the lowest cell
    /// first for exactly the same reason, from the other side. If those two
    /// lists ever stop being permutations of each other, one of them has
    /// grown a cell the other cannot reach, and a drone will stand next to
    /// work it refuses to do — the same failure the reach/stations pair is
    /// tested against.
    #[test]
    fn placing_reaches_the_same_cells_as_cutting_in_the_opposite_order() {
        let mut cut = REACH_OFFSETS;
        let mut place = PLACE_OFFSETS;
        cut.sort_unstable();
        place.sort_unstable();
        assert_eq!(cut, place, "reach and place are not the same neighbourhood");

        let heights: Vec<i32> = PLACE_OFFSETS.iter().map(|offset| offset[1]).collect();
        assert!(
            heights.windows(2).all(|pair| pair[0] <= pair[1]),
            "placing is not ordered lowest first: a block would be stacked on air"
        );
        // And neither list contains the drone's own cell, which is what stops
        // a machine building itself into a box.
        assert!(!PLACE_OFFSETS.contains(&[0, 0, 0]));
        assert!(!REACH_OFFSETS.contains(&[0, 0, 0]));
    }

    #[test]
    fn undermining_is_allowed_over_solid_ground_and_refused_over_a_void() {
        // One block of undermining is a step down. Undermining into open space
        // is a fall, and possibly a lost drone.
        let world = bedrock_world(60);
        let drone = Drone::new(DroneId(0), BlockPos::new(0, 40, 0));
        assert!(drone.may_undermine(&world), "solid rock below should be safe");
        assert!(drone.reach(&world).contains(&BlockPos::new(0, 39, 0)));

        // Standing on a one-block-thick shelf ten blocks over the ground.
        let world = shelf_world(60, 70);
        let drone = Drone::new(DroneId(0), BlockPos::new(0, 71, 0));
        assert!(!drone.may_undermine(&world), "the shelf is the only floor");
        assert!(
            !drone.reach(&world).contains(&BlockPos::new(0, 70, 0)),
            "it is willing to cut the shelf out from under itself"
        );
    }

    #[test]
    fn undermining_comes_last_so_the_face_is_cut_from_above_first() {
        let world = bedrock_world(60);
        let drone = Drone::new(DroneId(0), BlockPos::new(0, 40, 0));
        let reach = drone.reach(&world);
        assert_eq!(reach.last(), Some(&BlockPos::new(0, 39, 0)));
    }

    #[test]
    fn a_block_can_be_cut_from_exactly_the_cells_that_reach_it() {
        // The pairing that keeps "drive there" and "dig that" agreeing. If a
        // station is not a cell that reaches the block, a drone arrives and
        // refuses to work; if a reachable cell is not a station, it is never
        // sent there in the first place.
        let world = bedrock_world(60);
        let block = BlockPos::new(4, 40, 4);

        for station in super::stations_for(&world, block) {
            let drone = Drone::new(DroneId(0), station);
            assert!(
                drone.reach(&world).contains(&block),
                "a drone at {station:?} is sent to dig {block:?} but cannot reach it"
            );
        }
    }

    #[test]
    fn every_cell_that_reaches_a_block_is_listed_as_a_station() {
        let world = bedrock_world(60);
        let block = BlockPos::new(4, 40, 4);
        let stations = super::stations_for(&world, block);

        for dy in -2..=2 {
            for dz in -2..=2 {
                for dx in -2..=2 {
                    let candidate = block.offset([dx, dy, dz]);
                    let drone = Drone::new(DroneId(0), candidate);
                    if drone.reach(&world).contains(&block) {
                        assert!(
                            stations.contains(&candidate),
                            "{candidate:?} reaches {block:?} but is not offered as a station"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn capacity_decides_when_it_is_full() {
        let mut drone = drone();
        drone.capacity = 3;
        drone.cargo.add("engine:stone", 2);
        assert!(!drone.is_full());
        drone.cargo.add("engine:copper_ore", 1);
        assert!(drone.is_full(), "capacity counts every kind aboard, not one");
    }
}
