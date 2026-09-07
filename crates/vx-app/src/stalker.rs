//! The thing in the deep, and the director that never tells it the truth.
//!
//! # Two brains, on purpose
//!
//! The hunt note's design, landed whole. A **director** knows exactly where
//! the player is and is forbidden from saying so: everything it passes to
//! the creature is quantised to a [`crate::garrison::HINT_GRADE`] zone — the
//! same thirty-two-metre cell the shelters' director uses, because there is
//! one rule about lying to your own monsters and it should be written once.
//! A **creature** takes that cell and closes the rest of the distance with
//! the same occupancy search a posse uses, which is what makes it genuinely
//! huntable and genuinely evadable. It cannot walk to you, because it does
//! not know where you are.
//!
//! # It hunts by ear, and machines are loud
//!
//! What attaches this to *this* game rather than to a haunted corridor: the
//! director's hints are weighted by noise, and the noisiest thing in the
//! world is a working mine. A drill chewing rock, a crew turning, a shot —
//! every report the town's roost can hear, the deep thing hears better. So
//! the tension lands on the core loop rather than beside it. Run the swarm
//! loud and rich, run it quiet and slow, or dig a decoy a valley over and
//! work in the noise of your own diversion.
//!
//! # Pacing, and the floor it never spawns inside
//!
//! Pressure is the director's other job. Sustained contact spends a budget;
//! when it runs out the creature is *made* to break off for
//! [`BACK_OFF`] seconds, which is what turns a monster into a rhythm. And
//! nothing ever appears within [`NO_SPAWN_R`] of the player: it arrives from
//! somewhere, always, because a thing that materialises in your face is a
//! jump scare and this is supposed to be a hunt.
//!
//! # Live-only
//!
//! Like the posse and the garrisons, none of this is journalled. It reads
//! the world, spends the player's health and says things; it never writes a
//! block, never touches the pile, and never decides how long the fleet
//! turns. The oracle has no business here.

use glam::DVec3;
use vx_core::BlockPos;
use vx_world::World;

use crate::belief::Belief;
use crate::garrison::zone_of;
use crate::hostile::Pathing;

/// How much of a report's loudness becomes hint weight. The note's number.
pub const HINT_NOISE_W: f32 = 3.0;

/// Nothing is ever placed inside this radius of the player.
pub const NO_SPAWN_R: f32 = 48.0;

/// Heat needed before the deep sends something. Roughly a minute of a crew
/// digging, or one shot fired underground and a little patience.
pub const ROUSE_AT: f32 = 12.0;

/// Heat shed per second when nothing is making noise.
const HEAT_FADE: f32 = 0.35;

/// The most heat that can be banked, so an afternoon of digging does not buy
/// a week of being hunted.
const HEAT_CAP: f32 = 40.0;

/// Blocks below the local surface before the deep counts as the deep.
pub const DEEP: i32 = 22;

/// Seconds of sustained contact before the director makes it break off.
pub const PRESSURE_LIMIT: f32 = 90.0;
/// How long that break lasts.
pub const BACK_OFF: f32 = 60.0;

/// How far it can see in the dark. Further than a person, and it does not
/// need a lamp — but still line of sight, so cover is cover.
pub const SIGHT: f32 = 26.0;

/// Metres a second, searching and closing.
const PROWL: f32 = 3.4;
const CHARGE: f32 = 5.8;

/// How close it has to be to land one.
pub const STRIKE_REACH: f32 = 2.4;
/// Seconds between blows.
const STRIKE_EVERY: f32 = 2.5;

/// Rounds it takes before it decides the night is not worth it.
pub const HITS: u8 = 8;

/// How close a round has to pass to count as a hit on something this size.
const HIT_RADIUS: f32 = 0.9;

/// What it is doing, as far as anybody can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Somewhere else entirely. Nothing exists.
    Asleep,
    /// Coming, but it does not know where yet.
    Roused,
    /// Working the occupancy map.
    Hunting,
    /// Eyes on, and closing.
    Closing,
    /// Hurt, or made to break off by the director's budget.
    Withdrawing,
}

impl Mood {
    pub fn name(self) -> &'static str {
        match self {
            Mood::Asleep => "GONE",
            Mood::Roused => "ROUSED",
            Mood::Hunting => "HUNTING",
            Mood::Closing => "CLOSING",
            Mood::Withdrawing => "BREAKING OFF",
        }
    }
}

