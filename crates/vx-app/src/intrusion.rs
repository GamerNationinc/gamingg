//! Hacking through machines: the Security line's work, done at arm's length.
//!
//! # Not a minigame
//!
//! Stage 11 set the rule that locks are gated by hard floors, not dice —
//! below the Security floor you cannot start, above it the work takes the
//! time it takes. Moving that work onto a drone changes *who is exposed and
//! where the operator stands*. It does not change the odds, because there
//! are no odds, and that is what keeps an intrusion journallable, replayable
//! and honest on a machine with no mouse to wiggle.
//!
//! # The machine is exposed; the owner is billed
//!
//! Remote intrusion's whole appeal is that you are not standing at the lock,
//! so the witness rule has to say plainly what happens when the drone is
//! seen instead: a witnessed machine marks its owner, exactly as a witnessed
//! hand would. What the coil buys is distance from the *scene*, never from
//! the *consequence*. And an autonomous machine caught at a lock is grabbed
//! — impounded, with a fine at the counter — while one you are personally
//! flying can still be flown away, which is the honest difference between
//! leaving a tool somewhere and holding it.
//!
//! # The tool is a module, the ceiling is the skill
//!
//! A coil says *where* the work can happen: the kestrel's small frame takes
//! the light one, heavier frames take the heavy one. What the work may
//! attempt is capped by the operator's Security level — the same floors
//! stage 11 already enforces. Neither substitutes for the other, so the shop
//! cannot sell what practice has not earned.

use std::io::{Read, Write};
use std::path::Path;

use vx_core::BlockPos;
use vx_world::town::plan::Tier;

use crate::skills;

const MAGIC: &[u8; 4] = b"VXIN";
/// Version 2 adds the watch box's three hack deadlines. A version-1 file
/// still loads, with no hacks standing — see [`Intrusions::save`].
const VERSION: u32 = 2;
const VERSION_WITHOUT_THE_HACKS: u32 = 1;

/// The coil a small airframe takes: the kestrel, and nothing heavier.
pub const LIGHT_COIL: &str = "light coil";
/// The coil a ground drone or a full-size flier carries.
pub const HEAVY_COIL: &str = "heavy coil";
/// The counter-module: raises a machine's own effective lock grade. Nothing
/// hacks *you* until factions land, so this is bought early and needed late
/// — which is the point of teaching the rules with the player's own conduct.
pub const HARDENED_LINK: &str = "hardened link";

/// Every module the garage fits, in the order the shop lists them.
pub const MODULES: [&str; 3] = [LIGHT_COIL, HEAVY_COIL, HARDENED_LINK];

/// What each module costs. Flat, not a rising curve: a module is a thing you
/// own once per fleet, not a machine you accumulate.
pub const MODULE_COST: [u64; MODULES.len()] = [450, 900, 700];

/// Metres from the operator to the machine, beyond which the link drops.
/// Relays widen this later; jammers narrow it later still.
pub const LINK_RANGE: f32 = 120.0;

/// Metres from the machine to what it is working on.
pub const REACH: f32 = 3.5;

/// Bounty for a machine of yours seen working a lock, billed to the name on
/// the garage papers.
pub const BOUNTY_MACHINE: u64 = 25;

/// The flat half of an impound fee; the rest is a tenth of what the machine
/// is worth.
pub const IMPOUND_FLAT: u64 = 40;

/// Seconds of work a lock takes at Security level one, by grade. Slower than
/// talking it open by hand, faster than drilling it out.
pub fn lock_base(tier: Tier) -> f32 {
    match tier {
        Tier::One => 12.0,
        Tier::Two => 30.0,
        Tier::Three => 75.0,
    }
}

/// Which frame a machine has, and therefore which coil it can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// The kestrel: one utility slot, and a small one.
    Light,
    /// A ground drone or a full-size flier.
    Heavy,
}

impl Frame {
    /// The coil this frame mounts.
    pub fn coil(self) -> &'static str {
        match self {
            Frame::Light => LIGHT_COIL,
            Frame::Heavy => HEAVY_COIL,
        }
    }

    /// Whether this frame's coil can reach a grade of lock. The scout stays
    /// a scout: it opens houses and shops, never a bunker.
    pub fn covers(self, tier: Tier) -> bool {
        match self {
            Frame::Light => !matches!(tier, Tier::Three),
            Frame::Heavy => true,
        }
    }
}

