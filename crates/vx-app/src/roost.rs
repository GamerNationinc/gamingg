//! The roost: the town's own kestrel, in a box on the security office roof.
//!
//! # One machine, two owners
//!
//! The sheriff's watcher is the same airframe the player can buy, with the
//! same movement, the same senses and the same battery problem. The player
//! learns exactly what the law can see by owning the thing that sees it, and
//! every counter to being watched is honest mechanics: break sky line of
//! sight until the mark decays, bait it aloft and outlast its endurance —
//! while it recharges in the box the town is blind, which is the heist
//! window — or destroy it, which is property, and priced accordingly.
//!
//! # It hears; it does not aim
//!
//! The box wakes on loud *report* events — a lock breached, a shot fired in
//! town — flies to where the noise came from, and watches. A first sighting
//! gets you *observed*: the drone overhead, visibly, is the warning.
//! Offences committed while observed are witnessed, feeding the bounty
//! ledger stage 11 built. The roost never attacks; the escalation it
//! carries is the ladder that already exists, with an eye that does not
//! blink first.
//!
//! Like the kestrel's marks, all of this is live-side: the roost never
//! touches ground, so the replay oracle never needs to hear about it.

use glam::DVec3;
use vx_agent::Flier;
use vx_core::BlockPos;
use vx_world::World;

/// Blocks within which the box hears a gunshot.
pub const HEARING_GUNSHOT: f32 = 80.0;

/// Blocks within which it hears a lock being breached — quieter than a shot.
pub const HEARING_BREACH: f32 = 60.0;

/// Ticks between the wake and the launch: the pop-out is readable.
pub const LAUNCH_TICKS: u32 = 24;

/// Ticks it can stay aloft. Longer than the pack kestrel's cell — it is
/// plugged into a building.
pub const ENDURANCE: u32 = 960;

/// Ticks to recharge after docking. The heist window, stated in one number.
pub const RECHARGE: u32 = 480;

/// Blocks within which an aloft roost can see (and witness) a contact,
/// given line of sight.
pub const WATCH_RADIUS: f32 = 24.0;

/// Ticks it keeps watching a quiet scene before heading home.
const PATIENCE: u32 = 320;

/// Cruise height over the scene: high enough to see over walls, low enough
/// to be seen doing it.
const OVERWATCH: i32 = 9;

/// What woke it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    Gunshot,
    Breach,
}

