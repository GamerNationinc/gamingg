//! Driving a swarm: the tick that turns a mine plan into a hole in the ground.
//!
//! # The loop
//!
//! Each drone, each tick, in order: fall if it is standing on nothing, run its
//! load home if it is full, take a job if it has none, cut something if
//! anything in its job is within reach, otherwise drive toward the work.
//!
//! # Reaching first, pathfinding second
//!
//! Checking the twenty-five cells a drone can reach is free; building a flow
//! field is not. Ore bodies are contiguous, so once a drone is at the face
//! almost every tick finds its next block right there and no field is built at
//! all. Fields get rebuilt when the drone runs out of face — which is exactly
//! when the route has changed anyway, because the drone changed it.

use vx_core::{BlockPos, EventBus};
use vx_world::{break_block, World};

use crate::aabb::VoxelAabb;
use crate::drone::{CachedRoute, Drone, DroneState, RouteKey};
use crate::flow::{self, FlowField};
use crate::job::{DroneId, Job, JobBoard, JobKind};
use crate::mine::MinePlan;
use crate::stockpile::Stockpile;

/// Cells a single flow field may cover before the operation gives up on it.
///
/// A field is a breadth-first sweep of every cell in its bounds, so an
/// unbounded one is not slow, it is a hang. Hitting this means a drone was
/// asked to work somewhere absurdly far from where it stands, and reporting
/// that as [`DroneState::Stuck`] is far kinder than freezing.
const MAX_FIELD_CELLS: u64 = 2_000_000;

/// How close a drone has to be to the stockpile to unload into it.
const DROP_OFF_RANGE: i32 = 2;

/// What one tick achieved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Blocks removed from the world this tick.
    pub dug: u64,
    /// Drones that moved.
    pub moved: u32,
    /// Jobs retired this tick.
    pub completed: u32,
    /// Blocks unloaded into the stockpile.
    pub delivered: u64,
    /// Blocks **added** to the world this tick, stacked into a spoil heap.
    ///
    /// New in stage 56, and it has to be here or the stall detector lies: a
    /// crew that is doing nothing but building would report an idle tick every
    /// tick and `run` would call a working heap `Stalled` after `PATIENCE`.
    pub placed: u64,
}

impl TickReport {
    /// Did anything at all happen? A run of ticks where nothing does means the
    /// operation has stalled rather than finished.
    ///
    /// `placed` counts here for the same reason `dug` does, and it had to be
    /// added when the crew learned to build: a derive on the struct meant a
    /// heap being stacked one block a tick reported as idle, and `run` would
    /// have declared a working operation `Stalled`.
    pub fn is_idle(&self) -> bool {
        *self == TickReport::default()
    }
}

/// How a [`Operation::run`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The board emptied and every drone unloaded.
    Finished,
    /// Nothing changed for long enough to call it stuck.
    Stalled,
    /// Ran out of ticks with work still outstanding.
    OutOfTicks,
}

/// How far outside its excavation an operation reads the world.
///
/// The widest read is the haul route home, whose flow field is built over
/// `VoxelAabb::new(drone_position, home).expanded(12)`. Anything inside this
/// margin of the excavation-plus-home box may be consulted, so anything inside
/// it must be real ground rather than the air an unloaded chunk reports.
pub const WORK_MARGIN: i32 = 12;

/// The block box an operation over `region`, hauling to `home`, will read.
///
/// Pin this before dispatching (see `World::pin_span`). Reading unloaded ground
/// is not unsafe — it reports air, and a drone conservatively refuses to drive
/// onto ground it cannot see — but it makes the drone's decisions depend on
/// which chunks are resident, and residency follows the player's camera. Pinned,
/// the same dispatch produces the same excavation however the player wandered.
pub fn working_span(region: VoxelAabb, home: BlockPos) -> VoxelAabb {
    region
        .union(VoxelAabb::single(home))
        .expanded(WORK_MARGIN)
        .clamped_to_world()
}

/// A mining operation: the work, the drones doing it, and the pile it feeds.
#[derive(Debug)]
pub struct Operation {
    pub board: JobBoard,
    pub stockpile: Stockpile,
    /// Where hauled blocks are dropped off.
    pub home: BlockPos,
    pub drones: Vec<Drone>,
    /// Flow fields built since the operation started. Purely diagnostic: the
    /// route cache exists so a travel leg costs one build, and this is the
    /// number a test can watch to keep that true.
    pub fields_built: u64,
    /// The spoil heap this crew is stacking, if one was ordered.
    ///
    /// Held on the operation rather than on the jobs because a `Job` carries a
    /// region and a heap is a *list of cells in an order* — the ordering is
    /// the safety property (see [`crate::heap`]), and a bounding box loses it.
    /// The jobs say which course; the plan says which cells.
    pub heap: Option<crate::heap::HeapPlan>,
    /// Blocks stacked into the heap so far.
    ///
    /// The fourth honest place a block can be, after the mine-mouth pile and a
    /// drone's own cargo — see [`Operation::accounted_blocks`].
    pub stacked: u64,
    /// The drone the player is driving, if any. One machine, one pair
    /// of hands — `Option` rather than a per-drone flag makes "at most one"
    /// unrepresentable-otherwise.
    controlled: Option<usize>,
}

/// An excavation's whole persistent state, as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationSnapshot {
    pub board: crate::job::BoardSnapshot,
    pub stockpile: Stockpile,
    pub home: BlockPos,
    pub drones: Vec<crate::drone::DroneSnapshot>,
    pub fields_built: u64,
    pub heap: Option<crate::heap::HeapPlan>,
    pub stacked: u64,
    /// Which drone the player had the wheel of. Restored, because a session
    /// that saved mid-drive and reloaded with the machine back on autopilot
    /// would quietly countermand an order the player gave.
    pub controlled: Option<usize>,
}

impl Operation {
    pub fn new(home: BlockPos) -> Self {
        Operation {
            board: JobBoard::new(),
            stockpile: Stockpile::new(),
            home,
            drones: Vec::new(),
            fields_built: 0,
            heap: None,
            stacked: 0,
            controlled: None,
        }
    }

    /// Everything about this excavation that outlives a session.
    ///
    /// The crew, the board with its claims, the mine-mouth pile and who has
    /// the wheel. Stage 52's whole point: until then `Mining` held this in a
    /// private field, nothing wrote it, and a dispatch died at every save.
    pub fn snapshot(&self) -> OperationSnapshot {
        OperationSnapshot {
            board: self.board.snapshot(),
            stockpile: self.stockpile.clone(),
            home: self.home,
            drones: self.drones.iter().map(Drone::snapshot).collect(),
            fields_built: self.fields_built,
            heap: self.heap.clone(),
            stacked: self.stacked,
            controlled: self.controlled,
        }
    }

    /// Build an operation back from one.
    pub fn restore(snapshot: OperationSnapshot) -> Self {
        Operation {
            board: JobBoard::restore(snapshot.board),
            stockpile: snapshot.stockpile,
            home: snapshot.home,
            drones: snapshot.drones.into_iter().map(Drone::restore).collect(),
            fields_built: snapshot.fields_built,
            heap: snapshot.heap,
            stacked: snapshot.stacked,
            controlled: snapshot.controlled,
        }
    }

    /// Add a drone, starting it at `position`.
    pub fn add_drone(&mut self, position: BlockPos) -> DroneId {
        let id = DroneId(self.drones.len() as u32);
        self.drones.push(Drone::new(id, position));
        id
    }

    /// Turn a mine plan into jobs.
    ///
    /// Access first and in order — the outermost cut carries the highest
    /// priority, so the route down is opened from the top and the drone is
    /// never asked to cut a bench it would have to fall into. Extraction sits
    /// below all of it, because ore dug before there is a way out is ore that
    /// stays where it is.
    pub fn post_plan(&mut self, plan: &MinePlan) {
        let access_count = plan.access.len() as i32;
        for (order, region) in plan.access.iter().enumerate() {
            self.board
                .post(JobKind::Access, *region, 1_000 + access_count - order as i32);
        }
        // Extraction layers are ordered top-down too, and for the same reason:
        // the bench under the drone has to still be there when it cuts the one
        // above.
        let layers = plan.extraction.len() as i32;
        for (order, region) in plan.extraction.iter().enumerate() {
            self.board.post(JobKind::Extract, *region, layers - order as i32);
        }
    }

    /// Post a spoil heap: one job per course, lowest course first.
    ///
    /// A course per job rather than one job for the whole heap, for the same
    /// reason `post_plan` posts a job per bench: it is the unit a drone can
    /// finish, and it is what lets a crew of several share the work without
    /// treading on each other. Priorities run **below** every cut job
    /// (`post_plan` starts at 1 and access at 1,000), because the hole comes
    /// first and the heap is what you do with what came out of it.
    pub fn post_heap(&mut self, plan: &crate::heap::HeapPlan) {
        let floor = plan.cells.first().map_or(0, |cell| cell.y);
        let courses = plan.courses();
        for step in 0..courses {
            let y = floor + step;
            let cells: Vec<BlockPos> =
                plan.cells.iter().copied().filter(|cell| cell.y == y).collect();
            let Some(region) = VoxelAabb::containing(cells) else {
                continue;
            };
            // Negative, and falling with height: below every cut job (which
            // never goes below 1), and the lowest course of the heap before
            // the one above it — which is the ordering the whole module note
            // is about. The sign of `step` here is the whole safety argument
            // on the board's side, and getting it backwards is not a subtle
            // failure: a drone sent to the apex of a pyramid before anything
            // is underneath it finds nowhere to stand, gives up, and carries
            // its load home for ever.
            self.board.post(JobKind::Stack, region, -1_000 - step);
        }
        self.heap = Some(plan.clone());
    }