/// What can be done to the town's watch box, hardest last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    /// It stands down. Loud in its own way: the town notices a dark box.
    Blind,
    /// It flies its patrols and files nothing. Nobody notices until an
    /// offence it should have witnessed goes strangely unpunished.
    Silence,
    /// It works for the sheriff *and* for you: its sightings mirror to your
    /// handheld. The strongest intelligence in the game, and the view from
    /// the other side of the lens.
    Tap,
}

impl Grade {
    pub const ALL: [Grade; 3] = [Grade::Blind, Grade::Silence, Grade::Tap];

    /// The Security level this grade demands.
    ///
    /// Scaled onto the ladder the locks already use — 1 for a house, 20 for
    /// a shop, 60 for a bunker — rather than a scale of its own: blinding a
    /// box is a low bar, silencing it is journeyman work, and the tap is the
    /// top of the line.
    pub fn min_security(self) -> u32 {
        match self {
            Grade::Blind => 15,
            Grade::Silence => 35,
            Grade::Tap => 70,
        }
    }

    /// Seconds of work at level one.
    pub fn base_seconds(self) -> f32 {
        match self {
            Grade::Blind => 25.0,
            Grade::Silence => 45.0,
            Grade::Tap => 90.0,
        }
    }

    /// Journal ticks the effect holds before routine maintenance finds it.
    /// A blinded box is noticed soonest because it is *obviously* dark; a
    /// tap survives longest because nothing about the box looks wrong.
    pub fn hold_ticks(self) -> u64 {
        match self {
            Grade::Blind => 2_400,   // five minutes: the lockbox rebuild time
            Grade::Silence => 4_800, // ten
            Grade::Tap => 9_600,     // twenty
        }
    }

    /// Experience for pulling it off.
    pub fn xp(self) -> u64 {
        match self {
            Grade::Blind => 400,
            Grade::Silence => 1_600,
            Grade::Tap => 6_000,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Grade::Blind => "BLIND",
            Grade::Silence => "SILENCE",
            Grade::Tap => "TAP",
        }
    }

    /// Only the light coil is barred from the tap: holding a mirrored feed
    /// open takes more transmitter than a palm-sized frame carries.
    fn needs_heavy(self) -> bool {
        matches!(self, Grade::Tap)
    }
}

/// What an intrusion is being run against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A lockbox: success grants the claim, and the lock is left untouched.
    Lock { at: BlockPos, tier: Tier },
    /// The town's watch box.
    Roost { at: BlockPos, grade: Grade },
}

impl Target {
    pub fn at(&self) -> BlockPos {
        match self {
            Target::Lock { at, .. } | Target::Roost { at, .. } => *at,
        }
    }
}

/// Everything one attempt needs to know about itself. Gathered by the
/// caller, judged here, so the rules are one testable function rather than
/// a condition scattered through the frame loop.
#[derive(Debug, Clone, Copy)]
pub struct Attempt {
    pub frame: Frame,
    /// Whether the machine's coil is actually owned and fitted.
    pub fitted: bool,
    pub security: u32,
    /// Metres from the machine to the target.
    pub reach: f32,
    /// Metres from the operator to the machine.
    pub link: f32,
    pub target: Target,
}

/// Why an attempt cannot start (or cannot continue), or `None` if it can.
///
/// Order matters: the reasons are checked from "you brought the wrong gear"
/// through "you are not close enough", so the message names the thing the
/// player can most usefully fix.
pub fn refuse(attempt: &Attempt) -> Option<String> {
    if !attempt.fitted {
        return Some(format!(
            "NO {} FITTED",
            attempt.frame.coil().to_uppercase()
        ));
    }
    match attempt.target {
        Target::Lock { tier, .. } => {
            if !attempt.frame.covers(tier) {
                return Some("THAT LOCK NEEDS A HEAVY COIL".into());
            }
            let needed = crate::permits::min_security(tier);
            if attempt.security < needed {
                return Some(format!("THAT LOCK NEEDS SECURITY {needed}"));
            }
        }
        Target::Roost { grade, .. } => {
            if grade.needs_heavy() && attempt.frame == Frame::Light {
                return Some("A TAP NEEDS A HEAVY COIL".into());
            }
            let needed = grade.min_security();
            if attempt.security < needed {
                return Some(format!("THAT NEEDS SECURITY {needed}"));
            }
        }
    }
    if attempt.reach > REACH {
        return Some("THE MACHINE IS NOT AT IT".into());
    }
    if attempt.link > LINK_RANGE {
        return Some("OUT OF LINK".into());
    }
    None
}

