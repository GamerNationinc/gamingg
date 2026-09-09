//! Marking a body, choosing how to mine it, and watching a drone do it.
//!
//! This is the thin layer between the player and [`vx_agent`], which knows
//! nothing about windows, input or rendering. Everything here is presentation
//! and intent: pick two corners, look at what the game proposes, disagree if
//! you like, then press go.
//!
//! # The game proposes and the player disposes
//!
//! [`vx_agent::options`] ranks every method that applies, cheapest first, and
//! the top one is preselected. Cycling through the rest shows what disagreeing
//! costs. That is deliberately a *suggestion*: the ranking is a defensible
//! model, not a fact, and a player who wants a pit because a pit looks better
//! should be able to have one.

use std::time::Duration;

use glam::{DVec3, Vec3};
use vx_agent::{
    options, DroneState, Fleet, FlierState, MineMethod, MinePlan, Operation, PilotCommand, Sector,
    VoxelAabb,
};
use vx_core::{BlockPos, EventBus};
use vx_agent::FleetReport;
use vx_render::tiles::slot;
use vx_render::Object;
use vx_world::World;

use crate::rig::{self, Rig};

/// Drone ticks per second — the journal's clock.
///
/// A drone cuts one block per tick, so this is also its digging rate. Slow
/// enough to watch the excavation take shape, which is most of the appeal.
/// Crate-visible because the arsenal steps its projectiles on the same clock:
/// one tick of `Advance` is one tick of everything.
pub(crate) const TICK_RATE: f64 = 8.0;

/// Blocks of run per block of rise the starting drone's ramps are cut to.
/// Turn `from` toward `to` by `fraction`, the short way round.
///
/// Angles wrap, so a nose swinging from just under π to just over -π is a
/// hair's movement and must not be drawn as very nearly a full revolution.
/// Wrapping the difference into ±π first is what makes it the short way.
fn turned(from: f32, to: f32, fraction: f32) -> f32 {
    let turn = std::f32::consts::TAU;
    let mut delta = (to - from) % turn;
    if delta > std::f32::consts::PI {
        delta -= turn;
    } else if delta < -std::f32::consts::PI {
        delta += turn;
    }
    from + delta * fraction.clamp(0.0, 1.0)
}

const GRADE: i32 = vx_agent::DEFAULT_GRADE;

/// Edge length of the cubes drawn at the corners of a marked area.
const MARKER_SIZE: f32 = 0.35;

/// Edge length of a hovering ping marker.
const PING_SIZE: f32 = 0.5;

/// Radians per second the drill and rotor turn while working.
const SPIN_RATE: f32 = 9.0;

// How many drones a dispatch fields is no longer a constant here: it is
// whatever the player has bought, and `crate::garage` is the only place that
// knows.

/// What the player is currently doing with the mining tools.
#[derive(Debug)]
pub struct Mining {
    /// Corners picked so far. Two makes an area.
    corners: Vec<BlockPos>,
    /// The marked area, once both corners are down.
    area: Option<VoxelAabb>,
    /// Every method that applies here, cheapest first.
    plans: Vec<MinePlan>,
    /// Which of `plans` is selected.
    chosen: usize,
    /// The running excavation, once started.
    operation: Option<Operation>,
    /// What the fleet burns. Lives here rather than in `main` because it
    /// decides how much ground gets dug, and ground is what the hash covers:
    /// replay carries this struct, so replay carries the tank.
    pub tank: crate::fuel::Tank,
    /// What the work is costing the machines doing it. Beside the tank for
    /// the same reason and by the same argument: a worn crew digs slower, a
    /// seized one stops, and how long a crew dug is what decides where the
    /// hole ends up — so replay carries this struct, and replay carries the
    /// wear.
    pub wear: crate::wear::Wear,
    /// The holes this player has sunk. Here rather than in `main` for the
    /// tank's reason and the wear's: a well puts goods on the pile, the pile
    /// decides how long the fleet turns, and how long the fleet turns is
    /// what the world hash covers. Replay carries this struct, so replay
    /// carries the wells.
    pub wells: crate::well::Wells,
    /// What the wells did on the last `advance`. Kept beside them rather
    /// than returned, because `FleetReport` belongs to `vx-agent` and
    /// `vx-agent` has never heard of oil.
    pub well_report: crate::well::WellReport,
    /// Fractional ticks carried between frames, so the drone runs at a steady
    /// rate rather than at whatever the frame rate happens to be.
    pending: f64,
    /// The air side: the flier, its surveys, and the base container.
    pub fleet: Fleet,
    /// The pack scout, once bought. Deliberately *not* in the fleet's
    /// roster: `dispatch_scan` can never grab it, so the survey layer stays
    /// the paid flier's trade by construction.
    pub kestrel: Option<vx_agent::Kestrel>,
    /// Nose directions, remembered so parked machines do not twitch.
    /// Chunks held resident for the running operation, released when it ends.
    pinned: Vec<vx_core::ChunkPos>,
    /// Ticks the last `update` turned its elapsed time into. What the command
    /// journal records, because wall time is not reproducible and this is.
    last_ticks: u32,
    drone_yaws: Vec<f32>,
    flier_yaws: Vec<f32>,
    /// Where each nose pointed at the *last* tick.
    ///
    /// Position has always been interpolated between ticks and heading never
    /// was, which nothing noticed while the camera ignored yaw entirely. Hang
    /// the camera off the nose and the omission becomes a jolt on every turn,
    /// so heading now glides on the same clock position does — and the rigs
    /// you are merely watching stop snapping round as well.
    previous_drone_yaws: Vec<f32>,
    previous_flier_yaws: Vec<f32>,
    /// Drill/rotor angle, advanced while machines work.
    spin: f32,
    /// The rig shapes, built once.
    digger_rig: Rig,
    flier_rig: Rig,
    /// The machine the player has the wheel of, if any.
    piloted: Option<MachineRef>,
    /// This frame's held pilot input, re-issued on every simulation tick so
    /// driving is frame-rate independent exactly like the autonomous path.
    pilot_command: PilotCommand,
    /// Where the pilot is looking: a driven machine points its nose along the
    /// player's view rather than along its last step.
    pilot_look: f32,
    /// The kestrel's standing order, stashed while the player has its wheel.
    kestrel_suspended: Option<vx_agent::KestrelMode>,
    kestrel_rig: Rig,
    kestrel_yaw: f32,
    previous_kestrel_yaw: f32,
}

/// One of the fleet's machines, by kind and index.
///
/// A thin seam rather than a shared entity abstraction: drones and fliers have
/// almost nothing in common beyond "the player can look through it", and
/// inventing a trait for two cases would cost more than it saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineRef {
    Digger(usize),
    Flier(usize),
    /// The pack scout. No index: one per person, by design.
    Kestrel,
}

/// One row of the handheld's roster.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineListing {
    pub machine: MachineRef,
    /// "DIGGER 1", "FLIER 1".
    pub name: String,
    /// What it is up to, in a word.
    pub state: String,
    /// How it is holding up. The roster is where a player learns which
    /// machine is the one dragging the crew.
    pub condition: crate::wear::Condition,
    /// How far the player is from it, in blocks.
    pub distance: f32,
    pub cargo: u64,
    pub capacity: u64,
}