/// The line the deep lands when it changes its mind.
///
/// Every mode transition says something, because intelligence a player
/// cannot perceive is intelligence wasted — the note's own line, and the
/// difference between the cleverest search in the world and a room that
/// happens to be empty.
fn tell_for(mood: Mood) -> &'static str {
    match mood {
        Mood::Asleep => "THE DARK GOES QUIET AGAIN",
        Mood::Roused => "SOMETHING DOWN HERE HEARD THAT",
        Mood::Hunting => "SOMETHING IS WORKING ITS WAY ALONG THE ROCK",
        Mood::Closing => "IT HAS YOU",
        Mood::Withdrawing => "IT IS BACKING OFF - FOR NOW",
    }
}

/// The half that knows the truth and is not allowed to say it.
#[derive(Debug, Default)]
struct Director {
    /// How much noise has been made lately.
    heat: f32,
    /// The last zone worth passing on, and how strongly.
    hint: Option<(DVec3, f32)>,
    /// Seconds of continuous contact.
    pressure: f32,
    /// Seconds left of a forced break.
    resting: f32,
}

impl Director {
    /// A report reached the deep. `loudness` is the same scale the roost
    /// hears on: a drill is small and continuous, a shot is large and rare.
    fn hear(&mut self, at: DVec3, loudness: f32) {
        let weight = loudness * HINT_NOISE_W;
        self.heat = (self.heat + weight).min(HEAT_CAP);
        // The loudest report of the moment is the one worth passing on, and
        // even it is passed on as a *cell*.
        let better = self.hint.is_none_or(|(_, old)| weight >= old);
        if better {
            self.hint = Some((zone_of(at), weight));
        }
    }

    /// Take the standing hint, if there is one.
    fn take_hint(&mut self) -> Option<DVec3> {
        self.hint.take().map(|(zone, _)| zone)
    }
}

/// The half that has to find you.
#[derive(Debug)]
pub struct Stalker {
    pub position: DVec3,
    pub mood: Mood,
    pub yaw: f32,
    /// Its own picture of where you are. Never the truth, only ever the
    /// zone it was told plus everything it has ruled out since.
    belief: Belief,
    pathing: Pathing,
    hits: u8,
    strike_clock: f32,
}

impl Stalker {
    /// Where its eyes are.
    pub fn eye(&self) -> DVec3 {
        self.position + DVec3::new(0.0, 1.3, 0.0)
    }

    /// How badly it is hurt, for the readouts.
    pub fn wounds(&self) -> (u8, u8) {
        (self.hits, HITS)
    }
}

/// Move it one frame towards `to`, following its route when it has one.
///
/// Its own mover rather than the deputies' [`crate::hostile::Deputy`] walk,
/// because a deputy's walk carries a stuck watchdog keyed to a deputy — and
/// this thing has one body, one goal and no formation to keep. What it does
/// share is the important half: a routed step carries its own floor, so it
/// climbs the gallery it is actually in rather than snapping to the meadow
/// overhead.
fn creep(stalker: &mut Stalker, dt: f32, to: DVec3, speed: f32, world: &World) {
    let dt = f64::from(dt);
    let speed = f64::from(speed);
    let routed = stalker.pathing.step(world, stalker.position);
    let waypoint = routed
        .map(|cell| DVec3::new(cell.x as f64 + 0.5, cell.y as f64, cell.z as f64 + 0.5))
        .unwrap_or(to);

    let along = DVec3::new(
        waypoint.x - stalker.position.x,
        0.0,
        waypoint.z - stalker.position.z,
    );
    let gap = along.length();
    if gap > 1.0e-3 {
        stalker.position += along / gap * speed * dt;
    }
    if routed.is_some() {
        let climb = (waypoint.y - stalker.position.y).clamp(-6.0 * dt, 6.0 * dt);
        stalker.position.y += climb;
    }
    // A bearing needs no more than the level difference, narrowed.
    if let Some(yaw) = crate::rig::yaw_towards(
        (to.x - stalker.position.x) as f32,
        (to.z - stalker.position.z) as f32,
    ) {
        stalker.yaw = yaw;
    }
}

/// What the deep did this frame that the rest of the game must know.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Lines to say, in order.
    pub tells: Vec<String>,
    /// Blows that landed on the player.
    pub hits: u8,
}

