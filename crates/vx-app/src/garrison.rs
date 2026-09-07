//! The bunkers, occupied: holders on the shelters, and the director that
//! paces them.
//!
//! # The loot loop grows teeth
//!
//! Bunkers have been free salvage since stage 19 — find the hatch, walk
//! down, strip the caches. Now every shelter is *held*: a squad derived
//! from the bunker's own seed, standing watch on its hatch, running the
//! same composure, cover and fire-discipline machinery the warrant's
//! deputies run. Same bunker, same holders, every session — the roster is
//! worldgen, like everything else about a shelter.
//!
//! # The director never tells the truth
//!
//! The hunt note's stalker discipline, landed early on the garrisons: what
//! a squad learns from *noise* — a shot, a working drill — is never a
//! position, only a zone. Hints are quantised to [`HINT_GRADE`]-metre cell
//! centres before the belief ever sees them, asserted at the interface, so
//! a garrison that heard you must still close the gap with the same
//! occupancy search as everyone else. Machines are loud: a dig site in
//! held country is a dinner bell, and the tension attaches to the mining
//! loop rather than to a haunted corridor.
//!
//! # Pairs, bounded on purpose
//!
//! Squad tactics are exactly two rules, as the people note prescribed:
//! pairs move alternately — one moves while one watches — and a pair that
//! loses its partner takes the ally-down composure hit, which `rake`
//! already charges. Suppression arcs and command layers are out of scope,
//! and the morale system generates enough behaviour to read without them.
//!
//! # Live-only, like every reaction to the player
//!
//! Holders never touch a block, their rounds damage you and not the world,
//! and a cleared shelter is session state. Nothing here reaches the replay
//! oracle, and the journal never learns a bunker was held.

use glam::DVec3;

use vx_world::bunker::Tier;
use vx_world::World;

use crate::belief::Belief;
use crate::hostile::{self, Deputy, Mode, Pathing, Report};

/// The zone grade: a noise hint is never finer than this many metres.
pub const HINT_GRADE: i32 = 32;

/// Metres beyond a bunker's own reach at which its garrison musters.
pub const MUSTER_MARGIN: f32 = 30.0;

/// How far from the hatch a holder will chase before the leash pulls.
pub const LEASH: f32 = 46.0;

/// Seconds each half of a pair spends moving while the other watches.
pub const OVERWATCH_SECONDS: f32 = 2.0;

/// Seconds a challenged stranger has to turn around before a truce-holding
/// squad decides they are not a stranger passing through.
pub const GRACE_SECONDS: f32 = 6.0;

/// Inside this fraction of the leash, a challenge stops being a courtesy.
pub const INNER_RING: f32 = 0.5;

/// What turning in a surrendered holder pays, by the shelter's tier — the
/// board's premium postings, finally posted.
pub fn capture_pay(tier: Tier) -> u64 {
    match tier {
        Tier::Small => 80,
        Tier::Medium => 120,
        Tier::Large => 180,
    }
}

/// Holders a shelter keeps, by tier.
pub fn strength(tier: Tier) -> usize {
    match tier {
        Tier::Small => 2,
        Tier::Medium => 3,
        Tier::Large => 4,
    }
}

/// Quantise a heard position to its zone centre. The director's whole
/// vocabulary: it may say "that cell", never "right there".
pub fn zone_of(at: DVec3) -> DVec3 {
    let grade = HINT_GRADE as f64;
    DVec3::new(
        (at.x / grade).floor() * grade + grade * 0.5,
        at.y,
        (at.z / grade).floor() * grade + grade * 0.5,
    )
}

/// One held shelter.
#[derive(Debug)]
pub struct Garrison {
    /// The bunker this squad holds, by its centre — the same key everything
    /// else about a bunker hangs off.
    pub centre: (i32, i32),
    pub tier: Tier,
    /// Where the hatch is: home, and the anchor of the leash.
    hatch: DVec3,
    pub holders: Vec<Deputy>,
    belief: Belief,
    pathing: Pathing,
    /// Seconds of walking left in the current overwatch window, and whose
    /// turn it is: pairs alternate, one moving while one watches.
    window: f32,
    movers_even: bool,
    /// The truce is over: they have been shot at, robbed, or crowded, and
    /// standing no longer buys the player a challenge before contact.
    grudge: bool,
    /// Whether the stranger has been told to walk on, and how long they
    /// have left to do it.
    challenged: bool,
    grace: f32,
}

