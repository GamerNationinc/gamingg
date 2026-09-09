//! The flying drone: the half of the supply chain that is above all the
//! trouble.
//!
//! # Movement is trivial by design
//!
//! The ground drone's whole existence is shaped by what it cannot climb; the
//! flier's is shaped by not caring. It flies at a clearance above the surface
//! of whatever column it is over, steps one block horizontally per tick, and
//! climbs or descends as the ground underneath demands. No flow fields, no
//! reachability — not needing them *is the fantasy of the flier*, and it is
//! also what makes it nearly free to simulate.
//!
//! # The one rule that is not negotiable
//!
//! The flier is never inside terrain. It only advances into a column once it
//! is already above that column's safe altitude, climbing first if it must.
//! A cliff therefore costs climb time instead of a collision, which is both
//! the honest physics and the visible behaviour: the aircraft rears up over a
//! ridge instead of clipping through it.

use vx_core::BlockPos;
use vx_world::World;

use crate::prospect::Sector;
use crate::stockpile::Stockpile;

/// Blocks of air kept between the flier and the ground below it.
pub const CLEARANCE: i32 = 6;

/// Blocks the flier can climb or descend in one tick. Faster than the ground
/// drone's single step because rotors, but bounded so a cliff still reads as a
/// climb rather than a teleport.
pub const CLIMB_RATE: i32 = 3;

/// Blocks a starting flier can carry per ferry trip.
pub const DEFAULT_FLIER_CAPACITY: u64 = 96;

/// Half-width of the scanner's swath: columns within this distance of the
/// flight line count as scanned. Wider swaths (an upgrade hook) mean fewer
/// passes per sector.
pub const SWATH: i32 = 8;

/// What a flier is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlierState {
    Idle,
    /// Sweeping a sector, `waypoint` steps into its lawnmower path.
    Scanning { sector: Sector, waypoint: usize },
    /// Flying to a mine's stockpile to load.
    ToPickup { mine: usize },
    /// Flying home to the base container.
    ToBase,
    /// Under the player's direct control.
    Manual,
}

/// A single flying drone.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Flier {
    pub position: BlockPos,
    /// Where it was last tick, for the renderer to interpolate from.
    pub previous_position: BlockPos,
    pub state: FlierState,
    pub cargo: Stockpile,
    pub capacity: u64,
}

impl Flier {
    pub fn new(position: BlockPos) -> Self {
        Flier {
            position,
            previous_position: position,
            state: FlierState::Idle,
            cargo: Stockpile::new(),
            capacity: DEFAULT_FLIER_CAPACITY,
        }
    }

    pub fn carrying(&self) -> u64 {
        self.cargo.total()
    }

    /// Move, keeping the previous position for the renderer to interpolate
    /// from. Public since the scout round: the kestrel and the roost reuse
    /// the flier's movement wholesale rather than growing their own.
    pub fn move_to(&mut self, to: BlockPos) {
        self.previous_position = self.position;
        self.position = to;
    }

    /// The lowest altitude the flier may occupy over the column `(x, z)`.
    ///
    /// Over unloaded ground there is no surface to read; hold altitude rather
    /// than descending into the unknown.
    pub fn safe_altitude(world: &World, x: i32, z: i32, fallback: i32) -> i32 {
        world
            .surface_y(x, z)
            .map(|clear| clear + CLEARANCE)
            .unwrap_or(fallback)
    }

    /// Fly one tick toward the column `(x, z)`. Returns true on arrival —
    /// hovering anywhere over the target column at a safe altitude.
    ///
    /// The order of concerns is the invariant: climb to the *next* column's
    /// safe altitude before entering it, never after.
    pub fn fly_towards(&mut self, world: &World, target: (i32, i32)) -> bool {
        let here = self.position;
        let (dx, dz) = (target.0 - here.x, target.1 - here.z);

        if dx == 0 && dz == 0 {
            // Over the target: settle toward its safe altitude and report
            // arrival once cruising there.
            let desired = Self::safe_altitude(world, here.x, here.z, here.y);
            let step = (desired - here.y).clamp(-CLIMB_RATE, CLIMB_RATE);
            if step != 0 {
                self.move_to(here.offset([0, step, 0]));
            }
            return self.position.y == desired;
        }

        // One block along the axis with the most distance left, tie broken
        // toward x so routes are deterministic.
        let step = if dx.abs() >= dz.abs() {
            [dx.signum(), 0, 0]
        } else {
            [0, 0, dz.signum()]
        };
        let next = (here.x + step[0], here.z + step[2]);

        // Never advance into a column while below its safe altitude.
        let needed = Self::safe_altitude(world, next.0, next.1, here.y);
        if here.y < needed {
            let climb = (needed - here.y).min(CLIMB_RATE);
            self.move_to(here.offset([0, climb, 0]));
            return false;
        }

        // Advance, descending toward the new column's cruise height if there
        // is room to.
        let descend = (needed - here.y).clamp(-CLIMB_RATE, 0);
        self.move_to(BlockPos::new(next.0, here.y + descend, next.1));
        false
    }
}