impl Report {
    fn hearing(self) -> f32 {
        match self {
            Report::Gunshot => HEARING_GUNSHOT,
            Report::Breach => HEARING_BREACH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// In the box. `recharge` ticks until it can launch again.
    Boxed { recharge: u32 },
    /// Lid open, spinning up.
    Launching { to: BlockPos, count: u32 },
    /// En route to the report.
    Flying { to: BlockPos },
    /// Over the scene. `quiet` counts ticks since anything held its interest.
    Watching { over: BlockPos, quiet: u32 },
    /// Battery spent or scene gone cold: heading home.
    Returning,
}

/// The town's watcher.
#[derive(Debug)]
pub struct Roost {
    /// The box on the office roof.
    pub home: BlockPos,
    craft: Flier,
    state: State,
    /// Ticks spent aloft since the last launch.
    aloft: u32,
    /// The journal tick it was last stepped at, so the deadline questions
    /// can be asked without threading the clock through every caller.
    last_tick: u64,
    /// The player has been seen by this flight. Cleared when it boxes:
    /// being observed is a state of the scene, not a permanent record —
    /// the permanent record is the bounty ledger.
    pub observed: bool,
    /// Journal ticks until each hack wears off. Zero means never hacked,
    /// and a tick in the past means routine maintenance has since found it.
    blinded_until: u64,
    silenced_until: u64,
    tapped_until: u64,
}

impl Roost {
    /// A roost on its box. `home` is the box's top face — derived by the
    /// caller from the security office's claim, so worldgen stays the one
    /// authority on where the office stands.
    pub fn new(home: BlockPos) -> Self {
        Roost {
            home,
            craft: Flier::new(home),
            state: State::Boxed { recharge: 0 },
            aloft: 0,
            last_tick: 0,
            observed: false,
            blinded_until: 0,
            silenced_until: 0,
            tapped_until: 0,
        }
    }

    /// The three hack deadlines, for [`crate::intrusion`] to write down.
    ///
    /// Blinding, silencing or tapping a watch box costs a spoofer charge, and
    /// the charge is saved — `intrusion.dat` has carried the kit since stage
    /// 15. The *effect* was not, so until stage 54 a save between placing a
    /// hack and using it charged you for nothing: you paid the coil, quit,
    /// came back, and the box was watching again. Paying across a save and
    /// not receiving across the same save is the asymmetry, and it is the
    /// reason these three numbers are worth eight bytes each.
    pub fn hacks(&self) -> (u64, u64, u64) {
        (self.blinded_until, self.silenced_until, self.tapped_until)
    }

    /// Put them back.
    pub fn restore_hacks(&mut self, (blinded, silenced, tapped): (u64, u64, u64)) {
        self.blinded_until = blinded;
        self.silenced_until = silenced;
        self.tapped_until = tapped;
    }

    /// Whether the eye is in the sky right now.
    pub fn aloft(&self) -> bool {
        !matches!(self.state, State::Boxed { .. })
    }

    /// Whether the box could answer a report right now. Read by the tests
    /// today and by stage 15's intrusion panel next — said here so
    /// dead-code analysis is being overridden knowingly.
    #[allow(dead_code)]
    pub fn ready(&self) -> bool {
        matches!(self.state, State::Boxed { recharge: 0 })
    }

    /// Where the machine is, for drawing and sighting.
    pub fn position(&self) -> BlockPos {
        self.craft.position
    }

    /// Something loud happened at `at`. The box launches if it can hear it
    /// and has the charge; an already-flying roost re-tasks to the newer
    /// noise — fresh trouble outranks a cold scene.
    pub fn report(&mut self, at: BlockPos, kind: Report) {
        let level = DVec3::new(
            (at.x - self.home.x) as f64,
            0.0,
            (at.z - self.home.z) as f64,
        );
        if level.length() > f64::from(kind.hearing()) {
            return;
        }
        if self.blind_at(self.last_tick) {
            // A box that has been talked into standing down does not
            // answer the door, however loud the knock.
            return;
        }
        match self.state {
            State::Boxed { recharge: 0 } => {
                self.state = State::Launching {
                    to: at,
                    count: LAUNCH_TICKS,
                };
            }
            State::Boxed { .. } => {}
            State::Launching { count, .. } => {
                self.state = State::Launching { to: at, count };
            }
            State::Flying { .. } | State::Watching { .. } | State::Returning => {
                self.state = State::Flying { to: at };
            }
        }
    }

    /// One journal tick. `now` is the journal clock, which is what the
    /// hacked-until deadlines are measured against.
    pub fn tick(&mut self, world: &World, now: u64) {
        self.last_tick = now;
        if self.blind_at(now) && self.aloft() {
            // Blinded mid-flight: it goes home and sits there.
            self.state = State::Returning;
        }
        if self.aloft() {
            self.aloft += 1;
            if self.aloft >= ENDURANCE {
                self.state = State::Returning;
            }
        }
        match self.state {
            State::Boxed { recharge } => {
                self.state = State::Boxed {
                    recharge: recharge.saturating_sub(1),
                };
            }
            State::Launching { to, count } => {
                if count == 0 {
                    self.state = State::Flying { to };
                } else {
                    self.state = State::Launching {
                        to,
                        count: count - 1,
                    };
                }
            }
            State::Flying { to } => {
                if self.craft.fly_towards(world, (to.x, to.z)) {
                    // Climb to overwatch rather than cruise clearance: the
                    // whole point is seeing over the wall it is parked above.
                    self.state = State::Watching { over: to, quiet: 0 };
                }
            }
            State::Watching { over, quiet } => {
                let seat = overwatch_height(world, over);
                let here = self.craft.position;
                if here.y < seat {
                    self.craft
                        .move_to(here.offset([0, (seat - here.y).min(3), 0]));
                }
                if quiet >= PATIENCE {
                    self.state = State::Returning;
                } else {
                    self.state = State::Watching {
                        over,
                        quiet: quiet + 1,
                    };
                }
            }
            State::Returning => {
                let home = (self.home.x, self.home.z);
                if self.craft.fly_towards(world, home) {
                    // Down onto the box, then lights out.
                    self.craft.move_to(self.home);
                    self.state = State::Boxed { recharge: RECHARGE };
                    self.aloft = 0;
                    self.observed = false;
                }
            }
        }
    }

    /// Can the watcher see this eye right now? The same occlusion raycast
    /// as every other witness — a roof is cover from above by geometry, not
    /// by rule.
    ///
    /// This is the *geometric* question. Whether what it sees counts against
    /// you is [`Roost::witnesses`], which a silenced box answers no to while
    /// still flying its patrols in plain view — that gap is exactly what
    /// silencing buys, and why nobody notices it until an offence goes
    /// strangely unpunished.
    pub fn sees(&self, world: &World, target_eye: DVec3) -> bool {
        if !self.aloft() {
            return false;
        }
        let at = self.craft.position;
        let eye = DVec3::new(at.x as f64 + 0.5, at.y as f64 + 0.3, at.z as f64 + 0.5);
        if (target_eye - eye).length() > f64::from(WATCH_RADIUS) {
            return false;
        }
        vx_world::sight::sees(
            world,
            world.registry(),
            eye,
            target_eye,
            WATCH_RADIUS + 2.0,
        )
    }

    /// Does what it sees go on your sheet? Not while it is silenced.
    pub fn witnesses(&self, world: &World, target_eye: DVec3) -> bool {
        !self.silenced_at(self.last_tick) && self.sees(world, target_eye)
    }

    /// Apply an intrusion's outcome, holding until `until` ticks.
    pub fn hack(&mut self, grade: crate::intrusion::Grade, until: u64) {
        match grade {
            crate::intrusion::Grade::Blind => self.blinded_until = until,
            crate::intrusion::Grade::Silence => self.silenced_until = until,
            crate::intrusion::Grade::Tap => self.tapped_until = until,
        }
    }

    /// Knock it out until `until` — what drilling the box out buys, and the
    /// loud way to arrange the same silence a hack arranges quietly.
    pub fn knock_out(&mut self, until: u64) {
        self.blinded_until = self.blinded_until.max(until);
    }

    /// Is it standing down right now?
    pub fn blinded(&self) -> bool {
        self.blind_at(self.last_tick)
    }

    /// Is its feed mirrored to the player?
    pub fn tapped(&self) -> bool {
        self.last_tick < self.tapped_until
    }

    fn blind_at(&self, now: u64) -> bool {
        now < self.blinded_until
    }

    fn silenced_at(&self, now: u64) -> bool {
        now < self.silenced_until
    }

    /// Something it was watching moved: keep the scene warm.
    pub fn hold_interest(&mut self) {
        if let State::Watching { over, .. } = self.state {
            self.state = State::Watching { over, quiet: 0 };
        }
    }

    /// One line of status for panels.
    pub fn status(&self) -> &'static str {
        if self.blinded() {
            return "DARK";
        }
        if self.tapped() {
            return "TAPPED";
        }
        if self.silenced_at(self.last_tick) {
            return "SILENCED";
        }
        match self.state {
            State::Boxed { recharge: 0 } => "BOXED",
            State::Boxed { .. } => "RECHARGING",
            State::Launching { .. } => "LAUNCHING",
            State::Flying { .. } => "RESPONDING",
            State::Watching { .. } => "WATCHING",
            State::Returning => "RETURNING",
        }
    }
}

/// The hover height over a scene: overwatch above the local surface.
fn overwatch_height(world: &World, over: BlockPos) -> i32 {
    world
        .surface_y(over.x, over.z)
        .map(|surface| surface + OVERWATCH)
        .unwrap_or(over.y + OVERWATCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;

    fn town_world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 3);
        world
    }

