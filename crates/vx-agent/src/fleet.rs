//! The air side of the operation: scanning sectors and ferrying ore home.
//!
//! Mirrors [`crate::operation`]: a coordinator owning the aircraft and the
//! knowledge they produce, ticked once per simulation step. The division of
//! labour is deliberate — [`crate::flier::Flier`] knows how to fly, and the
//! fleet decides where to; drones stay dumb so the swarm stays cheap.
//!
//! # Scanning is progressive
//!
//! A sweep takes real time, and pings exist only for ground already overflown:
//! interrupt a scan halfway and you know half the sector. The covered set is
//! reclustered as the sweep advances, so two halves of one body found on
//! neighbouring passes merge into a single ping rather than lingering as two.
//!
//! # The chain conserves blocks
//!
//! Mine stockpiles, flier cargo and the base pile are the same blocks moving
//! through stations. While nothing digs, their total is constant — the same
//! conservation discipline the ground operation is held to, extended across
//! the whole chain, and the test that catches a duplicated or dropped load.

use std::collections::{HashMap, HashSet};

use vx_core::BlockPos;
use vx_world::World;

use crate::flier::{sweep_path, swath_columns, Flier, FlierState};
use crate::operation::Operation;
use crate::prospect::{column_hit, cluster_pings, Ping, Sector};
use crate::stockpile::Stockpile;

/// The base: a container block the player placed, and what has arrived in it.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Base {
    pub position: BlockPos,
    pub stockpile: Stockpile,
}

/// One partly- or fully-swept sector.
#[derive(Debug, Clone, Default)]
struct Survey {
    /// Columns the scanner has covered.
    covered: HashSet<(i32, i32)>,
    /// Covered columns with ore in range: `(depth, hover_y)` per column.
    hits: HashMap<(i32, i32), (i32, i32)>,
    complete: bool,
}

/// What one fleet tick achieved — the hooks player progression feeds on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FleetReport {
    /// Blocks unloaded into the base this tick.
    pub delivered: u64,
    /// Sector sweeps that finished this tick.
    pub sectors_completed: u32,
    /// Pings known in the sectors that finished this tick.
    pub pings_found: u32,
}

/// One sector's survey, as plain data. Sorted on the way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurveySnapshot {
    pub sector: Sector,
    pub covered: Vec<(i32, i32)>,
    /// `(column, (depth, hover_y))` for every covered column with ore in it.
    pub hits: Vec<((i32, i32), (i32, i32))>,
    pub complete: bool,
}

/// The air side's whole persistent state, as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSnapshot {
    pub fliers: Vec<Flier>,
    pub base: Option<Base>,
    pub scan_depth: i32,
    pub surveys: Vec<SurveySnapshot>,
    pub controlled: Option<usize>,
    /// Goods waiting for a container. Part of the *pile's* concern rather than
    /// the air side's, so [`crate::Fleet::snapshot`] carries it but the app's
    /// `fleet.dat` leaves it to `pile.dat`.
    pub orphan: Stockpile,
}

/// The fleet: fliers, the base, and everything the scanner has learned.
#[derive(Debug)]
pub struct Fleet {
    pub fliers: Vec<Flier>,
    pub base: Option<Base>,
    /// How deep the scanner senses, in blocks below the surface. Starts at
    /// [`crate::prospect::SCAN_DEPTH`]; the app raises it as the player's
    /// Prospecting level does.
    pub scan_depth: i32,
    surveys: HashMap<Sector, Survey>,
    /// Goods from a container that was broken, waiting for a new one. See
    /// [`Fleet::clear_base`].
    orphan: Stockpile,
    /// The flier the player is flying, if any.
    controlled: Option<usize>,
    /// What that flier was doing before the player took the stick.
    ///
    /// A survey lives *only* in the flier — unlike a mining job, there is no
    /// board to hand it back to — so taking control has to stash it or the
    /// sweep is simply lost.
    suspended: Option<FlierState>,
}