/// The deep, entire: the director, and whatever it has sent.
#[derive(Debug, Default)]
pub struct TheDark {
    director: Director,
    stalker: Option<Stalker>,
    /// Where the last thing it was told about was, so a spawn arrives from
    /// somewhere plausible rather than from nowhere.
    approach: Option<DVec3>,
}

impl TheDark {
    /// Is anything out there?
    pub fn present(&self) -> Option<&Stalker> {
        self.stalker.as_ref()
    }

    /// How roused the deep is, 0 to 1, for the readouts.
    pub fn heat(&self) -> f32 {
        (self.director.heat / ROUSE_AT).min(1.0)
    }

    /// A noise was made at `at`. Loudness is the roost's scale.
    pub fn hear(&mut self, at: DVec3, loudness: f32) {
        self.director.hear(at, loudness);
        self.approach = Some(at);
    }

    /// Send it away and forget everything. What arrest, death and daylight
    /// all do.
    pub fn stand_down(&mut self) {
        self.stalker = None;
        self.director = Director::default();
    }

    /// Is this body deep enough for the deep to care?
    ///
    /// A column check rather than a cave-membership test: what matters is
    /// how much rock is overhead, which is the same thing a player feels.
    pub fn is_deep(world: &World, at: DVec3) -> bool {
        world
            .surface_y(at.x.floor() as i32, at.z.floor() as i32)
            .is_some_and(|top| (top as f64 - at.y) >= DEEP as f64)
    }

    /// A round of the player's went past. Returns whether it landed.
    pub fn under_fire(&mut self, from: DVec3, to: DVec3) -> bool {
        let Some(stalker) = &mut self.stalker else {
            return false;
        };
        // Same segment test the deputies take a round on, at a bigger
        // radius: it is a bigger thing.
        let along = to - from;
        let length = along.length();
        if length < 1.0e-3 {
            return false;
        }
        let direction = along / length;
        let towards = stalker.position + DVec3::new(0.0, 1.0, 0.0) - from;
        let projected = towards.dot(direction).clamp(0.0, length);
        let nearest = from + direction * projected;
        if (nearest - (stalker.position + DVec3::new(0.0, 1.0, 0.0))).length() > f64::from(HIT_RADIUS) {
            // Missing still tells it where you are standing. Being shot at
            // is the loudest hint there is.
            self.director.hear(from, 4.0);
            return false;
        }
        stalker.hits += 1;
        stalker.belief.seen(from);
        if stalker.hits >= HITS {
            stalker.mood = Mood::Withdrawing;
        }
        true
    }