/// How long this attempt takes, in seconds. Levelling buys speed, never
/// certainty — the same curve a hand-picked lock runs on.
pub fn duration(attempt: &Attempt) -> f32 {
    let base = match attempt.target {
        Target::Lock { tier, .. } => lock_base(tier),
        Target::Roost { grade, .. } => grade.base_seconds(),
    };
    skills::bypass_seconds(base, attempt.security)
}

/// A running intrusion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Job {
    pub target: Target,
    /// Seconds of work done.
    pub done: f32,
    /// Seconds the whole job takes, fixed when it started: a level-up
    /// mid-job does not retroactively shorten it, which keeps a replayed
    /// session's timings the same as the live one's.
    pub total: f32,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        if self.total <= 0.0 {
            return 1.0;
        }
        (self.done / self.total).clamp(0.0, 1.0)
    }
}

/// What a tick of work produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Still going, with the fraction done.
    Working(f32),
    /// A lock is open: the grant is yours and the lock is untouched.
    Opened { xp: u64 },
    /// The watch box now answers to you, until `hold` ticks from now.
    Graded { grade: Grade, hold: u64, xp: u64 },
    /// Stopped, with the reason.
    Refused(String),
}

/// The player's intrusion kit and whatever it is doing.
#[derive(Debug, Default)]
pub struct Intrusions {
    pub job: Option<Job>,
    /// A fee owed to get an impounded machine back. `None` means nothing of
    /// yours is in the pound.
    pub impounded: Option<u64>,
    /// Where your own watch box stands, once bought and mounted. It keeps
    /// house here rather than in `homestead.dat` so that owning one costs
    /// nobody an old save: this file is new, and a new file needs no
    /// migration.
    pub roost_at: Option<BlockPos>,
    /// The town watch box's three hack deadlines, carried across a save.
    ///
    /// They belong to the roost, and they live here because the spoofer
    /// charge that bought them lives here. See [`Intrusions::save`].
    pub hacks: (u64, u64, u64),
}

impl Intrusions {
    /// Start a job, replacing whatever was running. Refusals are checked by
    /// the caller through [`refuse`] first, so this is the accepting half.
    pub fn begin(&mut self, attempt: &Attempt) {
        self.job = Some(Job {
            target: attempt.target,
            done: 0.0,
            total: duration(attempt),
        });
    }

    pub fn abort(&mut self) {
        self.job = None;
    }

    /// Work for `dt` seconds against the conditions as they are *now*.
    ///
    /// Re-judging every tick is deliberate: fly the machine out of reach, or
    /// walk out of link, and the job stops where it stands. An intrusion is
    /// a planned act you have to stay committed to, not fire-and-forget.
    pub fn work(&mut self, attempt: &Attempt, dt: f32) -> Progress {
        if let Some(reason) = refuse(attempt) {
            self.job = None;
            return Progress::Refused(reason);
        }
        let Some(job) = &mut self.job else {
            return Progress::Refused("NOTHING TO WORK".into());
        };
        if job.target != attempt.target {
            // The machine drifted onto something else; that is a new job.
            self.job = None;
            return Progress::Refused("TARGET CHANGED".into());
        }
        job.done += dt;
        if job.done < job.total {
            return Progress::Working(job.fraction());
        }
        let finished = job.target;
        self.job = None;
        match finished {
            Target::Lock { tier, .. } => Progress::Opened {
                xp: skills::BYPASS_XP[match tier {
                    Tier::One => 0,
                    Tier::Two => 1,
                    Tier::Three => 2,
                }],
            },
            Target::Roost { grade, .. } => Progress::Graded {
                grade,
                hold: grade.hold_ticks(),
                xp: grade.xp(),
            },
        }
    }

    /// A machine of yours was seized at a lock. The job dies with it.
    pub fn impound(&mut self, machine_value: u64) {
        self.job = None;
        let fee = IMPOUND_FLAT + machine_value / 10;
        self.impounded = Some(self.impounded.unwrap_or(0) + fee);
    }

    /// Settle the fee. Returns what was owed, so the caller can charge it.
    pub fn release(&mut self) -> Option<u64> {
        self.impounded.take()
    }

