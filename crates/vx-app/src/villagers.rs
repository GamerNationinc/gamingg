//! The village's inhabitants: deterministic wanderers with a word for you.
//!
//! No RNG state anywhere — waypoints and pauses come from hashing the
//! villager's index and how many legs of their stroll they have walked, so
//! two runs fed the same frame times are bit-identical, and nothing needs
//! saving. Each villager owns a rectangle of the village authored to contain
//! no buildings, walks its plateau at a stroll, and pauses like somebody with
//! somewhere to be eventually.
//!
//! Greetings use hysteresis: a line fires when the player comes within
//! [`GREET_RANGE`] and cannot fire again until they have stepped back out
//! past [`REARM_RANGE`] — once per approach, not once per frame.

use glam::{DVec3, Vec3};
use vx_render::Object;
use vx_world::town::{self, TownSite};
use vx_world::World;

use crate::awareness::{self, Perception, Sighting, Surroundings, TargetKind};
use crate::clock::TimeOfDay;
use crate::schedule::{self, Place};

use crate::rig::{self, Rig};

/// Metres from the player at which a villager speaks.
pub const GREET_RANGE: f32 = 3.0;

/// Metres beyond which the greeting re-arms.
pub const REARM_RANGE: f32 = 5.0;

/// Close enough that somebody grunts at you.
///
/// Well inside [`GREET_RANGE`], and deliberately not a *seeing* range: at
/// under two blocks you are in somebody's space whether they were watching
/// the street or not, and a person who did not see you coming is exactly the
/// person who grunts.
pub const GRUNT_RANGE: f32 = 1.7;

/// Stroll speed, metres per second.
const WALK_SPEED: f32 = 1.2;

/// Panic speed, metres per second. Nobody strolls away from a muzzle.
const RUN_SPEED: f32 = 3.4;

/// Metres within which having a launcher pointed your way registers.
pub const MENACE_RANGE: f32 = 18.0;

/// How tightly the muzzle must line up on someone to read as "at them":
/// cosine of the aim cone's half angle (about ten degrees).
const MENACE_COS: f32 = 0.985;

/// Metres within which a muzzle blast or an impact panics bystanders
/// outright, seen or heard.
const STARTLE_RANGE: f32 = 12.0;

/// Metres within which a blast at least snaps heads around.
const ALARM_RANGE: f32 = 25.0;

/// Seconds a panic lasts before somebody comes back out.
const PANIC_SECONDS: f32 = 20.0;

/// How close to the office door a runner must get to have reported you.
const REPORT_REACH: f32 = 2.5;

/// What a frightened villager does about it.
///
/// Chosen deterministically per villager and stroll leg — the same salted
/// hash streams the wander runs on — so the same person under the same gun
/// makes the same choice every time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panic {
    /// De-escalate: run home and hide.
    Fleeing,
    /// Escalate: run for the security office and report it.
    Alarming,
}

/// A wander rectangle on the plaza: x0, z0, x1, z1 (inclusive-ish bounds in
/// block coordinates, authored clear of every building footprint).
type Patch = (f32, f32, f32, f32);

/// Where somebody goes when the sun is down: the doorstep outside their
/// container, then the spot inside it. Two authored nodes rather than a path
/// search — the straight segments never cross a wall, which is both cheaper
/// and directly testable.
type HomeRoute = &'static [(f32, f32)];

/// Who lives here: a daytime patch, the way home, a line to say, a body to
/// wear. Every coordinate is an offset from the town centre.
const ROSTER: &[(Patch, HomeRoute, &str, usize)] = &[
    (
        (-8.0, -8.0, 8.0, 4.0),
        // The east-door container, west along the high street.
        &[(-10.0, -1.0), (-14.0, -0.5)],
        "MORNIN. FINE DAY FOR DIGGIN.",
        0,
    ),
    (
        (-9.0, 0.0, -5.0, 12.0),
        // The stacked bunks north of the plaza.
        &[(-1.0, 3.0), (-1.0, -21.0)],
        "SHOP IS JUST UP THE PATH.",
        1,
    ),
    (
        (4.0, -8.0, 9.0, 8.0),
        // The west-door container, east along the high street.
        &[(9.0, -1.0), (13.0, -0.5)],
        "MIND THE DRONES OUT THERE.",
        2,
    ),
];

/// Which way a yaw points, on the level. The inverse of
/// [`crate::rig::yaw_towards`], which is what set it.
fn heading(yaw: f32) -> Vec3 {
    Vec3::new(yaw.cos(), 0.0, -yaw.sin())
}

/// Everyone worth looking at from `from`: the player, the other townsfolk, and
/// the machines trundling past.
fn sightings_around(
    around: &Surroundings,
    from: DVec3,
    skip_villager: Option<usize>,
) -> Vec<Sighting> {
    let mut targets = Vec::new();
    if let Some(player) = around.player {
        let mut seen = Sighting::new(
            TargetKind::Player,
            0,
            player,
            (player - from).length() as f32,
        );
        // The player's stance decides how tall they are to look at.
        seen.eye = around.eye();
        targets.push(seen);
    }
    for (machine, (kind, at)) in around.machines.iter().enumerate() {
        targets.push(Sighting::new(*kind, machine, *at, (*at - from).length() as f32));
    }
    let _ = skip_villager;
    targets
}

/// Where a villager is in their day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Out on their patch.
    Roaming,
    /// Walking leg `n` of the route home.
    HeadingHome(usize),
    /// In for the night.
    Home,
    /// Walking the route back out, from leg `n` down.
    HeadingOut(usize),
}

/// The feet-level height a town's villagers walk at: on top of its plaza.
fn walk_height(site: &TownSite) -> f64 {
    f64::from(site.ground + 1)
}

