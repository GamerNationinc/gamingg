//! Drone swarms: what needs digging, how to reach it, and who is doing it.
//!
//! This crate depends on `vx-world` and `vx-core` and **nothing to do with
//! rendering**. That is deliberate and enforced by the manifest rather than by
//! discipline: the swarm has to be testable headlessly and at speed, and the
//! reliable way to keep it that way is to make the alternative unavailable.
//!
//! The pieces, in the order they matter:
//!
//! - [`aabb`] — integer boxes, the unit every job and excavation is described in.
//! - [`flow`] — what a machine that *drives* can and cannot get to. One
//!   breadth-first sweep serves any number of drones.
//! - [`mine`] — adit, decline or open pit: how to open a mine on a body, which
//!   for a ground drone is the same question as how to reach it.
//! - [`heap`] — the same question with the sign flipped: what to stack the
//!   spoil into, and in what order so that nothing is ever built on air.
//! - [`job`] — the shared board. Drones take work from it rather than each
//!   planning their own.
//! - [`drone`] — one machine's position, cargo and reach.
//! - [`operation`] — the tick that turns a plan into a hole in the ground.
//! - [`prospect`] — finding ore: by eye, and by the scanner's column walk.
//! - [`flier`] — the aircraft: trivial movement, above all the trouble.
//! - [`fleet`] — the air coordinator: sweeps, pings, the ferry loop, the base.
//! - [`stockpile`] — what came back, keyed by block name.

pub mod aabb;
pub mod drone;
#[cfg(test)]
mod fixture;
pub mod fleet;
pub mod flier;
pub mod flow;
pub mod heap;
pub mod job;
pub mod kestrel;
pub mod mine;
pub mod operation;
pub mod pilot;
pub mod prospect;
pub mod stockpile;

pub use aabb::VoxelAabb;
pub use drone::{Drone, DroneSnapshot, DroneState, DEFAULT_CAPACITY, DEFAULT_GRADE};
pub use fleet::{Base, Fleet, FleetReport, FleetSnapshot, SurveySnapshot};
pub use flier::{Flier, FlierState, CLEARANCE, DEFAULT_FLIER_CAPACITY};
pub use flow::{is_standable, settle, FlowField, STEP, UNREACHABLE};
pub use heap::{HeapPlan, HeapShape};
pub use job::{BoardSnapshot, DroneId, Job, JobBoard, JobId, JobKind};
pub use kestrel::{Kestrel, KestrelMode};
pub use mine::{options, plan, propose, MineMethod, MinePlan, PIT_MAX_DEPTH};
pub use operation::{
    working_span, Operation, OperationSnapshot, RunOutcome, TickReport, WORK_MARGIN,
};
pub use pilot::{Heading, PilotCommand, PilotReport};
pub use prospect::{find_body, is_ore, scan_columns, Ping, Sector, SCAN_DEPTH, SECTOR_SIZE};
pub use stockpile::Stockpile;