impl Default for Fleet {
    fn default() -> Self {
        Fleet {
            fliers: Vec::new(),
            base: None,
            scan_depth: crate::prospect::SCAN_DEPTH,
            surveys: HashMap::new(),
            orphan: Stockpile::new(),
            controlled: None,
            suspended: None,
        }
    }
}

impl Fleet {
    pub fn new() -> Self {
        Fleet::default()
    }

    pub fn add_flier(&mut self, position: BlockPos) -> usize {
        self.fliers.push(Flier::new(position));
        self.fliers.len() - 1
    }

    /// Declare the base at a placed container block. Replaces any previous
    /// base but keeps nothing from it — the old pile lives in the old block's
    /// world position conceptually, and losing track of it on replace would
    /// be a conservation leak, so the pile transfers.
    /// Everything about the air side that outlives a session.
    ///
    /// The fliers, how deep the scanner reaches, and — the one that matters —
    /// **every sector already surveyed and what it found**. A sweep costs HHO
    /// off the pile, and until stage 52 `pile.dat` wrote the base and nothing
    /// else, so the fuel was spent and the pings it bought were thrown away at
    /// the next save. The player was left re-scanning ground they had already
    /// paid to scan, with no way of knowing they had.
    ///
    /// Surveys come out in sector order and each survey's columns in column
    /// order: they live in `HashMap`s whose iteration order is not stable, and
    /// the same fleet must write the same bytes twice.
    pub fn snapshot(&self) -> FleetSnapshot {
        let mut surveys: Vec<SurveySnapshot> = self
            .surveys
            .iter()
            .map(|(sector, survey)| {
                let mut covered: Vec<(i32, i32)> = survey.covered.iter().copied().collect();
                covered.sort_unstable();
                let mut hits: Vec<((i32, i32), (i32, i32))> =
                    survey.hits.iter().map(|(at, hit)| (*at, *hit)).collect();
                hits.sort_unstable();
                SurveySnapshot {
                    sector: *sector,
                    covered,
                    hits,
                    complete: survey.complete,
                }
            })
            .collect();
        surveys.sort_unstable_by_key(|survey| (survey.sector.x, survey.sector.z));
        FleetSnapshot {
            fliers: self.fliers.clone(),
            base: self.base.clone(),
            scan_depth: self.scan_depth,
            surveys,
            controlled: self.controlled,
            orphan: self.orphan.clone(),
        }
    }

    /// Build a fleet back from one.
    ///
    /// `suspended` is not restored and cannot be: it is what a flier was doing
    /// before the player took its stick, and a reload hands the stick back —
    /// the flier arrives idle over the base and is re-tasked like any other.
    pub fn restore(snapshot: FleetSnapshot) -> Self {
        Fleet {
            fliers: snapshot.fliers,
            base: snapshot.base,
            scan_depth: snapshot.scan_depth,
            surveys: snapshot
                .surveys
                .into_iter()
                .map(|survey| {
                    (
                        survey.sector,
                        Survey {
                            covered: survey.covered.into_iter().collect(),
                            hits: survey.hits.into_iter().collect(),
                            complete: survey.complete,
                        },
                    )
                })
                .collect(),
            controlled: snapshot.controlled,
            orphan: snapshot.orphan,
            suspended: None,
        }
    }

    pub fn set_base(&mut self, position: BlockPos) {
        let mut stockpile = self
            .base
            .take()
            .map(|base| base.stockpile)
            .unwrap_or_default();
        // Anything left over from a container that was broken comes back.
        // Goods do not evaporate because the box holding them did; see
        // `clear_base`.
        for (name, count) in std::mem::take(&mut self.orphan).drain() {
            stockpile.add(name, count);
        }
        self.base = Some(Base {
            position,
            stockpile,
        });
    }