/// Hash a villager's stroll leg into `0..1`. Same construction as the tile
/// jitter; index and leg get their own streams via the salt.
fn hash01(index: usize, leg: u32, salt: u64) -> f32 {
    let mut hash = salt
        ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (leg as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    hash = hash.wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 31;
    ((hash >> 40) as f32) / ((1u32 << 24) as f32)
}

struct Villager {
    position: DVec3,
    /// Where the last frame left them, for deriving facing.
    previous: DVec3,
    yaw: f32,
    /// The rectangle they are wandering *right now* — swapped by the
    /// schedule as the day moves them between work, market and leisure.
    patch: Patch,
    /// Their authored leisure rectangle, the one the roster gave them.
    home_patch: Patch,
    route: HomeRoute,
    phase: Phase,
    greeting: &'static str,
    variant: usize,
    /// Which leg of the stroll they are on; the hash streams key off it.
    leg: u32,
    waypoint: DVec3,
    /// Seconds left standing around before the next leg.
    pause: f32,
    /// Whether the current player approach has been greeted.
    greeted: bool,
    /// Whether this approach has already earned a grunt. Same hysteresis as
    /// `greeted`, one range in.
    grunted: bool,
    /// What their eyes are on, if anything. Set from the same sighting
    /// `react` turns their body toward — one perception, two uses.
    gaze: Option<DVec3>,
    /// What this villager can see and remembers.
    perception: Perception,
    /// Seconds left standing and watching rather than strolling.
    attention: f32,
    /// Whether they are running, and where to.
    panic: Option<Panic>,
    /// Seconds of panic left.
    panic_timer: f32,
}

impl Villager {
    fn new(
        index: usize,
        patch: Patch,
        route: HomeRoute,
        greeting: &'static str,
        variant: usize,
        site: &TownSite,
    ) -> Self {
        let start = waypoint_in(index, 0, patch, site);
        Villager {
            position: start,
            previous: start,
            yaw: 0.0,
            patch,
            home_patch: patch,
            route,
            phase: Phase::Roaming,
            greeting,
            variant,
            leg: 0,
            waypoint: waypoint_in(index, 1, patch, site),
            pause: 0.0,
            greeted: false,
            grunted: false,
            gaze: None,
            perception: Perception::default(),
            attention: 0.0,
            panic: None,
            panic_timer: 0.0,
        }
    }
    /// A world position from a route node, which is authored town-relative.
    fn route_node(&self, leg: usize, site: &TownSite) -> DVec3 {
        let (x, z) = self.route[leg.min(self.route.len() - 1)];
        DVec3::new(
            f64::from(site.centre.0) + f64::from(x),
            walk_height(site),
            f64::from(site.centre.1) + f64::from(z),
        )
    }

    /// Swap the wander rectangle when the schedule moves this person on —
    /// market stall, workplace, evening square. The wander machinery itself
    /// is untouched; a place is just a different patch for it to idle in.
    fn repatch(&mut self, index: usize, place: Place, site: &TownSite) {
        let patch = schedule::patch_for(index, place, self.home_patch);
        if patch == self.patch {
            return;
        }
        self.patch = patch;
        if self.phase == Phase::Roaming {
            // Set off for the new spot now rather than finishing the old
            // stroll leg first: work starts when work starts.
            self.leg += 1;
            self.waypoint = waypoint_in(index, self.leg + 1, patch, site);
            self.pause = 0.0;
        }
    }

    /// Point them at wherever the hour says they should be.
    ///
    /// Only ever *starts* a journey; the walking itself is the same code that
    /// handles a stroll, so there is one movement path rather than two.
    /// `out` is the schedule's verdict: false means the rule stack said
    /// *home*, true means the street in one of its flavours.
    fn retarget(&mut self, index: usize, out: bool, site: &TownSite) {
        match (out, self.phase) {
            // Dusk: set off for the doorstep.
            (false, Phase::Roaming) => {
                self.phase = Phase::HeadingHome(0);
                self.waypoint = self.route_node(0, site);
                self.pause = 0.0;
            }
            // Dawn: back out the way they came in.
            (true, Phase::Home) => {
                let last = self.route.len() - 1;
                self.phase = Phase::HeadingOut(last);
                self.waypoint = self.route_node(last, site);
                self.pause = 0.0;
            }
            // Caught mid-journey by the hour turning back: turn around.
            (true, Phase::HeadingHome(leg)) => {
                self.phase = Phase::HeadingOut(leg);
                self.waypoint = self.route_node(leg, site);
            }
            (false, Phase::HeadingOut(leg)) => {
                self.phase = Phase::HeadingHome(leg);
                self.waypoint = self.route_node(leg, site);
            }
            _ => {
                let _ = index;
            }
        }
    }

    /// Arrived at the current waypoint: pick the next one.
    fn arrive(&mut self, index: usize, site: &TownSite) {
        match self.phase {
            Phase::Roaming => {
                self.leg += 1;
                self.pause = 0.8 + hash01(index, self.leg, 0xa3) * 2.5;
                self.waypoint = waypoint_in(index, self.leg + 1, self.patch, site);
            }
            Phase::HeadingHome(leg) => {
                if leg + 1 < self.route.len() {
                    self.phase = Phase::HeadingHome(leg + 1);
                    self.waypoint = self.route_node(leg + 1, site);
                } else {
                    // In, and settled for the night.
                    self.phase = Phase::Home;
                }
            }
            Phase::HeadingOut(leg) => {
                if leg > 0 {
                    self.phase = Phase::HeadingOut(leg - 1);
                    self.waypoint = self.route_node(leg - 1, site);
                } else {
                    // Out on the street: back to the day's stroll.
                    self.phase = Phase::Roaming;
                    self.leg += 1;
                    self.waypoint = waypoint_in(index, self.leg + 1, self.patch, site);
                }
            }
            Phase::Home => {}
        }
    }

    /// One frame of running scared. Returns 1 on the frame this villager
    /// reaches the office and reports the player, else 0.
    fn run_for_it(&mut self, dt: f32, site: &TownSite, office: Option<DVec3>) -> u32 {
        self.panic_timer = (self.panic_timer - dt).max(0.0);
        if self.panic_timer <= 0.0 {
            // Composure returns; the ordinary state machine picks them back
            // up wherever the run left them.
            self.panic = None;
            return 0;
        }
        let home = self.route_node(self.route.len() - 1, site);
        let (target, reach) = match (self.panic, office) {
            // Escalate — unless the town has no office to escalate to.
            (Some(Panic::Alarming), Some(door)) => (door, REPORT_REACH),
            _ => (home, 0.0),
        };
        let to = target - self.position;
        let distance = to.length();
        let step = f64::from(RUN_SPEED * dt);
        if let Some(yaw) = rig::yaw_towards(to.x as f32, to.z as f32) {
            self.yaw = yaw;
        }
        if distance <= f64::from(reach).max(step) {
            if matches!(self.panic, Some(Panic::Alarming)) && office.is_some() {
                // Reported. Now get out of the way like everybody else.
                self.panic = Some(Panic::Fleeing);
                return 1;
            }
            // Indoors, and staying there until the panic burns off.
            self.position = target;
            self.phase = Phase::Home;
        } else {
            self.position += to / distance * step;
        }
        0
    }

    /// Start panicking, choosing fight-adjacent or flight by the villager's
    /// own deterministic coin.
    fn take_fright(&mut self, index: usize) {
        if self.panic.is_some() {
            self.panic_timer = PANIC_SECONDS;
            return;
        }
        let escalates = hash01(index, self.leg, 0xa4) < 0.5;
        self.panic = Some(if escalates {
            Panic::Alarming
        } else {
            Panic::Fleeing
        });
        self.panic_timer = PANIC_SECONDS;
        self.attention = 0.0;
    }
}

/// The `leg`-th waypoint inside a patch, in world coordinates.
///
/// Patches are authored as offsets from the town centre, so one roster serves
/// every town on the lattice.
fn waypoint_in(index: usize, leg: u32, patch: Patch, site: &TownSite) -> DVec3 {
    let (x0, z0, x1, z1) = patch;
    DVec3::new(
        f64::from(site.centre.0) + f64::from(x0 + hash01(index, leg, 0xa1) * (x1 - x0)),
        walk_height(site),
        f64::from(site.centre.1) + f64::from(z0 + hash01(index, leg, 0xa2) * (z1 - z0)),
    )
}

/// Every villager in town.
pub struct Villagers {
    /// The town these people live in. Patches are offsets from its centre.
    site: TownSite,
    folk: Vec<Villager>,
    /// Counts `update` calls, not wall time — which is what keeps the
    /// round-robin line-of-sight schedule deterministic.
    tick: u64,
    /// Where an alarmed runner heads: the security office, if the town has
    /// one.
    office: Option<DVec3>,
    /// Alarms that reached the office since the last collection.
    reports: u32,
    /// Which calendar day it is, set by the caller from the journal clock.
    /// An input like the hour, not saved state: the schedule derives from it.
    day: u32,
}

impl Default for Villagers {
    fn default() -> Self {
        Self::new()
    }
}

impl Villagers {
    /// The hometown's people.
    pub fn new() -> Self {
        Villagers::for_site(&town::home_site())
    }

    /// The people of one town on the lattice.
    pub fn for_site(site: &TownSite) -> Self {
        let office = town::plan::buildings(site)
            .into_iter()
            .find(|building| building.role == town::plan::Role::Security)
            .map(|building| {
                DVec3::new(
                    (building.min.x + building.max.x) as f64 * 0.5,
                    walk_height(site),
                    (building.min.z + building.max.z) as f64 * 0.5,
                )
            });
        Villagers {
            site: *site,
            tick: 0,
            office,
            reports: 0,
            day: 0,
            folk: ROSTER
                .iter()
                .enumerate()
                .map(|(index, (patch, route, line, variant))| {
                    Villager::new(index, *patch, route, line, *variant, site)
                })
                .collect(),
        }
    }

    /// Which town these people belong to.
    pub fn site(&self) -> &TownSite {
        &self.site
    }

    /// Tell the town what day it is. The caller derives it from the journal
    /// clock ([`schedule::TICKS_PER_DAY`]); the schedule needs it for market
    /// days and everything else weekly.
    pub fn set_day(&mut self, day: u32) {
        self.day = day;
    }

    /// The rigs the roster wears, in variant order for [`Villagers::objects`].
    pub fn rigs() -> Vec<Rig> {
        (0..3).map(Rig::villager).collect()
    }

    /// Advance every stroll by `dt` seconds, and let the town notice what is
    /// around it.
    ///
    /// The stroll runs first and is untouched by what anyone sees, so the
    /// deterministic wander stays deterministic; awareness only decides
    /// whether a villager *stops* and which way they face. Panic is the one
    /// deliberate exception — a muzzle changes where people go — and it is
    /// still deterministic per villager: the flee-or-alarm coin comes off the
    /// same salted hash streams the wander itself runs on, and nothing about
    /// it feeds the replay oracle.
    pub fn update(&mut self, dt: f32, time: TimeOfDay, around: &Surroundings) {
        let site = self.site;
        let day = self.day;

        let office = self.office;
        let mut fresh_reports = 0;
        for (index, villager) in self.folk.iter_mut().enumerate() {
            villager.previous = villager.position;

            // Fear overrides the clock. A running villager neither strolls
            // nor keeps town hours until the panic burns off — panic *is*
            // the schedule's rule one, implemented rather than looked up.
            if villager.panic.is_some() {
                fresh_reports += Villager::run_for_it(villager, dt, &site, office);
                villager.perception.forget(dt);
                continue;
            }

            // The schedule decides where they are heading; the walking below
            // is the same walking as ever.
            let place = schedule::where_is(&site, index, day, time, false);
            villager.repatch(index, place, &site);
            villager.retarget(index, place != Place::Home, &site);

            // Standing and watching counts as standing: a villager who has
            // stopped to look at you should not slide across the plaza. But
            // somebody on their way in for the night keeps walking — being
            // caught somebody's eye is not a reason to sleep in the street.
            let travelling = matches!(
                villager.phase,
                Phase::HeadingHome(_) | Phase::HeadingOut(_)
            );
            if villager.attention > 0.0 && !travelling {
                villager.attention = (villager.attention - dt).max(0.0);
            } else if villager.pause > 0.0 {
                villager.pause = (villager.pause - dt).max(0.0);
            } else if villager.phase != Phase::Home {
                let to = villager.waypoint - villager.position;
                let distance = to.length();
                let step = f64::from(WALK_SPEED * dt);
                if distance <= step {
                    villager.position = villager.waypoint;
                    villager.arrive(index, &site);
                } else {
                    villager.position += to / distance * step;
                }
                let moved = villager.position - villager.previous;
                if let Some(yaw) = rig::yaw_towards(moved.x as f32, moved.z as f32) {
                    villager.yaw = yaw;
                }
            }
            villager.perception.forget(dt);
        }

        self.reports += fresh_reports;
        self.observe(around);
        self.react();
        self.tick = self.tick.wrapping_add(1);
    }

    /// Re-cast line of sight for whichever villagers are due this update.
    fn observe(&mut self, around: &Surroundings) {
        if self.folk.is_empty() {
            return;
        }
        let positions: Vec<DVec3> = self.folk.iter().map(|villager| villager.position).collect();

        for index in awareness::due(self.tick, self.folk.len()) {
            let eye = positions[index] + DVec3::Y * f64::from(TargetKind::Villager.eye_height());

            // Everyone worth looking at: the player, the other townsfolk, and
            // the machines trundling past.
            let mut targets = sightings_around(around, positions[index], Some(index));
            for (other, at) in positions.iter().enumerate() {
                if other == index {
                    continue;
                }
                targets.push(Sighting::new(
                    TargetKind::Villager,
                    other,
                    *at,
                    (*at - positions[index]).length() as f32,
                ));
            }

            let registry = around.world.map(|world| world.registry());
            let facing = Self::facing_of(&self.folk[index]);
            self.folk[index].perception.observe(
                around.world,
                registry,
                eye,
                Some(facing),
                &targets,
                awareness::SIGHT_RANGE,
            );
        }
    }

    /// Which way somebody is looking, for the sight cone.
    fn facing_of(villager: &Villager) -> Vec3 {
        heading(villager.yaw)
    }

    /// How many townsfolk can see the player *right now*.
    ///
    /// A fresh cast over everybody, not the cached answer: sight is refreshed
    /// one villager per frame, so the cache can be several frames stale — fine
    /// for deciding whether to say good morning, useless for deciding whether
    /// somebody watched you jemmy a lock. Three raycasts, once, at the moment
    /// of the crime.
    pub fn witnesses(&self, around: &Surroundings) -> usize {
        let Some(player) = around.player else {
            return 0;
        };
        let registry = around.world.map(|world| world.registry());
        let eye_of_player = player + DVec3::Y * f64::from(around.eye());

        self.folk
            .iter()
            .filter(|villager| {
                let eye = villager.position + DVec3::Y * f64::from(TargetKind::Villager.eye_height());
                let distance = (player - villager.position).length() as f32;
                if distance > awareness::SIGHT_RANGE {
                    return false;
                }
                if !awareness::in_cone(eye, Some(Self::facing_of(villager)), player, distance) {
                    return false;
                }
                match (around.world, registry) {
                    (Some(world), Some(registry)) => vx_world::sight::sees(
                        world,
                        registry,
                        eye,
                        eye_of_player,
                        awareness::SIGHT_RANGE,
                    ),
                    _ => true,
                }
            })
            .count()
    }

    /// How many townsfolk can see a point in the world right now.
    ///
    /// [`Villagers::witnesses`] asks this about the player; a machine at a
    /// lock needs the same question asked about *it*, because that is the
    /// whole trade remote intrusion offers — your body is elsewhere, your
    /// property is not.
    pub fn watchers_of(&self, world: &World, at: DVec3) -> usize {
        let registry = world.registry();
        self.folk
            .iter()
            .filter(|villager| {
                let eye = villager.position + DVec3::Y * f64::from(TargetKind::Villager.eye_height());
                let distance = (at - villager.position).length() as f32;
                if distance > awareness::SIGHT_RANGE {
                    return false;
                }
                if !awareness::in_cone(eye, Some(Self::facing_of(villager)), at, distance) {
                    return false;
                }
                vx_world::sight::sees(
                    world,
                    registry,
                    eye,
                    at,
                    awareness::SIGHT_RANGE,
                )
            })
            .count()
    }

    /// A launcher is being pointed. Everyone under the muzzle who can see
    /// the player panics; returns how many just did — each one is a menacing
    /// charge for the caller to bill.
    ///
    /// Live-only, like [`Villagers::witnesses`]: fear is a reaction to the
    /// player, and the replay oracle only ever re-checks the ground.
    pub fn menaced(&mut self, muzzle: DVec3, aim: Vec3, around: &Surroundings) -> usize {
        let Some(player) = around.player else {
            return 0;
        };
        let registry = around.world.map(|world| world.registry());
        let eye_of_player = player + DVec3::Y * f64::from(around.eye());
        let mut frightened = 0;
        for (index, villager) in self.folk.iter_mut().enumerate() {
            if villager.panic.is_some() {
                continue;
            }
            let chest = villager.position + DVec3::Y;
            let to = chest - muzzle;
            let distance = to.length() as f32;
            if !(1.0e-3..=MENACE_RANGE).contains(&distance) {
                continue;
            }
            // The aim is a direction; the offset is narrowed to meet it.
            if aim.dot((to / f64::from(distance)).as_vec3()) < MENACE_COS {
                continue;
            }
            // You cannot be menaced by a gun you cannot see: same occlusion
            // rule as witnessing, cast fresh at the moment it matters.
            let eye = villager.position + DVec3::Y * f64::from(TargetKind::Villager.eye_height());
            let seen = match (around.world, registry) {
                (Some(world), Some(registry)) => vx_world::sight::sees(
                    world,
                    registry,
                    eye,
                    eye_of_player,
                    awareness::SIGHT_RANGE,
                ),
                _ => true,
            };
            if !seen {
                continue;
            }
            villager.take_fright(index);
            frightened += 1;
        }
        frightened
    }

    /// A blast or an impact happened at `at`. Close bystanders panic whether
    /// or not they saw it — hearing needs no line of sight — and everybody
    /// else in earshot at least turns to look.
    pub fn startled(&mut self, at: DVec3) {
        for (index, villager) in self.folk.iter_mut().enumerate() {
            let distance = (at - villager.position).length() as f32;
            if distance <= STARTLE_RANGE {
                villager.take_fright(index);
            } else if distance <= ALARM_RANGE {
                if let Some(yaw) = rig::yaw_towards(
                    (at.x - villager.position.x) as f32,
                    (at.z - villager.position.z) as f32,
                ) {
                    villager.yaw = yaw;
                }
                villager.attention = villager.attention.max(1.5);
            }
        }
    }

    /// Alarms that reached the security office since last asked. Each one is
    /// the office's own witness statement.
    pub fn take_reports(&mut self) -> u32 {
        std::mem::take(&mut self.reports)
    }

    /// How many townsfolk are running right now, for the HUD and the tests.
    pub fn panicking(&self) -> usize {
        self.folk
            .iter()
            .filter(|villager| villager.panic.is_some())
            .count()
    }

    /// Turn what each villager can see into what they do about it.
    fn react(&mut self) {
        for villager in &mut self.folk {
            let Some(watched) = villager.perception.watching() else {
                // Nothing to look at: the eyes go back to centre, which is
                // the difference between a person and a mannequin whose
                // pupils are stuck where they last saw somebody.
                villager.gaze = None;
                continue;
            };
            if watched.distance > awareness::NOTICE_RANGE {
                villager.gaze = None;
                continue;
            }
            // The eyes go to the same sighting the body turns toward, at
            // head height rather than at their feet.
            villager.gaze = Some(watched.position + DVec3::new(0.0, 1.4, 0.0));
            // Face whatever has their attention — including the remembered
            // spot, which is what stops the head snapping away the instant
            // somebody steps behind a tree.
            let to = watched.position - villager.position;
            if let Some(yaw) = rig::yaw_towards(to.x as f32, to.z as f32) {
                villager.yaw = yaw;
            }
            // And stop to look, if it is close, actually in view, and worth
            // gawking at — a neighbour is not. Townsfolk stopping for each
            // other deadlocks the moment two schedules cross paths: both
            // stand refreshing the other's attention forever, six feet
            // apart, and nobody makes it to work.
            if villager.perception.visible.is_some()
                && watched.distance < GREET_RANGE * 2.0
                && watched.kind != TargetKind::Villager
            {
                villager.attention = villager.attention.max(0.35);
            }
        }
    }

    /// The line a villager says when the player walks up — and only when they
    /// can actually *see* them. A wall between you and a villager should not
    /// produce a cheery hello.
    ///
    /// Hysteresis unchanged in shape: a line fires inside [`GREET_RANGE`] and
    /// re-arms only once the player is back past [`REARM_RANGE`].
    pub fn greeting_for(&mut self) -> Option<&'static str> {
        let mut spoken = None;
        for villager in &mut self.folk {
            // Nobody running for their life stops to say good morning.
            if villager.panic.is_some() {
                continue;
            }
            let seen = villager.perception.sees_player();
            let distance = seen.map(|player| player.distance);
            match distance {
                Some(distance) if distance < GREET_RANGE && !villager.greeted => {
                    // Everyone who can see you counts as met — walking into a
                    // group gets one line, not a queue of them.
                    villager.greeted = true;
                    if spoken.is_none() {
                        spoken = Some(villager.greeting);
                    }
                }
                // Out of view, or far enough away, re-arms the greeting.
                None => villager.greeted = false,
                Some(distance) if distance > REARM_RANGE => villager.greeted = false,
                Some(_) => {}
            }
        }
        spoken
    }

    /// Somebody the player has just crowded, if anybody.
    ///
    /// Returns the variant, which is all the caller needs: it picks the
    /// pitch of the grunt, so a town of nine is not one voice nine times.
    ///
    /// Deliberately not gated on line of sight, unlike [`Villagers::greeting_for`]:
    /// a greeting is somebody choosing to speak to you and a grunt is
    /// somebody discovering you are already too close.
    pub fn grunt_for(&mut self, player: DVec3) -> Option<usize> {
        let mut grunted = None;
        for villager in &mut self.folk {
            // Somebody running for their life is making a different noise.
            if villager.panic.is_some() {
                villager.grunted = true;
                continue;
            }
            let gap = (player - villager.position).length() as f32;
            if gap < GRUNT_RANGE && !villager.grunted {
                villager.grunted = true;
                if grunted.is_none() {
                    grunted = Some(villager.variant);
                }
            } else if gap > REARM_RANGE {
                villager.grunted = false;
            }
        }
        grunted
    }

    /// This frame's drawn bodies. `rigs` comes from [`Villagers::rigs`];
    /// `relative` measures a place from the render origin, so every body is
    /// built in the camera's frame and marked so.
    pub fn objects(&self, rigs: &[Rig], relative: impl Fn(DVec3) -> Vec3) -> Vec<Object> {
        self.folk
            .iter()
            .flat_map(|villager| {
                let rig = &rigs[villager.variant % rigs.len()];
                // Head height matches the rig's own: the gaze is measured
                // from the eyes, not from the boots, or everybody spends the
                // day looking at the ground in front of them.
                let eye = villager.position + DVec3::new(0.0, 1.4, 0.0);
                let gaze = villager
                    .gaze
                    .map_or(rig::Gaze::AHEAD, |at| rig::Gaze::towards(eye, villager.yaw, at));
                rig.objects_looking(relative(villager.position), villager.yaw, 0.0, gaze)
                    .into_iter()
                    .map(Object::already_relative)
            })
            .collect()
    }

    /// Everyone's feet, for the tests and for anything scanning the street
    /// from above.
    pub fn positions(&self) -> Vec<DVec3> {
        self.folk.iter().map(|villager| villager.position).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strolls_stay_inside_their_patches_and_on_the_plateau() {
        let mut town = Villagers::new();
        for _ in 0..4000 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &Surroundings::empty());
        }
        for (index, villager) in town.folk.iter().enumerate() {
            let (x0, z0, x1, z1) = villager.patch;
            let at = villager.position;
            assert!(
                at.x >= f64::from(x0) - 0.01
                    && at.x <= f64::from(x1) + 0.01
                    && at.z >= f64::from(z0) - 0.01
                    && at.z <= f64::from(z1) + 0.01,
                "villager {index} wandered off their patch {:?}: at {at:?} phase {:?} waypoint {:?} pause {} attention {}",
                villager.patch, villager.phase, villager.waypoint, villager.pause, villager.attention
            );
            assert_eq!(at.y, walk_height(&town::home_site()), "a villager left the ground");
            assert!(
                vx_world::town::plan::cell_at(&town::home_site(), at.x as i32, town::HOME_GROUND_Y + 1, at.z as i32)
                    .is_none(),
                "a patch overlaps a building at {at:?}"
            );
        }
    }

    #[test]
    fn two_runs_fed_the_same_frames_are_identical() {
        let mut a = Villagers::new();
        let mut b = Villagers::new();
        for step in 0..2000 {
            // Uneven frame times, same sequence for both.
            let dt = 1.0 / 60.0 + (step % 7) as f64 * 0.001;
            a.update(dt as f32, TimeOfDay::NOON, &Surroundings::empty());
            b.update(dt as f32, TimeOfDay::NOON, &Surroundings::empty());
        }
        assert_eq!(a.positions(), b.positions());
    }

    /// Drive the town for one update with the player standing at `player`,
    /// long enough for the round-robin to have looked at everybody.
    fn look_around(town: &mut Villagers, player: DVec3) {
        let around = Surroundings {
            player_eye: awareness::PLAYER_EYE,
            world: None,
            player: Some(player),
            machines: &[],
        };
        for _ in 0..town.folk.len() {
            town.update(0.0, TimeOfDay::NOON, &around);
        }
    }

    #[test]
    fn a_greeting_fires_once_per_approach() {
        let mut town = Villagers::new();
        // Park the others out of earshot so only villager 0 is in play.
        town.folk[1].position.x += 1000.0;
        town.folk[2].position.x -= 1000.0;
        let villager = town.folk[0].position;
        let near = DVec3::new(villager.x + 1.0, walk_height(&town::home_site()), villager.z);
        let far = DVec3::new(villager.x + f64::from(REARM_RANGE) + 2.0, walk_height(&town::home_site()), villager.z);

        look_around(&mut town, near);
        assert!(town.greeting_for().is_some(), "no greeting on approach");
        look_around(&mut town, near);
        assert!(town.greeting_for().is_none(), "greeted every frame");

        // Just outside greeting range but inside re-arm range must NOT re-arm.
        let lurking = DVec3::new(villager.x + f64::from(GREET_RANGE) + 0.5, walk_height(&town::home_site()), villager.z);
        look_around(&mut town, lurking);
        assert!(town.greeting_for().is_none());
        look_around(&mut town, near);
        assert!(town.greeting_for().is_none(), "re-armed too eagerly");

        // Properly away, then back: a fresh approach, a fresh hello.
        look_around(&mut town, far);
        assert!(town.greeting_for().is_none());
        look_around(&mut town, near);
        assert!(town.greeting_for().is_some(), "never re-armed");
    }

    #[test]
    fn a_villager_turns_to_face_a_player_they_can_see() {
        let mut town = Villagers::new();
        let at = town.folk[0].position;
        // Stand due +x of them.
        let player = DVec3::new(at.x + 2.0, walk_height(&town::home_site()), at.z);
        look_around(&mut town, player);

        let facing = town.folk[0].yaw;
        let nose = glam::Mat4::from_rotation_y(facing).transform_vector3(Vec3::X);
        assert!(
            nose.x > 0.9,
            "villager faced {nose:?} instead of toward the player"
        );
    }

    #[test]
    fn a_villager_behind_a_wall_neither_turns_nor_greets() {
        // Real terrain with a wall dropped between the two.
        let mut world = vx_world::World::new(2024);
        world.load_around(vx_core::ChunkPos::new(0, 0), 1);
        let stone = world.registry().id_of("engine:stone").unwrap();

        let mut town = Villagers::new();
        town.folk[1].position.x += 1000.0;
        town.folk[2].position.x -= 1000.0;
        let at = town.folk[0].position;
        town.folk[0].yaw = 0.0;
        let player = DVec3::new(at.x + 2.5, at.y, at.z);

        let ground = at.y as i32;
        for dy in 0..4 {
            for dz in -3..=3 {
                world.set_block(
                    vx_core::BlockPos::new(at.x as i32 + 1, ground + dy, at.z as i32 + dz),
                    stone,
                );
            }
        }

        let around = Surroundings {
            player_eye: awareness::PLAYER_EYE,
            world: Some(&world),
            player: Some(player),
            machines: &[],
        };
        for _ in 0..town.folk.len() {
            town.update(0.0, TimeOfDay::NOON, &around);
        }

        assert!(town.greeting_for().is_none(), "greeted through a wall");
        assert_eq!(town.folk[0].yaw, 0.0, "turned to face somebody they cannot see");
    }

    #[test]
    fn a_watched_villager_pauses_their_stroll_and_resumes_after() {
        let mut town = Villagers::new();
        town.folk[1].position.x += 1000.0;
        town.folk[2].position.x -= 1000.0;
        // Clear any starting pause so movement is the only variable.
        town.folk[0].pause = 0.0;
        let at = town.folk[0].position;
        let player = DVec3::new(at.x + 1.5, walk_height(&town::home_site()), at.z);

        let around = Surroundings {
            player_eye: awareness::PLAYER_EYE,
            world: None,
            player: Some(player),
            machines: &[],
        };
        for _ in 0..3 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &around);
        }
        let watching_from = town.folk[0].position;
        for _ in 0..10 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &around);
        }
        assert_eq!(
            town.folk[0].position, watching_from,
            "kept strolling while being watched"
        );

        // Player leaves: the stroll picks back up.
        let alone = Surroundings::empty();
        for _ in 0..120 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &alone);
        }
        assert_ne!(
            town.folk[0].position, watching_from,
            "never resumed the stroll"
        );
    }

    #[test]
    fn a_villager_notices_a_passing_drone() {
        let mut town = Villagers::new();
        let at = town.folk[0].position;
        let drone = DVec3::new(at.x + 3.0, at.y, at.z);
        let machines = [(TargetKind::Digger, drone)];
        let around = Surroundings {
            player_eye: awareness::PLAYER_EYE,
            world: None,
            player: None,
            machines: &machines,
        };
        for _ in 0..town.folk.len() {
            town.update(0.0, TimeOfDay::NOON, &around);
        }

        let watched = town.folk[0].perception.watching().expect("saw nothing at all");
        assert_eq!(watched.kind, TargetKind::Digger);
    }

    #[test]
    fn two_runs_over_the_same_world_and_route_stay_identical() {
        // The stronger determinism claim: not just an empty room, but real
        // terrain and a player walking about.
        let mut world = vx_world::World::new(2024);
        world.load_around(vx_core::ChunkPos::new(0, 0), 1);

        let run = || {
            let mut town = Villagers::new();
            for step in 0..600 {
                let dt = 1.0 / 60.0 + (step % 7) as f64 * 0.001;
                let player = DVec3::new(
                    (step as f64 * 0.02).sin() * 6.0,
                    walk_height(&town::home_site()),
                    (step as f64 * 0.017).cos() * 6.0,
                );
                let around = Surroundings {
                    player_eye: awareness::PLAYER_EYE,
                    world: Some(&world),
                    player: Some(player),
                    machines: &[],
                };
                town.update(dt as f32, TimeOfDay::NOON, &around);
            }
            town.positions()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn the_town_goes_indoors_at_night_and_back_out_at_dawn() {
        let mut town = Villagers::new();
        let alone = Surroundings::empty();

        // A night's worth of updates: everyone should be home and settled.
        for _ in 0..4000 {
            town.update(1.0 / 60.0, TimeOfDay::MIDNIGHT, &alone);
        }
        for (index, villager) in town.folk.iter().enumerate() {
            assert_eq!(villager.phase, Phase::Home, "villager {index} never got home");
            let inside = villager.route_node(villager.route.len() - 1, &town.site);
            assert!(
                (villager.position - inside).length() < 1.0,
                "villager {index} settled at {:?}, not indoors at {inside:?}",
                villager.position
            );
        }

        // And a morning's worth: back out on the plaza.
        for _ in 0..4000 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &alone);
        }
        for (index, villager) in town.folk.iter().enumerate() {
            assert_eq!(villager.phase, Phase::Roaming, "villager {index} stayed in bed");
            let (x0, z0, x1, z1) = villager.patch;
            let at = villager.position;
            let centre = town.site.centre;
            assert!(
                at.x >= f64::from(centre.0) + f64::from(x0) - 0.01
                    && at.x <= f64::from(centre.0) + f64::from(x1) + 0.01
                    && at.z >= f64::from(centre.1) + f64::from(z0) - 0.01
                    && at.z <= f64::from(centre.1) + f64::from(z1) + 0.01,
                "villager {index} came out somewhere odd: {at:?}"
            );
        }
    }

    #[test]
    fn nobody_walks_through_a_wall_on_the_way_home() {
        // The route is two authored nodes precisely so its straight segments
        // stay in the doorway. Sample them against the town's own blueprint.
        let site = town::home_site();
        let town_folk = Villagers::new();
        for (index, villager) in town_folk.folk.iter().enumerate() {
            let mut nodes = vec![villager.position];
            for leg in 0..villager.route.len() {
                nodes.push(villager.route_node(leg, &site));
            }
            for pair in nodes.windows(2) {
                for step in 0..=20 {
                    let t = step as f64 / 20.0;
                    let at = pair[0] + (pair[1] - pair[0]) * t;
                    let (x, z) = (at.x.round() as i32, at.z.round() as i32);
                    // Head height: a wall blocks, a doorway does not.
                    let cell = vx_world::town::plan::cell_at(&site, x, site.ground + 2, z);
                    assert!(
                        cell.is_none(),
                        "villager {index} walks through {cell:?} at ({x},{z})"
                    );
                }
            }
        }
    }

    #[test]
    fn the_clock_is_an_input_so_two_identical_days_are_identical() {
        // Determinism has to survive the schedule: the hour is a parameter,
        // never a wall-clock read.
        let run = || {
            let mut town = Villagers::new();
            let alone = Surroundings::empty();
            for step in 0..3000 {
                let time = TimeOfDay::new(step as f32 / 3000.0);
                town.update(1.0 / 60.0, time, &alone);
            }
            town.positions()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn the_schedule_moves_the_town_through_its_day() {
        // Noon on a plain day is work; noon on market day is the square. The
        // schedule module says so in the abstract — this pins that the
        // walking machinery actually takes people there.
        let site = town::home_site();
        let alone = Surroundings::empty();

        let mut town = Villagers::new();
        town.set_day(schedule::market_weekday(&site) + 1);
        for _ in 0..6000 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &alone);
        }
        for (index, villager) in town.folk.iter().enumerate() {
            assert_eq!(
                villager.patch,
                schedule::patch_for(index, Place::Workplace, villager.home_patch),
                "villager {index} is not on their workplace patch at noon"
            );
        }

        town.set_day(schedule::market_weekday(&site));
        for _ in 0..6000 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &alone);
        }
        for (index, villager) in town.folk.iter().enumerate() {
            assert_eq!(
                villager.patch,
                schedule::patch_for(index, Place::Plaza, villager.home_patch),
                "villager {index} skipped the market"
            );
        }
    }

    #[test]
    fn the_town_draws_one_body_per_villager() {
        let town = Villagers::new();
        let rigs = Villagers::rigs();
        let objects = town.objects(&rigs, |at| at.as_vec3());
        let parts: usize = town
            .folk
            .iter()
            .map(|villager| rigs[villager.variant % rigs.len()].parts.len())
            .sum();
        assert_eq!(objects.len(), parts);
    }
}