impl Default for Mining {
    fn default() -> Self {
        Mining {
            corners: Vec::new(),
            area: None,
            plans: Vec::new(),
            chosen: 0,
            operation: None,
            tank: crate::fuel::Tank::default(),
            wear: crate::wear::Wear::default(),
            wells: crate::well::Wells::default(),
            well_report: crate::well::WellReport::default(),
            pinned: Vec::new(),
            last_ticks: 0,
            pending: 0.0,
            fleet: Fleet::default(),
            kestrel: None,
            drone_yaws: Vec::new(),
            flier_yaws: Vec::new(),
            previous_drone_yaws: Vec::new(),
            previous_flier_yaws: Vec::new(),
            spin: 0.0,
            digger_rig: Rig::digger(),
            flier_rig: Rig::flier(),
            piloted: None,
            pilot_command: PilotCommand::default(),
            pilot_look: 0.0,
            kestrel_suspended: None,
            kestrel_rig: Rig::kestrel(),
            kestrel_yaw: 0.0,
            previous_kestrel_yaw: 0.0,
        }
    }
}

impl Mining {
    /// Make sure the fleet has its aircraft, hovering near `position`.
    ///
    /// Idempotent so callers need not track whether it happened; one flier is
    /// this milestone's scope.
    pub fn ensure_flier(&mut self, position: DVec3) {
        if self.fleet.fliers.is_empty() {
            self.fleet.add_flier(BlockPos::new(
                position.x.floor() as i32,
                position.y.floor() as i32 + 6,
                position.z.floor() as i32,
            ));
        }
    }

    /// Send the flier to sweep the sector containing the column `(x, z)`.
    pub fn dispatch_scan(&mut self, x: i32, z: i32) -> bool {
        self.fleet.dispatch_scan(Sector::containing(x, z))
    }

    /// Add a corner at `pos`, replacing the previous pair once two are down.
    pub fn mark(&mut self, world: &mut World, pos: BlockPos) {
        if self.operation.is_some() {
            return;
        }
        if self.corners.len() >= 2 {
            self.corners.clear();
            self.plans.clear();
            self.area = None;
        }
        self.corners.push(pos);

        if let [first, second] = self.corners[..] {
            let area = VoxelAabb::new(first, second);
            // Planning reads the world too — it looks for a hillside to drive
            // into and a grade it can climb — so the ground has to be resident
            // before it is consulted, not just before a drone drives on it.
            // Released by `cancel`, or replaced by the plan's own span at
            // `start`.
            self.release_ground(world);
            let scouted = vx_agent::working_span(area, area.min);
            self.pinned = world.pin_span(scouted.min, scouted.max);

            self.area = Some(area);
            self.plans = options(world, area, GRADE);
            self.chosen = 0;
        }
    }

    /// The corners you have picked, and which method is selected.
    ///
    /// For [`crate::dig`], which writes them down: a marked-but-undispatched
    /// area is a decision the player made by eye, and it used to be thrown
    /// away by a save. Only the corners and the choice are kept — the area
    /// and the plans are derived from them by [`Mining::mark`], and deriving
    /// them again on load is both cheaper than storing them and the only way
    /// to be sure a plan still matches ground that may have moved.
    pub fn marked(&self) -> (&[BlockPos], usize) {
        (&self.corners, self.chosen)
    }

    /// Put a mark back, exactly as though it had been clicked.
    pub fn restore_mark(&mut self, world: &mut World, corners: &[BlockPos], chosen: usize) {
        if self.operation.is_some() {
            return;
        }
        for corner in corners.iter().take(2) {
            self.mark(world, *corner);
        }
        if !self.plans.is_empty() {
            self.chosen = chosen.min(self.plans.len() - 1);
        }
    }

    /// Choose the next method in the list.
    pub fn cycle_method(&mut self) {
        if !self.plans.is_empty() && self.operation.is_none() {
            self.chosen = (self.chosen + 1) % self.plans.len();
        }
    }

    pub fn selected_plan(&self) -> Option<&MinePlan> {
        self.plans.get(self.chosen)
    }

    /// Post the selected plan and put a drone on it.
    ///
    /// The drone starts at the portal and the stockpile sits there too, so the
    /// haul route is the excavation itself — which is the whole point of
    /// choosing a method the drone can drive.
    /// Post the selected plan and put the crew you own on it.
    ///
    /// `crew` is what the garage says you have bought. Owning none is not a
    /// silent no-op: the caller reports it, because "I marked a dig and nothing
    /// happened" is the worst way to learn that machines cost money now.
    pub fn start(&mut self, world: &mut World, crew: u32) -> Option<MineMethod> {
        if crew == 0 {
            return None;
        }
        let plan = self.plans.get(self.chosen)?.clone();
        let start = vx_agent::settle(world, plan.portal);

        // Hold the ground the drones will read for as long as the job lasts.
        // Without this a drone near the edge of the loaded set sees air where
        // there is rock, and the same dispatch digs a different hole depending
        // on where the player happened to stand while it ran.
        let span = vx_agent::working_span(plan.span(), start);
        self.release_ground(world);
        self.pinned = world.pin_span(span.min, span.max);

        let mut operation = Operation::new(start);
        // A crew, not a single machine. The job board has always been built for
        // contention — claims, releases, nearest-first — and until now exactly
        // one drone ever existed, so none of that ever ran.
        for _ in 0..crew {
            operation.add_drone(start);
        }
        operation.post_plan(&plan);
        self.operation = Some(operation);
        Some(plan.method)
    }

    /// The running dispatch, as plain data a save file can hold. `None` when
    /// nothing is being dug.
    pub fn operation_snapshot(&self) -> Option<vx_agent::OperationSnapshot> {
        self.operation.as_ref().map(Operation::snapshot)
    }

    /// Put a saved dispatch back to work, and **hold its ground again**.
    ///
    /// The pin is not an optimisation. A drone reads the world to decide what
    /// to cut, an unloaded chunk reads as air, and a crew whose span is not
    /// resident therefore digs a *different hole* depending on where the
    /// player happens to be standing — which is exactly the class of
    /// non-determinism `Mining::start` pins against in the first place
    /// (`self.pinned = world.pin_span(..)`). A restore that forgot it would
    /// hand the replay oracle a dispatch whose outcome depended on the camera.
    ///
    /// The span is derived rather than saved: the union of every job region
    /// still on the board with the drop-off, through the same
    /// `vx_agent::working_span` that sized it when the job was posted. Derived
    /// beats stored here, because the board is what the crew is actually going
    /// to read.
    pub fn restore_operation(
        &mut self,
        world: &mut World,
        snapshot: vx_agent::OperationSnapshot,
    ) {
        let operation = Operation::restore(snapshot);
        // The reach is the work *and the crew*, not the work alone.
        //
        // Deriving it from the remaining job regions is the obvious thing and
        // it is wrong: jobs complete, so the span shrinks as the dig goes on,
        // and a drone part way down a ramp it has already cut ends up outside
        // the ground its own route needs. Unloaded ground reads as air, air is
        // not standable, and the drone gives its job back and reports Stuck —
        // which is exactly what the first `--payroll` run did after a reload,
        // eight blocks in.
        let reach = operation
            .drones
            .iter()
            .map(|drone| drone.position)
            .chain(std::iter::once(operation.home))
            .fold(VoxelAabb::single(operation.home), |span, at| {
                span.including(at)
            });
        let reach = operation
            .board
            .jobs()
            .fold(reach, |span, job| span.union(job.region));
        let span = vx_agent::working_span(reach, operation.home);
        self.release_ground(world);
        self.pinned = world.pin_span(span.min, span.max);
        self.operation = Some(operation);
    }