impl Garrison {
    /// Raise a shelter's squad from its own seed: same bunker, same
    /// holders, every session.
    pub fn muster(site: &vx_world::bunker::BunkerSite) -> Self {
        let hatch = DVec3::new(
            site.hatch.0 as f64 + 0.5,
            (site.hatch_ground + 1) as f64,
            site.hatch.1 as f64 + 0.5,
        );
        let holders = (0..strength(site.tier))
            .map(|index| {
                let angle = index as f64 * std::f64::consts::TAU / strength(site.tier) as f64;
                let at = hatch + DVec3::new(angle.cos() * 4.0, 0.0, angle.sin() * 4.0);
                Deputy::new(
                    at,
                    crate::people::temperament_from(site.seed ^ (index as u64) << 3),
                    index % 3,
                )
            })
            .collect();
        Garrison {
            centre: site.centre,
            tier: site.tier,
            hatch,
            holders,
            belief: Belief::default(),
            pathing: Pathing::default(),
            window: OVERWATCH_SECONDS,
            movers_even: true,
            grudge: false,
            challenged: false,
            grace: GRACE_SECONDS,
        }
    }

    /// Holders still standing and unsurrendered.
    pub fn active(&self) -> usize {
        self.holders.iter().filter(|holder| holder.active()).count()
    }

    /// Is this shelter's squad finished — everyone down or taken in?
    pub fn broken(&self) -> bool {
        self.holders.iter().all(|holder| !holder.active())
    }

    /// A noise reached this garrison. The director's one sentence: the
    /// zone, never the spot, and only when it does not already know better.
    pub fn hear(&mut self, at: DVec3) {
        let gap = (at - self.hatch).length();
        if gap > f64::from(LEASH) + f64::from(HINT_GRADE) {
            return;
        }
        // Noise *on* their ground is a provocation, truce or none: a drill
        // chewing rock inside the leash is not a stranger passing through.
        if gap <= f64::from(LEASH) {
            self.grudge = true;
        }
        // Fresh eyes beat a rumour: a hint never overwrites a confident
        // sighting, only a stale or absent one.
        if self.belief.confidence() > 0.5 {
            return;
        }
        self.belief.seen(zone_of(at));
    }

    /// A round of the player's crossed this squad.
    pub fn under_fire(&mut self, from: DVec3, to: DVec3) -> Report {
        // A round is the end of any conversation.
        self.grudge = true;
        // Being shot from anywhere near tells them roughly where from.
        self.hear(from);
        let exposed = self.belief.confidence() > 0.9;
        hostile::rake(&mut self.holders, exposed, from, to)
    }