#[cfg(test)]
mod witness_tests {
    use super::*;
    use vx_core::{BlockPos, ChunkPos};
    use vx_world::World;

    /// A flat world with a floor, so a wall can be built to hide behind.
    fn walled_world() -> World {
        let mut world = World::new(9);
        world.load_around(ChunkPos::new(0, 0), 1);
        let stone = world.registry().id_of("engine:stone").unwrap();
        for x in -20..20 {
            for z in -20..20 {
                for y in 0..80 {
                    let fill = if y == 40 { stone } else { vx_core::BlockId::AIR };
                    world.set_block(BlockPos::new(x, y, z), fill);
                }
            }
        }
        world
    }

    /// One villager, planted, facing the player.
    fn watcher(at: DVec3, facing_towards: DVec3) -> Villagers {
        let mut town = Villagers::new();
        town.folk.truncate(1);
        town.folk[0].position = at;
        let to = facing_towards - at;
        town.folk[0].yaw = crate::rig::yaw_towards(to.x as f32, to.z as f32).unwrap_or(0.0);
        town
    }

    fn seen_by(town: &Villagers, world: &World, player: DVec3, eye: f32) -> usize {
        town.witnesses(&Surroundings {
            player_eye: eye,
            world: Some(world),
            player: Some(player),
            machines: &[],
        })
    }