    /// Write the kit down.
    ///
    /// From stage 54 that includes the watch box's three hack deadlines. They
    /// are the roost's numbers, not the kit's, but they belong in this file:
    /// the *charge* that bought them has been saved here since stage 15, and
    /// splitting a purchase from what it purchased across two files is how a
    /// player came to pay a coil and get nothing for it.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "intrusion.dat")?;
        self.write_kit(&mut file)?;
        file.commit()
    }

    fn write_kit(&self, file: &mut impl Write) -> std::io::Result<()> {
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        // A job in flight is not saved: it is seconds of standing still, and
        // resuming one across a reload would be resuming a scene that is no
        // longer there. The impound is a debt, and debts persist.
        file.write_all(&self.impounded.unwrap_or(0).to_le_bytes())?;
        match self.roost_at {
            Some(at) => {
                file.write_all(&[1u8])?;
                file.write_all(&at.x.to_le_bytes())?;
                file.write_all(&at.y.to_le_bytes())?;
                file.write_all(&at.z.to_le_bytes())?;
            }
            None => file.write_all(&[0u8; 13])?,
        }
        let (blinded, silenced, tapped) = self.hacks;
        file.write_all(&blinded.to_le_bytes())?;
        file.write_all(&silenced.to_le_bytes())?;
        file.write_all(&tapped.to_le_bytes())?;
        Ok(())
    }

    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("intrusion.dat");
        match read_intrusions(&path) {
            Ok(Some((owed, roost_at, hacks))) => {
                self.impounded = (owed > 0).then_some(owed);
                self.roost_at = roost_at;
                self.hacks = hacks;
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting fresh", path.display());
                *self = Intrusions::default();
            }
        }
    }
}

type Saved = (u64, Option<BlockPos>, (u64, u64, u64));

