//! The job board: what needs doing, and who has claimed it.
//!
//! # Why a board rather than plans
//!
//! Every drone deciding for itself what to do next means every drone paying for
//! that decision, which is what makes swarms expensive. Instead there is one
//! shared list of work and drones take from it. Adding a hundred drones adds a
//! hundred claims, not a hundred planners.
//!
//! # Claiming and completing are separate on purpose
//!
//! It would be tempting to retire a job the moment its region contains no
//! blocks. That works for digging and nothing else. A job that is *held* — a
//! well producing over time, a machine being tended — never empties anything,
//! and inferring completion from the world would have no answer for it. Making
//! completion an explicit act keeps that door open at no cost today.

use vx_core::BlockPos;

use crate::aabb::VoxelAabb;

/// Identifies a job for its whole life. Never reused, so a stale reference to a
/// finished job resolves to nothing instead of to whatever took its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

/// Identifies a drone. Same reasoning as [`JobId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DroneId(pub u32);

/// What a job is for.
///
/// The kind carries no behaviour — it exists so a report can say "still cutting
/// the ramp" rather than "still working", and so access can be ordered ahead of
/// extraction without relying on priority numbers meaning something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Excavation that creates the route to a body.
    Access,
    /// The body itself.
    Extract,
}

/// One piece of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub region: VoxelAabb,
    /// Higher goes first. Access carries more than extraction so the way in is
    /// cut before the ore it reaches — dig the body first and there is no route
    /// to haul it out along.
    pub priority: i32,
}

#[derive(Debug, Clone)]
struct Entry {
    job: Job,
    claimed_by: Option<DroneId>,
}

/// The shared list of outstanding work.
#[derive(Debug, Default)]
pub struct JobBoard {
    entries: Vec<Entry>,
    next_id: u64,
}

/// A board's whole persistent state, as plain data.
///
/// The entries and the id counter are private to the board — a claim is not
/// something a caller should be able to forge — but they have to survive a
/// save, or a crew restored from disk comes back to work nobody posted and
/// claims it has already made. Public fields, no behaviour; see
/// [`crate::drone::DroneSnapshot`] for the argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot {
    /// Each job and who, if anybody, holds it.
    pub entries: Vec<(Job, Option<DroneId>)>,
    /// The next id to hand out. Ids are never reused, so this has to come
    /// back with the board or a fresh job could be handed an id a stale
    /// reference still points at.
    pub next_id: u64,
}

impl JobBoard {
    pub fn new() -> Self {
        JobBoard::default()
    }

    /// Everything about this board that outlives a session.
    pub fn snapshot(&self) -> BoardSnapshot {
        BoardSnapshot {
            entries: self
                .entries
                .iter()
                .map(|entry| (entry.job.clone(), entry.claimed_by))
                .collect(),
            next_id: self.next_id,
        }
    }

    /// Build a board back from one.
    pub fn restore(snapshot: BoardSnapshot) -> Self {
        JobBoard {
            entries: snapshot
                .entries
                .into_iter()
                .map(|(job, claimed_by)| Entry { job, claimed_by })
                .collect(),
            next_id: snapshot.next_id,
        }
    }

    /// Add work, returning its id.
    pub fn post(&mut self, kind: JobKind, region: VoxelAabb, priority: i32) -> JobId {
        let id = JobId(self.next_id);
        self.next_id += 1;
        self.entries.push(Entry {
            job: Job {
                id,
                kind,
                region,
                priority,
            },
            claimed_by: None,
        });
        id
    }

    /// Hand `drone` the best unclaimed job, or `None` when there is none.
    ///
    /// Highest priority first, then nearest to `from`. Priority outranks
    /// distance so a drone standing on the orebody still goes and cuts the ramp
    /// first; distance then decides between equals, which is the work-stealing
    /// part — whoever is closest takes it, with no assignment step.
    pub fn claim_nearest(&mut self, drone: DroneId, from: BlockPos) -> Option<Job> {
        let best = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.claimed_by.is_none())
            .min_by_key(|(_, entry)| {
                let centre = entry.job.region.centre();
                let dx = (centre.x - from.x) as i64;
                let dy = (centre.y - from.y) as i64;
                let dz = (centre.z - from.z) as i64;
                // Negated priority so the sort stays a plain minimum.
                (-entry.job.priority, dx * dx + dy * dy + dz * dz, entry.job.id)
            })
            .map(|(index, _)| index)?;