    #[test]
    fn a_villager_looking_at_you_is_a_witness() {
        let world = walled_world();
        let player = DVec3::new(6.0, 41.0, 0.0);
        let town = watcher(DVec3::new(0.0, 41.0, 0.0), player);
        assert_eq!(seen_by(&town, &world, player, awareness::PLAYER_EYE), 1);
    }

    #[test]
    fn a_villager_facing_away_sees_nothing() {
        // There is a behind now. Before this round sight was a full circle and
        // hiding was impossible to reason about.
        let world = walled_world();
        let player = DVec3::new(6.0, 41.0, 0.0);
        let away = DVec3::new(-30.0, 41.0, 0.0);
        let town = watcher(DVec3::new(0.0, 41.0, 0.0), away);
        assert_eq!(seen_by(&town, &world, player, awareness::PLAYER_EYE), 0);
    }

    #[test]
    fn somebody_at_your_elbow_notices_regardless() {
        // A cone with no close-range exception lets you stand nose to nose
        // with a villager unnoticed, which reads as a bug, not as stealth.
        let world = walled_world();
        let player = DVec3::new(1.5, 41.0, 0.0);
        let away = DVec3::new(-30.0, 41.0, 0.0);
        let town = watcher(DVec3::new(0.0, 41.0, 0.0), away);
        assert_eq!(seen_by(&town, &world, player, awareness::PLAYER_EYE), 1);
    }