fn read_intrusions(path: &Path) -> std::io::Result<Option<Saved>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("bad magic"));
    }
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    let version = u32::from_le_bytes(word);
    if version != VERSION && version != VERSION_WITHOUT_THE_HACKS {
        return Err(std::io::Error::other("unknown version"));
    }
    let mut owed = [0u8; 8];
    file.read_exact(&mut owed)?;
    let mut flag = [0u8; 1];
    file.read_exact(&mut flag)?;
    let mut at = [0u8; 12];
    file.read_exact(&mut at)?;
    let roost_at = (flag[0] != 0).then(|| {
        let word = |n: usize| i32::from_le_bytes([at[n], at[n + 1], at[n + 2], at[n + 3]]);
        BlockPos::new(word(0), word(4), word(8))
    });
    // A version-1 file simply has no hacks in it, and none standing is the
    // honest reading: it was written by a build that could not keep them.
    let mut hacks = (0u64, 0u64, 0u64);
    if version == VERSION {
        let mut stamp = [0u8; 8];
        file.read_exact(&mut stamp)?;
        hacks.0 = u64::from_le_bytes(stamp);
        file.read_exact(&mut stamp)?;
        hacks.1 = u64::from_le_bytes(stamp);
        file.read_exact(&mut stamp)?;
        hacks.2 = u64::from_le_bytes(stamp);
    }
    Ok(Some((u64::from_le_bytes(owed), roost_at, hacks)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> BlockPos {
        BlockPos::new(10, 74, 4)
    }

    fn lock_attempt(frame: Frame, tier: Tier, security: u32) -> Attempt {
        Attempt {
            frame,
            fitted: true,
            security,
            reach: 1.0,
            link: 10.0,
            target: Target::Lock { at: at(), tier },
        }
    }

    #[test]
    fn the_gear_and_the_skill_are_both_required_and_neither_substitutes() {
        // No coil: refused however good you are.
        let mut bare = lock_attempt(Frame::Light, Tier::One, 99);
        bare.fitted = false;
        assert!(refuse(&bare).is_some(), "hacked with no coil fitted");

        // Coil but no level: still refused.
        let green = lock_attempt(Frame::Heavy, Tier::Two, 5);
        let reason = refuse(&green).expect("a green operator opened a shop lock");
        assert!(reason.contains("SECURITY"), "wrong refusal: {reason}");

        // Level but the wrong frame: the scout stays a scout.
        let scout = lock_attempt(Frame::Light, Tier::Three, 99);
        assert!(refuse(&scout).is_some(), "the kestrel opened a bunker");
        assert!(refuse(&lock_attempt(Frame::Heavy, Tier::Three, 99)).is_none());

        // Everything in hand: allowed.
        assert!(refuse(&lock_attempt(Frame::Light, Tier::One, 1)).is_none());
    }

    #[test]
    fn the_leash_holds() {
        let mut far = lock_attempt(Frame::Light, Tier::One, 40);
        far.link = LINK_RANGE + 1.0;
        assert_eq!(refuse(&far).as_deref(), Some("OUT OF LINK"));

        let mut adrift = lock_attempt(Frame::Light, Tier::One, 40);
        adrift.reach = REACH + 0.5;
        assert_eq!(refuse(&adrift).as_deref(), Some("THE MACHINE IS NOT AT IT"));
    }

    #[test]
    fn a_job_runs_to_completion_and_pays_out() {
        let attempt = lock_attempt(Frame::Heavy, Tier::Two, 20);
        let mut kit = Intrusions::default();
        kit.begin(&attempt);
        let total = duration(&attempt);
        assert!(total > 0.0);

        // Most of the way: still working.
        let progress = kit.work(&attempt, total - 0.5);
        assert!(matches!(progress, Progress::Working(_)), "{progress:?}");

        match kit.work(&attempt, 1.0) {
            Progress::Opened { xp } => assert_eq!(xp, skills::BYPASS_XP[1]),
            other => panic!("the lock never opened: {other:?}"),
        }
        assert!(kit.job.is_none(), "a finished job is still running");
    }

    #[test]
    fn walking_out_of_link_stops_the_work_where_it_stands() {
        let attempt = lock_attempt(Frame::Heavy, Tier::Two, 30);
        let mut kit = Intrusions::default();
        kit.begin(&attempt);
        kit.work(&attempt, 1.0);

        let mut gone = attempt;
        gone.link = LINK_RANGE + 10.0;
        assert!(matches!(kit.work(&gone, 1.0), Progress::Refused(_)));
        assert!(kit.job.is_none(), "the job survived the link dropping");
    }

    #[test]
    fn levelling_buys_speed_and_only_speed() {
        let green = duration(&lock_attempt(Frame::Heavy, Tier::Two, 20));
        let sharp = duration(&lock_attempt(Frame::Heavy, Tier::Two, 80));
        assert!(sharp < green, "levelling bought nothing: {sharp} vs {green}");
        // And it never reaches zero — there is no instant hack.
        assert!(sharp > 1.0, "a hack became free: {sharp}");
    }

    #[test]
    fn the_roost_grades_climb_in_bar_time_and_reward() {
        let mut last = (0, 0.0, 0);
        for grade in Grade::ALL {
            let bar = grade.min_security();
            let seconds = grade.base_seconds();
            let xp = grade.xp();
            assert!(bar > last.0, "{} is no harder to reach", grade.label());
            assert!(seconds > last.1, "{} is no slower", grade.label());
            assert!(xp > last.2, "{} pays no better", grade.label());
            last = (bar, seconds, xp);
        }
        // The tap is the one thing a palm-sized frame cannot hold open.
        let tapping = Attempt {
            frame: Frame::Light,
            fitted: true,
            security: 99,
            reach: 1.0,
            link: 1.0,
            target: Target::Roost {
                at: at(),
                grade: Grade::Tap,
            },
        };
        assert!(refuse(&tapping).is_some(), "the kestrel tapped a roost");
    }

    #[test]
    fn an_impound_is_a_debt_that_survives_the_trip_to_disk() {
        let directory =
            std::env::temp_dir().join(format!("gamingg-intrusion-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");

        let mut kit = Intrusions::default();
        kit.begin(&lock_attempt(Frame::Light, Tier::One, 40));
        kit.impound(300);
        assert!(kit.job.is_none(), "a seized machine kept working");
        assert_eq!(kit.impounded, Some(IMPOUND_FLAT + 30));
        kit.save(&directory).expect("save");

        let mut back = Intrusions::default();
        back.load(&directory);
        assert_eq!(back.impounded, Some(IMPOUND_FLAT + 30));
        assert_eq!(back.release(), Some(IMPOUND_FLAT + 30));
        assert_eq!(back.release(), None, "paid the same fee twice");

        let _ = std::fs::remove_dir_all(&directory);
    }
}