    /// Release the ground an operation was holding. Safe to call with nothing
    /// pinned, which is what makes it safe to call from `cancel`.
    pub fn release_ground(&mut self, world: &mut World) {
        let span = std::mem::take(&mut self.pinned);
        world.unpin_span(&span);
    }

    /// Abandon whatever is marked or running. The hole stays dug, and the
    /// fleet — the flier, its surveys, the base — is not the plan's to throw
    /// away, so it survives.
    pub fn cancel(&mut self, world: &mut World) {
        // Let the ground go before dropping the state that remembers it, or
        // the pin outlives the job and streaming can never evict that span.
        self.release_ground(world);
        let fleet = std::mem::take(&mut self.fleet);
        *self = Mining {
            fleet,
            ..Mining::default()
        };
    }

    pub fn is_running(&self) -> bool {
        self.operation.is_some()
    }

    /// Where the ground drones are, for the minimap's dots.
    pub fn drone_positions(&self) -> Vec<BlockPos> {
        self.operation
            .iter()
            .flat_map(|operation| operation.drones.iter().map(|drone| drone.position))
            .collect()
    }

    /// Advance the excavation by however many ticks `elapsed` is worth.
    ///
    /// Returns the fleet's accumulated report, so the caller can turn work
    /// into experience without the fleet knowing players exist.
    pub fn update(&mut self, world: &mut World, events: &EventBus, elapsed: Duration) -> FleetReport {
        // Machines that are working turn their drills and rotors.
        let working = self
            .operation
            .as_ref()
            .is_some_and(|operation| {
                operation
                    .drones
                    .iter()
                    .any(|drone| matches!(drone.state, DroneState::Digging(_)))
            })
            || self
                .fleet
                .fliers
                .iter()
                .any(|flier| flier.state != FlierState::Idle);
        if working {
            self.spin += elapsed.as_secs_f32() * SPIN_RATE;
        }

        self.pending += elapsed.as_secs_f64() * TICK_RATE;
        // Cap the catch-up. After a long stall — loading, or a dragged window —
        // running hundreds of ticks in one frame would teleport the drone and
        // spike the frame it lands on.
        let ticks = (self.pending as u32).min(16);
        self.pending -= f64::from(ticks);
        // And *discard* the rest of the backlog rather than keeping it: the
        // first version only subtracted the ticks run, so a 30-second stall
        // drained at the cap for seconds afterwards — a ~120x fast-forward,
        // the very teleport the cap exists to prevent. Time lost to a stall
        // stays lost; the drone simply pauses with the game.
        self.pending = self.pending.min(1.0);

        self.last_ticks = ticks;
        self.advance(world, events, ticks)
    }

    /// How far the machines' rotors and drills have turned. Shared so anything
    /// else that draws a machine spins in step with the fleet.
    pub fn spin(&self) -> f32 {
        self.spin
    }

    /// The marked area, if two corners are down. What the journal records as
    /// the dispatch, since the plan itself is derivable from it.
    pub fn area(&self) -> Option<VoxelAabb> {
        self.area
    }

    /// Ticks the last [`Mining::update`] ran. Recorded by the journal.
    pub fn last_ticks(&self) -> u32 {
        self.last_ticks
    }

    /// Run exactly `ticks` simulation ticks.
    ///
    /// The seam the command log records against. Wall time decides *how many*
    /// ticks a frame is worth, and that answer depends on frame rate, stalls
    /// and how long a window was dragged — none of which is reproducible. The
    /// tick count is. So the log stores ticks and replays through here, and a
    /// session recorded on a machine managing nine frames a second replays
    /// identically on one managing three hundred.
    pub fn advance(&mut self, world: &mut World, events: &EventBus, ticks: u32) -> FleetReport {
        let mut report = FleetReport::default();

        // The wells first, and outside every gate below. A wellhead does not
        // burn the fleet's fuel, does not wear, and does not care whether
        // there is a dig running — which is exactly what makes it the thing
        // that rescues a crew that ran dry a long way from any water.
        // With no base there is nowhere to deliver: the hole still goes
        // down and still lifts, and what comes up goes on the ground. The
        // tick runs either way so that a well's life depends on how long it
        // ran rather than on when a container happened to be standing.
        self.well_report = match self.fleet.base.as_mut() {
            Some(base) => self.wells.tick(ticks, &mut base.stockpile),
            None => {
                let mut spoil = vx_agent::Stockpile::default();
                self.wells.tick(ticks, &mut spoil)
            }
        };

        if self.operation.is_none() {
            // The fleet still flies with no dig running.
            for _ in 0..ticks {
                if !self.fuelled() {
                    break;
                }
                if !self.turning() {
                    continue;
                }
                self.pilot_sub_tick(world, events);
                let tick = self.fleet.tick(world, &mut []);
                report.delivered += tick.delivered;
                report.sectors_completed += tick.sectors_completed;
                report.pings_found += tick.pings_found;
            }
            self.remember_yaws();
            return report;
        }

        // Fuel is spent per tick for the whole crew, then the crew runs. A
        // dry tick is a tick nobody works: the machines are still there and
        // still where they were, they simply have nothing to burn.
        let mut worked = 0;
        for _ in 0..ticks {
            if !self.fuelled() {
                break;
            }
            // Worn machines lose ticks; seized ones lose all of them. The
            // fuel is still spent — a machine that will not turn has still
            // been asked to.
            if !self.turning() {
                continue;
            }
            worked += 1;
            self.pilot_sub_tick(world, events);
            let operation = self.operation.as_mut().expect("checked above");
            operation.tick(world, events);
        }
        for _ in 0..worked {
            // The air side reads the world immutably and trades with the
            // mine's stockpile.
            let mines = self.operation.as_mut().map(std::slice::from_mut).unwrap_or(&mut []);
            let tick = self.fleet.tick(world, mines);
            report.delivered += tick.delivered;
            report.sectors_completed += tick.sectors_completed;
            report.pings_found += tick.pings_found;
        }
        self.remember_yaws();
        report
    }

    /// How many machines want fuel this tick.
    ///
    /// The kestrel is not among them: it runs on a charged cell and a
    /// cooldown, which is its own budget and was the whole point of that
    /// design. Counting it twice would be charging for the same wing twice.
    pub fn burners(&self) -> u32 {
        let diggers = self
            .operation
            .as_ref()
            .map_or(0, |operation| operation.drones.len() as u32);
        diggers + self.fleet.fliers.len() as u32
    }