    /// Cells of the heap inside `region` that are still air and still wanted.
    ///
    /// One function answers both "where is there work" and "is this job
    /// finished", exactly as the breakable scan does for a cut. Two separate
    /// answers to that question is how a job wedges open.
    fn heap_work(&self, world: &World, region: &VoxelAabb) -> Vec<BlockPos> {
        let Some(heap) = &self.heap else {
            return Vec::new();
        };
        heap.cells
            .iter()
            .copied()
            .filter(|cell| region.contains(*cell))
            .filter(|cell| !world.is_solid(*cell))
            .collect()
    }

    /// How much spoil is sitting on the mine-mouth pile, waiting to be stacked.
    fn heap_spoil_available(&self) -> u64 {
        self.stockpile
            .entries()
            .filter(|(name, _)| crate::heap::is_spoil(name))
            .map(|(_, count)| count)
            .fold(0u64, u64::saturating_add)
    }

    /// Fill a drone from the pile with spoil, and say how much went aboard.
    fn load_spoil(&mut self, index: usize) -> u64 {
        let capacity = self.drones[index].capacity;
        let mut aboard = 0;
        let rows: Vec<(String, u64)> = self
            .stockpile
            .entries()
            .filter(|(name, _)| crate::heap::is_spoil(name))
            .map(|(name, count)| (name.to_string(), count))
            .collect();
        for (name, count) in rows {
            if aboard >= capacity {
                break;
            }
            let want = count.min(capacity - aboard);
            let took = self.stockpile.take(&name, want);
            self.drones[index].cargo.add(name, took);
            aboard += took;
        }
        aboard
    }

    /// Bring spoil back from the yard, so a heap can be built out of it.
    ///
    /// **The mirror of the ferry, and the reason a heap is buildable at all
    /// once the hole is finished.** Spoil goes: face → cargo → mine-mouth pile
    /// → (flier) → the base pile in town. Order a heap while the crew is still
    /// cutting and there is rock at the mine mouth to stack; order one the
    /// day after and every block of it is in town, the crew has nothing to
    /// build with, and the order stands for ever with nothing happening. Which
    /// is what the first played run of the `--heap` fixture did: dig long
    /// enough for the dispatch to *finish*, and the heap came out at zero.
    ///
    /// So the yard hands rock back. Only spoil, only when the heap wants
    /// blocks and the mine mouth is bare, and only a load at a time — which
    /// bounds the churn against the ferry carrying the same rock the other
    /// way, and keeps the two of them from playing catch.
    ///
    /// `is_spoil` decides what in the yard counts as rock. It is a parameter
    /// rather than [`crate::heap::is_spoil`] alone because the yard is not the
    /// mine mouth: the base pile is also where the fleet keeps its **fuel**,
    /// and a canister is not ore, so the crate's own "anything that is not
    /// ore" rule would happily send the crew off to stack the tank into a
    /// pyramid. It did, once — the crew went dry on the next tick and the
    /// heap came out at zero. What is a good and what is rock is the app's
    /// question, so the app answers it.
    ///
    /// Returns how much came back.
    pub fn fetch_spoil(
        &mut self,
        world: &World,
        yard: &mut Stockpile,
        want: u64,
        is_spoil: impl Fn(&str) -> bool,
    ) -> u64 {
        if want == 0 || !self.heap_wants_blocks(world) || self.heap_spoil_available() > 0 {
            return 0;
        }
        // The drones might already be carrying enough between them; no point
        // dragging more out of town on top of it.
        if self.drones.iter().map(Drone::carrying).sum::<u64>() > 0 {
            return 0;
        }
        let rows: Vec<(String, u64)> = yard
            .entries()
            .filter(|(name, _)| crate::heap::is_spoil(name) && is_spoil(name))
            .map(|(name, count)| (name.to_string(), count))
            .collect();
        let mut back = 0;
        for (name, count) in rows {
            if back >= want {
                break;
            }
            let took = yard.take(&name, count.min(want - back));
            self.stockpile.add(name, took);
            back += took;
        }
        back
    }

    /// Whether there is a heap with room in it.
    fn heap_wants_blocks(&self, world: &World) -> bool {
        self.heap
            .as_ref()
            .is_some_and(|heap| !self.heap_work(world, &heap.span()).is_empty())
    }

    /// Claim a stacking job specifically, ignoring the board's ranking.
    ///
    /// `claim_nearest` ranks by priority, and stacking is deliberately the
    /// lowest priority there is, so a drone would never reach for one while a
    /// single block of rock remained to cut. That is the right rule for an
    /// *empty* drone and the wrong one for a full one: a machine that cannot
    /// carry any more has to put its load down somewhere, and the heap is
    /// nearer than home.
    fn claim_stack(&mut self, index: usize) -> Option<Job> {
        let id = self.drones[index].id;
        let from = self.drones[index].position;
        let job = self
            .board
            .claim_nearest_of(id, from, JobKind::Stack)?
            .clone();
        self.drones[index].job = Some(job.id);
        Some(job)
    }

    /// One tick of a drone that is stacking spoil into the heap.
    ///
    /// Returns `false` when the drone has nothing left to give, so the caller
    /// can put it back to work cutting.
    fn stack_step(
        &mut self,
        index: usize,
        job: &Job,
        world: &mut World,
        events: &EventBus,
        report: &mut TickReport,
    ) -> bool {
        // Nothing aboard to stack with. Load from the mine-mouth pile if the
        // drone is standing at it, and otherwise hand the job back.
        //
        // This is the arm that makes a heap finishable. Spoil goes on your
        // back at the face, but a drone that fills up runs it home long before
        // a heap is ordered, so by the time the crew has a heap to build most
        // of the rock is already on the pile. Without this the heap could only
        // ever catch what happened to be in a cargo bed at the moment the
        // order was given — which in a played run was nothing at all, and the
        // crew stood idle beside an empty footprint.
        //
        // Only spoil is taken. The ore stays on the pile, which is the whole
        // bargain the player agreed to: a heap costs you the stone you could
        // have sold, not the copper.
        if self.drones[index].carrying() == 0 {
            let home = flow::settle(world, self.home);
            let position = self.drones[index].position;
            let near = (position.x - home.x).abs() <= DROP_OFF_RANGE
                && (position.y - home.y).abs() <= DROP_OFF_RANGE
                && (position.z - home.z).abs() <= DROP_OFF_RANGE;
            if near && self.load_spoil(index) > 0 {
                self.drones[index].state = DroneState::Stacking(job.id);
                return true;
            }
            // Not at the pile, or the pile has no spoil left: walk home. If
            // there is nothing to fetch either, the caller idles it.
            if self.heap_spoil_available() > 0 {
                self.drones[index].state = DroneState::Hauling;
                return self.haul(index, world, report);
            }
            self.board.release(job.id);
            self.drones[index].job = None;
            self.drones[index].state = DroneState::Idle;
            return false;
        }

        let wanted = self.heap_work(world, &job.region);
        if wanted.is_empty() {
            self.board.complete(job.id);
            self.drones[index].job = None;
            self.drones[index].state = DroneState::Idle;
            report.completed += 1;
            return true;
        }

        // Something in reach to fill, without moving. `PLACE_OFFSETS` is
        // ordered lowest-first, which is the whole reason the drone never
        // stacks a block onto air.
        let position = self.drones[index].position;
        let target = crate::drone::PLACE_OFFSETS
            .iter()
            .map(|offset| position.offset(*offset))
            .find(|cell| {
                wanted.contains(cell) && !self.drones[index].denied.contains(cell)
            });

        if let Some(target) = target {
            // Whatever is aboard — a heap is made of what came out of the
            // hole, not of a recipe. Name order, so the same cargo always
            // stacks the same way and a replay agrees.
            let name = self.drones[index]
                .cargo
                .entries()
                .next()
                .map(|(name, _)| name.to_string());
            let Some(name) = name else {
                return false;
            };
            let block = world.registry().id_of(&name);
            let Some(block) = block else {
                // A good the registry lost with a mod. Drop it rather than
                // wedge the job: the region format's rule, applied to cargo.
                self.drones[index].cargo.take(&name, u64::MAX);
                return true;
            };
            // The drone's own cell is the obstruction: a machine must never
            // fill the space it is standing in, which is exactly what
            // `place_at` refuses when asked.
            let standing = self.drones[index].position;
            match vx_world::place_at(
                world,
                events,
                target,
                block,
                standing,
                vx_core::Face::PosY,
                |cell| cell == standing,
            ) {
                Ok(_) => {
                    self.drones[index].cargo.take(&name, 1);
                    self.drones[index].state = DroneState::Stacking(job.id);
                    self.stacked += 1;
                    report.placed += 1;
                }
                Err(error) => {
                    // A veto — the town's permit gate subscribes to the same
                    // event a player's build goes through, so a heap that
                    // crosses a claim is refused block by block. Remember it
                    // and move on, exactly as a denied cut does.
                    log::debug!("drone refused to stack at {target:?}: {error}");
                    self.drones[index].denied.insert(target);
                    self.drones[index].state = DroneState::Stacking(job.id);
                }
            }
            return true;
        }

        // Nothing in reach: walk to somewhere that is. `travel_to_work` is
        // already generic over "cells that want working" and routes by
        // `stations_for`, which is the same neighbourhood whether the drone is
        // going to take a block away or put one down.
        let workable: Vec<BlockPos> = wanted
            .into_iter()
            .filter(|cell| !self.drones[index].denied.contains(cell))
            .collect();
        if workable.is_empty() {
            self.board.release(job.id);
            self.drones[index].job = None;
            return false;
        }
        let region = job.region;
        self.travel_to_work(index, world, &region, &workable, job.id, report);

        // **A heap it cannot reach is not a reason to stop.**
        //
        // `travel_to_work` reports "no route" as `Stuck`, which is the right
        // answer for a cut — the work genuinely cannot be done and the player
        // should see it. It is the wrong answer here: a full drone that cannot
        // get to the heap can still run its load home, and a crew that stood
        // still with full beds because a spoil heap was ordered on the far
        // side of a hill would be an order that broke the mine. Which is
        // exactly what the first version of this did, found by the played
        // test: one drone, `STUCK`, carrying 64, for a thousand ticks.
        //
        // So a heap is a *preference*. Hand the job back, say nothing, and let
        // the caller fall through to hauling home.
        if self.drones[index].state == DroneState::Stuck {
            self.board.release(job.id);
            self.drones[index].job = None;
            self.drones[index].state = DroneState::Hauling;
            return false;
        }
        true
    }