    /// One frame of holding the shelter.
    pub fn update(
        &mut self,
        dt: f32,
        world: &World,
        player: DVec3,
        player_down: bool,
        truce: bool,
    ) -> Report {
        let mut report = Report::default();

        // The overwatch clock: each window, half the squad may walk and the
        // other half stands watch. One moves; one watches.
        self.window -= dt;
        if self.window <= 0.0 {
            self.window = OVERWATCH_SECONDS;
            self.movers_even = !self.movers_even;
        }

        let registry = world.registry();
        let eye_of_player = player + DVec3::Y * f64::from(crate::awareness::PLAYER_EYE);
        let near = (player - self.hatch).length() <= f64::from(LEASH);

        let seen_by: Vec<bool> = self
            .holders
            .iter()
            .map(|holder| {
                holder.active()
                    && near
                    && (player - holder.position).length() <= f64::from(crate::awareness::SIGHT_RANGE * 1.5)
                    && vx_world::sight::sees(
                        world,
                        registry,
                        holder.eye(),
                        eye_of_player,
                        crate::awareness::SIGHT_RANGE * 1.5,
                    )
            })
            .collect();
        let exposed = seen_by.iter().any(|seen| *seen);

        // The truce: while the shelters have no grudge against this player
        // — by standing, and by nothing having been done *here* — a
        // stranger in sight gets a challenge and a grace period, not a
        // volley. Sentries watch; they do not hunt. The truce ends at the
        // inner ring, at the end of an ignored grace, at any noise on
        // their ground, or at the first round fired.
        if truce && !self.grudge {
            if exposed {
                let gap = (player - self.hatch).length();
                if !self.challenged {
                    self.challenged = true;
                    report
                        .barks
                        .push("HOLDER: WALK ON - THIS GROUND IS HELD".into());
                }
                self.grace = (self.grace - dt).max(0.0);
                if gap <= f64::from(LEASH * INNER_RING) || (self.grace == 0.0 && gap <= f64::from(LEASH * 0.75)) {
                    self.grudge = true;
                    report.barks.push("HOLDER: YOU WERE TOLD".into());
                }
                for (holder, sees) in self.holders.iter_mut().zip(&seen_by) {
                    if *sees {
                        holder.face(player);
                    }
                }
            } else {
                // Out of sight, the courtesy slowly re-arms.
                self.grace = (self.grace + dt * 0.5).min(GRACE_SECONDS);
            }
            if !self.grudge {
                return report;
            }
        }

        if exposed {
            self.belief.seen(player);
        } else if self.belief.searching() {
            self.belief.diffuse(dt, |x, z| {
                world
                    .surface_y(x, z)
                    .is_some_and(|top| (top - self.hatch.y as i32).abs() < 12)
            });
            let eyes: Vec<DVec3> = self
                .holders
                .iter()
                .filter(|holder| holder.active())
                .map(|holder| holder.eye())
                .collect();
            self.belief.clear_seen(|x, z| {
                eyes.iter().any(|eye| {
                    let at = DVec3::new(x as f64 + 0.5, eye.y, z as f64 + 0.5);
                    (at - *eye).length() <= f64::from(crate::awareness::SIGHT_RANGE)
                        && vx_world::sight::sees(
                            world,
                            registry,
                            *eye,
                            at,
                            crate::awareness::SIGHT_RANGE,
                        )
                })
            });
        }

        // The route chases the belief; idle squads route home.
        let goal = if exposed {
            player
        } else if let Some((x, z)) = self.belief.search_target(self.hatch) {
            DVec3::new(x as f64 + 0.5, self.hatch.y, z as f64 + 0.5)
        } else {
            self.hatch
        };
        self.pathing.steer(dt, world, goal);

        let positions: Vec<DVec3> = self.holders.iter().map(|holder| holder.position).collect();
        let search = self.belief.search_target(self.hatch);

        for (index, sees) in seen_by.iter().copied().enumerate() {
            let allies: Vec<DVec3> = positions
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, at)| *at)
                .collect();
            // The pair rule: on this window, only half the squad walks.
            // Everyone still faces, fires and takes cover — watching is not
            // sleeping — but pursuit legs alternate.
            let may_move = (index % 2 == 0) == self.movers_even;
            let hit = Self::hold_one(
                &mut self.holders[index],
                dt,
                world,
                player,
                player_down,
                sees,
                may_move,
                &allies,
                search,
                self.hatch,
                &self.pathing,
                &mut report,
            );
            if hit {
                report.hits += 1;
            }
        }

        // Nobody left believing anything, nobody in sight: the shelter
        // settles back to its watch.
        if !exposed && !self.belief.searching() {
            self.belief.abandon();
        }
        report
    }

    /// One holder's frame. Returns whether they landed a round.
    #[allow(clippy::too_many_arguments)]
    fn hold_one(
        holder: &mut Deputy,
        dt: f32,
        world: &World,
        player: DVec3,
        player_down: bool,
        sees: bool,
        may_move: bool,
        allies: &[DVec3],
        search: Option<(i32, i32)>,
        hatch: DVec3,
        route: &Pathing,
        report: &mut Report,
    ) -> bool {
        if !holder.active() {
            return false;
        }
        holder.reload = (holder.reload - dt).max(0.0);
        if !sees {
            holder.composure =
                (holder.composure + hostile::STEADY_PER_SECOND * dt).min(hostile::FULL_COMPOSURE);
        }

        let was = holder.mode;
        holder.mode = if sees {
            hostile::band(holder.composure, &holder.temperament)
        } else if holder.mode == Mode::Engage {
            Mode::Investigate
        } else if holder.mode == Mode::Investigate && search.is_none() {
            Mode::Patrol
        } else {
            holder.mode
        };
        if holder.mode != was {
            if let Some(line) = hostile::bark_for(holder.mode) {
                report.barks.push(line.replace("DEPUTY", "HOLDER"));
            }
        }

        // A downed player is not theirs to arrest: the shelter got what it
        // wanted, which is you not coming down the stairs.
        if player_down {
            hostile::Posse::walk(holder, dt, hatch, 2.2, world, None);
            return false;
        }

        match holder.mode {
            Mode::Engage => {
                let gap = (player - holder.position).length();
                holder.face(player);
                let leashed = (player - hatch).length() > f64::from(LEASH);
                if gap > f64::from(hostile::ENGAGE_RANGE) && !leashed && may_move {
                    hostile::Posse::walk(holder, dt, player, 3.0, world, Some(route));
                    return false;
                }
                if gap > f64::from(hostile::ENGAGE_RANGE) {
                    return false;
                }
                if holder.reload > 0.0
                    || !hostile::lane_is_clear(holder.eye(), player, allies)
                {
                    return false;
                }
                holder.reload = hostile::SHOT_INTERVAL;
                true
            }
            Mode::Hide => {
                if may_move {
                    if let Some(spot) = hostile::best_cover(world, holder.position, player) {
                        hostile::Posse::walk(holder, dt, spot, 2.6, world, None);
                    }
                }
                holder.face(player);
                false
            }
            Mode::Flee => {
                // A holder flees *into* the shelter's ground, not off the
                // map: the hatch is the bolt-hole.
                hostile::Posse::walk(holder, dt, hatch, 3.6, world, None);
                false
            }
            Mode::Investigate => {
                if may_move {
                    if let Some((x, z)) = search {
                        let to = DVec3::new(x as f64 + 0.5, holder.position.y, z as f64 + 0.5);
                        hostile::Posse::walk(holder, dt, to, 2.4, world, Some(route));
                        holder.face(to);
                    }
                }
                false
            }
            Mode::Patrol => {
                // Home to the hatch watch, drifting rather than marching.
                if may_move && (holder.position - hatch).length() > 5.0 {
                    hostile::Posse::walk(holder, dt, hatch, 1.6, world, None);
                }
                false
            }
            Mode::Surrender | Mode::Down => false,
        }
    }

    /// The surrendered holder nearest `at`, within `reach`.
    pub fn surrendered_near(&self, at: DVec3, reach: f32) -> Option<usize> {
        self.holders
            .iter()
            .enumerate()
            .filter(|(_, holder)| holder.mode == Mode::Surrender)
            .map(|(index, holder)| (index, (holder.position - at).length()))
            .filter(|(_, gap)| *gap <= f64::from(reach))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// Take a surrendered holder in: they leave the world, and the board
    /// pays. The quiet track's payoff — a shelter can be cleared without a
    /// single body if you can break everyone's nerve.
    pub fn arrest(&mut self, index: usize) -> Option<u64> {
        let holder = self.holders.get_mut(index)?;
        if holder.mode != Mode::Surrender {
            return None;
        }
        holder.mode = Mode::Down;
        Some(capture_pay(self.tier))
    }
}