    /// Charge one tick's wear to the crew and answer whether it turns.
    ///
    /// A seized tick is a tick nobody works — exactly the shape of the dry
    /// tick above, and for the same reason: both are decided inside the
    /// call `Command::Advance` replays, so both sides reach the same answer
    /// without either re-deciding anything.
    fn turning(&mut self) -> bool {
        let diggers = self
            .operation
            .as_ref()
            .map_or(0, |operation| operation.drones.len());
        let fliers = self.fleet.fliers.len();
        self.wear.tick(diggers, fliers)
    }

    /// Spend one tick's fuel, drawing on the base pile. False when dry.
    fn fuelled(&mut self) -> bool {
        let machines = self.burners();
        match self.fleet.base.as_mut() {
            Some(base) => self.tank.burn(machines, &mut base.stockpile),
            // No base means no pile to draw on — and a fleet with nowhere to
            // deliver has bigger problems than fuel, so it is left alone.
            None => true,
        }
    }

    /// One simulation tick of the player's held controls.
    ///
    /// Runs inside the same accumulator loop as the autonomous tick, so a
    /// piloted machine covers exactly the same ground per second as one
    /// driving itself — the override is a change of driver, not of physics.
    fn pilot_sub_tick(&mut self, world: &mut World, events: &EventBus) {
        match self.piloted {
            Some(MachineRef::Digger(_)) => {
                if let Some(operation) = &mut self.operation {
                    operation.pilot_tick(world, events, self.pilot_command);
                }
            }
            Some(MachineRef::Flier(_)) => {
                self.fleet.pilot_tick(world, self.pilot_command);
            }
            Some(MachineRef::Kestrel) => {
                if let Some(kestrel) = &mut self.kestrel {
                    kestrel.craft.pilot_step(
                        world,
                        self.pilot_command.heading,
                        self.pilot_command.climb,
                    );
                }
            }
            None => {}
        }
    }

    /// Update remembered nose directions from this tick's movement.
    fn remember_yaws(&mut self) {
        // Last tick's headings, kept so this tick's can be glided into rather
        // than snapped to. Taken before anything is written, which is the
        // same moment `previous_position` is taken for the machines
        // themselves.
        self.previous_drone_yaws.clone_from(&self.drone_yaws);
        self.previous_flier_yaws.clone_from(&self.flier_yaws);
        self.previous_kestrel_yaw = self.kestrel_yaw;

        // A piloted machine points where the player is looking. Deriving its
        // nose from movement instead would leave a stationary drone facing
        // whatever direction its last step happened to take while the player
        // spins the camera around it.
        let piloted = self.piloted;
        if let Some(operation) = &self.operation {
            self.drone_yaws.resize(operation.drones.len(), 0.0);
            for (index, (yaw, drone)) in
                self.drone_yaws.iter_mut().zip(&operation.drones).enumerate()
            {
                if piloted == Some(MachineRef::Digger(index)) {
                    *yaw = self.pilot_look;
                    continue;
                }
                let delta = drone.position;
                let previous = drone.previous_position;
                if let Some(new) = rig::yaw_towards(
                    (delta.x - previous.x) as f32,
                    (delta.z - previous.z) as f32,
                ) {
                    *yaw = new;
                }
            }
        }
        if let Some(kestrel) = &self.kestrel {
            if piloted == Some(MachineRef::Kestrel) {
                self.kestrel_yaw = self.pilot_look;
            } else if let Some(new) = rig::yaw_towards(
                (kestrel.craft.position.x - kestrel.craft.previous_position.x) as f32,
                (kestrel.craft.position.z - kestrel.craft.previous_position.z) as f32,
            ) {
                self.kestrel_yaw = new;
            }
        }
        self.flier_yaws.resize(self.fleet.fliers.len(), 0.0);
        for (index, (yaw, flier)) in self.flier_yaws.iter_mut().zip(&self.fleet.fliers).enumerate()
        {
            if piloted == Some(MachineRef::Flier(index)) {
                *yaw = self.pilot_look;
                continue;
            }
            if let Some(new) = rig::yaw_towards(
                (flier.position.x - flier.previous_position.x) as f32,
                (flier.position.z - flier.previous_position.z) as f32,
            ) {
                *yaw = new;
            }
        }
    }

    /// How far between the last tick and the next the visuals should sit.
    fn tick_fraction(&self) -> f32 {
        (self.pending as f32).clamp(0.0, 1.0)
    }

    /// Apply the player's current skill effects to the machines.
    ///
    /// Called by the app whenever levels change (and cheaply on every grant):
    /// the fleet and drones never know skills exist, they just have stats.
    pub fn apply_skills(&mut self, scan_depth: i32, drone_capacity: u64, flier_capacity: u64) {
        self.fleet.scan_depth = scan_depth;
        for flier in &mut self.fleet.fliers {
            flier.capacity = flier_capacity;
        }
        if let Some(operation) = &mut self.operation {
            for drone in &mut operation.drones {
                drone.capacity = drone_capacity;
            }
        }
    }

    /// Where a machine sits *right now*, between its last tick and its next.
    ///
    /// The single source of truth for machine positions on screen. Both the
    /// rigs and the FPV camera go through it, which is what stops a piloted
    /// machine's view from drifting against its own body.
    fn interpolated(&self, from: BlockPos, to: BlockPos) -> DVec3 {
        let fraction = self.tick_fraction();
        let a = DVec3::new(from.x as f64 + 0.5, from.y as f64, from.z as f64 + 0.5);
        let b = DVec3::new(to.x as f64 + 0.5, to.y as f64, to.z as f64 + 0.5);
        a + (b - a) * f64::from(fraction)
    }

    /// Where a machine's camera hangs, in the rig's own frame: +X forward,
    /// +Y up, origin on the ground point under it.
    ///
    /// A gimbal under the nose, like a surveillance bird — which is what it
    /// should always have been. It used to be a bare lift up the Y axis, and
    /// every one of the three was wrong in its own way: the digger's sat
    /// *inside* its cab box (`y 0.66..0.92`), the flier's floated in the gap
    /// between its hull roof and its rotor, and the kestrel's hovered above
    /// its own blades. You got away with it only because the camera happened
    /// to land in air pockets and looked out through the back faces.
    ///
    /// Each offset is measured off the rig's real parts: forward of the
    /// hull, below its floor, above the skids and clear of anything that
    /// spins.
    pub fn gimbal(machine: MachineRef) -> Vec3 {
        match machine {
            // Under the drill (which bottoms at 0.30) and ahead of the front
            // wheels (which reach x 0.44).
            MachineRef::Digger(_) => Vec3::new(0.58, 0.20, 0.0),
            // Off the nose (hull ends at 0.45), below the hull floor (0.18),
            // above the skids (0.09), well inside the rotor disc.
            MachineRef::Flier(_) => Vec3::new(0.52, 0.13, 0.0),
            // The whole machine is 0.44 long, so this is a small step off a
            // small nose.
            MachineRef::Kestrel => Vec3::new(0.25, 0.05, 0.0),
        }
    }