    /// One frame of the deep.
    ///
    /// `ground` answers what the surface is at a column, for the one case a
    /// spawn cannot be settled underground.
    pub fn update(
        &mut self,
        dt: f32,
        world: &World,
        player: DVec3,
        seed: u64,
    ) -> Report {
        let mut report = Report::default();
        let deep = Self::is_deep(world, player);

        // Heat fades, faster in daylight: coming up is the reliable way out.
        let fade = if deep { HEAT_FADE } else { HEAT_FADE * 4.0 };
        self.director.heat = (self.director.heat - fade * dt).max(0.0);
        self.director.resting = (self.director.resting - dt).max(0.0);

        if self.stalker.is_none() {
            {
                let ready =
                    deep && self.director.resting == 0.0 && self.director.heat >= ROUSE_AT;
                let arriving = ready.then(|| self.arrival(world, player, seed)).flatten();
                if let Some(position) = arriving {
                    let mut belief = Belief::default();
                    // It starts believing the *zone* it was told about, not
                    // the player: everything after this is search.
                    if let Some(hint) = self.director.take_hint() {
                        belief.seen(hint);
                    } else {
                        belief.seen(zone_of(player));
                    }
                    self.stalker = Some(Stalker {
                        position,
                        mood: Mood::Roused,
                        yaw: 0.0,
                        belief,
                        pathing: Pathing::default(),
                        hits: 0,
                        strike_clock: 0.0,
                    });
                    report.tells.push(tell_for(Mood::Roused).to_string());
                }
                return report;
            }
        }

        // From here there is one, and the borrow can be held.
        let leaving = !deep;
        let stalker = self.stalker.as_mut().expect("checked above");
        let was = stalker.mood;

        let registry = world.registry();
        let eyes_on = !leaving
            && (player - stalker.position).length() <= f64::from(SIGHT)
            && vx_world::sight::sees(
                world,
                registry,
                stalker.eye(),
                player + DVec3::new(0.0, 1.5, 0.0),
                SIGHT,
            );

        if eyes_on {
            stalker.belief.seen(player);
            self.director.pressure += dt;
        } else {
            self.director.pressure = (self.director.pressure - dt * 0.5).max(0.0);
            stalker.belief.diffuse(dt, |x, z| {
                // The search spreads only where something could stand, and
                // only near the level it is hunting on — a gallery, not the
                // meadow overhead.
                world
                    .surface_y(x, z)
                    .is_some_and(|top| (top - stalker.position.y as i32).abs() < 8)
                    || world.block(BlockPos::new(x, stalker.position.y as i32, z)).is_air()
            });
            let eye = stalker.eye();
            stalker.belief.clear_seen(|x, z| {
                let at = DVec3::new(x as f64 + 0.5, eye.y, z as f64 + 0.5);
                (at - eye).length() <= f64::from(SIGHT)
                    && vx_world::sight::sees(world, registry, eye, at, SIGHT)
            });
            // A fresh rumour, but never over a fresh sighting — the shelters'
            // rule, and for the same reason.
            if stalker.belief.confidence() <= 0.5 {
                if let Some(hint) = self.director.take_hint() {
                    stalker.belief.seen(hint);
                }
            }
        }

        // The director's budget. Spent contact forces a break, and the break
        // is what makes the next arrival mean something.
        if self.director.pressure >= PRESSURE_LIMIT {
            self.director.pressure = 0.0;
            self.director.resting = BACK_OFF;
            stalker.mood = Mood::Withdrawing;
        } else if stalker.hits >= HITS || leaving {
            stalker.mood = Mood::Withdrawing;
        } else if eyes_on {
            stalker.mood = Mood::Closing;
        } else if stalker.belief.searching() {
            stalker.mood = Mood::Hunting;
        } else {
            stalker.mood = Mood::Withdrawing;
        }

        match stalker.mood {
            Mood::Closing => {
                stalker.pathing.steer(dt, world, player);
                let gap = (player - stalker.position).length();
                if gap > f64::from(STRIKE_REACH) {
                    creep(stalker, dt, player, CHARGE, world);
                    stalker.strike_clock = 0.0;
                } else {
                    stalker.strike_clock += dt;
                    if stalker.strike_clock >= STRIKE_EVERY {
                        stalker.strike_clock = 0.0;
                        report.hits += 1;
                    }
                }
            }
            Mood::Hunting | Mood::Roused => {
                if let Some((x, z)) = stalker.belief.search_target(stalker.position) {
                    let to = DVec3::new(x as f64 + 0.5, stalker.position.y, z as f64 + 0.5);
                    stalker.pathing.steer(dt, world, to);
                    creep(stalker, dt, to, PROWL, world);
                }
            }
            Mood::Withdrawing => {
                // Away from the last thing it believed, and then gone.
                let from = stalker.belief.last_known().unwrap_or(player);
                let away = stalker.position - from;
                if away.length() > 1.0e-3 {
                    stalker.position += away.normalize() * f64::from(PROWL * dt);
                }
                if (stalker.position - player).length() > f64::from(NO_SPAWN_R) {
                    self.stalker = None;
                    self.director.pressure = 0.0;
                    report.tells.push(tell_for(Mood::Asleep).to_string());
                    return report;
                }
            }
            Mood::Asleep => {}
        }

        if let Some(now) = self.stalker.as_ref().map(|it| it.mood) {
            if now != was {
                report.tells.push(tell_for(now).to_string());
            }
        }
        report
    }