    #[test]
    fn a_wall_between_you_is_not_a_witness() {
        let mut world = walled_world();
        let stone = world.registry().id_of("engine:stone").unwrap();
        for y in 41..45 {
            for z in -6..6 {
                world.set_block(BlockPos::new(3, y, z), stone);
            }
        }
        let player = DVec3::new(6.0, 41.0, 0.0);
        let town = watcher(DVec3::new(0.0, 41.0, 0.0), player);
        assert_eq!(seen_by(&town, &world, player, awareness::PLAYER_EYE), 0);
    }

    #[test]
    fn going_prone_behind_cover_hides_you() {
        // The whole stealth system in one assertion. The wall is one block
        // tall: standing, your eyes clear it and the villager's ray finds you;
        // flat on your belly at 0.35 m it does not. No visibility stat, no
        // detection meter — the raycast that was already running simply misses.
        let mut world = walled_world();
        let stone = world.registry().id_of("engine:stone").unwrap();
        for z in -6..6 {
            world.set_block(BlockPos::new(3, 41, z), stone);
        }

        let player = DVec3::new(6.0, 41.0, 0.0);
        let town = watcher(DVec3::new(0.0, 41.0, 0.0), player);

        let standing = crate::movement::Stance::Grounded.eye_cm() as f64 / 100.0;
        let prone = crate::movement::Stance::Prone.eye_cm() as f64 / 100.0;

        assert_eq!(seen_by(&town, &world, player, standing as f32), 1, "standing was hidden");
        assert_eq!(seen_by(&town, &world, player, prone as f32), 0, "prone was still seen");
    }