    /// A machine's nose direction *right now*, gliding between ticks the same
    /// way its position does.
    fn heading(&self, machine: MachineRef) -> f32 {
        let (from, to) = match machine {
            MachineRef::Digger(index) => (
                self.previous_drone_yaws.get(index).copied(),
                self.drone_yaws.get(index).copied(),
            ),
            MachineRef::Flier(index) => (
                self.previous_flier_yaws.get(index).copied(),
                self.flier_yaws.get(index).copied(),
            ),
            MachineRef::Kestrel => (Some(self.previous_kestrel_yaw), Some(self.kestrel_yaw)),
        };
        let to = to.unwrap_or(0.0);
        turned(from.unwrap_or(to), to, self.tick_fraction())
    }

    /// Where a machine's camera sits this frame.
    ///
    /// The gimbal, turned by the nose it is bolted to and added to the same
    /// interpolated ground point the rig is drawn at — so the view cannot
    /// drift against the body between ticks, which is the property
    /// `interpolated` exists to guarantee.
    pub fn machine_eye(&self, machine: MachineRef) -> Option<DVec3> {
        let base = match machine {
            MachineRef::Digger(index) => {
                let drone = self.operation.as_ref()?.drones.get(index)?;
                self.interpolated(drone.previous_position, drone.position)
            }
            MachineRef::Flier(index) => {
                let flier = self.fleet.fliers.get(index)?;
                self.interpolated(flier.previous_position, flier.position)
            }
            MachineRef::Kestrel => {
                let craft = &self.kestrel.as_ref()?.craft;
                self.interpolated(craft.previous_position, craft.position)
            }
        };
        let mount = glam::Quat::from_rotation_y(self.heading(machine)) * Self::gimbal(machine);
        Some(base + mount.as_dvec3())
    }

    /// Where a machine is, in whole blocks.
    pub fn machine_position(&self, machine: MachineRef) -> Option<BlockPos> {
        match machine {
            MachineRef::Digger(index) => {
                Some(self.operation.as_ref()?.drones.get(index)?.position)
            }
            MachineRef::Flier(index) => Some(self.fleet.fliers.get(index)?.position),
            MachineRef::Kestrel => Some(self.kestrel.as_ref()?.craft.position),
        }
    }

    /// The roster the handheld lists, nearest to `from` first.
    pub fn roster(&self, from: DVec3) -> Vec<MachineListing> {
        let mut rows = Vec::new();
        if let Some(operation) = &self.operation {
            for (index, drone) in operation.drones.iter().enumerate() {
                let machine = MachineRef::Digger(index);
                rows.push(MachineListing {
                    machine,
                    condition: self.wear.condition(machine),
                    name: format!("DIGGER {}", index + 1),
                    state: match drone.state {
                        DroneState::Idle => "IDLE".into(),
                        DroneState::Travelling(_) => "DRIVING".into(),
                        DroneState::Digging(_) => "DIGGING".into(),
                        DroneState::Hauling => "HAULING".into(),
                        DroneState::Stuck => "STUCK".into(),
                        DroneState::Manual => "PILOTED".into(),
                    },
                    distance: self
                        .machine_eye(machine)
                        .map_or(0.0, |at| (at - from).length() as f32),
                    cargo: drone.carrying(),
                    capacity: drone.capacity,
                });
            }
        }
        if let Some(kestrel) = &self.kestrel {
            let machine = MachineRef::Kestrel;
            rows.push(MachineListing {
                machine,
                condition: self.wear.condition(machine),
                name: "KESTREL".into(),
                state: kestrel_state(kestrel).into(),
                distance: self
                    .machine_eye(machine)
                    .map_or(0.0, |at| (at - from).length() as f32),
                cargo: 0,
                capacity: 0,
            });
        }
        for (index, flier) in self.fleet.fliers.iter().enumerate() {
            let machine = MachineRef::Flier(index);
            rows.push(MachineListing {
                machine,
                condition: self.wear.condition(machine),
                name: format!("FLIER {}", index + 1),
                state: match flier.state {
                    FlierState::Idle => "IDLE".into(),
                    FlierState::Scanning { .. } => "SCANNING".into(),
                    FlierState::ToPickup { .. } => "COLLECTING".into(),
                    FlierState::ToBase => "RETURNING".into(),
                    FlierState::Manual => "PILOTED".into(),
                },
                distance: self
                    .machine_eye(machine)
                    .map_or(0.0, |at| (at - from).length() as f32),
                cargo: flier.carrying(),
                capacity: flier.capacity,
            });
        }
        rows.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        rows
    }

    /// One machine's row, for the feed banner.
    pub fn listing(&self, machine: MachineRef, from: DVec3) -> Option<MachineListing> {
        self.roster(from)
            .into_iter()
            .find(|row| row.machine == machine)
    }

    /// Take the wheel. The machine's own work is suspended, not discarded.
    pub fn take_control(&mut self, machine: MachineRef) -> bool {
        if self.piloted.is_some() {
            return false;
        }
        let taken = match machine {
            MachineRef::Digger(index) => self
                .operation
                .as_mut()
                .is_some_and(|operation| operation.take_control(index)),
            MachineRef::Flier(index) => self.fleet.take_control(index),
            MachineRef::Kestrel => match &mut self.kestrel {
                // Taking the wheel launches a ready scout; the standing
                // order is stashed and restored on release, the same shape
                // as the fleet's suspended flier state.
                Some(kestrel) if kestrel.aloft() || kestrel.ready() => {
                    self.kestrel_suspended = Some(kestrel.mode);
                    kestrel.order(vx_agent::KestrelMode::Manual)
                }
                _ => false,
            },
        };
        if taken {
            self.piloted = Some(machine);
            self.pilot_command = PilotCommand::default();
        }
        taken
    }

    /// Hand the machine back to its own devices.
    pub fn release_control(&mut self) {
        let Some(machine) = self.piloted.take() else {
            return;
        };
        match machine {
            MachineRef::Digger(index) => {
                if let Some(operation) = &mut self.operation {
                    operation.release_control(index);
                }
            }
            MachineRef::Flier(index) => {
                self.fleet.release_control(index);
            }
            MachineRef::Kestrel => {
                if let Some(kestrel) = &mut self.kestrel {
                    let resume = self
                        .kestrel_suspended
                        .take()
                        .unwrap_or(vx_agent::KestrelMode::Returning);
                    kestrel.order(resume);
                }
            }
        }
        self.pilot_command = PilotCommand::default();
    }

    /// Which machine the player is driving.
    pub fn piloted(&self) -> Option<MachineRef> {
        self.piloted
    }

    /// This frame's held controls.
    pub fn set_pilot_command(&mut self, command: PilotCommand) {
        self.pilot_command = command;
    }

    /// Where the pilot is looking, for the driven machine's nose.
    pub fn set_pilot_look(&mut self, yaw: f32) {
        self.pilot_look = yaw;
    }