    /// The hometown's box, derived the same way main derives it.
    fn box_home() -> BlockPos {
        let site = vx_world::town::home_site();
        let office = vx_world::town::plan::buildings(&site)
            .into_iter()
            .find(|building| building.role == vx_world::town::plan::Role::Security)
            .expect("the hometown lost its security office");
        BlockPos::new(office.max.x - 2, office.max.y, office.max.z - 2)
    }

    #[test]
    fn a_breach_in_earshot_launches_within_the_stated_ticks() {
        let world = town_world();
        let mut roost = Roost::new(box_home());
        let scene = BlockPos::new(roost.home.x - 10, roost.home.y - 3, roost.home.z - 8);

        roost.report(scene, Report::Breach);
        let mut clock = 0u64;
        for _ in 0..LAUNCH_TICKS {
            assert!(
                !matches!(roost.state, State::Flying { .. }),
                "launched before the pop-out finished"
            );
            clock += 1;
            roost.tick(&world, clock);
        }
        clock += 1;
        roost.tick(&world, clock);
        assert!(
            matches!(roost.state, State::Flying { .. } | State::Watching { .. }),
            "the box slept through a breach: {:?}",
            roost.state
        );
    }

    #[test]
    fn noise_beyond_hearing_is_nobody_at_home() {
        let mut roost = Roost::new(box_home());
        let far = BlockPos::new(roost.home.x + 500, roost.home.y, roost.home.z);
        roost.report(far, Report::Gunshot);
        assert!(!roost.aloft(), "heard a shot half a kilometre off");
    }