    #[test]
    fn crouching_helps_less_than_going_flat() {
        // Crouch is the compromise: quicker than prone, and it clears a
        // one-block wall, so it buys you nothing against this particular
        // cover. That ordering is what makes prone worth its speed penalty.
        let standing = crate::movement::Stance::Grounded.eye_cm();
        let crouched = crate::movement::Stance::Crouched.eye_cm();
        let prone = crate::movement::Stance::Prone.eye_cm();
        assert!(crouched < standing);
        assert!(prone < crouched);
    }

    #[test]
    fn distance_still_hides_you() {
        let world = walled_world();
        let player = DVec3::new(f64::from(awareness::SIGHT_RANGE) + 4.0, 41.0, 0.0);
        let town = watcher(DVec3::new(0.0, 41.0, 0.0), player);
        assert_eq!(seen_by(&town, &world, player, awareness::PLAYER_EYE), 0);
    }

    #[test]
    fn nobody_about_means_nobody_saw() {
        let world = walled_world();
        let mut town = Villagers::new();
        town.folk.clear();
        assert_eq!(seen_by(&town, &world, DVec3::new(0.0, 41.0, 0.0), 1.62), 0);
    }

    #[test]
    fn a_machine_at_a_lock_is_watched_the_same_as_a_body() {
        // The whole trade remote intrusion offers, in one assertion: your
        // body is elsewhere, your property is not, and the town looks at
        // your property with exactly the same eyes.
        let mut world = walled_world();
        let mut town = Villagers::new();
        town.folk[1].position.x += 1000.0;
        town.folk[2].position.x -= 1000.0;
        town.folk[0].position = DVec3::new(0.0, 41.0, 0.0);

        let in_the_open = DVec3::new(6.0, 41.5, 0.0);
        town.folk[0].yaw = rig::yaw_towards(in_the_open.x as f32, in_the_open.z as f32).unwrap_or(0.0);
        assert_eq!(
            town.watchers_of(&world, in_the_open),
            1,
            "nobody noticed a drone hovering at arm's length"
        );

        // Put a wall between them and the machine is nobody's business.
        let stone = world.registry().id_of("engine:stone").unwrap();
        for y in 41..45 {
            for z in -6..6 {
                world.set_block(BlockPos::new(3, y, z), stone);
            }
        }
        assert_eq!(
            town.watchers_of(&world, in_the_open),
            0,
            "saw a machine through a stone wall"
        );
    }