    /// The cubes to draw this frame: corner markers and drones, built in
    /// the camera's frame — `relative` measures a place from the render
    /// origin — and marked so.
    /// `feed` names the machine the player is looking *through*, whose own
    /// rig is skipped. It never mattered while the camera sat in an air
    /// pocket inside the hull; hang it off the nose and the machine's own
    /// body is the first thing in frame.
    pub fn objects(
        &self,
        relative: impl Fn(DVec3) -> Vec3,
        feed: Option<MachineRef>,
    ) -> Vec<Object> {
        let mut objects = Vec::new();
        let marker = |centre: DVec3, size: f32| {
            let half = DVec3::splat(f64::from(size) * 0.5);
            Object::box_between(relative(centre - half), relative(centre + half), slot::COPPER_ORE)
                .already_relative()
        };

        if let Some(area) = self.area {
            for corner in area_corners(area) {
                objects.push(marker(corner, MARKER_SIZE));
            }
        } else {
            // One corner down: show it on its own, so it is obvious the second
            // click is still owed.
            for corner in &self.corners {
                let centre = DVec3::new(
                    f64::from(corner.x) + 0.5,
                    f64::from(corner.y) + 1.0,
                    f64::from(corner.z) + 0.5,
                );
                objects.push(marker(centre, MARKER_SIZE));
            }
        }

        // Machines are rigs, gliding between their tick positions.
        let lerp = |from: BlockPos, to: BlockPos| -> Vec3 { relative(self.interpolated(from, to)) };
        let placed = |built: Vec<Object>| built.into_iter().map(Object::already_relative);

        if let Some(operation) = &self.operation {
            for (index, drone) in operation.drones.iter().enumerate() {
                if feed == Some(MachineRef::Digger(index)) {
                    continue;
                }
                let yaw = self.heading(MachineRef::Digger(index));
                let spin = if matches!(drone.state, DroneState::Digging(_)) {
                    self.spin
                } else {
                    0.0
                };
                objects.extend(placed(self.digger_rig.objects(
                    lerp(drone.previous_position, drone.position),
                    yaw,
                    spin,
                )));
            }
        }

        // The scout, only while off the pack: a docked kestrel is stowed,
        // not a second hat on the player's head.
        if let Some(kestrel) = self
            .kestrel
            .as_ref()
            .filter(|kestrel| kestrel.aloft() && feed != Some(MachineRef::Kestrel))
        {
            objects.extend(placed(self.kestrel_rig.objects(
                lerp(kestrel.craft.previous_position, kestrel.craft.position),
                self.heading(MachineRef::Kestrel),
                self.spin * 3.0,
            )));
        }
        for (index, flier) in self.fleet.fliers.iter().enumerate() {
            if feed == Some(MachineRef::Flier(index)) {
                continue;
            }
            let yaw = self.heading(MachineRef::Flier(index));
            objects.extend(placed(self.flier_rig.objects(
                lerp(flier.previous_position, flier.position),
                yaw,
                self.spin * 2.5,
            )));
        }

        // Pings hover and read as copper, since copper is what they promise.
        for ping in self.fleet.pings() {
            let centre = DVec3::new(
                f64::from(ping.position.x) + 0.5,
                f64::from(ping.position.y) + 1.0,
                f64::from(ping.position.z) + 0.5,
            );
            objects.push(marker(centre, PING_SIZE));
        }

        objects
    }

    /// A one-line readout for the title bar.
    pub fn status(&self) -> Option<String> {
        // A seized crew outranks everything: it is the reason nothing else
        // on this line is happening, and a player staring at a stopped dig
        // deserves to be told why rather than left to work it out.
        let diggers = self
            .operation
            .as_ref()
            .map_or(0, |operation| operation.drones.len());
        if let Some(complaint) = self.wear.complaint(diggers, self.fleet.fliers.len()) {
            return Some(complaint.to_lowercase());
        }

        // A working flier outranks the mining readout: it is the thing the
        // player just ordered.
        if let Some(machine) = self.piloted {
            let name = match machine {
                MachineRef::Digger(index) => format!("DIGGER {}", index + 1),
                MachineRef::Flier(index) => format!("FLIER {}", index + 1),
                MachineRef::Kestrel => "KESTREL".to_string(),
            };
            return Some(format!("piloting {name}"));
        }

        if let Some(flier) = self.fleet.fliers.first() {
            match flier.state {
                FlierState::Manual => {}
                FlierState::Scanning { .. } => {
                    return Some(format!(
                        "scanning: {} pings so far",
                        self.fleet.pings().len()
                    ));
                }
                FlierState::ToPickup { .. } | FlierState::ToBase => {
                    return Some(format!("ferrying: {} aboard", flier.carrying()));
                }
                FlierState::Idle => {}
            }
        }

        if let Some(operation) = &self.operation {
            let drone = operation.drones.first()?;
            return Some(format!(
                "mining: {} jobs left, {} hauled, drone {:?}",
                operation.board.len(),
                operation.stockpile.total(),
                drone.state
            ));
        }

        let plan = self.selected_plan()?;
        Some(format!(
            "{} — {} blocks (option {}/{}, Tab to change, Enter to dig)",
            plan.method.name(),
            plan.volume,
            self.chosen + 1,
            self.plans.len()
        ))
    }
}

impl Mining {
    /// The kestrel rig at an arbitrary position: the roost borrows the same
    /// silhouette — one machine, two owners, made literal on screen.
    pub fn kestrel_objects(&self, at: BlockPos, relative: impl Fn(DVec3) -> Vec3) -> Vec<Object> {
        let centre = DVec3::new(f64::from(at.x) + 0.5, f64::from(at.y), f64::from(at.z) + 0.5);
        self.kestrel_rig
            .objects(relative(centre), 0.0, self.spin * 3.0)
            .into_iter()
            .map(Object::already_relative)
            .collect()
    }
}

/// The kestrel's state, in a word, for the roster and the HUD.
pub fn kestrel_state(kestrel: &vx_agent::Kestrel) -> &'static str {
    use vx_agent::KestrelMode;
    match kestrel.mode {
        KestrelMode::Docked if kestrel.cooldown > 0 => "COOLING",
        KestrelMode::Docked => "DOCKED",
        KestrelMode::Orbit => "ORBITING",
        KestrelMode::Sortie { .. } => "ON SORTIE",
        KestrelMode::Perch { .. } => "PERCHED",
        KestrelMode::Vanguard => "SCOUTING AHEAD",
        KestrelMode::Manual => "PILOTED",
        KestrelMode::Returning => "RETURNING",
    }
}