    /// The container was broken: no base until another is placed.
    ///
    /// **The goods are kept**, not returned to the caller to drop on the
    /// floor. Breaking your own container used to destroy everything in it —
    /// `main.rs` took the pile back, logged how much was "set aside", and let
    /// it fall out of scope — so the single most expensive mistake available
    /// to a player was mining one block by accident. Now the pile is held
    /// undeclared and the next container you place picks it up.
    ///
    /// Returns how much went into holding, for the line the player is shown.
    pub fn clear_base(&mut self) -> u64 {
        let Some(mut base) = self.base.take() else {
            return 0;
        };
        let held = base.stockpile.total();
        for (name, count) in base.stockpile.drain() {
            self.orphan.add(name, count);
        }
        held
    }

    /// Take the held goods out, leaving none. What a loader uses when it is
    /// about to rebuild the fleet around them.
    pub fn orphan_take(&mut self) -> Stockpile {
        std::mem::take(&mut self.orphan)
    }

    /// Put goods into holding directly. What a loader uses to restore what a
    /// broken container was carrying.
    pub fn orphan_goods(&mut self, goods: impl IntoIterator<Item = (String, u64)>) {
        for (name, count) in goods {
            self.orphan.add(name, count);
        }
    }

    /// Goods with no container to sit in, waiting for one.
    pub fn orphaned(&self) -> &Stockpile {
        &self.orphan
    }

    /// Everything the fleet is holding anywhere on the ground: the declared
    /// pile plus whatever is waiting for a container. The number a
    /// conservation check has to use, because a broken box moves goods
    /// between the two without changing the total.
    pub fn held(&self) -> u64 {
        self.base
            .as_ref()
            .map_or(0, |base| base.stockpile.total())
            .saturating_add(self.orphan.total())
    }

    /// Send an idle flier to sweep `sector`. Returns whether one was free.
    ///
    /// Re-dispatching a finished sector rescans it — that is a feature, not a
    /// waste: the world changes, and a stale survey lies.
    pub fn dispatch_scan(&mut self, sector: Sector) -> bool {
        let Some(index) = self
            .fliers
            .iter()
            .position(|flier| flier.state == FlierState::Idle)
        else {
            return false;
        };
        self.surveys.insert(sector, Survey::default());
        self.fliers[index].state = FlierState::Scanning { sector, waypoint: 0 };
        true
    }

    /// Every ping the fleet currently knows, across all surveys, in a stable
    /// order.
    pub fn pings(&self) -> Vec<Ping> {
        let mut sectors: Vec<&Sector> = self.surveys.keys().collect();
        sectors.sort_by_key(|sector| (sector.x, sector.z));
        sectors
            .into_iter()
            .flat_map(|sector| cluster_pings(&self.surveys[sector].hits))
            .collect()
    }

    /// Has this sector been fully swept?
    pub fn is_surveyed(&self, sector: Sector) -> bool {
        self.surveys
            .get(&sector)
            .is_some_and(|survey| survey.complete)
    }