/// Every held shelter the session has met.
#[derive(Debug, Default)]
pub struct Garrisons {
    pub squads: Vec<Garrison>,
    /// Shelters whose squads are finished: cleared is cleared, for the
    /// session — no respawning watch behind your back.
    cleared: Vec<(i32, i32)>,
}

impl Garrisons {
    /// Muster the garrison of any bunker the player has come near.
    pub fn muster_near(&mut self, world: &World, player: DVec3) {
        let sites = world
            .generator()
            .bunkers_near((player.x.floor() as i32, player.z.floor() as i32), 160);
        for site in sites {
            let hatch = DVec3::new(
                site.hatch.0 as f64,
                (site.hatch_ground + 1) as f64,
                site.hatch.1 as f64,
            );
            let margin = f64::from(site.reach()) + f64::from(MUSTER_MARGIN);
            if (DVec3::new(player.x, hatch.y, player.z) - hatch).length() > margin {
                continue;
            }
            if self.cleared.contains(&site.centre)
                || self.squads.iter().any(|squad| squad.centre == site.centre)
            {
                continue;
            }
            self.squads.push(Garrison::muster(&site));
        }
    }

    /// One frame for every mustered squad; broken ones become cleared.
    pub fn update(
        &mut self,
        dt: f32,
        world: &World,
        player: DVec3,
        player_down: bool,
        truce: bool,
    ) -> Report {
        let mut report = Report::default();
        for squad in &mut self.squads {
            let one = squad.update(dt, world, player, player_down, truce);
            report.hits += one.hits;
            report.downed += one.downed;
            report.barks.extend(one.barks);
        }
        let cleared = &mut self.cleared;
        self.squads.retain(|squad| {
            if squad.broken() {
                cleared.push(squad.centre);
                report.barks.push("THE SHELTER IS YOURS".into());
                report.cleared += 1;
                false
            } else {
                true
            }
        });
        report
    }