    /// Advance every drone by one tick.
    pub fn tick(&mut self, world: &mut World, events: &EventBus) -> TickReport {
        let mut report = TickReport::default();
        for index in 0..self.drones.len() {
            // A drone under manual control answers to the player, not to the
            // board. Gravity still applies to it, from `pilot_tick`.
            if self.controlled == Some(index) {
                continue;
            }
            self.tick_drone(index, world, events, &mut report);
        }
        report
    }

    /// Tick until the work is done, it stalls, or `max_ticks` runs out.
    ///
    /// Returns the outcome and how many ticks it took.
    pub fn run(
        &mut self,
        world: &mut World,
        events: &EventBus,
        max_ticks: u64,
    ) -> (RunOutcome, u64) {
        // Enough consecutive quiet ticks to be sure it is not just a drone
        // walking a long corridor with nothing to report.
        const PATIENCE: u32 = 64;
        let mut quiet = 0;

        for tick in 1..=max_ticks {
            let report = self.tick(world, events);

            if self.board.is_empty() && self.drones.iter().all(|drone| drone.carrying() == 0) {
                return (RunOutcome::Finished, tick);
            }

            if report.is_idle() {
                quiet += 1;
                if quiet >= PATIENCE {
                    return (RunOutcome::Stalled, tick);
                }
            } else {
                quiet = 0;
            }
        }
        (RunOutcome::OutOfTicks, max_ticks)
    }

    /// Take a drone off the board and hand it to the player.
    ///
    /// The claimed job is **released, not completed** — the work still needs
    /// doing, and another drone should be free to pick it up while this one is
    /// being driven around. Hand-back needs no bookkeeping at all: the drone
    /// goes `Idle`, and the next tick claims lazily through `current_job`.
    pub fn take_control(&mut self, index: usize) -> bool {
        if index >= self.drones.len() || self.controlled.is_some() {
            return false;
        }
        if let Some(job) = self.drones[index].job.take() {
            self.board.release(job);
        }
        self.drones[index].route = None;
        self.drones[index].state = DroneState::Manual;
        self.controlled = Some(index);
        true
    }

    /// Give a drone back to the board.
    pub fn release_control(&mut self, index: usize) -> bool {
        if self.controlled != Some(index) {
            return false;
        }
        self.controlled = None;
        self.drones[index].state = DroneState::Idle;
        true
    }

    /// Which drone the player is driving.
    pub fn controlled(&self) -> Option<usize> {
        self.controlled
    }

    /// Advance the piloted drone by one tick of the player's input.
    ///
    /// Order matches the autonomous path: gravity, then cut, then drive. A
    /// full drone refuses to cut and says so, which is the pilot's cue to take
    /// it home.
    pub fn pilot_tick(
        &mut self,
        world: &mut World,
        events: &EventBus,
        command: crate::pilot::PilotCommand,
    ) -> crate::pilot::PilotReport {
        let mut report = crate::pilot::PilotReport::default();
        let Some(index) = self.controlled else {
            return report;
        };

        report.moved = self.drones[index].pilot_settle(world);

        if command.cut {
            if self.drones[index].is_full() {
                report.blocked = true;
            } else if let Some(target) = self.drones[index].pilot_target(world, command.heading) {
                match break_block(world, events, target) {
                    Ok(removed) => {
                        if !self.drones[index].cargo.add_block(world.registry(), removed, 1) {
                            log::warn!("pilot cut an unregistered block at {target:?}");
                        }
                        report.dug += 1;
                        // Take the trunk, take the tree — the same rule the
                        // autonomous cutter follows, for the same reason.
                        if crate::mine::is_vegetation_id(world.registry(), removed) {
                            report.dug += self.fell_tree(index, world, events, target);
                        }
                    }
                    Err(error) => {
                        // A mod said no. Piloting is not a way around a veto.
                        log::debug!("pilot denied at {target:?}: {error}");
                        report.blocked = true;
                    }
                }
            } else {
                report.blocked = true;
            }
        }

        if let Some(heading) = command.heading {
            if self.drones[index].pilot_step(world, heading) {
                report.moved = true;
            } else {
                report.blocked = true;
            }
        }

        report
    }

    fn tick_drone(
        &mut self,
        index: usize,
        world: &mut World,
        events: &EventBus,
        report: &mut TickReport,
    ) {
        // Gravity first: a drone that cut the ground out from under itself last
        // tick is falling, and everything below assumes it is standing.
        let settled = flow::settle(world, self.drones[index].position);
        if settled != self.drones[index].position {
            self.drones[index].move_to(settled);
            report.moved += 1;
        }

        // A full drone with a heap standing puts its load *there* rather than
        // running it home. That is the whole of stage 56's routing change: the
        // heap is nearer than the base and the spoil was never worth carrying
        // home anyway, and it is the reason a heap costs you the stone you
        // could have sold rather than costing you time.
        if self.drones[index].is_full() && self.heap_wants_blocks(world) {
            let job = self
                .drones[index]
                .job
                .and_then(|id| self.board.get(id).cloned())
                .filter(|job| job.kind == JobKind::Stack)
                .or_else(|| self.claim_stack(index));
            if let Some(job) = job {
                if self.stack_step(index, &job, world, events, report) {
                    return;
                }
            }
        }

        // Otherwise a full drone runs its load home — unless it cannot get
        // out, in which case it keeps cutting.
        //
        // That fallback is what makes the whole thing live. A drone working the
        // middle of a layer can fill up before it has cut its way back to the
        // bench, and stopping there would be a deadlock: it will not dig
        // because it is full, and it cannot leave because it has not dug. A
        // loaded machine that keeps cutting until it can get out is both the
        // obvious real answer and the one that always terminates, since digging
        // only ever adds routes. The cost is that a drone may come home over
        // its rated load, which is a far better failure than a frozen one.
        if self.drones[index].is_full() && self.haul(index, world, report) {
            return;
        }

        let Some(job) = self.current_job(index) else {
            // Nothing left to do. A part-load still has to come home, or the
            // last few blocks of every body would sit in a parked drone.
            if self.drones[index].carrying() > 0 && self.haul(index, world, report) {
                return;
            }
            self.drones[index].state = DroneState::Idle;
            return;
        };

        // **Which verb is this job?**
        //
        // Until stage 56 there was only one, so this branch did not exist and
        // `tick_drone` was kind-blind: an `Access` job and an `Extract` job get
        // the same treatment, namely "empty the region of breakables". The
        // moment a job means *fill* rather than *empty*, that stops being
        // harmless — the first version of this without the branch placed a
        // block, dropped below full, fell through to the cut path, and dug the
        // block it had just placed straight back out. Once a tick, for ever.
        if job.kind == JobKind::Stack {
            if self.stack_step(index, &job, world, events, report) {
                return;
            }
            // The heap turned the drone down — nothing aboard, or no route to
            // it. Fall through rather than return: a machine holding a load it
            // cannot stack still has somewhere to put it, and returning here
            // was a livelock. It claimed a stacking job, failed to route,
            // released it, returned, and did the whole thing again next tick,
            // for ever, carrying a part load it never took home.
            if self.drones[index].carrying() > 0 && self.haul(index, world, report) {
                return;
            }
            self.drones[index].state = DroneState::Idle;
            return;
        }

        // Cheap path: something to cut without moving.
        let region = job.region;
        if let Some(target) = self.drones[index].next_cut(world, &region) {
            match break_block(world, events, target) {
                Ok(removed) => {
                    let registry = world.registry();
                    if !self.drones[index].cargo.add_block(registry, removed, 1) {
                        log::warn!("drone dug an unregistered block at {target:?}");
                    }
                    self.drones[index].state = DroneState::Digging(job.id);
                    report.dug += 1;
                    // Take the trunk, take the tree: a felled tree's crown
                    // cannot be pruned from mid-air, so cutting any part of
                    // one brings the connected whole down into the cargo bed.
                    if crate::mine::is_vegetation_id(world.registry(), removed) {
                        report.dug += self.fell_tree(index, world, events, target);
                    }
                }
                Err(error) => {
                    // A veto — `next_cut` already refuses bedrock, so a mod
                    // said no. Remember the position so the drone moves on
                    // instead of chewing the same block until the end of time,
                    // which is exactly what the previous version did while its
                    // comment claimed otherwise.
                    log::debug!("drone denied at {target:?}: {error}");
                    self.drones[index].denied.insert(target);
                    self.drones[index].state = DroneState::Digging(job.id);
                }
            }
            return;
        }

        // Cached route still valid? While the world is unedited, the region's
        // remaining work cannot have changed either, so both the scan and the
        // field build below are skippable and travelling costs one step.
        if let Some(step) = self.cached_step(index, RouteKey::Work(job.id), world) {
            match step {
                Some(next) => {
                    self.drones[index].move_to(next);
                    self.drones[index].state = DroneState::Travelling(job.id);
                    report.moved += 1;
                }
                None => {
                    self.drones[index].route = None;
                    self.give_up(index, job.id);
                }
            }
            return;
        }

        // Slow path: is the job even still outstanding? This is the only place
        // the region is scanned, which is why the fast path above is worth
        // having.
        //
        // "Outstanding" counts *breakable* blocks. Bedrock does not count as
        // work, so a region overlapping the world floor completes once all the
        // rock that can come out has come out. Vegetation does not count
        // either: a crown hanging into a bench from a tree rooted elsewhere
        // has no cell to stand and prune it from, and it falls with its trunk
        // when any drone finally cuts one — it must never wedge a job open.
        let remaining: Vec<BlockPos> = region
            .clamped_to_world()
            .blocks()
            .filter(|pos| {
                crate::drone::is_breakable(world, *pos)
                    && !crate::mine::is_vegetation(world, *pos)
            })
            .collect();

        if remaining.is_empty() {
            self.board.complete(job.id);
            self.drones[index].job = None;
            self.drones[index].state = DroneState::Idle;
            report.completed += 1;
            return;
        }

        // Breakable work remains, but everything left may have been vetoed for
        // this drone. Release rather than complete — the blocks genuinely
        // remain, and claiming otherwise would lie — and the operation stalls
        // *visibly* with the job outstanding, which is the correct outcome for
        // "a mod forbids this dig".
        let workable: Vec<BlockPos> = remaining
            .iter()
            .copied()
            .filter(|pos| !self.drones[index].denied.contains(pos))
            .collect();
        if workable.is_empty() {
            self.give_up(index, job.id);
            return;
        }

        self.travel_to_work(index, world, &region, &workable, job.id, report);
    }