    #[test]
    fn the_heist_window_exists() {
        // Bait it aloft, outlast its battery: the town is blind for exactly
        // the recharge, and a report during the window goes unanswered.
        let world = town_world();
        let mut roost = Roost::new(box_home());
        let scene = BlockPos::new(roost.home.x - 8, roost.home.y - 3, roost.home.z - 6);
        roost.report(scene, Report::Gunshot);

        let mut clock = 0u64;
        let mut boxed_after = None;
        for tick in 0..(ENDURANCE + 400) {
            clock += 1;
            roost.tick(&world, clock);
            // A watched scene stays interesting, so patience never sends it
            // home — only the battery does.
            roost.hold_interest();
            if !roost.aloft() {
                boxed_after = Some(tick);
                break;
            }
        }
        assert!(boxed_after.is_some(), "the watcher never ran out of battery");

        // The window: reports fall on a recharging box.
        roost.report(scene, Report::Gunshot);
        assert!(!roost.aloft(), "no heist window: it relaunched flat");
        for _ in 0..RECHARGE {
            assert!(!roost.aloft(), "the window closed early");
            clock += 1;
            roost.tick(&world, clock);
        }
        roost.report(scene, Report::Gunshot);
        clock += 1;
        roost.tick(&world, clock);
        assert!(
            matches!(roost.state, State::Launching { .. }),
            "recharged and reported, but stayed boxed: {:?}",
            roost.state
        );
    }

    #[test]
    fn watching_sees_the_exposed_and_not_the_sheltered() {
        let world = town_world();
        let mut roost = Roost::new(box_home());
        let scene = BlockPos::new(roost.home.x - 6, roost.home.y - 3, roost.home.z - 4);
        roost.report(scene, Report::Gunshot);
        let mut clock = 0u64;
        for _ in 0..400 {
            clock += 1;
            roost.tick(&world, clock);
            if matches!(roost.state, State::Watching { .. }) {
                break;
            }
        }
        assert!(
            matches!(roost.state, State::Watching { .. }),
            "never reached the scene"
        );

        let at = roost.position();
        let exposed = DVec3::new(at.x as f64 + 3.5, at.y as f64 - 4.0, at.z as f64 + 0.5);
        assert!(roost.sees(&world, exposed), "missed a contact in the open");

        // Inside the security office, under its roof, is under cover.
        let indoors = DVec3::new(
            (roost.home.x - 3) as f64,
            (roost.home.y - 3) as f64,
            (roost.home.z - 2) as f64,
        );
        assert!(
            !roost.sees(&world, indoors),
            "saw through the office roof"
        );
    }