/// The eight corners of a marked area, in world coordinates.
///
/// Blocks are cells, so the far corner of the box is one block past `max`.
fn area_corners(area: VoxelAabb) -> Vec<DVec3> {
    let lo = DVec3::new(area.min.x as f64, area.min.y as f64, area.min.z as f64);
    let hi = DVec3::new(
        area.max.x as f64 + 1.0,
        area.max.y as f64 + 1.0,
        area.max.z as f64 + 1.0,
    );
    let mut corners = Vec::with_capacity(8);
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                corners.push(DVec3::new(x, y, z));
            }
        }
    }
    corners
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world with ground near the origin and a fleet with one flier.
    fn flying_world() -> (World, Mining) {
        let mut world = World::new(2024);
        world.load_around(vx_core::ChunkPos::new(0, 0), 2);
        let ground = world.surface_y(0, 0).unwrap_or(80);
        let mut mining = Mining::default();
        mining.ensure_flier(DVec3::new(0.5, ground as f64, 0.5));
        (world, mining)
    }

    #[test]
    fn a_dry_fleet_stands_still_and_a_fuelled_one_flies() {
        // The fuel loop's whole claim, at the seam that matters: a tick the
        // fleet cannot pay for is a tick nobody works. Anything less and a
        // shortage would be a HUD line rather than a consequence.
        let (mut world, mut mining) = flying_world();
        let events = vx_core::EventBus::new();
        let ground = world.surface_y(0, 0).unwrap_or(80);
        mining.fleet.set_base(BlockPos::new(0, ground, 0));

        let before = mining.fleet.fliers[0].position;
        mining.advance(&mut world, &events, 40);
        assert!(mining.tank.dry, "an unfuelled fleet did not report itself dry");
        assert_eq!(
            mining.fleet.fliers[0].position, before,
            "a dry flier moved anyway"
        );

        // One canister and the same forty ticks: now it flies, and the tank
        // has paid for exactly the ticks it worked.
        if let Some(base) = mining.fleet.base.as_mut() {
            base.stockpile.add(crate::fuel::CELL, 1);
        }
        mining.advance(&mut world, &events, 40);
        assert!(!mining.tank.dry);
        assert_eq!(
            mining.tank.charge,
            crate::fuel::TICKS_PER_CELL - 40 * mining.burners(),
            "the tank did not pay for the ticks it bought"
        );
    }

    #[test]
    fn an_idle_tank_is_never_drawn_on() {
        // No fleet, no burn: a player who has sold every machine should not
        // watch their fuel evaporate.
        let (mut world, mut mining) = flying_world();
        let events = vx_core::EventBus::new();
        let ground = world.surface_y(0, 0).unwrap_or(80);
        mining.fleet.set_base(BlockPos::new(0, ground, 0));
        mining.fleet.fliers.clear();
        if let Some(base) = mining.fleet.base.as_mut() {
            base.stockpile.add(crate::fuel::CELL, 2);
        }
        mining.advance(&mut world, &events, 200);
        assert_eq!(mining.burners(), 0);
        assert_eq!(
            mining.fleet.base.as_ref().unwrap().stockpile.count(crate::fuel::CELL),
            2,
            "an idle fleet burned fuel"
        );
    }

    #[test]
    fn the_roster_lists_every_machine_nearest_first() {
        let (world, mut mining) = flying_world();
        mining.fleet.add_flier(BlockPos::new(120, 100, 120));
        let _ = &world;

        let rows = mining.roster(DVec3::new(0.5, 80.0, 0.5));
        assert_eq!(rows.len(), 2, "both fliers should be listed");
        assert!(
            rows[0].distance <= rows[1].distance,
            "roster is not nearest first: {:?}",
            rows.iter().map(|row| row.distance).collect::<Vec<_>>()
        );
        assert_eq!(rows[0].name, "FLIER 1");
        assert!(rows.iter().all(|row| !row.state.is_empty()));
    }

    #[test]
    fn a_machine_eye_matches_where_its_rig_is_drawn() {
        // The view and the body must come from one interpolation, or a piloted
        // machine's camera drifts against its own hull between ticks.
        //
        // Stage 49a moved the camera off the hull and onto a gimbal under the
        // nose, so the offset is no longer a bare lift — but the property is
        // the one that always mattered and it is unchanged: whatever the
        // sub-tick fraction, the eye is the drawn ground point plus the mount,
        // turned by the same heading the rig is drawn with.
        let (_, mut mining) = flying_world();
        mining.fleet.fliers[0].previous_position = BlockPos::new(0, 90, 0);
        mining.fleet.fliers[0].position = BlockPos::new(4, 94, 4);
        mining.previous_flier_yaws = vec![0.6];
        mining.flier_yaws = vec![1.9];

        for fraction in [0.0f64, 0.25, 0.5, 0.9] {
            mining.pending = fraction;
            let drawn = mining.interpolated(
                mining.fleet.fliers[0].previous_position,
                mining.fleet.fliers[0].position,
            );
            let heading = mining.heading(MachineRef::Flier(0));
            let mount = glam::Quat::from_rotation_y(heading) * Mining::gimbal(MachineRef::Flier(0));
            let eye = mining.machine_eye(MachineRef::Flier(0)).unwrap();
            assert!(
                (eye - (drawn + mount.as_dvec3())).length() < 1.0e-5,
                "eye {eye:?} does not sit on the drawn hull {drawn:?} at fraction {fraction}"
            );
        }
    }

    /// The bug the gimbal exists to fix, stated as a property: the camera is
    /// **outside** the machine it is bolted to. The old lift put the digger's
    /// eye inside its own cab box and the kestrel's above its own rotor.
    #[test]
    fn the_gimbal_hangs_outside_every_hull() {
        let machines = [
            (MachineRef::Digger(0), Rig::digger()),
            (MachineRef::Flier(0), Rig::flier()),
            (MachineRef::Kestrel, Rig::kestrel()),
        ];
        for (machine, rig) in machines {
            let mount = Mining::gimbal(machine);
            assert!(
                mount.x > 0.0,
                "{machine:?}'s camera is not ahead of its own centre"
            );
            assert!(mount.y > 0.0, "{machine:?}'s camera is underground");
            for part in &rig.parts {
                let half = part.size * 0.5;
                let inside = (mount.x - part.centre.x).abs() < half.x
                    && (mount.y - part.centre.y).abs() < half.y
                    && (mount.z - part.centre.z).abs() < half.z;
                assert!(
                    !inside,
                    "{machine:?}'s camera is inside its own {:?} at {mount:?}",
                    part.centre
                );
            }
        }
    }

    /// A nose swinging past the wrap point takes the short way round, not
    /// very nearly a full revolution the other way.
    #[test]
    fn a_turning_nose_glides_and_takes_the_short_way() {
        let quarter = std::f32::consts::FRAC_PI_2;
        // An ordinary turn, halfway through.
        assert!((turned(0.0, quarter, 0.5) - quarter * 0.5).abs() < 1.0e-6);
        // Ends are exact.
        assert!((turned(0.3, 2.0, 0.0) - 0.3).abs() < 1.0e-6);
        assert!((turned(0.3, 2.0, 1.0) - 2.0).abs() < 1.0e-6);

        // The wrap: just under +pi to just over -pi is a hair of movement.
        let almost = std::f32::consts::PI - 0.05;
        let over = -std::f32::consts::PI + 0.05;
        let middle = turned(almost, over, 0.5);
        let step = (middle - almost).abs();
        assert!(
            step < 0.2,
            "the nose spun {step} the long way round instead of a hair"
        );
    }

    /// You do not see the machine you are looking out of.
    #[test]
    fn a_machine_is_not_drawn_in_its_own_feed() {
        let (_, mining) = flying_world();
        let watching = mining.objects(|at| at.as_vec3(), None).len();
        let through = mining
            .objects(|at| at.as_vec3(), Some(MachineRef::Flier(0)))
            .len();
        assert!(
            through < watching,
            "the flier still drew {through} of its own parts on its own feed"
        );
    }

    #[test]
    fn taking_control_of_a_flier_marks_it_piloted_and_hand_back_restores_it() {
        let (_, mut mining) = flying_world();
        assert_eq!(mining.piloted(), None);

        assert!(mining.take_control(MachineRef::Flier(0)));
        assert_eq!(mining.piloted(), Some(MachineRef::Flier(0)));
        assert_eq!(mining.fleet.fliers[0].state, FlierState::Manual);
        assert_eq!(
            mining.roster(DVec3::ZERO)[0].state,
            "PILOTED",
            "the roster should say who has the wheel"
        );
        // One pair of hands.
        assert!(!mining.take_control(MachineRef::Flier(0)));

        mining.release_control();
        assert_eq!(mining.piloted(), None);
        assert_ne!(mining.fleet.fliers[0].state, FlierState::Manual);
    }

    #[test]
    fn a_piloted_machine_takes_its_yaw_from_the_look_not_the_movement() {
        let (_, mut mining) = flying_world();
        mining.take_control(MachineRef::Flier(0));
        // Move it one way, look another.
        mining.fleet.fliers[0].previous_position = BlockPos::new(0, 90, 0);
        mining.fleet.fliers[0].position = BlockPos::new(1, 90, 0);
        mining.set_pilot_look(2.5);
        mining.remember_yaws();
        assert_eq!(mining.flier_yaws[0], 2.5, "the nose ignored the pilot's view");

        // And an autonomous one still follows its own travel.
        mining.release_control();
        mining.remember_yaws();
        assert_ne!(mining.flier_yaws[0], 2.5);
    }

    #[test]
    fn a_piloted_machine_covers_the_same_ground_per_second_as_an_autonomous_one() {
        // Piloting is a change of driver, not of physics: one cell per tick.
        let (mut world, mut mining) = flying_world();
        let events = EventBus::new();
        mining.take_control(MachineRef::Flier(0));
        mining.set_pilot_command(PilotCommand {
            heading: Some(vx_agent::Heading::PosX),
            ..Default::default()
        });

        let start = mining.fleet.fliers[0].position;
        // Two seconds of held input at the fixed tick rate.
        for _ in 0..120 {
            mining.update(&mut world, &events, Duration::from_secs_f32(1.0 / 60.0));
        }
        let travelled = mining.fleet.fliers[0].position.x - start.x;
        let expected = (2.0 * TICK_RATE) as i32;
        assert!(
            (travelled - expected).abs() <= 2,
            "piloted travel was {travelled} cells, the tick rate says about {expected}"
        );
    }

    #[test]
    fn held_pilot_input_is_reissued_once_per_sub_tick_not_once_per_frame() {
        // One long frame must advance the machine as many cells as the ticks
        // it covers, or piloting would stutter with the frame rate.
        let (mut world, mut mining) = flying_world();
        let events = EventBus::new();
        mining.take_control(MachineRef::Flier(0));
        mining.set_pilot_command(PilotCommand {
            heading: Some(vx_agent::Heading::PosX),
            ..Default::default()
        });

        let start = mining.fleet.fliers[0].position;
        // Half a second in a single frame is four ticks at 8 Hz.
        mining.update(&mut world, &events, Duration::from_secs_f32(0.5));
        let travelled = mining.fleet.fliers[0].position.x - start.x;
        assert!(
            travelled >= 3,
            "one long frame only advanced {travelled} cells; input was not re-issued"
        );
    }

    #[test]
    fn a_marked_area_has_eight_distinct_corners() {
        let area = VoxelAabb::new(BlockPos::new(0, 60, 0), BlockPos::new(3, 63, 3));
        let corners = area_corners(area);
        assert_eq!(corners.len(), 8);

        // The box wraps the blocks rather than sitting on their origins, so it
        // spans four blocks, not three.
        let xs: Vec<f64> = corners.iter().map(|corner| corner.x).collect();
        assert_eq!(
            xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                - xs.iter().cloned().fold(f64::INFINITY, f64::min),
            4.0
        );
    }

    #[test]
    fn the_first_corner_alone_does_not_make_an_area() {
        let mut world = World::new(1);
        let mut mining = Mining::default();
        mining.mark(&mut world, BlockPos::new(0, 60, 0));

        assert!(mining.area.is_none());
        assert!(mining.selected_plan().is_none());
        // Still worth drawing, so the player can see the click landed.
        assert_eq!(mining.objects(|at| at.as_vec3(), None).len(), 1);
    }

    #[test]
    fn a_third_mark_starts_a_fresh_area() {
        // Otherwise a mis-click would need a separate cancel key to undo.
        let mut world = World::new(1);
        let mut mining = Mining::default();
        mining.mark(&mut world, BlockPos::new(0, 60, 0));
        mining.mark(&mut world, BlockPos::new(3, 63, 3));
        assert!(mining.area.is_some());

        mining.mark(&mut world, BlockPos::new(10, 60, 10));
        assert!(mining.area.is_none());
        assert_eq!(mining.corners, vec![BlockPos::new(10, 60, 10)]);
    }

    #[test]
    fn cycling_wraps_around_the_available_methods() {
        let mut mining = Mining::default();
        // No plans: cycling must not divide by zero or panic.
        mining.cycle_method();
        assert_eq!(mining.chosen, 0);
    }

    #[test]
    fn cancelling_clears_everything() {
        let mut world = World::new(1);
        let mut mining = Mining::default();
        mining.mark(&mut world, BlockPos::new(0, 60, 0));
        mining.mark(&mut world, BlockPos::new(3, 63, 3));

        mining.cancel(&mut world);
        assert!(mining.area.is_none());
        assert!(mining.objects(|at| at.as_vec3(), None).is_empty());
        assert!(!mining.is_running());
    }

    #[test]
    fn a_long_stall_does_not_run_hundreds_of_ticks_at_once() {
        // Catch-up has to be capped, or the frame after a window drag runs the
        // whole excavation in one go and the drone appears to teleport.
        let mut mining = Mining::default();
        let mut world = World::new(1);
        let events = EventBus::new();
        // No operation running — but the fleet still metes out its ticks
        // through the same accumulator, so the invariant is the clamp, not
        // emptiness.
        mining.update(&mut world, &events, Duration::from_secs(30));
        assert!(mining.pending <= 1.0, "backlog survived the stall frame");
    }

    #[test]
    fn the_backlog_from_a_stall_is_discarded_not_drained() {
        // Review finding A4: the cap alone kept the backlog and drained it at
        // 16 ticks per frame — a ~120x fast-forward for seconds after a stall.
        // Time lost to a stall must stay lost.
        let mut mining = Mining {
            operation: Some(Operation::new(BlockPos::new(0, 61, 0))),
            ..Mining::default()
        };

        let mut world = World::new(1);
        let events = EventBus::new();
        mining.update(&mut world, &events, Duration::from_secs(30));

        assert!(
            mining.pending <= 1.0,
            "{} ticks still queued after the stall frame; the next frames will fast-forward",
            mining.pending
        );
    }
}