    /// The drone's claimed job, taking a new one if it has none.
    fn current_job(&mut self, index: usize) -> Option<Job> {
        let held = self.drones[index].job;
        if let Some(id) = held {
            if let Some(job) = self.board.get(id) {
                return Some(job.clone());
            }
            // Completed by someone else while this drone held the id.
            self.drones[index].job = None;
        }

        let from = self.drones[index].position;
        let id = self.drones[index].id;
        let job = self.board.claim_nearest(id, from)?;
        self.drones[index].job = Some(job.id);
        // Denials are per job, and survive re-claiming the *same* job so a
        // veto stays learned; a different job starts clean.
        if self.drones[index].denied_job != Some(job.id) {
            self.drones[index].denied.clear();
            self.drones[index].denied_job = Some(job.id);
        }
        Some(job)
    }

    /// Step one cell toward somewhere the job can be worked from.
    fn travel_to_work(
        &mut self,
        index: usize,
        world: &World,
        region: &VoxelAabb,
        remaining: &[BlockPos],
        job: crate::job::JobId,
        report: &mut TickReport,
    ) {
        // Anywhere a drone could stand and cut a block that is ready to be cut.
        // Built from the remaining blocks rather than from the region, so a
        // half-dug region does not send drones to its empty end — and built
        // with `stations_for`, which is the exact inverse of the reach rule, so
        // a drone that arrives can always actually work.
        //
        let position = self.drones[index].position;
        let goals: Vec<BlockPos> = remaining
            .iter()
            .flat_map(|pos| crate::drone::stations_for(world, *pos))
            .filter(|pos| flow::is_standable(world, *pos))
            // Never route a drone to where it already is. Standing on a station
            // it cannot work from is normal — the block under its feet may be
            // waiting on something still left above — and treating that as
            // "arrived" would strand it on the spot instead of sending it to
            // one of the other faces.
            .filter(|pos| *pos != position)
            .collect();
        let bounds = region
            .union(VoxelAabb::single(position))
            .expanded(8)
            .clamped_to_world();

        if goals.is_empty() || bounds.volume() > MAX_FIELD_CELLS {
            self.give_up(index, job);
            return;
        }

        let field = FlowField::build(world, bounds, goals);
        self.fields_built += 1;
        match field.step_from(world, position) {
            Some(next) => {
                self.drones[index].move_to(next);
                self.drones[index].state = DroneState::Travelling(job);
                self.drones[index].route = Some(CachedRoute {
                    key: RouteKey::Work(job),
                    edits: world.edit_count(),
                    field,
                });
                report.moved += 1;
            }
            None => self.give_up(index, job),
        }
    }

    /// One step from the drone's cached field, if the cache is still valid for
    /// `key`. `Some(None)` means "valid field, but no route from here" — which
    /// is an answer, not a miss.
    fn cached_step(
        &self,
        index: usize,
        key: RouteKey,
        world: &World,
    ) -> Option<Option<BlockPos>> {
        let cached = self.drones[index].route.as_ref()?;
        (cached.key == key && cached.edits == world.edit_count())
            .then(|| cached.field.step_from(world, self.drones[index].position))
    }

    /// Fell the vegetation connected to a just-cut block: flood out over
    /// touching vegetation cells, breaking each through the same cancellable
    /// event path as any other edit (a veto stops the spread through that
    /// cell), and load the lot. Bounded, so a modded megaflora cannot hold a
    /// tick hostage.
    fn fell_tree(
        &mut self,
        index: usize,
        world: &mut World,
        events: &EventBus,
        seed: BlockPos,
    ) -> u64 {
        const FELL_LIMIT: usize = 512;
        const SIDES: [[i32; 3]; 6] = [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ];

        let mut queue = vec![seed];
        let mut seen = std::collections::HashSet::from([seed]);
        let mut felled = 0u64;
        while let Some(at) = queue.pop() {
            for side in SIDES {
                let next = at.offset(side);
                if seen.len() >= FELL_LIMIT || !seen.insert(next) {
                    continue;
                }
                if !crate::mine::is_vegetation(world, next) {
                    continue;
                }
                let Ok(removed) = break_block(world, events, next) else {
                    continue;
                };
                if !self.drones[index].cargo.add_block(world.registry(), removed, 1) {
                    log::warn!("drone felled an unregistered block at {next:?}");
                }
                felled += 1;
                queue.push(next);
            }
        }
        felled
    }

    /// Hand the job back and mark the drone stuck.
    ///
    /// Releasing rather than completing matters: the work still needs doing,
    /// and a drone with a better route — or a later one, once more access is
    /// cut — should be able to pick it up.
    fn give_up(&mut self, index: usize, job: crate::job::JobId) {
        self.drones[index].route = None;
        self.board.release(job);
        self.drones[index].job = None;
        self.drones[index].state = DroneState::Stuck;
    }

    /// Move toward the stockpile, unloading on arrival.
    ///
    /// Returns whether the drone is dealing with its load. `false` means there
    /// is no route home at all, and the caller puts it back to work rather than
    /// letting it stand there full.
    fn haul(&mut self, index: usize, world: &World, report: &mut TickReport) -> bool {
        let position = self.drones[index].position;

        // The drop-off is where `home` *settles to today*, and the proximity
        // check and the navigation target must both use it. Checking closeness
        // against the unsettled `home` while walking toward the settled one
        // livelocked a drone the day the home column itself got dug away: it
        // stood exactly on the settled spot, forever four blocks from a point
        // in mid-air.
        let drop_off = flow::settle(world, self.home);
        let close = (position.x - drop_off.x).abs() <= DROP_OFF_RANGE
            && (position.y - drop_off.y).abs() <= DROP_OFF_RANGE
            && (position.z - drop_off.z).abs() <= DROP_OFF_RANGE;

        if close {
            let delivered = self.drones[index].cargo.total();
            let entries: Vec<(String, u64)> = self.drones[index]
                .cargo
                .entries()
                .map(|(name, count)| (name.to_string(), count))
                .collect();
            for (name, count) in entries {
                self.stockpile.add(name, count);
            }
            self.drones[index].cargo = Stockpile::new();
            self.drones[index].state = DroneState::Idle;
            report.delivered += delivered;
            return true;
        }

        // The homeward leg caches its field exactly like the outward one.
        if let Some(step) = self.cached_step(index, RouteKey::Home, world) {
            return match step {
                Some(next) => {
                    self.drones[index].move_to(next);
                    self.drones[index].state = DroneState::Hauling;
                    report.moved += 1;
                    true
                }
                None => {
                    self.drones[index].route = None;
                    false
                }
            };
        }

        let bounds = VoxelAabb::new(position, self.home)
            .expanded(12)
            .clamped_to_world();
        if bounds.volume() > MAX_FIELD_CELLS {
            return false;
        }

        let field = FlowField::build(world, bounds, [drop_off]);
        self.fields_built += 1;
        match field.step_from(world, position) {
            Some(next) => {
                self.drones[index].move_to(next);
                self.drones[index].state = DroneState::Hauling;
                self.drones[index].route = Some(CachedRoute {
                    key: RouteKey::Home,
                    edits: world.edit_count(),
                    field,
                });
                report.moved += 1;
                true
            }
            None => false,
        }
    }