    #[test]
    fn a_muzzle_in_the_face_causes_panic_and_it_burns_off() {
        let mut town = Villagers::new();
        // Aim square at villager 0's chest from a few metres out, with the
        // others parked far away.
        town.folk[1].position.x += 1000.0;
        town.folk[2].position.x -= 1000.0;
        let victim = town.folk[0].position;
        let muzzle = victim + DVec3::new(-5.0, 1.62, 0.0);
        let aim = (victim + DVec3::Y - muzzle).normalize();
        let around = Surroundings {
            world: None,
            player: Some(muzzle - DVec3::Y * f64::from(awareness::PLAYER_EYE)),
            player_eye: awareness::PLAYER_EYE,
            machines: &[],
        };
        assert_eq!(town.menaced(muzzle, aim.as_vec3(), &around), 1, "nobody panicked");
        assert_eq!(town.menaced(muzzle, aim.as_vec3(), &around), 0, "panicked twice");
        assert_eq!(town.panicking(), 1);

        // Fear has a half-life: after the timer, composure returns.
        for _ in 0..((PANIC_SECONDS / 0.25) as usize + 4) {
            town.update(0.25, TimeOfDay::NOON, &Surroundings::empty());
        }
        assert_eq!(town.panicking(), 0, "panic never burned off");
    }