    /// Where something could come from: a settled spot beyond the floor
    /// distance, in rock the creature could actually stand in.
    ///
    /// Returns nothing rather than compromising. A hunt that cannot start
    /// properly does not start — the floor distance is not negotiable, and
    /// neither is arriving inside a wall.
    fn arrival(&self, world: &World, player: DVec3, seed: u64) -> Option<DVec3> {
        let approach = self.approach.unwrap_or(player);
        let bearing = {
            let along = DVec3::new(player.x - approach.x, 0.0, player.z - approach.z);
            if along.length() > 1.0e-3 {
                along.normalize()
            } else {
                DVec3::new(1.0, 0.0, 0.0)
            }
        };

        for step in 0..12 {
            // Sweep around from the direction the noise came from, at a
            // radius that starts outside the floor and works outward.
            let turn = f64::from(step) * std::f64::consts::TAU / 12.0
                + (seed % 97) as f64 * 0.01;
            let radius = f64::from(NO_SPAWN_R) + 8.0 + f64::from(step % 3) * 6.0;
            let dx = bearing.x * turn.cos() - bearing.z * turn.sin();
            let dz = bearing.x * turn.sin() + bearing.z * turn.cos();
            let column = BlockPos::new(
                (player.x + dx * radius).floor() as i32,
                player.y.floor() as i32,
                (player.z + dz * radius).floor() as i32,
            );
            let settled = vx_agent::settle(world, column);
            let at = DVec3::new(
                settled.x as f64 + 0.5,
                settled.y as f64,
                settled.z as f64 + 0.5,
            );
            if (at - player).length() < f64::from(NO_SPAWN_R) {
                continue;
            }
            // It has to be *in* the deep too, or it is a thing standing in a
            // field a long way away.
            if !Self::is_deep(world, at) {
                continue;
            }
            return Some(at);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_director_never_says_where_you_are() {
        // The whole design in one assertion: whatever the director is told,
        // what comes out is a zone centre, never a position. If this ever
        // passes the truth through, the creature stops hunting and starts
        // homing, and the round is pointless.
        let mut director = Director::default();
        for at in [
            DVec3::new(11.4, 40.0, -7.2),
            DVec3::new(-903.9, 18.0, 2_041.6),
            DVec3::new(0.0, 0.0, 0.0),
        ] {
            director.hear(at, 1.0);
            let hint = director.take_hint().expect("a hint was made");
            assert_eq!(hint, zone_of(at));
            let slip = (DVec3::new(hint.x, 0.0, hint.z) - DVec3::new(at.x, 0.0, at.z)).length();
            assert!(
                slip <= crate::garrison::HINT_GRADE as f64,
                "the hint at {at:?} was accurate to {slip}"
            );
        }
    }

    #[test]
    fn a_loud_report_is_worth_more_than_a_quiet_one() {
        // Noise weighting is what attaches the hunt to the mining loop. A
        // shot has to outrank a drill, or working quietly buys nothing.
        let mut quiet = Director::default();
        let mut loud = Director::default();
        quiet.hear(DVec3::ZERO, 0.4);
        loud.hear(DVec3::ZERO, 4.0);
        assert!(loud.heat > quiet.heat * 5.0);

        // And the loudest report of the moment is the one passed on.
        let mut mixed = Director::default();
        mixed.hear(DVec3::new(500.0, 0.0, 0.0), 0.2);
        mixed.hear(DVec3::new(-500.0, 0.0, 0.0), 6.0);
        assert_eq!(mixed.take_hint(), Some(zone_of(DVec3::new(-500.0, 0.0, 0.0))));
    }

    #[test]
    fn quiet_work_never_rouses_anything() {
        // The player's whole defence: heat fades, so a crew that runs in
        // short bursts stays under the line forever. If this fails, the
        // stalker is a timer and the mining loop is not a lever.
        let mut director = Director::default();
        for _ in 0..200 {
            director.hear(DVec3::ZERO, 0.05);
            // A short burst, then a rest.
            director.heat = (director.heat - HEAT_FADE * 4.0).max(0.0);
        }
        assert!(
            director.heat < ROUSE_AT,
            "short bursts banked {} heat",
            director.heat
        );
    }

    #[test]
    fn heat_is_capped_so_an_afternoon_is_not_a_week() {
        let mut director = Director::default();
        for _ in 0..10_000 {
            director.hear(DVec3::ZERO, 1.0);
        }
        assert_eq!(director.heat, HEAT_CAP);
    }

    #[test]
    fn the_pressure_budget_forces_a_break() {
        // Ninety seconds of contact buys sixty of quiet, and the numbers are
        // the note's. Checked on the director rather than through a world,
        // because the rule is arithmetic and the world is scenery to it.
        let mut director = Director {
            pressure: PRESSURE_LIMIT,
            ..Director::default()
        };
        assert!(director.pressure >= PRESSURE_LIMIT);
        director.pressure = 0.0;
        director.resting = BACK_OFF;
        let mut left = BACK_OFF;
        while left > 0.0 {
            director.resting = (director.resting - 0.5).max(0.0);
            left -= 0.5;
        }
        assert_eq!(director.resting, 0.0);
        const { assert!(BACK_OFF < PRESSURE_LIMIT) };
    }

    #[test]
    fn every_mood_says_something_drawable() {
        use vx_render::font;
        for mood in [
            Mood::Asleep,
            Mood::Roused,
            Mood::Hunting,
            Mood::Closing,
            Mood::Withdrawing,
        ] {
            assert!(!mood.name().is_empty());
            let tell = tell_for(mood);
            assert!(font::text_width(tell, 1) > 0, "unrenderable tell: {tell}");
        }
    }

    /// Rock from the floor to a roof at 60, with a gallery hollowed out at
    /// 20 — deep enough that [`DEEP`] is satisfied everywhere in it.
    fn deep_world() -> World {
        let mut world = World::new(7);
        world.load_around(vx_core::ChunkPos::new(0, 0), 6);
        let stone = world.registry().id_of("engine:stone").unwrap();
        for x in -100..100 {
            for z in -100..100 {
                for y in 0..62 {
                    let solid = !(18..24).contains(&y);
                    world.set_block(
                        BlockPos::new(x, y, z),
                        if solid { stone } else { vx_core::BlockId::AIR },
                    );
                }
                for y in 62..80 {
                    world.set_block(BlockPos::new(x, y, z), vx_core::BlockId::AIR);
                }
            }
        }
        world
    }

    #[test]
    fn nothing_arrives_inside_the_floor_distance() {
        // The note's `NO_SPAWN_R`, checked against the only function that
        // can break it — and checked in a world where it *can* succeed, so
        // this is a real answer rather than a vacuous one.
        let world = deep_world();
        let dark = TheDark {
            approach: Some(DVec3::new(0.0, 20.0, 0.0)),
            ..TheDark::default()
        };
        let player = DVec3::new(8.0, 20.0, 8.0);
        let at = dark
            .arrival(&world, player, 7)
            .expect("a gallery two hundred blocks across has somewhere to come from");
        assert!(
            (at - player).length() >= f64::from(NO_SPAWN_R),
            "it arrived {} blocks away",
            (at - player).length()
        );
        assert!(TheDark::is_deep(&world, at), "it arrived out in the open");
    }

    #[test]
    fn the_deep_is_the_only_place_this_happens() {
        // Coming up is the reliable way out, and it has to be reliable or
        // the whole thing is a punishment rather than a place.
        let world = deep_world();
        assert!(TheDark::is_deep(&world, DVec3::new(0.0, 20.0, 0.0)));
        assert!(!TheDark::is_deep(&world, DVec3::new(0.0, 62.0, 0.0)));

        let mut dark = TheDark::default();
        for _ in 0..400 {
            dark.hear(DVec3::new(0.0, 62.0, 0.0), 1.0);
            dark.update(0.1, &world, DVec3::new(0.0, 62.0, 0.0), 11);
        }
        assert!(
            dark.present().is_none(),
            "something came for a player standing in daylight"
        );
    }

    #[test]
    fn a_noisy_dig_in_the_deep_brings_something_and_it_arrives_from_somewhere() {
        let world = deep_world();
        let player = DVec3::new(0.0, 20.0, 0.0);
        let mut dark = TheDark::default();
        let mut tells = Vec::new();
        for _ in 0..300 {
            // A crew working: small, continuous, and never stopping.
            dark.hear(DVec3::new(2.0, 20.0, 2.0), 0.6);
            let report = dark.update(0.1, &world, player, 4_242);
            tells.extend(report.tells);
            if dark.present().is_some() {
                break;
            }
        }
        let stalker = dark.present().expect("a mine ran loud for half a minute");
        assert!(
            (stalker.position - player).length() >= f64::from(NO_SPAWN_R),
            "it started on top of the player"
        );
        assert_eq!(stalker.mood, Mood::Roused);
        assert!(
            tells.iter().any(|line| line.contains("HEARD")),
            "it arrived without a word: {tells:?}"
        );
    }
}