    /// A noise for every squad in earshot.
    pub fn hear(&mut self, at: DVec3) {
        for squad in &mut self.squads {
            squad.hear(at);
        }
    }

    /// A player round, past every squad.
    pub fn under_fire(&mut self, from: DVec3, to: DVec3) -> Report {
        let mut report = Report::default();
        for squad in &mut self.squads {
            let one = squad.under_fire(from, to);
            report.hits += one.hits;
            report.downed += one.downed;
            report.barks.extend(one.barks);
        }
        report
    }

    /// Is the scout's link jammed at `eye`?
    ///
    /// The spoofers stage 15 taught the player have arrived in the other
    /// side's hands: a shelter with a grudge runs a coil of its own, and
    /// inside its leash the kestrel's contact marks simply do not take.
    /// Only grudged shelters jam — running a jammer is itself a declaration,
    /// and a truce-holding squad would not tip its hand.
    pub fn jamming_at(&self, eye: DVec3) -> bool {
        self.squads.iter().any(|squad| {
            squad.grudge
                && !squad.broken()
                && (eye - squad.hatch).length() <= f64::from(LEASH)
        })
    }

    /// Holders actively after the player right now, for the HUD.
    pub fn hunting(&self) -> usize {
        self.squads
            .iter()
            .flat_map(|squad| &squad.holders)
            .filter(|holder| {
                holder.active() && !matches!(holder.mode, Mode::Patrol)
            })
            .count()
    }