    /// Blocks held by the operation and by every drone still carrying.
    ///
    /// The conservation figure: it must equal the blocks actually removed from
    /// the world, or work is being double-counted or dropped somewhere.
    /// Every block this operation has taken out of the ground and not lost.
    ///
    /// Three places until stage 56 — the mine-mouth pile and the drones' own
    /// cargo — and now a fourth: stacked into a spoil heap. A heap is not a
    /// leak, it is somewhere the blocks went, and saying so is what keeps this
    /// a conservation check rather than a number that quietly stops adding up
    /// the moment a crew builds anything.
    pub fn accounted_blocks(&self) -> u64 {
        self.stockpile.total()
            + self.drones.iter().map(Drone::carrying).sum::<u64>()
            + self.stacked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{ore_body, solid_count as solid_blocks};
    use crate::mine::{self, MineMethod};
    use vx_core::Cancellable;

    /// Wide enough that a decline's run-up lands inside the loaded world.
    const RADIUS: i32 = 8;

    fn flat_world(floor: i32) -> World {
        crate::fixture::flat(RADIUS, floor)
    }

    /// Count every solid block across a generous box around a work site.
    fn solid_total(world: &World, around: VoxelAabb) -> u64 {
        solid_blocks(world, around.expanded(30).clamped_to_world())
    }

    /// A crew, its board and its claims survive a snapshot exactly.
    ///
    /// The whole of stage 52 rests on this: until then a dispatch lived in a
    /// private field of `Mining` that no save file named, so buying drones and
    /// setting them cutting was work you lost the moment you quit.
    #[test]
    fn an_operation_round_trips_through_a_snapshot() {
        let mut operation = Operation::new(BlockPos::new(3, 64, -7));
        operation.add_drone(BlockPos::new(3, 64, -7));
        operation.add_drone(BlockPos::new(4, 64, -7));
        let job = operation.board.post(
            crate::job::JobKind::Extract,
            VoxelAabb::new(BlockPos::new(0, 40, 0), BlockPos::new(4, 44, 4)),
            3,
        );
        operation.board.claim_nearest(DroneId(1), BlockPos::new(4, 64, -7));
        operation.stockpile.add("engine:copper_ore", 9);
        operation.drones[0].cargo.add("engine:stone", 2);
        operation.drones[0].state = DroneState::Hauling;

        let snapshot = operation.snapshot();
        let back = Operation::restore(snapshot.clone());

        assert_eq!(back.home, operation.home);
        assert_eq!(back.stockpile.total(), 9);
        assert_eq!(back.drones.len(), 2);
        assert_eq!(back.drones[0].cargo.total(), 2);
        assert_eq!(back.drones[0].state, DroneState::Hauling);
        assert_eq!(
            back.board.claimant(job),
            Some(DroneId(1)),
            "the claim did not come back"
        );
        // And the snapshot of the restored operation is the same snapshot,
        // which is what makes saving twice write the same bytes.
        assert_eq!(back.snapshot(), snapshot);
    }

    /// **A heap is claimed from the ground up.**
    ///
    /// The plan is bottom-up and `PLACE_OFFSETS` is bottom-up, and neither of
    /// those matters if the *board* hands out the apex first. A heap is posted
    /// one job per course, and the order those jobs come off the board is the
    /// last link in the same safety argument: a drone sent seven blocks up to
    /// a cell with nothing under it finds no station it can stand on, gives
    /// the job back, and takes its load home — every tick, for ever.
    ///
    /// That is exactly what a played session did, and the trace named it:
    /// `1 workable, 0 goals` for a single cell at the top of an unbuilt
    /// pyramid. The crate tests did not catch it because they all built
    /// shafts, which are two courses tall on flat ground and reachable from
    /// the floor either way round. So: a shape with real height, and the
    /// courses checked in the order they are actually handed out.
    #[test]
    fn a_heap_is_claimed_from_the_ground_up() {
        let world = flat_world(64);
        let footprint = VoxelAabb::new(BlockPos::new(2, 0, 2), BlockPos::new(8, 0, 8));
        let plan = crate::heap::plan(&world, footprint, crate::heap::HeapShape::Pyramid)
            .expect("a pyramid on flat ground");
        assert!(
            plan.courses() > 2,
            "a heap this short cannot show an ordering"
        );

        let mut operation = Operation::new(BlockPos::new(0, 65, 0));
        operation.post_heap(&plan);

        let floor = plan.cells.first().expect("a planned cell").y;
        let from = BlockPos::new(0, 65, 0);
        let claimed: Vec<i32> = (0..plan.courses())
            .map(|n| {
                operation
                    .board
                    .claim_nearest_of(DroneId(n as u32), from, JobKind::Stack)
                    .expect("a course to claim")
                    .region
                    .min
                    .y
            })
            .collect();
        let expected: Vec<i32> = (0..plan.courses()).map(|step| floor + step).collect();
        assert_eq!(
            claimed, expected,
            "the courses came off the board out of order"
        );
    }

    /// **The yard hands the rock back.**
    ///
    /// Order a heap the day after the hole is finished and every block of
    /// spoil is already in town. Without this the order stands for ever with
    /// nothing happening — which is exactly what the `--heap` fixture did the
    /// first time it dug long enough to finish the dispatch.
    #[test]
    fn spoil_comes_back_out_of_the_yard_for_a_heap() {
        let world = flat_world(64);
        let footprint = VoxelAabb::new(BlockPos::new(2, 0, 2), BlockPos::new(6, 0, 6));
        let plan = crate::heap::plan(&world, footprint, crate::heap::HeapShape::Shaft)
            .expect("a shaft on flat ground");

        let mut operation = Operation::new(BlockPos::new(0, 65, 0));
        operation.add_drone(BlockPos::new(0, 65, 0));
        operation.post_heap(&plan);

        let mut yard = Stockpile::new();
        yard.add("engine:stone", 200);
        // The ore stays where it is: a heap costs you the stone you could
        // have sold, not the copper.
        yard.add("engine:copper_ore", 40);

        let back = operation.fetch_spoil(&world, &mut yard, 64, |_| true);
        assert_eq!(back, 64, "the yard kept the spoil");
        assert_eq!(operation.stockpile.count("engine:stone"), 64);
        assert_eq!(yard.count("engine:stone"), 136);
        assert_eq!(yard.count("engine:copper_ore"), 40, "the ore went to the heap");

        // And it does not keep dragging rock out of town on top of what the
        // crew already has at the mine mouth.
        assert_eq!(operation.fetch_spoil(&world, &mut yard, 64, |_| true), 0);
    }

    /// No heap, no fetch: a crew with nothing to build takes nothing back.
    #[test]
    fn the_yard_keeps_its_rock_when_no_heap_is_ordered() {
        let world = flat_world(64);
        let mut operation = Operation::new(BlockPos::new(0, 65, 0));
        operation.add_drone(BlockPos::new(0, 65, 0));
        let mut yard = Stockpile::new();
        yard.add("engine:stone", 200);
        assert_eq!(operation.fetch_spoil(&world, &mut yard, 64, |_| true), 0);
        assert_eq!(yard.count("engine:stone"), 200);
    }

    /// **The crew can put a block back.**
    ///
    /// Fifty-five stages of this crate could only ever take blocks away, and
    /// the argument that everything terminates rested on it: digging only adds
    /// routes, so a stuck drone can always dig itself out. This is the first
    /// test that asks a machine to make the world *bigger*, and it checks the
    /// three things that matter — the blocks appear, they come out of the
    /// drone rather than out of nowhere, and the drone is still standing
    /// somewhere afterwards.
    #[test]
    fn a_drone_stacks_what_it_is_carrying_into_the_heap() {
        let mut world = flat_world(64);
        let events = EventBus::new();
        let footprint = VoxelAabb::new(BlockPos::new(2, 0, 2), BlockPos::new(4, 0, 4));
        let plan = crate::heap::plan(&world, footprint, crate::heap::HeapShape::Shaft)
            .expect("a shaft on flat ground");

        let mut operation = Operation::new(BlockPos::new(0, 65, 0));
        operation.add_drone(BlockPos::new(0, 65, 0));
        operation.post_heap(&plan);
        // A full load of spoil, the way a drone comes off a face.
        let capacity = operation.drones[0].capacity;
        operation.drones[0].cargo.add("engine:stone", capacity);
        let carried = operation.drones[0].carrying();

        let before = solid_blocks(&world, plan.span());
        let mut placed = 0;
        for _ in 0..600 {
            let report = operation.tick(&mut world, &events);
            placed += report.placed;
            if operation.drones[0].carrying() == 0 {
                break;
            }
        }

        assert!(placed > 0, "the crew never put a single block down");
        let after = solid_blocks(&world, plan.span());
        assert_eq!(
            after - before,
            placed,
            "the world gained a different number of blocks than the crew placed"
        );
        // Cargo bed, heap, or mine-mouth pile: three honest places, and no
        // fourth. A drone that finishes the heap with rock to spare runs the
        // rest home rather than standing on it, so the pile is part of the
        // identity and not an escape from it.
        assert_eq!(
            operation.drones[0].carrying() + placed + operation.stockpile.total(),
            carried,
            "blocks were conjured or lost between the cargo bed and the ground"
        );
        assert_eq!(operation.stacked, placed);
        assert!(
            plan.cells
                .iter()
                .filter(|cell| world.is_solid(**cell))
                .count() as u64
                >= placed,
            "blocks landed somewhere the plan did not ask for"
        );
        // And the machine is still on solid ground rather than sealed in.
        assert!(
            flow::is_standable(&world, operation.drones[0].position),
            "the drone built itself somewhere it cannot stand"
        );
    }

    /// A drone never fills the cell it is standing in.
    ///
    /// `place_at`'s obstruction predicate is what guarantees it, and this is
    /// the test that says the predicate is actually wired up — the failure it
    /// catches is a machine that seals itself into the heap it is building.
    #[test]
    fn a_drone_never_stacks_a_block_into_itself() {
        let mut world = flat_world(64);
        let events = EventBus::new();
        // A footprint the drone is standing right in the middle of.
        let footprint = VoxelAabb::new(BlockPos::new(-2, 0, -2), BlockPos::new(2, 0, 2));
        let plan = crate::heap::plan(&world, footprint, crate::heap::HeapShape::Shaft)
            .expect("a shaft on flat ground");

        let mut operation = Operation::new(BlockPos::new(0, 65, 0));
        operation.add_drone(BlockPos::new(0, 65, 0));
        operation.post_heap(&plan);
        operation.drones[0].cargo.add("engine:stone", 200);

        for _ in 0..400 {
            operation.tick(&mut world, &events);
            let standing = operation.drones[0].position;
            assert!(
                !world.is_solid(standing),
                "the drone is inside a block it placed at {standing:?}"
            );
        }
    }

    /// Building counts as work. Without `TickReport::placed` a crew doing
    /// nothing but stacking reports an idle tick every tick, and `run` calls a
    /// perfectly healthy operation `Stalled`.
    #[test]
    fn stacking_is_not_an_idle_tick() {
        let mut world = flat_world(64);
        let events = EventBus::new();
        let footprint = VoxelAabb::new(BlockPos::new(3, 0, 3), BlockPos::new(6, 0, 6));
        let plan = crate::heap::plan(&world, footprint, crate::heap::HeapShape::Shaft)
            .expect("a shaft on flat ground");

        let mut operation = Operation::new(BlockPos::new(0, 65, 0));
        operation.add_drone(BlockPos::new(3, 65, 3));
        operation.post_heap(&plan);
        operation.drones[0].cargo.add("engine:stone", 64);

        let mut placing = None;
        for _ in 0..400 {
            let report = operation.tick(&mut world, &events);
            if report.placed > 0 {
                placing = Some(report);
                break;
            }
        }
        let report = placing.expect("the crew never placed anything to report on");
        assert!(!report.is_idle(), "a tick that built something read as idle");
    }

    /// The cached route is *not* in it, and must not be: it holds a whole flow
    /// field keyed on the world's edit count, which the next broken block
    /// invalidates anyway.
    #[test]
    fn a_restored_drone_carries_no_stale_route() {
        let mut operation = Operation::new(BlockPos::new(0, 64, 0));
        operation.add_drone(BlockPos::new(0, 64, 0));
        let back = Operation::restore(operation.snapshot());
        assert!(
            back.drones[0].route.is_none(),
            "a route survived a snapshot"
        );
    }

    struct Site {
        world: World,
        operation: Operation,
        plan: MinePlan,
    }

    /// A named site: a world to build and the body buried in it.
    type Case = (MineMethod, fn() -> World, VoxelAabb);

    /// A world, a plan and a drone at the portal, ready to dig.
    fn site(floor: i32, body: VoxelAabb, method: MineMethod) -> Site {
        site_in(flat_world(floor), body, method)
    }

    fn site_in(mut world: World, body: VoxelAabb, method: MineMethod) -> Site {
        ore_body(&mut world, body);

        let plan = mine::plan(&world, body, 3, method)
            .unwrap_or_else(|| panic!("no {} plan for this body", method.name()));

        let start = flow::settle(&world, plan.portal);
        let mut operation = Operation::new(start);
        operation.add_drone(start);
        operation.post_plan(&plan);

        Site {
            world,
            operation,
            plan,
        }
    }

    #[test]
    fn posting_a_plan_puts_access_ahead_of_extraction() {
        let site = site(60, VoxelAabb::new(BlockPos::new(0, 40, 0), BlockPos::new(2, 42, 2)), MineMethod::Decline);
        let access: Vec<i32> = site
            .operation
            .board
            .jobs()
            .filter(|job| job.kind == JobKind::Access)
            .map(|job| job.priority)
            .collect();
        let extract: Vec<i32> = site
            .operation
            .board
            .jobs()
            .filter(|job| job.kind == JobKind::Extract)
            .map(|job| job.priority)
            .collect();

        assert!(!access.is_empty() && !extract.is_empty());
        assert!(
            access.iter().min() > extract.iter().max(),
            "extraction outranks access somewhere: {access:?} against {extract:?}"
        );
    }

    #[test]
    fn the_outermost_access_cut_is_claimed_first() {
        // Cutting a ramp from the bottom up is not a thing a drone can do.
        let mut site = site(60, VoxelAabb::new(BlockPos::new(0, 40, 0), BlockPos::new(2, 42, 2)), MineMethod::Decline);
        let first = site.plan.access[0];
        let claimed = site
            .operation
            .board
            .claim_nearest(DroneId(0), site.plan.portal)
            .unwrap();
        assert_eq!(claimed.region, first);
    }

    #[test]
    fn a_drone_clears_a_marked_body_and_the_haul_is_tallied() {
        // The milestone, end to end: mark a body, the game plans the mine, a
        // drone cuts its way in, digs it out, and the pile matches.
        let body = VoxelAabb::new(BlockPos::new(0, 52, 0), BlockPos::new(3, 55, 3));
        let mut site = site(60, body, MineMethod::Decline);
        let events = EventBus::new();

        let before = solid_total(&site.world, site.plan.access.iter().fold(body, |a, b| a.union(*b)));
        let (outcome, ticks) = site.operation.run(&mut site.world, &events, 200_000);

        assert_eq!(
            outcome,
            RunOutcome::Finished,
            "the operation did not finish after {ticks} ticks \
             ({} jobs left, drone {:?})",
            site.operation.board.len(),
            site.operation.drones[0].state
        );

        assert_eq!(
            solid_blocks(&site.world, body),
            0,
            "the body still has ore in it"
        );

        let after = solid_total(&site.world, site.plan.access.iter().fold(body, |a, b| a.union(*b)));
        let removed = before - after;
        assert_eq!(
            site.operation.accounted_blocks(),
            removed,
            "the pile holds {} but {removed} blocks left the world: jobs are being \
             double-counted or dropped",
            site.operation.accounted_blocks()
        );
        assert!(
            site.operation.stockpile.count("engine:copper_ore") >= body.volume(),
            "only {} ore reached the pile from a {}-block body",
            site.operation.stockpile.count("engine:copper_ore"),
            body.volume()
        );
        eprintln!("decline: {ticks} ticks, {removed} blocks moved");
    }

    #[test]
    fn every_method_gets_the_ore_out() {
        // The reachability invariant, proved the hard way: not "a flow field
        // says it could", but "a drone actually did". All three excavation
        // shapes, one drone, ore in the pile at the end or the test fails.
        let cases: [Case; 3] = [
            (
                MineMethod::Pit,
                || flat_world(60),
                VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            ),
            (
                MineMethod::Decline,
                || flat_world(60),
                VoxelAabb::new(BlockPos::new(0, 44, 0), BlockPos::new(2, 46, 2)),
            ),
            (
                MineMethod::Adit,
                || crate::fixture::slope(RADIUS, 60, 0, 2),
                VoxelAabb::new(BlockPos::new(-12, 30, 0), BlockPos::new(-10, 33, 2)),
            ),
        ];

        for (method, build, body) in cases {
            let mut site = site_in(build(), body, method);
            let events = EventBus::new();
            let (outcome, ticks) = site.operation.run(&mut site.world, &events, 200_000);

            assert_eq!(
                outcome,
                RunOutcome::Finished,
                "{}: stopped after {ticks} ticks with the drone {:?}",
                method.name(),
                site.operation.drones[0].state
            );
            assert_eq!(
                solid_blocks(&site.world, body),
                0,
                "{}: ore left in the ground",
                method.name()
            );
            eprintln!("{}: {ticks} ticks", method.name());
        }
    }

    #[test]
    fn the_drone_comes_home_rather_than_stranding_itself() {
        // The reason climb and drop share a limit. A drone that dug its way
        // down and could not get back would show up here as an unfinished run
        // with cargo still aboard.
        let body = VoxelAabb::new(BlockPos::new(0, 40, 0), BlockPos::new(3, 44, 3));
        let mut site = site(60, body, MineMethod::Decline);
        site.operation.drones[0].capacity = 12; // force several round trips
        let events = EventBus::new();

        let (outcome, _) = site.operation.run(&mut site.world, &events, 400_000);
        assert_eq!(outcome, RunOutcome::Finished);
        assert_eq!(site.operation.drones[0].carrying(), 0, "cargo never got home");
        assert!(site.operation.stockpile.total() > 50);
    }

    #[test]
    fn waste_rock_and_ore_stay_separate_in_the_pile() {
        // Opening a mine moves a lot of nothing. Keeping the two apart is what
        // will let a later readout say what the operation actually cost.
        let body = VoxelAabb::new(BlockPos::new(0, 44, 0), BlockPos::new(2, 46, 2));
        let mut site = site(60, body, MineMethod::Decline);
        let events = EventBus::new();
        site.operation.run(&mut site.world, &events, 200_000);

        let ore = site.operation.stockpile.count("engine:copper_ore");
        let stone = site.operation.stockpile.count("engine:stone");
        assert!(ore > 0, "no ore in the pile");
        assert!(stone > 0, "no waste rock in the pile; the ramp dug itself?");
        assert!(
            stone > ore,
            "a ramp {} blocks down should move more waste ({stone}) than ore ({ore})",
            60 - body.max.y
        );
    }

    #[test]
    fn a_mod_can_veto_a_drone_exactly_as_it_vetoes_a_player() {
        // Drone digging goes through the same `break_block` the player uses, so
        // the cancellable event already covers it with no new plumbing. Worth
        // pinning: it is the whole reason digging was not given its own path.
        let body = VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2));
        let mut site = site(60, body, MineMethod::Pit);