    #[test]
    fn aiming_wide_or_from_behind_a_wall_scares_nobody() {
        let world = walled_world();
        let mut town = Villagers::new();
        town.folk[1].position.x += 1000.0;
        town.folk[2].position.x -= 1000.0;
        let victim = town.folk[0].position;
        let muzzle = victim + DVec3::new(-5.0, 1.62, 0.0);
        let wide = DVec3::new(0.0, 0.0, 1.0);
        let around = Surroundings {
            world: None,
            player: Some(muzzle - DVec3::Y * f64::from(awareness::PLAYER_EYE)),
            player_eye: awareness::PLAYER_EYE,
            machines: &[],
        };
        assert_eq!(town.menaced(muzzle, wide.as_vec3(), &around), 0, "a wide aim read as a threat");

        // Real terrain with a wall dropped between the two: villager 0
        // relocated behind it, shooter aiming dead at them — but the muzzle
        // cannot be seen through stone.
        let mut world = world;
        let stone = world.registry().id_of("engine:stone").unwrap();
        for y in 41..45 {
            for z in -6..6 {
                world.set_block(BlockPos::new(3, y, z), stone);
            }
        }
        town.folk[0].position = DVec3::new(0.0, 41.0, 0.0);
        let shooter = DVec3::new(6.0, 41.0, 0.0);
        let hidden = Surroundings {
            world: Some(&world),
            player: Some(shooter),
            player_eye: 0.35,
            machines: &[],
        };
        let muzzle = shooter + DVec3::Y * 0.35;
        let target = town.folk[0].position + DVec3::Y - muzzle;
        assert_eq!(
            town.menaced(muzzle, target.normalize().as_vec3(), &hidden),
            0,
            "a muzzle nobody can see still frightened somebody"
        );
    }

    #[test]
    fn an_alarmed_villager_reports_at_the_office_then_goes_home() {
        let mut town = Villagers::new();
        assert!(town.office.is_some(), "the home town lost its security office");
        town.folk[0].panic = Some(Panic::Alarming);
        town.folk[0].panic_timer = PANIC_SECONDS;

        let mut reported = 0;
        for _ in 0..2000 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &Surroundings::empty());
            reported += town.take_reports();
            if reported > 0 {
                break;
            }
        }
        assert_eq!(reported, 1, "the runner never reached the office");
        // Having reported, they de-escalate into an ordinary flight home.
        assert_eq!(town.folk[0].panic, Some(Panic::Fleeing));
    }

    #[test]
    fn a_blast_nearby_panics_the_bystander() {
        let mut town = Villagers::new();
        town.folk[1].position.x += 1000.0;
        town.folk[2].position.x -= 1000.0;
        let at = town.folk[0].position + DVec3::new(3.0, 0.0, 0.0);
        town.startled(at);
        assert_eq!(town.panicking(), 1, "a blast three metres away went unremarked");
    }

    #[test]
    fn crowding_somebody_earns_one_grunt_and_then_re_arms() {
        // The greeting's hysteresis, one range in: once per approach, not
        // once per frame, and a step back is what buys the next one.
        let mut town = Villagers::new();
        let standing = town.folk[0].position;

        assert_eq!(town.grunt_for(standing), Some(town.folk[0].variant));
        assert_eq!(town.grunt_for(standing), None, "it grunted twice on one approach");

        // Backing off past the re-arm range and returning earns another.
        // Far enough out to be clear of *everybody*: the town stands about
        // eight blocks apart, so a step of one re-arm range lands on the
        // next person's toes instead.
        let away = standing + DVec3::new(0.0, 0.0, 40.0);
        assert_eq!(town.grunt_for(away), None);
        assert!(town.grunt_for(standing).is_some(), "the grunt never re-armed");
    }

    #[test]
    fn a_grunt_needs_no_line_of_sight_but_a_greeting_does() {
        // The distinction the two ranges encode: a greeting is somebody
        // choosing to speak to you, and a grunt is somebody discovering you
        // are already standing on them. Nothing here has seen the player at
        // all — `update` was never run with them in it.
        let mut town = Villagers::new();
        let standing = town.folk[0].position;
        assert!(town.greeting_for().is_none(), "an unseen player was greeted");
        assert!(town.grunt_for(standing).is_some(), "being trodden on went unremarked");
    }

    #[test]
    fn a_villager_who_is_watching_you_looks_at_you() {
        // The eyes come off the same sighting the body turns toward, so if
        // somebody has noticed the player they have a gaze — and when the
        // player leaves, the eyes go back to centre rather than staying
        // stuck where somebody used to be.
        let mut town = Villagers::new();
        let watcher = town.folk[0].position;
        let player = watcher + DVec3::new(2.0, 0.0, 1.0);
        let seen = Surroundings {
            player: Some(player),
            ..Surroundings::empty()
        };
        for _ in 0..120 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &seen);
        }
        assert!(
            town.folk.iter().any(|villager| villager.gaze.is_some()),
            "nobody in town looked at a player standing in front of them"
        );

        // And when the player leaves, nobody is left staring at the hole
        // they left. Gazes do not have to be *empty* — the townsfolk watch
        // each other, which is half of why a street looks alive — they have
        // to stop being about somebody who is not there.
        let alone = Surroundings::empty();
        for _ in 0..900 {
            town.update(1.0 / 60.0, TimeOfDay::NOON, &alone);
        }
        assert!(
            town.folk.iter().all(|villager| villager
                .gaze
                .is_none_or(|at| (at - player).length() > 2.0)),
            "somebody is still staring at where the player used to be"
        );
    }
    /// **The townsfolk stroll the same stroll three thousand kilometres
    /// out.** The home town's plan moved to a far centre, the same four
    /// thousand frames: every villager's position relative to the town
    /// centre agrees to a micrometre.
    #[test]
    fn the_townsfolk_stroll_the_same_stroll_three_thousand_kilometres_out() {
        let stroll = |centre: (i32, i32)| {
            let mut site = town::home_site();
            site.centre = centre;
            let mut town = Villagers::for_site(&site);
            for _ in 0..4000 {
                town.update(1.0 / 60.0, TimeOfDay::NOON, &Surroundings::empty());
            }
            let origin = DVec3::new(f64::from(centre.0), 0.0, f64::from(centre.1));
            town.positions().into_iter().map(|at| at - origin).collect::<Vec<_>>()
        };
        let here = stroll((0, 0));
        let there = stroll((3_000_000, 3_000_000));
        assert_eq!(here.len(), there.len());
        assert!(!here.is_empty());
        for (index, (a, b)) in here.iter().zip(&there).enumerate() {
            assert!(
                (*a - *b).length() < 1.0e-6,
                "villager {index} stood at {a} at home and {b} far out"
            );
        }
    }

}