/// The serpentine flight path that covers a sector with `SWATH`-wide passes.
///
/// Rows are spaced two swath-widths apart so adjacent passes tile the sector
/// exactly; the x direction alternates so the turn at each row end is short.
pub fn sweep_path(sector: Sector) -> Vec<(i32, i32)> {
    let (min_x, min_z) = sector.min_column();
    let size = crate::prospect::SECTOR_SIZE;

    let mut path = Vec::new();
    let mut row = 0;
    let mut z = min_z + SWATH;
    while z < min_z + size + SWATH {
        let row_z = z.min(min_z + size - 1);
        let xs: Vec<i32> = if row % 2 == 0 {
            (min_x..min_x + size).collect()
        } else {
            (min_x..min_x + size).rev().collect()
        };
        for x in xs {
            path.push((x, row_z));
        }
        row += 1;
        z += SWATH * 2;
    }
    path
}

/// The columns a flier at `at` covers with its scanner.
pub fn swath_columns(at: (i32, i32)) -> impl Iterator<Item = (i32, i32)> {
    (-SWATH..=SWATH).map(move |dz| (at.0, at.1 + dz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use std::collections::HashSet;

    #[test]
    fn the_sweep_path_covers_every_column_of_the_sector() {
        // A row spacing bug leaves blind stripes the scanner honestly reports
        // as "no ore", which is worse than a crash: it looks like truth.
        let sector = Sector { x: 0, z: 0 };
        let covered: HashSet<(i32, i32)> = sweep_path(sector)
            .into_iter()
            .flat_map(swath_columns)
            .collect();

        for column in sector.columns() {
            assert!(covered.contains(&column), "{column:?} is never scanned");
        }
    }

    #[test]
    fn the_flier_reaches_a_target_over_flat_ground() {
        let world = fixture::flat(4, 60);
        let mut flier = Flier::new(BlockPos::new(0, 67, 0));

        let mut arrived = false;
        for _ in 0..200 {
            if flier.fly_towards(&world, (40, 25)) {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "never arrived; stuck at {:?}", flier.position);
        assert_eq!((flier.position.x, flier.position.z), (40, 25));
        assert_eq!(flier.position.y, 61 + CLEARANCE);
    }

    #[test]
    fn the_flier_is_never_inside_terrain_crossing_a_cliff() {
        // The non-negotiable rule, exercised over a 30-block wall: the flier
        // must climb first and cross after, never clip through.
        let world = fixture::shaped(4, |x| if x < 20 { 40 } else { 70 });
        let mut flier = Flier::new(BlockPos::new(0, 41 + CLEARANCE, 0));

        for tick in 0..400 {
            let arrived = flier.fly_towards(&world, (40, 0));
            assert!(
                !world.is_solid(flier.position),
                "inside the cliff at {:?} on tick {tick}",
                flier.position
            );
            if arrived {
                return;
            }
        }
        panic!("never made it over the cliff");
    }

    #[test]
    fn climbing_is_rate_limited_rather_than_instant() {
        let world = fixture::shaped(4, |x| if x < 5 { 40 } else { 90 });
        let mut flier = Flier::new(BlockPos::new(0, 41 + CLEARANCE, 0));

        let mut last_y = flier.position.y;
        for _ in 0..100 {
            flier.fly_towards(&world, (20, 0));
            assert!(
                (flier.position.y - last_y).abs() <= CLIMB_RATE,
                "climbed {} in one tick",
                flier.position.y - last_y
            );
            last_y = flier.position.y;
        }
    }

    #[test]
    fn arrival_means_hovering_at_cruise_height_over_the_target() {
        let world = fixture::flat(2, 60);
        let mut flier = Flier::new(BlockPos::new(5, 100, 5));

        // Already over the column, far too high: arrival only once descended.
        let mut ticks = 0;
        while !flier.fly_towards(&world, (5, 5)) {
            ticks += 1;
            assert!(ticks < 50, "never settled to cruise height");
        }
        assert_eq!(flier.position.y, 61 + CLEARANCE);
    }
}