        let mut events = EventBus::new();
        events.subscribe("guard", |event: &mut vx_world::BlockBreakEvent| {
            event.cancel();
        });

        let before = solid_blocks(&site.world, body);
        site.operation.run(&mut site.world, &events, 2_000);

        assert_eq!(
            solid_blocks(&site.world, body),
            before,
            "the veto was ignored and the drone dug anyway"
        );
        assert_eq!(site.operation.accounted_blocks(), 0);
    }

    /// Dig a fixed body in a *generated* world, having loaded `resident`
    /// chunks around the origin up front, and report what the ground looks
    /// like afterwards.
    ///
    /// The point of the parameter is that it should not matter. Pinning loads
    /// whatever the work needs; `resident` only changes how much was already
    /// there when the job was dispatched — which, in the running game, is
    /// decided by where the player happened to be standing.
    fn dig_with_residency(resident: i32) -> (u64, u64, BlockPos) {
        let mut world = World::new(2024);
        world.load_around(vx_core::ChunkPos::new(0, 0), resident);

        // A body a few blocks under the surface, in real generated terrain.
        let ground = world.generator().height_at(4, 4);
        let body = VoxelAabb::new(
            BlockPos::new(2, ground - 7, 2),
            BlockPos::new(5, ground - 5, 5),
        );

        // Planning reads the world, so the ground it reads must be resident
        // first — exactly as `Mining::mark` now does.
        let scouted = crate::working_span(body, body.min);
        world.pin_span(scouted.min, scouted.max);

        let plan = mine::plan(&world, body, 3, MineMethod::Pit)
            .expect("no pit plan for this body");
        let start = flow::settle(&world, plan.portal);
        let span = crate::working_span(plan.span(), start);
        world.pin_span(span.min, span.max);

        let mut operation = Operation::new(start);
        operation.add_drone(start);
        operation.post_plan(&plan);

        let events = EventBus::new();
        for _ in 0..1_500 {
            operation.tick(&mut world, &events);
        }

        (
            vx_world::region_hash(&world, span.min, span.max),
            operation.stockpile.total(),
            operation.drones[0].position,
        )
    }

    #[test]
    fn an_excavation_is_the_same_however_much_of_the_world_was_resident() {
        // The gate. `World::block` reports unloaded chunks as air, and agents
        // read through it, so before pinning a drone's decisions depended on
        // which chunks the camera happened to have streamed in. Two runs of the
        // same dispatch could diverge purely because the player stood somewhere
        // different — not a crash, but the end of any claim that this
        // simulation is reproducible.
        //
        // Pinned, the dispatch is closed over its own ground: the same order
        // digs the same hole whether the world around it was fully resident or
        // barely there at all.
        let barely = dig_with_residency(0);
        let fully = dig_with_residency(4);

        assert_eq!(
            barely, fully,
            "the same excavation came out differently depending on what was loaded"
        );
    }

    /// Run one excavation with `crew` drones for `ticks` and report what came
    /// of it. Kept short: the claim invariant is checked every tick, and at
    /// sixty-four drones an unoptimised build feels every one of them.
    fn dig_with_crew(crew: u32, ticks: u32) -> (u64, u64, usize) {
        let mut world = World::new(2024);
        world.load_around(vx_core::ChunkPos::new(0, 0), 2);
        let ground = world.generator().height_at(4, 4);
        let body = VoxelAabb::new(
            BlockPos::new(1, ground - 8, 1),
            BlockPos::new(7, ground - 5, 7),
        );

        let scouted = crate::working_span(body, body.min);
        world.pin_span(scouted.min, scouted.max);
        let plan = mine::plan(&world, body, 3, MineMethod::Pit).expect("no pit plan");
        let start = flow::settle(&world, plan.portal);
        let span = crate::working_span(plan.span(), start);
        world.pin_span(span.min, span.max);

        let mut operation = Operation::new(start);
        for _ in 0..crew {
            operation.add_drone(start);
        }
        operation.post_plan(&plan);

        let events = EventBus::new();
        for _ in 0..ticks {
            operation.tick(&mut world, &events);

            // The invariant the board has never had to keep: one job, one
            // drone. Checked every tick rather than at the end, because a
            // double-claim that resolves itself would otherwise go unseen.
            let mut claimed: Vec<crate::JobId> = operation
                .drones
                .iter()
                .filter_map(|drone| match drone.state {
                    DroneState::Travelling(job) | DroneState::Digging(job) => Some(job),
                    _ => None,
                })
                .collect();
            let before = claimed.len();
            claimed.sort_by_key(|job| job.0);
            claimed.dedup_by_key(|job| job.0);
            assert_eq!(
                before,
                claimed.len(),
                "two drones of a crew of {crew} claimed the same job"
            );
        }

        // Hash the excavation, not the whole pinned span: the span includes
        // the haul-route margin, which is hundreds of thousands of untouched
        // blocks and dominates the test's runtime for no extra coverage.
        let cut = plan.span();
        (
            vx_world::region_hash(&world, cut.min, cut.max),
            operation.accounted_blocks(),
            operation.drones.len(),
        )
    }

    #[test]
    fn a_crew_never_double_claims_and_stays_deterministic() {
        // The job board was built for a swarm — claims, releases, nearest-first
        // — and until now exactly one drone ever existed, so none of it ran.
        // `dig_with_crew` asserts the one-job-one-drone invariant every tick.
        //
        // Tick counts are per crew size on purpose: contention shows up in the
        // first few dozen ticks, when every drone is scrambling for the same
        // outermost access cuts, so sixty-four of them need very few. Progress
        // and determinism need a run long enough to dig, but only a handful of
        // machines. Sixty-four drones for six hundred ticks would be four
        // times this test's whole runtime and prove nothing extra.
        let (_, dug_1, count_1) = dig_with_crew(1, 250);
        let (hash_8, dug_8, count_8) = dig_with_crew(8, 250);
        let (_, dug_64, count_64) = dig_with_crew(64, 40);

        assert_eq!((count_1, count_8, count_64), (1, 8, 64));
        assert!(dug_1 > 0 && dug_8 > 0 && dug_64 > 0, "nobody dug anything");

        // A bigger crew clears more of the same hole in the same time — not a
        // different hole, and never less of one.
        assert!(
            dug_8 >= dug_1,
            "eight drones ({dug_8}) moved less than one ({dug_1})"
        );

        // And a crew size is reproducible: same seed, same count, same ground.
        assert_eq!(
            hash_8,
            dig_with_crew(8, 250).0,
            "a crew of eight is not deterministic"
        );
    }

    #[test]
    fn digging_is_deterministic() {
        // Same seed, same plan, same tick count, same hole. Without this the
        // conservation checks above could pass by luck.
        let body = VoxelAabb::new(BlockPos::new(0, 50, 0), BlockPos::new(2, 52, 2));
        let outcome: Vec<(u64, u64, BlockPos)> = (0..2)
            .map(|_| {
                let mut site = site(60, body, MineMethod::Decline);
                let events = EventBus::new();
                for _ in 0..500 {
                    site.operation.tick(&mut site.world, &events);
                }
                (
                    site.operation.stockpile.total(),
                    site.operation.accounted_blocks(),
                    site.operation.drones[0].position,
                )
            })
            .collect();

        assert_eq!(outcome[0], outcome[1], "two identical runs diverged");
    }

    #[test]
    fn a_drone_with_nothing_to_do_goes_idle_rather_than_spinning() {
        let mut world = flat_world(60);
        let events = EventBus::new();
        let mut operation = Operation::new(BlockPos::new(0, 61, 0));
        operation.add_drone(BlockPos::new(0, 61, 0));

        let report = operation.tick(&mut world, &events);
        assert!(report.is_idle());
        assert_eq!(operation.drones[0].state, DroneState::Idle);
    }

    #[test]
    fn work_it_cannot_reach_is_given_back_to_the_board() {
        // A released job is one another drone — or the same one, after more
        // access is cut — can still take. Completing it would lose the work.
        let mut world = flat_world(60);
        let events = EventBus::new();
        let mut operation = Operation::new(BlockPos::new(0, 61, 0));
        operation.add_drone(BlockPos::new(0, 61, 0));

        // A region of solid rock sealed under the surface, far from any
        // excavation, with no route to it.
        let sealed = VoxelAabb::new(BlockPos::new(20, 30, 20), BlockPos::new(22, 32, 22));
        let id = operation.board.post(JobKind::Extract, sealed, 0);

        operation.tick(&mut world, &events);
        assert_eq!(operation.drones[0].state, DroneState::Stuck);
        assert!(operation.board.get(id).is_some(), "the job was thrown away");
        assert!(
            operation.board.claimant(id).is_none(),
            "the job is still held by a drone that cannot do it"
        );
    }

    #[test]
    fn a_long_approach_builds_one_field_not_one_per_step() {
        // Finding A6. Travelling changes nothing about the world, so a whole
        // leg should reuse one cached field; rebuilding a full BFS every step
        // was the single biggest cost in the loop. The counter is the guard
        // that keeps the cache honest.
        let mut world = flat_world(60);
        let region = VoxelAabb::new(BlockPos::new(30, 60, 0), BlockPos::new(32, 60, 2));
        let start = BlockPos::new(-30, 61, 1);

        let mut operation = Operation::new(start);
        operation.add_drone(start);
        operation.board.post(JobKind::Extract, region, 0);
        let events = EventBus::new();

        // Walk the ~60-block approach until the first dig happens.
        let mut ticks = 0;
        while operation.stockpile.total() + operation.drones[0].carrying() == 0 {
            operation.tick(&mut world, &events);
            ticks += 1;
            assert!(ticks < 2_000, "never reached the work");
        }

        assert!(ticks > 40, "the approach was too short to prove anything");
        assert!(
            operation.fields_built <= 3,
            "{} fields built over a {ticks}-tick approach; the route cache is not caching",
            operation.fields_built
        );
    }

    #[test]
    fn taking_control_releases_the_claimed_job_back_to_the_board() {
        let mut site = site(
            60,
            VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            MineMethod::Decline,
        );
        let events = EventBus::new();
        // Let the drone claim something first.
        for _ in 0..40 {
            site.operation.tick(&mut site.world, &events);
        }
        assert!(site.operation.drones[0].job().is_some(), "never claimed a job");
        let unclaimed_before = site.operation.board.unclaimed_count();

        assert!(site.operation.take_control(0));
        assert_eq!(site.operation.controlled(), Some(0));
        assert_eq!(site.operation.drones[0].state, DroneState::Manual);
        assert_eq!(site.operation.drones[0].job(), None, "kept holding the job");
        assert_eq!(
            site.operation.board.unclaimed_count(),
            unclaimed_before + 1,
            "the job did not go back on the board"
        );
        // The board still owes the work — released, never completed.
        assert!(!site.operation.board.is_empty());
    }

    #[test]
    fn a_controlled_drone_is_skipped_by_the_tick() {
        let mut site = site(
            60,
            VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            MineMethod::Decline,
        );
        let events = EventBus::new();
        site.operation.take_control(0);
        let parked = site.operation.drones[0].position;

        for _ in 0..200 {
            site.operation.tick(&mut site.world, &events);
        }
        assert_eq!(
            site.operation.drones[0].position, parked,
            "the AI drove a drone the player was holding"
        );
        assert_eq!(site.operation.drones[0].state, DroneState::Manual);
    }

    #[test]
    fn another_drone_can_claim_the_job_a_takeover_released() {
        let mut site = site(
            60,
            VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            MineMethod::Decline,
        );
        let events = EventBus::new();
        let start = site.operation.drones[0].position;
        site.operation.add_drone(start);

        for _ in 0..40 {
            site.operation.tick(&mut site.world, &events);
        }
        site.operation.take_control(0);
        for _ in 0..80 {
            site.operation.tick(&mut site.world, &events);
        }
        assert!(
            site.operation.drones[1].job().is_some(),
            "the second drone never picked up the released work"
        );
    }

    #[test]
    fn handing_back_lets_the_drone_reclaim_and_finish_the_dig() {
        // The claim the whole override rests on: a mine interrupted by a
        // joyride still completes.
        let mut site = site(
            60,
            VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            MineMethod::Decline,
        );
        let events = EventBus::new();
        for _ in 0..60 {
            site.operation.tick(&mut site.world, &events);
        }

        assert!(site.operation.take_control(0));
        // Drive it somewhere unhelpful for a while.
        let command = crate::pilot::PilotCommand {
            heading: Some(crate::pilot::Heading::PosX),
            ..Default::default()
        };
        for _ in 0..30 {
            site.operation.pilot_tick(&mut site.world, &events, command);
        }
        assert!(site.operation.release_control(0));
        assert_eq!(site.operation.drones[0].state, DroneState::Idle);

        let (outcome, _) = site.operation.run(&mut site.world, &events, 400_000);
        assert_eq!(outcome, RunOutcome::Finished, "the dig never recovered");
    }

    #[test]
    fn a_pilot_cut_loads_the_drone_s_own_cargo_and_conserves_blocks() {
        let mut site = site(
            60,
            VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            MineMethod::Decline,
        );
        let events = EventBus::new();
        // Generous: the decline's portal can sit well away from the body, and
        // a box that misses where the drone actually stands would count the
        // wrong blocks.
        let region = VoxelAabb::new(BlockPos::new(-80, 1, -80), BlockPos::new(80, 120, 80));
        let before = solid_blocks(&site.world, region);

        site.operation.take_control(0);
        let command = crate::pilot::PilotCommand { cut: true, ..Default::default() };
        let mut dug = 0;
        for _ in 0..20 {
            dug += site.operation.pilot_tick(&mut site.world, &events, command).dug;
        }

        assert!(dug > 0, "the cutter never took anything");
        assert_eq!(
            site.operation.drones[0].carrying(),
            dug,
            "cut blocks did not land in the drone's own bed"
        );
        assert_eq!(
            before - solid_blocks(&site.world, region),
            dug,
            "blocks left the world without being accounted for"
        );
    }

    #[test]
    fn a_full_drone_refuses_to_cut_and_reports_blocked() {
        let mut site = site(
            60,
            VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            MineMethod::Decline,
        );
        let events = EventBus::new();
        site.operation.take_control(0);
        site.operation.drones[0].capacity = 1;
        let command = crate::pilot::PilotCommand { cut: true, ..Default::default() };

        // First cut fills it; the next must be refused rather than silently
        // overloading the machine.
        site.operation.pilot_tick(&mut site.world, &events, command);
        let report = site.operation.pilot_tick(&mut site.world, &events, command);
        assert_eq!(report.dug, 0);
        assert!(report.blocked, "a full drone kept cutting");
    }

    #[test]
    fn a_pilot_cut_goes_through_break_block_so_a_mod_veto_still_wins() {
        // Piloting is not a way around a mod's veto.
        let mut site = site(
            60,
            VoxelAabb::new(BlockPos::new(0, 54, 0), BlockPos::new(2, 56, 2)),
            MineMethod::Decline,
        );
        let mut events = EventBus::new();
        events.subscribe("guard", |event: &mut vx_world::BlockBreakEvent| {
            event.cancel();
        });

        site.operation.take_control(0);
        let region = VoxelAabb::new(BlockPos::new(-80, 1, -80), BlockPos::new(80, 120, 80));
        let before = solid_blocks(&site.world, region);
        let command = crate::pilot::PilotCommand { cut: true, ..Default::default() };

        for _ in 0..10 {
            let report = site.operation.pilot_tick(&mut site.world, &events, command);
            assert_eq!(report.dug, 0, "the veto was ignored");
            assert!(report.blocked);
        }
        assert_eq!(before, solid_blocks(&site.world, region), "the world changed");
    }

    #[test]
    fn a_region_floored_with_bedrock_still_completes() {
        // Finding A1, half one: unbreakable blocks are not work. A job whose
        // region contains bedrock must finish once everything that *can* come
        // out has, rather than chewing the unbreakable block forever.
        //
        // A single surface layer, deliberately: a plain posted box with depth
        // strands the drone in its own straight-sided hole, which is exactly
        // why real excavations come from benched mine plans.
        let mut world = flat_world(60);
        let bedrock = world.registry().id_of("engine:bedrock").unwrap();
        let region = VoxelAabb::new(BlockPos::new(0, 60, 0), BlockPos::new(2, 60, 2));
        let unbreakable = BlockPos::new(1, 60, 1);
        world.set_block(unbreakable, bedrock);

        let start = BlockPos::new(-2, 61, 1);
        let mut operation = Operation::new(start);
        operation.add_drone(start);
        operation.board.post(JobKind::Extract, region, 0);

        let events = EventBus::new();
        let (outcome, ticks) = operation.run(&mut world, &events, 20_000);

        assert_eq!(
            outcome,
            RunOutcome::Finished,
            "stalled after {ticks} ticks with the drone {:?}",
            operation.drones[0].state
        );
        // The eight stone blocks came out; the bedrock did not, and the job
        // still counts as done.
        assert_eq!(operation.stockpile.count("engine:stone"), 8);
        assert_eq!(world.block(unbreakable), bedrock);
    }

    #[test]
    fn a_vetoed_block_is_skipped_and_the_job_is_never_falsely_completed() {
        // Finding A1, half two: a mod veto is dynamic, so the drone learns the
        // position, digs everything else, and *releases* the job rather than
        // completing it — the block genuinely remains, and the visible stall is
        // the honest outcome of "a mod forbids this dig".
        let mut world = flat_world(60);
        let forbidden = BlockPos::new(1, 60, 1);
        let region = VoxelAabb::new(BlockPos::new(0, 60, 0), BlockPos::new(2, 60, 2));

        let mut events = EventBus::new();
        events.subscribe("guard", move |event: &mut vx_world::BlockBreakEvent| {
            if event.position == forbidden {
                event.cancel();
            }
        });

        let start = BlockPos::new(-2, 61, 1);
        let mut operation = Operation::new(start);
        operation.add_drone(start);
        let id = operation.board.post(JobKind::Extract, region, 0);

        let (outcome, _) = operation.run(&mut world, &events, 20_000);

        // Not Finished — and crucially not an endless loop: the run terminates
        // as a visible stall with the job still on the board.
        assert_eq!(outcome, RunOutcome::Stalled);
        assert!(
            operation.board.get(id).is_some(),
            "the job was completed or lost while its block still stands"
        );
        assert!(world.is_solid(forbidden), "the veto was ignored");
        // Everything the mod allowed came out.
        let left: Vec<BlockPos> = region
            .blocks()
            .filter(|pos| world.is_solid(*pos))
            .collect();
        assert_eq!(left, vec![forbidden], "more than the vetoed block remains");
        assert_eq!(operation.stockpile.total() + operation.drones[0].carrying(), 8);
    }

    #[test]
    fn a_drone_never_falls_further_than_it_can_climb() {
        // The invariant that keeps a drone recoverable. Undermining itself by
        // one block is allowed and is how it descends; dropping further is how
        // it ends up at the bottom of its own hole with no way back, and the
        // first anyone would know is a haul that never arrives.
        let body = VoxelAabb::new(BlockPos::new(0, 50, 0), BlockPos::new(3, 54, 3));
        let mut site = site(60, body, MineMethod::Decline);
        let events = EventBus::new();

        for _ in 0..5_000 {
            let before = site.operation.drones[0].position;
            site.operation.tick(&mut site.world, &events);
            let after = site.operation.drones[0].position;

            assert!(
                before.y - after.y <= flow::STEP,
                "the drone fell {} blocks in one tick, from {before:?} to {after:?}",
                before.y - after.y
            );
        }
    }
}