    /// Sectors fully swept, for the map to shade as explored.
    pub fn surveyed_sectors(&self) -> impl Iterator<Item = Sector> + '_ {
        self.surveys
            .iter()
            .filter(|(_, survey)| survey.complete)
            .map(|(sector, _)| *sector)
    }

    /// Advance every flier one tick, reporting what happened.
    pub fn tick(&mut self, world: &World, mines: &mut [Operation]) -> FleetReport {
        let mut report = FleetReport::default();
        for index in 0..self.fliers.len() {
            // The player is flying this one.
            if self.controlled == Some(index) {
                continue;
            }
            self.tick_flier(index, world, mines, &mut report);
        }
        report
    }

    /// Take the stick from a flier.
    ///
    /// Its current task is **stashed, not discarded**: a survey exists only in
    /// the flier's own state, so releasing it the way a mining job is released
    /// would throw away the whole sweep. Jobs are shared; surveys are not.
    pub fn take_control(&mut self, index: usize) -> bool {
        if index >= self.fliers.len() || self.controlled.is_some() {
            return false;
        }
        self.suspended = Some(self.fliers[index].state);
        self.fliers[index].state = FlierState::Manual;
        self.controlled = Some(index);
        true
    }

    /// Hand the stick back, and the flier picks its task up where it left off.
    pub fn release_control(&mut self, index: usize) -> bool {
        if self.controlled != Some(index) {
            return false;
        }
        self.fliers[index].state = self.suspended.take().unwrap_or(FlierState::Idle);
        self.controlled = None;
        true
    }

    /// Which flier the player is flying.
    pub fn controlled(&self) -> Option<usize> {
        self.controlled
    }

    /// Advance the piloted flier by one tick of the player's input.
    pub fn pilot_tick(
        &mut self,
        world: &World,
        command: crate::pilot::PilotCommand,
    ) -> crate::pilot::PilotReport {
        let mut report = crate::pilot::PilotReport::default();
        let Some(index) = self.controlled else {
            return report;
        };
        report.moved = self.fliers[index].pilot_step(world, command.heading, command.climb);
        report
    }

    fn tick_flier(
        &mut self,
        index: usize,
        world: &World,
        mines: &mut [Operation],
        report: &mut FleetReport,
    ) {
        match self.fliers[index].state {
            // A piloted flier is the player's business, not the fleet's.
            FlierState::Manual => {}
            FlierState::Idle => self.consider_ferrying(index, mines),
            FlierState::Scanning { sector, waypoint } => {
                self.advance_scan(index, world, sector, waypoint, report)
            }
            FlierState::ToPickup { mine } => {
                // The mine may have been dismantled between dispatch and
                // arrival; go home rather than orbiting a ghost.
                let Some(operation) = mines.get_mut(mine) else {
                    self.fliers[index].state = FlierState::Idle;
                    return;
                };
                let target = (operation.home.x, operation.home.z);
                if self.fliers[index].fly_towards(world, target) {
                    Self::transfer(&mut operation.stockpile, index, &mut self.fliers[..]);
                    self.fliers[index].state = FlierState::ToBase;
                }
            }
            FlierState::ToBase => {
                let Some(base) = &mut self.base else {
                    // Base broken mid-flight: hold the cargo and wait. The
                    // blocks stay aboard, so conservation holds.
                    self.fliers[index].state = FlierState::Idle;
                    return;
                };
                let target = (base.position.x, base.position.z);
                if self.fliers[index].fly_towards(world, target) {
                    let cargo = std::mem::take(&mut self.fliers[index].cargo);
                    report.delivered += cargo.total();
                    for (name, count) in cargo.entries() {
                        base.stockpile.add(name.to_string(), count);
                    }
                    self.fliers[index].state = FlierState::Idle;
                }
            }
        }
    }

    /// Idle, with a base and ore waiting somewhere: go get it.
    fn consider_ferrying(&mut self, index: usize, mines: &mut [Operation]) {
        if self.base.is_none() {
            return;
        }
        let here = self.fliers[index].position;
        let nearest = mines
            .iter()
            .enumerate()
            .filter(|(_, operation)| !operation.stockpile.is_empty())
            .min_by_key(|(_, operation)| {
                let (dx, dz) = (
                    (operation.home.x - here.x) as i64,
                    (operation.home.z - here.z) as i64,
                );
                dx * dx + dz * dz
            })
            .map(|(mine, _)| mine);

        if let Some(mine) = nearest {
            self.fliers[index].state = FlierState::ToPickup { mine };
        }
    }

    /// Load up to capacity from `pile` into flier `index`.
    fn transfer(pile: &mut Stockpile, index: usize, fliers: &mut [Flier]) {
        let flier = &mut fliers[index];
        let mut room = flier.capacity.saturating_sub(flier.carrying());
        let kinds: Vec<String> = pile.entries().map(|(name, _)| name.to_string()).collect();
        for name in kinds {
            if room == 0 {
                break;
            }
            let taken = pile.take(&name, room);
            flier.cargo.add(name, taken);
            room -= taken;
        }
    }

    /// One tick of a sweep: fly to the next waypoint, scan the swath there.
    fn advance_scan(
        &mut self,
        index: usize,
        world: &World,
        sector: Sector,
        waypoint: usize,
        report: &mut FleetReport,
    ) {
        let path = sweep_path(sector);
        let Some(&target) = path.get(waypoint) else {
            let survey = self.surveys.entry(sector).or_default();
            survey.complete = true;
            report.sectors_completed += 1;
            report.pings_found += cluster_pings(&survey.hits).len() as u32;
            self.fliers[index].state = FlierState::Idle;
            return;
        };

        if !self.fliers[index].fly_towards(world, target) {
            return;
        }

        // Over the waypoint: the swath under the flight line is now covered.
        let scan_depth = self.scan_depth;
        let survey = self.surveys.entry(sector).or_default();
        let (min_x, min_z) = sector.min_column();
        let size = crate::prospect::SECTOR_SIZE;
        for column in swath_columns(target) {
            let inside = (min_x..min_x + size).contains(&column.0)
                && (min_z..min_z + size).contains(&column.1);
            if inside && survey.covered.insert(column) {
                if let Some(hit) = column_hit(world, column.0, column.1, scan_depth) {
                    survey.hits.insert(column, hit);
                }
            }
        }

        self.fliers[index].state = FlierState::Scanning {
            sector,
            waypoint: waypoint + 1,
        };
    }

    /// Blocks held across the whole air side: aboard fliers plus in the base.
    ///
    /// Together with each mine's `accounted_blocks`, this is the conservation
    /// figure for the entire chain.
    pub fn accounted_blocks(&self) -> u64 {
        let aboard: u64 = self.fliers.iter().map(Flier::carrying).sum();
        aboard
            + self
                .base
                .as_ref()
                .map(|base| base.stockpile.total())
                .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aabb::VoxelAabb;
    use crate::fixture::{flat, ore_body};
    use crate::flier::CLEARANCE;

    /// A fleet with one flier hovering near the origin over flat ground.
    fn fleet_over(world: &World) -> Fleet {
        let clear = world.surface_y(0, 0).expect("origin loaded");
        let mut fleet = Fleet::new();
        fleet.add_flier(BlockPos::new(0, clear + CLEARANCE, 0));
        fleet
    }

    /// The air side survives a snapshot — fliers, base, and every sector the
    /// scanner has already covered.
    ///
    /// The surveys are the point. A sweep burns HHO off the pile, and until
    /// stage 52 `pile.dat` wrote the base and nothing else: the fuel was spent
    /// and the pings it bought were binned at the next save, so a player
    /// re-scanned ground they had already paid for and had no way to know it.
    #[test]
    fn a_fleet_round_trips_through_a_snapshot() {
        let mut world = flat(5, 60);
        ore_body(&mut world, VoxelAabb::new(BlockPos::new(10, 55, 4), BlockPos::new(12, 60, 6)));
        let mut fleet = fleet_over(&world);
        fleet.set_base(BlockPos::new(-2, 61, 3));
        if let Some(base) = fleet.base.as_mut() {
            base.stockpile.add("engine:copper_ore", 12);
        }
        fleet.scan_depth = 41;
        assert!(fleet.dispatch_scan(Sector { x: 0, z: 0 }));
        // Part way through a sweep on purpose: a half-known sector is the
        // interesting thing to save, because it is the state a player who
        // quit mid-scan is actually in.
        let mut ticks = 0;
        while fleet.pings().is_empty() {
            fleet.tick(&world, &mut []);
            ticks += 1;
            assert!(ticks < 5_000, "the sweep never found the body");
        }
        assert!(!fleet.is_surveyed(Sector { x: 0, z: 0 }));

        let snapshot = fleet.snapshot();
        let back = Fleet::restore(snapshot.clone());

        assert_eq!(back.scan_depth, 41);
        assert_eq!(
            back.base.as_ref().map(|base| base.stockpile.total()),
            Some(12)
        );
        assert_eq!(back.fliers.len(), fleet.fliers.len());
        assert_eq!(back.pings(), fleet.pings(), "the pings did not come back");
        assert_eq!(
            back.is_surveyed(Sector { x: 0, z: 0 }),
            fleet.is_surveyed(Sector { x: 0, z: 0 }),
            "a half-finished sweep came back finished, or the other way round"
        );
        // Same snapshot out of the restored fleet: sorted on the way out, so
        // the same fleet writes the same bytes however the maps hash today.
        assert_eq!(back.snapshot(), snapshot);
    }

    /// Run the fleet until the flier idles or `limit` ticks pass.
    fn run_until_idle(fleet: &mut Fleet, world: &World, mines: &mut [Operation], limit: u32) {
        for _ in 0..limit {
            fleet.tick(world, mines);
            if fleet.fliers.iter().all(|flier| flier.state == FlierState::Idle)
                && mines.iter().all(|mine| mine.stockpile.is_empty())
            {
                return;
            }
        }
    }

    #[test]
    fn taking_control_of_a_flier_suspends_its_survey_and_hand_back_resumes_it() {
        // A survey lives only in the flier — there is no board to hand it back
        // to — so a takeover must stash it rather than throw it away.
        let world = flat(5, 60);
        let mut fleet = fleet_over(&world);
        let sector = Sector::containing(20, 20);
        assert!(fleet.dispatch_scan(sector));
        for _ in 0..40 {
            fleet.tick(&world, &mut []);
        }
        let mid_sweep = fleet.fliers[0].state;
        assert!(
            matches!(mid_sweep, FlierState::Scanning { .. }),
            "expected a sweep in progress, got {mid_sweep:?}"
        );

        assert!(fleet.take_control(0));
        assert_eq!(fleet.controlled(), Some(0));
        assert_eq!(fleet.fliers[0].state, FlierState::Manual);

        assert!(fleet.release_control(0));
        assert_eq!(fleet.controlled(), None);
        assert_eq!(
            fleet.fliers[0].state, mid_sweep,
            "the survey was lost instead of resumed"
        );

        // And it actually finishes from there.
        for _ in 0..20_000 {
            fleet.tick(&world, &mut []);
            if fleet.is_surveyed(sector) {
                break;
            }
        }
        assert!(fleet.is_surveyed(sector), "the resumed sweep never completed");
    }

    #[test]
    fn a_controlled_flier_is_skipped_by_the_fleet_tick() {
        let world = flat(5, 60);
        let mut fleet = fleet_over(&world);
        fleet.dispatch_scan(Sector::containing(20, 20));
        fleet.take_control(0);
        let parked = fleet.fliers[0].position;

        for _ in 0..200 {
            fleet.tick(&world, &mut []);
        }
        assert_eq!(
            fleet.fliers[0].position, parked,
            "the fleet flew a bird the player was holding"
        );
    }

    #[test]
    fn a_sweep_finds_the_buried_body_and_reports_its_depth() {
        // The scanner's whole reason to exist: ore the eye cannot see.
        let mut world = flat(5, 60);
        let body = VoxelAabb::new(BlockPos::new(20, 48, 20), BlockPos::new(24, 52, 24));
        ore_body(&mut world, body);

        let mut fleet = fleet_over(&world);
        assert!(fleet.dispatch_scan(Sector { x: 0, z: 0 }));
        run_until_idle(&mut fleet, &world, &mut [], 5_000);

        assert!(fleet.is_surveyed(Sector { x: 0, z: 0 }));
        let pings = fleet.pings();
        assert_eq!(pings.len(), 1, "expected one ping, got {pings:?}");
        // Surface at 60, body top at 52: eight blocks of overburden.
        assert_eq!(pings[0].depth, 8);
        assert_eq!(pings[0].ore_columns, 25);
        assert!(body.expanded(1).contains(BlockPos::new(
            pings[0].position.x,
            body.min.y,
            pings[0].position.z
        )));
    }

    #[test]
    fn pings_exist_only_for_ground_already_overflown() {
        // Progressive scanning: interrupt a sweep halfway and you know half
        // the sector — no more.
        let mut world = flat(5, 60);
        // One body early in the sweep (low z), one late (high z).
        ore_body(&mut world, VoxelAabb::new(BlockPos::new(10, 55, 4), BlockPos::new(12, 60, 6)));
        ore_body(&mut world, VoxelAabb::new(BlockPos::new(10, 55, 58), BlockPos::new(12, 60, 60)));

        let mut fleet = fleet_over(&world);
        fleet.dispatch_scan(Sector { x: 0, z: 0 });

        // Tick until the first ping appears, then stop immediately.
        let mut ticks = 0;
        while fleet.pings().is_empty() {
            fleet.tick(&world, &mut []);
            ticks += 1;
            assert!(ticks < 5_000, "the sweep never found the first body");
        }

        let pings = fleet.pings();
        assert_eq!(pings.len(), 1, "found more than the overflown body: {pings:?}");
        assert!(pings[0].position.z < 32, "the late body pinged before being overflown");
        assert!(!fleet.is_surveyed(Sector { x: 0, z: 0 }));
    }

    #[test]
    fn a_deeper_scanner_finds_what_the_stock_one_misses() {
        // The Prospecting upgrade hook, proved end to end: same world, same
        // sweep, different instrument.
        let mut world = flat(5, 60);
        // Body top at 30: depth 30 below the surface at 60 — past the stock
        // 24, within an upgraded 36.
        let body = VoxelAabb::new(BlockPos::new(20, 25, 20), BlockPos::new(24, 30, 24));
        ore_body(&mut world, body);

        let mut stock = fleet_over(&world);
        stock.dispatch_scan(Sector { x: 0, z: 0 });
        run_until_idle(&mut stock, &world, &mut [], 5_000);
        assert!(stock.pings().is_empty(), "the stock scanner should miss depth 30");

        let mut upgraded = fleet_over(&world);
        upgraded.scan_depth = 36;
        upgraded.dispatch_scan(Sector { x: 0, z: 0 });
        run_until_idle(&mut upgraded, &world, &mut [], 5_000);
        let pings = upgraded.pings();
        assert_eq!(pings.len(), 1, "the upgraded scanner missed it too");
        assert_eq!(pings[0].depth, 30);
    }

    #[test]
    fn the_report_counts_exactly_what_lands_in_the_base() {
        // The Logistics XP source must match reality, or levels drift from
        // work done.
        let world = flat(6, 60);
        let mut fleet = fleet_over(&world);
        fleet.set_base(BlockPos::new(-25, 61, -25));

        let mut mine = Operation::new(BlockPos::new(30, 61, 30));
        mine.stockpile.add("engine:copper_ore", 150);
        let mut mines = [mine];

        let mut reported = 0u64;
        for _ in 0..10_000 {
            reported += fleet.tick(&world, &mut mines).delivered;
            if mines[0].stockpile.is_empty() && fleet.fliers[0].carrying() == 0 {
                break;
            }
        }
        let landed = fleet.base.as_ref().unwrap().stockpile.total();
        assert_eq!(reported, landed, "the report and the base pile disagree");
        assert_eq!(landed, 150);
    }

    #[test]
    fn a_body_deeper_than_scan_range_stays_invisible() {
        let mut world = flat(5, 60);
        let deep = VoxelAabb::new(BlockPos::new(20, 20, 20), BlockPos::new(24, 24, 24));
        ore_body(&mut world, deep);

        let mut fleet = fleet_over(&world);
        fleet.dispatch_scan(Sector { x: 0, z: 0 });
        run_until_idle(&mut fleet, &world, &mut [], 5_000);

        assert!(fleet.is_surveyed(Sector { x: 0, z: 0 }));
        assert!(fleet.pings().is_empty(), "pinged a body below SCAN_DEPTH");
    }

    #[test]
    fn a_mined_out_body_stops_pinging_on_rescan() {
        // The scanner reads the world as it is, not the deposit function —
        // this is the test that keeps that promise.
        let mut world = flat(5, 60);
        let body = VoxelAabb::new(BlockPos::new(20, 50, 20), BlockPos::new(22, 52, 22));
        ore_body(&mut world, body);

        let mut fleet = fleet_over(&world);
        fleet.dispatch_scan(Sector { x: 0, z: 0 });
        run_until_idle(&mut fleet, &world, &mut [], 5_000);
        assert_eq!(fleet.pings().len(), 1);

        // Mine it out by hand.
        let stone = world.registry().id_of("engine:stone").unwrap();
        for pos in body.blocks() {
            world.set_block(pos, stone);
        }

        fleet.dispatch_scan(Sector { x: 0, z: 0 });
        run_until_idle(&mut fleet, &world, &mut [], 5_000);
        assert!(fleet.pings().is_empty(), "a mined-out body still pings");
    }

    #[test]
    fn the_ferry_moves_every_block_and_loses_none() {
        // Conservation across the whole chain: mine pile + cargo + base pile
        // is constant while nothing digs, and the run ends with everything in
        // the base, by name.
        let world = flat(6, 60);
        let mut fleet = fleet_over(&world);
        fleet.set_base(BlockPos::new(-30, 61, -30));

        let mut mine = Operation::new(BlockPos::new(40, 61, 40));
        mine.stockpile.add("engine:copper_ore", 130);
        mine.stockpile.add("engine:stone", 70);
        let total = 200;

        let mut mines = [mine];
        for _ in 0..10_000 {
            fleet.tick(&world, &mut mines);
            let in_flight = mines[0].stockpile.total() + fleet.accounted_blocks();
            assert_eq!(in_flight, total, "blocks appeared or vanished mid-ferry");
            if mines[0].stockpile.is_empty() && fleet.fliers[0].carrying() == 0 {
                break;
            }
        }

        let base = fleet.base.as_ref().expect("base still set");
        assert_eq!(base.stockpile.count("engine:copper_ore"), 130);
        assert_eq!(base.stockpile.count("engine:stone"), 70);
        assert!(mines[0].stockpile.is_empty(), "ore left at the mine");
    }

    #[test]
    fn no_base_means_no_ferrying() {
        // Without somewhere to put it, hauling ore into the air would just be
        // carrying it around.
        let world = flat(4, 60);
        let mut fleet = fleet_over(&world);

        let mut mine = Operation::new(BlockPos::new(20, 61, 20));
        mine.stockpile.add("engine:copper_ore", 10);
        let mut mines = [mine];

        for _ in 0..50 {
            fleet.tick(&world, &mut mines);
        }
        assert_eq!(fleet.fliers[0].state, FlierState::Idle);
        assert_eq!(mines[0].stockpile.total(), 10);
    }

    #[test]
    fn a_busy_flier_cannot_be_dispatched_again() {
        let world = flat(4, 60);
        let mut fleet = fleet_over(&world);
        assert!(fleet.dispatch_scan(Sector { x: 0, z: 0 }));
        assert!(!fleet.dispatch_scan(Sector { x: 1, z: 0 }), "one flier took two jobs");
    }

    #[test]
    fn the_flier_never_enters_terrain_during_a_whole_errand() {
        // The flight-safety invariant over a real errand on rough ground:
        // scan, then ferry over a ridge between mine and base.
        let world = crate::fixture::shaped(6, |x| if (20..30).contains(&x) { 85 } else { 60 });
        let clear = world.surface_y(0, 0).unwrap();
        let mut fleet = Fleet::new();
        fleet.add_flier(BlockPos::new(0, clear + CLEARANCE, 0));
        fleet.set_base(BlockPos::new(-20, 61, 0));

        let mut mine = Operation::new(BlockPos::new(45, 61, 0));
        mine.stockpile.add("engine:stone", 40);
        let mut mines = [mine];

        for tick in 0..10_000 {
            fleet.tick(&world, &mut mines);
            assert!(
                !world.is_solid(fleet.fliers[0].position),
                "flier inside terrain at {:?} on tick {tick}",
                fleet.fliers[0].position
            );
            if mines[0].stockpile.is_empty() && fleet.fliers[0].carrying() == 0 {
                return;
            }
        }
        panic!("the ferry never completed");
    }
}