    /// Try to take in a surrendered holder near `at`. Returns the pay.
    pub fn arrest_near(&mut self, at: DVec3, reach: f32) -> Option<u64> {
        for squad in &mut self.squads {
            if let Some(index) = squad.surrendered_near(at, reach) {
                return squad.arrest(index);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_site() -> vx_world::bunker::BunkerSite {
        let ground = |_: i32, _: i32| 90;
        vx_world::bunker::bunkers_near(2024, (0, 0), 6_000, &ground)
            .into_iter()
            .next()
            .expect("no bunker within six kilometres of the origin")
    }

    #[test]
    fn a_shelter_raises_the_same_squad_every_session() {
        let site = a_site();
        let first = Garrison::muster(&site);
        let second = Garrison::muster(&site);
        assert_eq!(first.holders.len(), strength(site.tier));
        for (a, b) in first.holders.iter().zip(&second.holders) {
            assert_eq!(a.temperament, b.temperament, "the roster re-rolled");
            assert_eq!(a.position, b.position);
        }
    }

    #[test]
    fn the_director_never_tells_the_truth() {
        // The note's interface assertion: what a garrison learns from noise
        // is a zone centre, never a position. Sampled everywhere.
        for step in 0..200 {
            let at = DVec3::new(step as f64 * 17.3 - 1000.0, 80.0, step as f64 * 11.7 - 700.0);
            let zone = zone_of(at);
            let grade = HINT_GRADE as f64;
            assert_eq!((zone.x - grade * 0.5) % grade, 0.0, "x is not a cell centre");
            assert_eq!((zone.z - grade * 0.5) % grade, 0.0, "z is not a cell centre");
            assert!((zone.x - at.x).abs() <= grade, "the zone lost the noise");
        }

        let site = a_site();
        let mut squad = Garrison::muster(&site);
        let noise = squad.hatch + DVec3::new(7.3, 0.0, -3.1);
        squad.hear(noise);
        let believed = squad.belief.last_known().expect("the hint was dropped");
        assert_eq!(
            DVec3::new(believed.x, 0.0, believed.z),
            DVec3::new(zone_of(noise).x, 0.0, zone_of(noise).z),
            "the belief was handed something finer than the zone"
        );
        assert_ne!(
            (believed.x, believed.z),
            (noise.x, noise.z),
            "the director told the truth"
        );
    }

    #[test]
    fn a_hint_never_overwrites_fresh_eyes() {
        let site = a_site();
        let mut squad = Garrison::muster(&site);
        let seen = squad.hatch + DVec3::new(2.0, 0.0, 2.0);
        squad.belief.seen(seen);
        squad.hear(squad.hatch + DVec3::new(30.0, 0.0, 30.0));
        assert_eq!(
            squad.belief.last_known(),
            Some(seen),
            "a rumour overwrote a sighting"
        );
    }

    #[test]
    fn noise_beyond_the_leash_is_nobody_here_business()
    {
        let site = a_site();
        let mut squad = Garrison::muster(&site);
        squad.hear(squad.hatch + DVec3::new(f64::from(LEASH) + f64::from(HINT_GRADE) + 20.0, 0.0, 0.0));
        assert!(squad.belief.last_known().is_none(), "they heard across the county");
    }

    #[test]
    fn capture_pays_by_tier_and_only_for_the_surrendered() {
        assert!(capture_pay(Tier::Large) > capture_pay(Tier::Small));

        let site = a_site();
        let mut squad = Garrison::muster(&site);
        // Nobody has surrendered: nothing to take in.
        assert_eq!(squad.surrendered_near(squad.hatch, 50.0), None);
        assert_eq!(squad.arrest(0), None, "arrested a holder still fighting");

        squad.holders[0].mode = Mode::Surrender;
        let near = squad.holders[0].position;
        let found = squad.surrendered_near(near, 3.0).expect("missed the surrender");
        let pay = squad.arrest(found).expect("the board did not pay");
        assert_eq!(pay, capture_pay(site.tier));
        // Taken in is taken in: not active, not arrestable twice.
        assert_eq!(squad.arrest(found), None, "paid twice for one head");
    }

    #[test]
    fn a_broken_squad_clears_the_shelter_for_good() {
        let site = a_site();
        let mut garrisons = Garrisons::default();
        garrisons.squads.push(Garrison::muster(&site));
        for holder in &mut garrisons.squads[0].holders {
            holder.mode = Mode::Down;
        }
        let world = World::new(2024);
        let report = garrisons.update(0.1, &world, DVec3::ZERO, false, false);
        assert!(report.barks.iter().any(|line| line.contains("YOURS")));
        assert!(garrisons.squads.is_empty());
        // And it stays cleared: mustering again finds nothing to raise.
        assert!(garrisons.cleared.contains(&site.centre));
    }

    #[test]
    fn a_neutral_stranger_is_challenged_and_fire_ends_the_courtesy() {
        let site = a_site();
        let mut squad = Garrison::muster(&site);
        let world = World::new(2024);
        // Standing far out under a truce: watched, not hunted — the update
        // returns before the engage machinery ever runs.
        let stranger = squad.hatch + DVec3::new(f64::from(LEASH * 0.9), 0.0, 0.0);
        let report = squad.update(0.1, &world, stranger, false, true);
        assert!(!squad.grudge, "a distant stranger already has a grudge");
        let _ = report;

        // One round fired and the truce is over, whatever the standing.
        squad.under_fire(stranger, squad.hatch);
        assert!(squad.grudge, "a volley did not end the conversation");

        // And noise on their ground is a provocation on its own.
        let mut quiet = Garrison::muster(&site);
        quiet.hear(quiet.hatch + DVec3::new(10.0, 0.0, 0.0));
        assert!(quiet.grudge, "a drill on their doorstep kept the truce");
        // While noise past the leash is a rumour, not an offence.
        let mut far = Garrison::muster(&site);
        far.hear(far.hatch + DVec3::new(f64::from(LEASH + 10.0), 0.0, 0.0));
        assert!(!far.grudge, "a distant shot ended a truce it should not");
    }

    #[test]
    fn the_pair_rule_holds_half_the_squad_still() {
        // One window in, only one parity may walk. The rule is the clock,
        // so the test just reads it.
        let site = a_site();
        let mut squad = Garrison::muster(&site);
        let before = squad.movers_even;
        // A full window flips the turn.
        let world = World::new(2024);
        squad.update(OVERWATCH_SECONDS + 0.01, &world, DVec3::ZERO, false, false);
        assert_ne!(squad.movers_even, before, "the overwatch clock never turned");
    }
}