    /// Fly it to a scene and leave it there, returning the clock it stopped
    /// at — the shared setup for the grade tests below.
    fn watching_over(roost: &mut Roost, world: &World, scene: BlockPos) -> u64 {
        roost.report(scene, Report::Gunshot);
        let mut clock = 0u64;
        for _ in 0..400 {
            clock += 1;
            roost.tick(world, clock);
            roost.hold_interest();
            if matches!(roost.state, State::Watching { .. }) {
                return clock;
            }
        }
        panic!("never reached the scene");
    }

    #[test]
    fn a_blinded_box_stands_down_and_comes_back_when_it_wears_off() {
        let world = town_world();
        let mut roost = Roost::new(box_home());
        let scene = BlockPos::new(roost.home.x - 6, roost.home.y - 3, roost.home.z - 4);

        let hold = crate::intrusion::Grade::Blind.hold_ticks();
        roost.hack(crate::intrusion::Grade::Blind, hold);
        roost.tick(&world, 1);
        assert!(roost.blinded());
        roost.report(scene, Report::Gunshot);
        assert!(!roost.aloft(), "a dark box answered the door");
        assert_eq!(roost.status(), "DARK");

        // Routine maintenance finds it, and the town is watching again.
        roost.tick(&world, hold);
        assert!(!roost.blinded(), "the blinding never wore off");
        roost.report(scene, Report::Gunshot);
        roost.tick(&world, hold + 1);
        assert!(roost.aloft(), "it stayed down after the hack expired");
    }

    #[test]
    fn a_silenced_box_still_flies_and_still_sees_but_files_nothing() {
        let world = town_world();
        let mut roost = Roost::new(box_home());
        let scene = BlockPos::new(roost.home.x - 6, roost.home.y - 3, roost.home.z - 4);
        let clock = watching_over(&mut roost, &world, scene);

        let at = roost.position();
        let exposed = DVec3::new(at.x as f64 + 3.5, at.y as f64 - 4.0, at.z as f64 + 0.5);
        assert!(roost.witnesses(&world, exposed), "it was not watching to begin with");

        roost.hack(crate::intrusion::Grade::Silence, clock + 1_000);
        roost.tick(&world, clock + 1);
        assert!(roost.aloft(), "silencing grounded it — that is blinding");
        assert!(
            roost.sees(&world, exposed),
            "a silenced box should still see; that is what makes it invisible"
        );
        assert!(
            !roost.witnesses(&world, exposed),
            "a silenced box filed a report anyway"
        );
        assert_eq!(roost.status(), "SILENCED");
    }

    #[test]
    fn a_tap_lasts_longest_and_says_so() {
        let world = town_world();
        let mut roost = Roost::new(box_home());
        let hold = crate::intrusion::Grade::Tap.hold_ticks();
        roost.hack(crate::intrusion::Grade::Tap, hold);
        roost.tick(&world, 1);
        assert!(roost.tapped());
        assert_eq!(roost.status(), "TAPPED");
        // Nothing about a tapped box looks wrong: it answers reports as
        // usual, which is why it survives longest.
        assert!(!roost.blinded());
        roost.tick(&world, hold);
        assert!(!roost.tapped(), "the tap outlived its hold");
        assert!(
            hold > crate::intrusion::Grade::Blind.hold_ticks(),
            "the tap should outlast the blinding"
        );
    }

    #[test]
    fn drilling_the_box_out_buys_the_same_darkness_loudly() {
        let world = town_world();
        let mut roost = Roost::new(box_home());
        roost.knock_out(2_000);
        roost.tick(&world, 1);
        assert!(roost.blinded(), "the box was drilled out and kept watching");
    }

    #[test]
    fn two_responses_to_the_same_report_are_identical() {
        let world = town_world();
        let respond = || {
            let mut roost = Roost::new(box_home());
            let scene = BlockPos::new(roost.home.x - 9, roost.home.y - 3, roost.home.z - 7);
            roost.report(scene, Report::Breach);
            let mut path = Vec::new();
            for clock in 1..=300u64 {
                roost.tick(&world, clock);
                path.push(roost.position());
            }
            path
        };
        assert_eq!(respond(), respond(), "the response is not deterministic");
    }
}