        self.entries[best].claimed_by = Some(drone);
        Some(self.entries[best].job.clone())
    }

    /// Give a claimed job back to the board.
    ///
    /// What happens when a drone breaks down mid-task: the work is still needed
    /// and someone else should be able to take it.
    pub fn release(&mut self, id: JobId) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.job.id == id) {
            entry.claimed_by = None;
        }
    }

    /// Release everything `drone` was holding.
    pub fn release_all(&mut self, drone: DroneId) {
        for entry in &mut self.entries {
            if entry.claimed_by == Some(drone) {
                entry.claimed_by = None;
            }
        }
    }

    /// Retire a job. Returns whether it was there to retire.
    pub fn complete(&mut self, id: JobId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.job.id != id);
        self.entries.len() != before
    }

    pub fn get(&self, id: JobId) -> Option<&Job> {
        self.entries
            .iter()
            .find(|entry| entry.job.id == id)
            .map(|entry| &entry.job)
    }

    pub fn claimant(&self, id: JobId) -> Option<DroneId> {
        self.entries
            .iter()
            .find(|entry| entry.job.id == id)
            .and_then(|entry| entry.claimed_by)
    }

    /// Jobs still on the board, claimed or not.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Jobs nobody has taken yet.
    pub fn unclaimed_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.claimed_by.is_none())
            .count()
    }

    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.entries.iter().map(|entry| &entry.job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: i32) -> VoxelAabb {
        VoxelAabb::new(BlockPos::new(x, 60, 0), BlockPos::new(x + 2, 62, 2))
    }

    #[test]
    fn a_posted_job_can_be_claimed_and_completed() {
        let mut board = JobBoard::new();
        let id = board.post(JobKind::Extract, region(0), 0);
        assert_eq!(board.len(), 1);

        let claimed = board.claim_nearest(DroneId(1), BlockPos::new(0, 60, 0)).unwrap();
        assert_eq!(claimed.id, id);
        assert_eq!(board.claimant(id), Some(DroneId(1)));

        assert!(board.complete(id));
        assert!(board.is_empty());
        assert!(!board.complete(id), "completing twice should report nothing done");
    }

    #[test]
    fn a_claimed_job_is_never_handed_out_again() {
        // The failure mode work-stealing queues have: two drones digging the
        // same hole, and the haul counted twice.
        let mut board = JobBoard::new();
        board.post(JobKind::Extract, region(0), 0);

        let first = board.claim_nearest(DroneId(1), BlockPos::new(0, 60, 0));
        let second = board.claim_nearest(DroneId(2), BlockPos::new(0, 60, 0));

        assert!(first.is_some());
        assert!(second.is_none(), "the same job was issued to two drones");
    }

    #[test]
    fn every_job_is_issued_exactly_once_across_many_drones() {
        let mut board = JobBoard::new();
        for x in 0..25 {
            board.post(JobKind::Extract, region(x * 4), 0);
        }

        let mut seen = Vec::new();
        for drone in 0..40u32 {
            if let Some(job) = board.claim_nearest(DroneId(drone), BlockPos::new(0, 60, 0)) {
                seen.push(job.id);
            }
        }

        let unique: std::collections::HashSet<JobId> = seen.iter().copied().collect();
        assert_eq!(seen.len(), 25, "not every job was taken");
        assert_eq!(unique.len(), 25, "a job was issued more than once");
        assert_eq!(board.unclaimed_count(), 0);
    }

    #[test]
    fn priority_outranks_distance() {
        // Access jobs must be cut before extraction even when a drone is
        // standing on the orebody, or it digs out ore it cannot haul away.
        let mut board = JobBoard::new();
        let far_but_urgent = board.post(JobKind::Access, region(200), 10);
        board.post(JobKind::Extract, region(0), 0);

        let claimed = board.claim_nearest(DroneId(1), BlockPos::new(0, 60, 0)).unwrap();
        assert_eq!(claimed.id, far_but_urgent);
        assert_eq!(claimed.kind, JobKind::Access);
    }

    #[test]
    fn among_equal_priorities_the_nearest_wins() {
        let mut board = JobBoard::new();
        board.post(JobKind::Extract, region(100), 0);
        let near = board.post(JobKind::Extract, region(4), 0);
        board.post(JobKind::Extract, region(60), 0);

        let claimed = board.claim_nearest(DroneId(1), BlockPos::new(0, 60, 0)).unwrap();
        assert_eq!(claimed.id, near);
    }

    #[test]
    fn releasing_puts_work_back_for_someone_else() {
        // What a breakdown does: the job did not get done and must not be lost.
        let mut board = JobBoard::new();
        let id = board.post(JobKind::Extract, region(0), 0);

        board.claim_nearest(DroneId(1), BlockPos::new(0, 60, 0)).unwrap();
        assert!(board.claim_nearest(DroneId(2), BlockPos::new(0, 60, 0)).is_none());

        board.release(id);
        let retaken = board.claim_nearest(DroneId(2), BlockPos::new(0, 60, 0)).unwrap();
        assert_eq!(retaken.id, id);
        assert_eq!(board.claimant(id), Some(DroneId(2)));
    }

    #[test]
    fn releasing_everything_a_drone_held_frees_all_of_it() {
        let mut board = JobBoard::new();
        board.post(JobKind::Extract, region(0), 0);
        board.post(JobKind::Extract, region(10), 0);

        board.claim_nearest(DroneId(7), BlockPos::new(0, 60, 0));
        board.claim_nearest(DroneId(7), BlockPos::new(0, 60, 0));
        assert_eq!(board.unclaimed_count(), 0);

        board.release_all(DroneId(7));
        assert_eq!(board.unclaimed_count(), 2);
    }

    #[test]
    fn ids_are_never_reused_after_completion() {
        // A drone holding a finished job's id must not find someone else's work
        // under it.
        let mut board = JobBoard::new();
        let first = board.post(JobKind::Extract, region(0), 0);
        board.complete(first);
        let second = board.post(JobKind::Extract, region(0), 0);

        assert_ne!(first, second);
        assert!(board.get(first).is_none());
    }

    #[test]
    fn claiming_from_an_empty_board_is_not_an_error() {
        let mut board = JobBoard::new();
        assert!(board.claim_nearest(DroneId(1), BlockPos::new(0, 0, 0)).is_none());
    }
}
