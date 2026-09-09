//! Who may edit what, who is watching, and what it costs when they are.
//!
//! # The ladder
//!
//! Three ways past a locked door, priced so the honest one is the best one:
//!
//! 1. **Authenticate.** Free, permanent, no risk. Trust earned through trade
//!    (stage 12) is what unlocks it; this round only the owner qualifies.
//! 2. **Hack.** Fast, needs the [`crate::skills::SECURITY`] line, and leaves
//!    the lock standing so nobody need ever know — if nobody sees you.
//! 3. **Break the lock.** Slow, needs a serious drill, throws the whole
//!    building open until the town puts a new box up, and is the loudest thing
//!    you can do.
//!
//! Every number in this file exists to keep that order true.
//!
//! # Claims are derived, never stored
//!
//! A town's buildings are a pure function of its site (`vx_world::town::plan`),
//! so a claim is too. What is *stored* is only what cannot be derived: who has
//! been let in, who holds office, what the player has been caught doing, and
//! how far through a lock they have drilled.
//!
//! # Why the gate lives on the event bus
//!
//! `break_block` and `place_block` have emitted cancellable events since stage
//! 2.x with nothing listening. Subscribing here means every caller is covered —
//! the player's drill, a drone's cutter, anything added later — without one
//! call site changing. Replay is exempt for free: it is handed a fresh
//! `EventBus`, so the oracle never sees the gate.

use std::io::{Read, Write};
use std::path::Path;

use vx_core::{BlockPos, Cancellable};
use vx_world::town::{self, plan};
use vx_world::{Role, Tier, TownSite};

const MAGIC: &[u8; 4] = b"VXPM";
/// Two makes an office belong to a town. Version one held a bare list of
/// offices with no town attached, which meant one badge opened every lock on
/// the frontier — read as the hometown's, which is where the `--sheriff`
/// override was always pointed.
const VERSION: u32 = 2;
const OLDEST: u32 = 1;

/// Bounty for being seen prying at something that is not yours.
pub const BOUNTY_PRYING: u64 = 5;
/// Bounty for being seen picking a lock.
pub const BOUNTY_HACK: u64 = 25;
/// Bounty for being seen destroying one.
pub const BOUNTY_BREACH: u64 = 60;
/// Bounty for being seen taking a bank's strongroom door off its hinges.
///
/// The maximum on the sheet, and by a long way. Every other crime in this
/// game is against one person or one machine; a bank holds what a whole town
/// left with it, so smashing into one is the only act here that is against
/// everybody at once. It is set well past [`WARRANT_THRESHOLD`] on purpose:
/// one vault, once, and the warrant chain is looking at you before you have
/// carried anything out of the building.
pub const BOUNTY_VAULT: u64 = 500;
/// Where stage 12's warrant chain starts paying attention.
pub const WARRANT_THRESHOLD: u64 = 100;
/// What seizing a town's chairs adds to the sheet: a vault's worth, so the
/// warrant chain in every neighbouring town is looking at you by the next
/// civic tick. The hostile route into office is priced by the bounty system
/// itself, which is what the civic note asked for.
pub const TAKEN_BOUNTY: u64 = BOUNTY_VAULT;

/// What breaching a lock costs, by the grade of what was breached.
///
/// The tier is already on the claim, and Tier Three is stamped on exactly one
/// building in a town — so "was this a bank?" needs nothing looked up. Note
/// that *picking* a bank's lock still costs the quiet price: that is the
/// whole shape of the permits round, and Security 60 is its own toll.
pub fn breach_bounty(tier: Option<Tier>) -> u64 {
    match tier {
        Some(Tier::Three) => BOUNTY_VAULT,
        _ => BOUNTY_BREACH,
    }
}

/// The lowest Security level that can attempt each grade of lock.
///
/// A hard floor, like the drill's power gate: below it you cannot start at all,
/// and the panel says so rather than letting you waste a minute finding out.
pub fn min_security(tier: Tier) -> u32 {
    match tier {
        Tier::One => 1,
        Tier::Two => 20,
        Tier::Three => 60,
    }
}

/// Seconds a bypass takes at level one, by grade.
pub fn bypass_base(tier: Tier) -> f32 {
    match tier {
        Tier::One => 8.0,
        Tier::Two => 20.0,
        Tier::Three => 45.0,
    }
}

/// The drill power each grade of lock demands before it will yield at all.
///
/// A hard gate, not merely a long time. "Impossible for a new player" cannot be
/// said in seconds — drilling is `hardness / power`, so any hardness eventually
/// gives — and a lock that a starter drill can open in twenty patient minutes
/// is not a locked door, it is a slow one.
///
/// Drill power runs 1.25 fresh to 13.84 at Mining 99 with the drill upgrade
/// maxed, so grade II wants real levels and grade III wants most of them.
pub fn min_power(tier: Tier) -> f32 {
    match tier {
        Tier::One => 1.0,
        Tier::Two => 4.0,
        Tier::Three => 10.0,
    }
}

/// How long the town takes to put a broken lock back up.
///
/// Long enough that a breach buys a real window, short enough that the town
/// does not stay robbed forever. Journal ticks — the same clock mining runs on.
pub const REBUILD_TICKS: u64 = 2_400;

/// An office somebody holds.
///
/// Ordered so a badge can live in a set keyed by `(town, office)` — the
/// ordering itself means nothing beyond giving the set a canonical walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Office {
    /// Owns the town's open ground, its paving and its civic buildings.
    Mayor,
    /// Owns the security office — and may open anything at all.
    Sheriff,
}

/// Who holds a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claimant {
    Player,
    /// A villager, by roster index.
    Resident(usize),
    Office(Office),
}

/// Where the player stands with respect to one claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// It is yours.
    Owner,
    /// You wear the badge, so it may as well be.
    Sheriff,
    /// You have been let in, or you let yourself in.
    Guest,
    /// You have no business here.
    Stranger,
}

/// A piece of ground somebody answers for.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub owner: Claimant,
    /// The building's bounds, or `None` for the town's open ground.
    pub bounds: Option<(BlockPos, BlockPos)>,
    pub tier: Option<Tier>,
    /// Where this claim's lockbox stands. A claim whose lock is down is
    /// dormant: that is what a breach *buys*, and without it destroying a box
    /// would be a loud way to achieve nothing.
    pub lock: Option<BlockPos>,
    pub label: String,
    /// A stable key for the claim, for grants and persistence: the town centre
    /// and the building's minimum corner. Derived, so it survives a reload
    /// without anything about the claim being written down.
    pub key: ClaimKey,
}

/// Identifies a claim across sessions without storing the claim itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClaimKey {
    pub town: (i32, i32),
    /// The building's minimum corner, or `None` for the town's open ground.
    pub building: Option<(i32, i32, i32)>,
}

/// Why an edit was refused.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub label: String,
    pub owner: Claimant,
}

impl Refusal {
    /// The line the player sees.
    pub fn line(&self) -> String {
        format!("{} IS NOT YOURS", self.label.to_uppercase())
    }
}

/// The roster index that holds each office, and the buildings residents own.
///
/// Villagers are identified by roster index and nothing else — there is no
/// name field anywhere — so the offices are indices too. The mapping from a
/// dwelling to the resident who sleeps in it comes from the roster's home
/// routes, which end inside a specific container.
const MAYOR: usize = 0;
const SHERIFF: usize = 2;
/// How many people live in a town, from the villager roster.
const RESIDENTS: usize = 3;

/// The office a resident holds, if any.
pub fn office_of(resident: usize) -> Option<Office> {
    match resident {
        MAYOR => Some(Office::Mayor),
        SHERIFF => Some(Office::Sheriff),
        _ => None,
    }
}

/// What a resident is called on a panel.
///
/// Derived from the office they hold rather than kept as a parallel list, so
/// moving the badge to a different villager cannot leave the two disagreeing.
fn resident_name(index: usize) -> &'static str {
    match office_of(index) {
        Some(Office::Mayor) => "THE MAYOR",
        Some(Office::Sheriff) => "THE SHERIFF",
        None => "OLD PRAT",
    }
}

/// Everything the town remembers about you.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Permits {
    /// Claims you have been let into — or picked your way into.
    grants: Vec<ClaimKey>,
    /// Offices the player holds, **and the town each one is for**.
    ///
    /// A badge belongs to a place. Until stage 40 this was a bare list, so a
    /// sheriff was the sheriff of everywhere — harmless while only a dev flag
    /// could set it, and exploitable the moment one could be won at a ballot
    /// box in the smallest town on the map.
    offices: std::collections::BTreeSet<((i32, i32), Office)>,
    /// What you have been caught doing.
    pub bounty: u64,
    /// How far through each lock you have drilled, 0..1. Persists across
    /// looking away, unlike ordinary drilling: a breach is a project.
    breach: Vec<(BlockPos, f32)>,
    /// Locks you have broken, and the tick they went down.
    broken: Vec<(BlockPos, u64)>,
    /// The last refusal, with the block it was about, so a stale message
    /// cannot be mistaken for a fresh one. Bedrock returns the same error
    /// *without* emitting an event, which is exactly how that would happen.
    last_refusal: Option<(BlockPos, Refusal)>,
    /// Towns whose law currently applies, refreshed as the player moves.
    ///
    /// Lives here rather than being captured by the gate's closure because the
    /// player walks: a fixed set taken at startup would enforce spawn's town
    /// law on the far side of the map and none at all on the near side.
    /// Derived every frame from the lattice, never saved.
    sites: Vec<TownSite>,
}

impl Permits {
    pub fn new() -> Self {
        Permits::default()
    }

    // -- offices and grants -------------------------------------------------

    /// The towns whose law is in force around the player.
    pub fn set_sites(&mut self, sites: Vec<TownSite>) {
        self.sites = sites;
    }

    /// The claim covering a block, under the law currently in force.
    pub fn claim_here(&self, at: BlockPos) -> Option<Claim> {
        claim_at(&self.sites, at)
    }

    /// The town whose ground `at` stands on, if any — the whole site, so a
    /// caller can speak its name.
    pub fn town_here(&self, at: BlockPos) -> Option<&TownSite> {
        self.sites
            .iter()
            .find(|site| town::footprint_contains(std::slice::from_ref(site), at.x, at.z))
    }

    /// A claim inside this box that the player may not edit, if any.
    ///
    /// The planner asks before it dispatches: a drone sent into somebody's
    /// wall would be refused block by block and stall halfway through the job,
    /// which is a worse way to learn the rule than being told at the outset.
    pub fn blocked_span(&self, min: BlockPos, max: BlockPos) -> Option<Claim> {
        for site in &self.sites {
            for claim in claims_for(site) {
                let Some((low, high)) = claim.bounds else {
                    continue;
                };
                let overlaps = min.x <= high.x
                    && max.x >= low.x
                    && min.y <= high.y
                    && max.y >= low.y
                    && min.z <= high.z
                    && max.z >= low.z;
                if overlaps && !self.may_edit(&claim) {
                    return Some(claim);
                }
            }
        }
        None
    }

    /// Does the player hold this office *here*?
    pub fn holds_in(&self, town: (i32, i32), office: Office) -> bool {
        self.offices.contains(&(town, office))
    }

    /// Every badge the player wears, for the panels and the roster.
    pub fn badges(&self) -> impl Iterator<Item = ((i32, i32), Office)> + '_ {
        self.offices.iter().copied()
    }

    /// Put a badge on the player.
    ///
    /// Stage 13's ballot box is what will call this in earnest; until then the
    /// `--sheriff` development flag does, which is how the override gets
    /// exercised in a real session rather than only in tests.
    pub fn take_office(&mut self, town: (i32, i32), office: Office) {
        self.offices.insert((town, office));
    }

    /// Replace the whole set with what the register says, which is the one
    /// place a badge is ever awarded: an election.
    pub fn seat_all(&mut self, held: impl Iterator<Item = ((i32, i32), Office)>) {
        self.offices = held.collect();
    }

    pub fn grant(&mut self, key: ClaimKey) {
        if !self.grants.contains(&key) {
            self.grants.push(key);
        }
    }

    pub fn granted(&self, key: ClaimKey) -> bool {
        self.grants.contains(&key)
    }

    // -- standing -----------------------------------------------------------

    /// Where the player stands with one claim.
    ///
    /// The sheriff opens everything: enforcement has to reach where bounties
    /// hide, and a lock the law cannot pass is a lock that makes crime safe.
    /// The mayor is *not* an override — they own the streets and the civic
    /// buildings, which is status, not a skeleton key.
    pub fn standing(&self, claim: &Claim) -> Standing {
        if claim.owner == Claimant::Player {
            return Standing::Owner;
        }
        // A badge opens the town that gave it, and nowhere else. The claim
        // carries the town it belongs to, so this needed no new lookup — only
        // for the question to stop being asked without one.
        let town = claim.key.town;
        if self.holds_in(town, Office::Sheriff) {
            return Standing::Sheriff;
        }
        if claim.owner == Claimant::Office(Office::Mayor) && self.holds_in(town, Office::Mayor) {
            return Standing::Owner;
        }
        if self.granted(claim.key) {
            return Standing::Guest;
        }
        Standing::Stranger
    }

    /// Is this claim's lock currently down?
    pub fn is_open(&self, claim: &Claim) -> bool {
        claim.lock.is_some_and(|lock| self.is_broken(lock))
    }

    pub fn may_edit(&self, claim: &Claim) -> bool {
        self.is_open(claim) || !matches!(self.standing(claim), Standing::Stranger)
    }

    // -- refusals -----------------------------------------------------------

    pub fn refuse(&mut self, at: BlockPos, refusal: Refusal) {
        self.last_refusal = Some((at, refusal));
    }

    /// The refusal for this block, if the last one was about it.
    pub fn refusal_for(&self, at: BlockPos) -> Option<&Refusal> {
        self.last_refusal
            .as_ref()
            .filter(|(position, _)| *position == at)
            .map(|(_, refusal)| refusal)
    }

    // -- bounty -------------------------------------------------------------

    /// Charge the player, but only if somebody saw.
    pub fn caught(&mut self, points: u64, witnesses: usize) -> bool {
        if witnesses == 0 {
            return false;
        }
        self.bounty += points;
        true
    }

    /// Charge the player with no witness test: for crimes with their own
    /// paper trail. A villager who reached the security office *is* the
    /// witness; a caravan that never arrived is on the shipping manifest.
    pub fn billed(&mut self, points: u64) {
        self.bounty += points;
    }

    /// Far enough past the threshold for the sheriff to take it to the
    /// mayor.
    ///
    /// **Not** "deputies are coming" any more. Stage 39 put a chain between
    /// the two: this is the sheriff's interest, and
    /// [`crate::warrant::Docket::granted_in`] is the signature that actually
    /// sends anybody. The HUD tints on this because the bounty being over
    /// the line is the thing a player needs to see; what the mayor does with
    /// it is on the beacon panel and the `TOWN` verb.
    pub fn wanted(&self) -> bool {
        self.bounty >= WARRANT_THRESHOLD
    }

    // -- breaching ----------------------------------------------------------

    pub fn breach_progress(&self, at: BlockPos) -> f32 {
        self.breach
            .iter()
            .find(|(position, _)| *position == at)
            .map_or(0.0, |(_, progress)| *progress)
    }

    pub fn set_breach(&mut self, at: BlockPos, progress: f32) {
        match self.breach.iter_mut().find(|(position, _)| *position == at) {
            Some(slot) => slot.1 = progress,
            None => self.breach.push((at, progress)),
        }
    }

    pub fn clear_breach(&mut self, at: BlockPos) {
        self.breach.retain(|(position, _)| *position != at);
    }

    /// A lock went down. The claim it held sleeps until the town rebuilds.
    pub fn broke(&mut self, at: BlockPos, now: u64) {
        self.clear_breach(at);
        if !self.broken.iter().any(|(position, _)| *position == at) {
            self.broken.push((at, now));
        }
    }

    pub fn is_broken(&self, at: BlockPos) -> bool {
        self.broken.iter().any(|(position, _)| *position == at)
    }

    /// The grade of a lock at a position, from the plan.
    pub fn lock_tier_at(&self, at: BlockPos) -> Option<Tier> {
        self.sites
            .iter()
            .flat_map(plan::lockboxes)
            .find(|(position, _)| *position == at)
            .map(|(_, tier)| tier)
    }

    /// Locks whose rebuild is due at `now`, cleared as they are reported.
    ///
    /// The caller puts the block back and records it, so a replay puts it back
    /// at the same tick.
    pub fn due_rebuilds(&mut self, now: u64) -> Vec<BlockPos> {
        let due: Vec<BlockPos> = self
            .broken
            .iter()
            .filter(|(_, when)| now.saturating_sub(*when) >= REBUILD_TICKS)
            .map(|(at, _)| *at)
            .collect();
        self.broken
            .retain(|(_, when)| now.saturating_sub(*when) < REBUILD_TICKS);
        due
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "permits.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;

        file.write_all(&(self.grants.len() as u32).to_le_bytes())?;
        for key in &self.grants {
            write_key(&mut file, *key)?;
        }
        file.write_all(&(self.offices.len() as u32).to_le_bytes())?;
        for ((x, z), office) in &self.offices {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&[match office {
                Office::Mayor => 0,
                Office::Sheriff => 1,
            }])?;
        }
        file.write_all(&self.bounty.to_le_bytes())?;

        file.write_all(&(self.breach.len() as u32).to_le_bytes())?;
        for (at, progress) in &self.breach {
            write_pos(&mut file, *at)?;
            file.write_all(&progress.to_le_bytes())?;
        }
        file.write_all(&(self.broken.len() as u32).to_le_bytes())?;
        for (at, when) in &self.broken {
            write_pos(&mut file, *at)?;
            file.write_all(&when.to_le_bytes())?;
        }
        file.commit()
    }

    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("permits.dat");
        match read_permits(&path) {
            Ok(Some(read)) => *self = read,
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting fresh", path.display());
                *self = Permits::new();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Deriving claims from the world
// ---------------------------------------------------------------------------

/// What a building's role means for who answers for it.
fn owner_of(role: Role) -> Claimant {
    match role {
        Role::PlayerHouse => Claimant::Player,
        Role::Security => Claimant::Office(Office::Sheriff),
        // The bank answers to the mayor like the rest of the town's own
        // buildings. Whose *goods* are inside is a separate ledger, and not
        // one the permits system has any business in.
        Role::Shop | Role::Civic | Role::Paving | Role::Bank | Role::Clinic => {
            Claimant::Office(Office::Mayor)
        }
        // Filled in per building below: which resident depends on which
        // container, and the roster's home routes are what say so.
        Role::Dwelling => Claimant::Resident(usize::MAX),
    }
}

/// The resident whose home route ends inside this building, if any.
fn resident_of(site: &TownSite, min: BlockPos) -> Option<usize> {
    const BEDS: [(usize, (f32, f32)); 3] = [
        (0, (-14.0, -0.5)),
        (1, (-1.0, -21.0)),
        (2, (13.0, -0.5)),
    ];
    BEDS.iter()
        .find(|(_, (x, z))| {
            let bed_x = site.centre.0 + *x as i32;
            let bed_z = site.centre.1 + *z as i32;
            (bed_x - min.x).abs() <= 8 && (bed_z - min.z).abs() <= 8
        })
        .map(|(index, _)| *index)
}

fn label_for(role: Role, owner: Claimant) -> String {
    match (role, owner) {
        (Role::PlayerHouse, _) => "YOUR HOUSE".into(),
        (Role::Shop, _) => "THE SUPPLY SHED".into(),
        (Role::Security, _) => "THE SECURITY OFFICE".into(),
        (Role::Civic, _) => "THE RADIO TOWER".into(),
        (Role::Paving, _) => "THE TOWN PAVING".into(),
        (Role::Bank, _) => "THE BANK".into(),
        (Role::Clinic, _) => "THE CLINIC".into(),
        (Role::Dwelling, Claimant::Resident(index)) if index < RESIDENTS => {
            format!("{}S HOUSE", resident_name(index))
        }
        (Role::Dwelling, _) => "SOMEBODYS HOUSE".into(),
    }
}

/// Every claim a town carries: one per building, plus the open ground.
pub fn claims_for(site: &TownSite) -> Vec<Claim> {
    let locks = plan::lockboxes(site);
    let mut claims: Vec<Claim> = plan::buildings(site)
        .into_iter()
        .map(|building| {
            let owner = match owner_of(building.role) {
                Claimant::Resident(_) => Claimant::Resident(
                    resident_of(site, building.min).unwrap_or(usize::MAX),
                ),
                other => other,
            };
            let lock = locks
                .iter()
                .find(|(at, _)| {
                    at.x >= building.min.x
                        && at.x <= building.max.x
                        && at.z >= building.min.z
                        && at.z <= building.max.z
                })
                .map(|(at, _)| *at);
            Claim {
                owner,
                bounds: Some((building.min, building.max)),
                tier: building.role.tier(),
                lock,
                label: label_for(building.role, owner),
                key: ClaimKey {
                    town: site.centre,
                    building: Some((building.min.x, building.min.y, building.min.z)),
                },
            }
        })
        .collect();

    // The streets. Everything inside the town line that no building already
    // answers for is the mayor's, which is why you cannot set up shop on the
    // plaza and call it yours.
    claims.push(Claim {
        owner: Claimant::Office(Office::Mayor),
        bounds: None,
        tier: None,
        // The streets have no lock, so they can never be forced — only earned.
        lock: None,
        label: "THE TOWN".into(),
        key: ClaimKey {
            town: site.centre,
            building: None,
        },
    });
    claims
}

/// The claim covering a block, if any. Buildings win over open ground.
pub fn claim_at(sites: &[TownSite], at: BlockPos) -> Option<Claim> {
    let mut open: Option<Claim> = None;
    for site in sites {
        for claim in claims_for(site) {
            match claim.bounds {
                Some((min, max)) => {
                    if at.x >= min.x
                        && at.x <= max.x
                        && at.y >= min.y
                        && at.y <= max.y
                        && at.z >= min.z
                        && at.z <= max.z
                    {
                        return Some(claim);
                    }
                }
                None => {
                    if open.is_none() && town::footprint_contains(sites, at.x, at.z) {
                        open = Some(claim);
                    }
                }
            }
        }
    }
    open
}

// ---------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------

fn write_pos(file: &mut impl Write, at: BlockPos) -> std::io::Result<()> {
    file.write_all(&at.x.to_le_bytes())?;
    file.write_all(&at.y.to_le_bytes())?;
    file.write_all(&at.z.to_le_bytes())
}

fn write_key(file: &mut impl Write, key: ClaimKey) -> std::io::Result<()> {
    file.write_all(&key.town.0.to_le_bytes())?;
    file.write_all(&key.town.1.to_le_bytes())?;
    match key.building {
        Some((x, y, z)) => {
            file.write_all(&[1u8])?;
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&y.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())
        }
        None => {
            file.write_all(&[0u8])?;
            file.write_all(&[0u8; 12])
        }
    }
}

fn read_i32(file: &mut impl Read) -> std::io::Result<i32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(i32::from_le_bytes(word))
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    Ok(read_i32(file)? as u32)
}

fn read_u64(file: &mut impl Read) -> std::io::Result<u64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

fn read_pos(file: &mut impl Read) -> std::io::Result<BlockPos> {
    Ok(BlockPos::new(
        read_i32(file)?,
        read_i32(file)?,
        read_i32(file)?,
    ))
}

fn read_key(file: &mut impl Read) -> std::io::Result<ClaimKey> {
    let town = (read_i32(file)?, read_i32(file)?);
    let mut flag = [0u8; 1];
    file.read_exact(&mut flag)?;
    let mut body = [0u8; 12];
    file.read_exact(&mut body)?;
    let building = (flag[0] == 1).then(|| {
        let word = |i: usize| i32::from_le_bytes(body[i * 4..i * 4 + 4].try_into().unwrap());
        (word(0), word(1), word(2))
    });
    Ok(ClaimKey { town, building })
}

fn read_permits(path: &Path) -> std::io::Result<Option<Permits>> {
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
    let version = read_u32(&mut file)?;
    // Anything from `OLDEST` up reads with this loader; the only thing that
    // changed is whether a badge carries the town it belongs to.
    if !(OLDEST..=VERSION).contains(&version) {
        return Err(std::io::Error::other("unknown version"));
    }

    let bounded = |count: u32| -> std::io::Result<u32> {
        if count > 8_192 {
            return Err(std::io::Error::other("implausible count"));
        }
        Ok(count)
    };

    let mut permits = Permits::new();
    for _ in 0..bounded(read_u32(&mut file)?)? {
        permits.grants.push(read_key(&mut file)?);
    }
    for _ in 0..bounded(read_u32(&mut file)?)? {
        // Version one had no town on a badge. Reading it as the hometown's is
        // the honest migration: that is the only town the override could
        // meaningfully have been aimed at, and it is strictly less reach than
        // the file was claiming.
        let town = if version == OLDEST {
            (0, 0)
        } else {
            let mut word = [0u8; 4];
            file.read_exact(&mut word)?;
            let x = i32::from_le_bytes(word);
            file.read_exact(&mut word)?;
            (x, i32::from_le_bytes(word))
        };
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte)?;
        permits.offices.insert((
            town,
            match byte[0] {
                0 => Office::Mayor,
                1 => Office::Sheriff,
                other => return Err(std::io::Error::other(format!("unknown office {other}"))),
            },
        ));
    }
    permits.bounty = read_u64(&mut file)?;
    for _ in 0..bounded(read_u32(&mut file)?)? {
        let at = read_pos(&mut file)?;
        let mut word = [0u8; 4];
        file.read_exact(&mut word)?;
        permits.breach.push((at, f32::from_le_bytes(word)));
    }
    for _ in 0..bounded(read_u32(&mut file)?)? {
        let at = read_pos(&mut file)?;
        permits.broken.push((at, read_u64(&mut file)?));
    }
    Ok(Some(permits))
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// A shared handle to the permits, because the bus demands one.
///
/// `EventBus` handlers are `Fn` and `emit` takes `&self`, so a handler that
/// accumulates state needs interior mutability — the pattern the bus's own
/// module doc names. One `Rc` lives in the closure, one in the app.
pub type Shared = std::rc::Rc<std::cell::RefCell<Permits>>;

/// Which grade of lock this block is, if it is one.
pub fn tier_of(registry: &vx_core::BlockRegistry, block: vx_core::BlockId) -> Option<Tier> {
    match registry.get_or_air(block).name.as_str() {
        "engine:permit_box_i" => Some(Tier::One),
        "engine:permit_box_ii" => Some(Tier::Two),
        "engine:permit_box_iii" => Some(Tier::Three),
        _ => None,
    }
}

/// Should this edit be refused? Records the reason if so.
///
/// Shared by both event handlers so break and place cannot drift apart.
fn judge(
    permits: &Shared,
    at: BlockPos,
    block: vx_core::BlockId,
    locks: &[vx_core::BlockId],
) -> bool {
    // The locks themselves are the attack surface, never the thing defended.
    // Gate them and no lock could ever be broken, which would quietly delete
    // two of the three ways through a door.
    if locks.contains(&block) {
        return false;
    }
    let mut permits = permits.borrow_mut();
    let Some(claim) = permits.claim_here(at) else {
        return false; // the frontier is free
    };
    if permits.may_edit(&claim) {
        return false;
    }
    permits.refuse(
        at,
        Refusal {
            label: claim.label,
            owner: claim.owner,
        },
    );
    true
}

/// Install the permission gate on an event bus.
///
/// This is the first production subscriber the bus has ever had — it has
/// carried cancellable edits since stage 2.x with nothing listening. Because it
/// hooks the events rather than the call sites, it covers the player's drill,
/// a drone's cutter and anything added later, without one caller changing.
///
/// **Replay is exempt for free.** `journal::replay` is handed a fresh
/// `EventBus`, so the oracle never sees this gate and recorded edits always
/// re-apply. `the_replay_oracle_runs_without_the_gate` pins that.
pub fn install(bus: &mut vx_core::EventBus, permits: Shared, registry: &vx_core::BlockRegistry) {
    // Resolved once, here, rather than by name on every edit: ids are stable
    // for the life of a registry and this runs on the hot path.
    let mut locks: Vec<vx_core::BlockId> = [Tier::One, Tier::Two, Tier::Three]
        .iter()
        .filter_map(|tier| registry.id_of(tier.block_name()))
        .collect();
    // The watch box joins the exemption for the same reason the lockboxes
    // have it: it is the thing you attack, not the thing being defended.
    // Gate it and drilling the town's eye out — the loud half of the answer
    // to being watched — would be quietly impossible.
    locks.extend(registry.id_of("engine:roost"));

    let breaking = permits.clone();
    let break_locks = locks.clone();
    bus.subscribe_with_priority(
        "permits",
        vx_core::PRIORITY_HIGH,
        move |event: &mut vx_world::BlockBreakEvent| {
            if judge(&breaking, event.position, event.block, &break_locks) {
                event.cancel();
            }
        },
    );

    let placing = permits;
    bus.subscribe_with_priority(
        "permits",
        vx_core::PRIORITY_HIGH,
        move |event: &mut vx_world::BlockPlaceEvent| {
            if judge(&placing, event.position, event.block, &locks) {
                event.cancel();
            }
        },
    );
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// Panel size in texture pixels, displayed at the shop's scale.
pub const PERMIT_WIDTH: u32 = 250;
pub const PERMIT_HEIGHT: u32 = 160;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const GOOD: [u8; 4] = [120, 220, 120, 255];
const BAD: [u8; 4] = [235, 90, 70, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 235];

/// What the lockbox panel is showing.
#[derive(Debug, Default)]
pub struct PermitPanel {
    pub open: bool,
    /// The lock being looked at, and the claim it holds.
    pub at: Option<BlockPos>,
    pub claim: Option<Claim>,
    pub feedback: Option<String>,
    /// Seconds of bypass done and the total it needs. Standing here while this
    /// runs is the actual price of picking a lock.
    pub bypass: Option<(f32, f32)>,
}

/// What happened when the player asked to bypass a lock.
#[derive(Debug, Clone, PartialEq)]
pub enum Bypass {
    /// Under way, with the fraction done.
    Working(f32),
    /// Open. The grant is theirs, and the lock is untouched — which is the
    /// point: nobody need ever know.
    Opened { xp: u64 },
    /// Refused, with the reason.
    Refused(String),
}

impl PermitPanel {
    pub fn open_at(&mut self, at: BlockPos, claim: Claim) {
        self.open = true;
        self.at = Some(at);
        self.claim = Some(claim);
        self.feedback = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.at = None;
        self.claim = None;
        self.feedback = None;
        // A bypass abandoned is a bypass lost: you cannot start picking a lock,
        // wander off for lunch and come back to it half-open. Unlike a breach,
        // which *is* a project you return to — that difference is the whole
        // reason to prefer the quiet route or the loud one deliberately.
        self.bypass = None;
    }

    /// Work at the lock for `dt` seconds.
    ///
    /// Deterministic: no roll, no chance. What levelling buys is speed, and
    /// what you risk is being seen standing there.
    pub fn work_bypass(
        &mut self,
        permits: &mut Permits,
        security_level: u32,
        dt: f32,
    ) -> Bypass {
        let Some(claim) = self.claim.clone() else {
            return Bypass::Refused("NOTHING TO PICK".into());
        };
        let Some(tier) = claim.tier else {
            return Bypass::Refused("NOTHING TO PICK".into());
        };
        if permits.may_edit(&claim) {
            return Bypass::Refused("YOU ARE ALREADY WELCOME HERE".into());
        }
        let needed = min_security(tier);
        if security_level < needed {
            return Bypass::Refused(format!("THIS LOCK NEEDS SECURITY {needed}"));
        }

        let total = crate::skills::bypass_seconds(bypass_base(tier), security_level);
        let done = self.bypass.map_or(0.0, |(done, _)| done) + dt;
        if done < total {
            self.bypass = Some((done, total));
            return Bypass::Working((done / total).clamp(0.0, 1.0));
        }

        self.bypass = None;
        permits.grant(claim.key);
        let xp = crate::skills::BYPASS_XP[match tier {
            Tier::One => 0,
            Tier::Two => 1,
            Tier::Three => 2,
        }];
        Bypass::Opened { xp }
    }
}

fn owner_line(owner: Claimant) -> String {
    match owner {
        Claimant::Player => "YOU".into(),
        Claimant::Office(Office::Mayor) => "THE MAYOR".into(),
        Claimant::Office(Office::Sheriff) => "THE SHERIFF".into(),
        Claimant::Resident(index) if index < RESIDENTS => resident_name(index).to_string(),
        Claimant::Resident(_) => "SOMEBODY".to_string(),
    }
}

fn standing_line(standing: Standing) -> (&'static str, [u8; 4]) {
    match standing {
        Standing::Owner => ("YOURS", GOOD),
        Standing::Sheriff => ("YOU WEAR THE BADGE", GOOD),
        Standing::Guest => ("YOU ARE WELCOME HERE", GOOD),
        Standing::Stranger => ("YOU ARE A STRANGER HERE", BAD),
    }
}

fn tier_line(tier: Option<Tier>) -> String {
    match tier {
        Some(Tier::One) => "LOCK GRADE I".into(),
        Some(Tier::Two) => "LOCK GRADE II".into(),
        Some(Tier::Three) => "LOCK GRADE III".into(),
        None => "NO LOCK".into(),
    }
}

/// Draw the lockbox panel. Pure in its inputs, like every panel here.
pub fn render_permit(panel: &PermitPanel, permits: &Permits) -> Vec<u8> {
    let mut pixels = vec![0u8; (PERMIT_WIDTH * PERMIT_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;
    let Some(claim) = &panel.claim else {
        vx_render::font::draw_text(&mut pixels, PERMIT_WIDTH, margin, y, 1, DIM, "NO LOCK HERE");
        return pixels;
    };

    vx_render::font::draw_text(&mut pixels, PERMIT_WIDTH, margin, y, 1, ACCENT, &claim.label);
    y += 14;

    let held = format!("HELD BY {}", owner_line(claim.owner));
    vx_render::font::draw_text(&mut pixels, PERMIT_WIDTH, margin, y, 1, TEXT, &held);
    y += 12;

    let standing = permits.standing(claim);
    let (line, tint) = standing_line(standing);
    vx_render::font::draw_text(&mut pixels, PERMIT_WIDTH, margin, y, 1, tint, line);
    y += 12;

    vx_render::font::draw_text(
        &mut pixels,
        PERMIT_WIDTH,
        margin,
        y,
        1,
        DIM,
        &tier_line(claim.tier),
    );
    y += 14;

    if standing == Standing::Stranger {
        vx_render::font::draw_text(
            &mut pixels,
            PERMIT_WIDTH,
            margin,
            y,
            1,
            DIM,
            "EARN THEIR TRUST, PICK IT, OR DRILL IT.",
        );
        y += 12;
        match panel.bypass {
            Some((done, total)) => {
                let percent = ((done / total) * 100.0).clamp(0.0, 100.0);
                let line = format!("PICKING... {percent:.0}%");
                vx_render::font::draw_text(&mut pixels, PERMIT_WIDTH, margin, y, 1, ACCENT, &line);
            }
            None => {
                vx_render::font::draw_text(
                    &mut pixels,
                    PERMIT_WIDTH,
                    margin,
                    y,
                    1,
                    TEXT,
                    "HOLD ENTER TO PICK THE LOCK.",
                );
            }
        }
        y += 12;
    }

    if permits.bounty > 0 {
        let wanted = format!("BOUNTY ON YOU: {}", permits.bounty);
        let tint = if permits.wanted() { BAD } else { ACCENT };
        vx_render::font::draw_text(&mut pixels, PERMIT_WIDTH, margin, y, 1, tint, &wanted);
        y += 12;
    }

    if let Some(feedback) = &panel.feedback {
        y += 2;
        vx_render::font::draw_text(&mut pixels, PERMIT_WIDTH, margin, y, 1, ACCENT, feedback);
    }

    vx_render::font::draw_text(
        &mut pixels,
        PERMIT_WIDTH,
        margin,
        PERMIT_HEIGHT as i32 - 14,
        1,
        DIM,
        "E LEAVES.",
    );
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vault_breach_is_the_maximum_on_the_sheet() {
        // The single loudest thing a player can do. Every other crime here is
        // against one person or one machine; a bank holds what a whole town
        // left with it.
        assert_eq!(breach_bounty(Some(Tier::Three)), BOUNTY_VAULT);
        for lesser in [None, Some(Tier::One), Some(Tier::Two)] {
            assert_eq!(breach_bounty(lesser), BOUNTY_BREACH);
        }
        const {
            assert!(BOUNTY_VAULT > BOUNTY_PRYING);
            assert!(BOUNTY_VAULT > BOUNTY_HACK);
            assert!(BOUNTY_VAULT > BOUNTY_BREACH);
        };

        // And one is enough to bring the warrant chain down on its own —
        // there is no walking this one off by keeping your head down.
        // A compile-time claim, so it fails at build rather than at test:
        // tuning the threshold up past the fine would quietly un-warrant the
        // worst crime in the game.
        const { assert!(BOUNTY_VAULT > WARRANT_THRESHOLD) };
        let mut permits = Permits::new();
        permits.set_sites(vec![town::home_site()]);
        assert!(permits.caught(BOUNTY_VAULT, 1), "nobody saw the loudest crime");
        assert!(
            permits.bounty >= WARRANT_THRESHOLD,
            "one vault did not reach the warrant threshold"
        );
    }

    fn home() -> TownSite {
        town::home_site()
    }

    fn claim_named(label: &str) -> Claim {
        claims_for(&home())
            .into_iter()
            .find(|claim| claim.label == label)
            .unwrap_or_else(|| panic!("no claim called {label}"))
    }

    #[test]
    fn your_own_house_is_yours() {
        let permits = Permits::new();
        let house = claim_named("YOUR HOUSE");
        assert_eq!(house.owner, Claimant::Player);
        assert_eq!(permits.standing(&house), Standing::Owner);
        assert!(permits.may_edit(&house));
    }

    #[test]
    fn a_neighbours_house_is_not() {
        let permits = Permits::new();
        let theirs = claims_for(&home())
            .into_iter()
            .find(|claim| matches!(claim.owner, Claimant::Resident(_)))
            .expect("the hometown has residents");
        assert_eq!(permits.standing(&theirs), Standing::Stranger);
        assert!(!permits.may_edit(&theirs));
    }

    #[test]
    fn the_sheriff_may_edit_anything() {
        // The override that makes enforcement possible: a lock the law cannot
        // pass is a lock that makes crime safe.
        let mut permits = Permits::new();
        permits.take_office(home().centre, Office::Sheriff);
        for claim in claims_for(&home()) {
            assert!(
                permits.may_edit(&claim),
                "the sheriff was refused at {}",
                claim.label
            );
        }
    }

    /// The badge stops at the town line.
    ///
    /// Stage 40's correction: before it, `holds` had no town in it at all, so
    /// one sheriff's badge opened every lock on the frontier. Winning a seat
    /// in the smallest town on the map would have unlocked the world.
    #[test]
    fn a_badge_opens_its_own_town_and_no_other() {
        let elsewhere = town::towns_near(2024, (0, 0), 6_000, &|_, _| 90)
            .into_iter()
            .find(|site| !site.is_home())
            .expect("no second town on the fixture frontier");

        let mut permits = Permits::new();
        permits.take_office(home().centre, Office::Sheriff);
        for claim in claims_for(&home()) {
            assert!(
                permits.may_edit(&claim),
                "the sheriff was refused at home: {}",
                claim.label
            );
        }
        let mut refused = 0;
        for claim in claims_for(&elsewhere) {
            if claim.owner == Claimant::Player {
                continue;
            }
            assert_ne!(
                permits.standing(&claim),
                Standing::Sheriff,
                "{}'s badge reached {}",
                home().name.head(),
                elsewhere.name.head()
            );
            refused += 1;
        }
        assert!(refused > 0, "the next town over has nothing to refuse");
    }

    /// A permits file from before badges had towns still loads, and its
    /// global badge comes back as the hometown's — strictly less reach than
    /// the file was claiming, which is the safe direction to migrate in.
    #[test]
    fn a_permits_file_from_before_towns_loads_as_the_hometowns() {
        use std::io::Write;
        let directory =
            std::env::temp_dir().join(format!("vx-permits-old-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        {
            let mut file = std::fs::File::create(directory.join("permits.dat")).unwrap();
            file.write_all(MAGIC).unwrap();
            file.write_all(&OLDEST.to_le_bytes()).unwrap();
            file.write_all(&0u32.to_le_bytes()).unwrap(); // no grants
            file.write_all(&1u32.to_le_bytes()).unwrap(); // one office
            file.write_all(&[1u8]).unwrap(); // the sheriff, with no town
            file.write_all(&140u64.to_le_bytes()).unwrap(); // bounty
            file.write_all(&0u32.to_le_bytes()).unwrap(); // no breaches
            file.write_all(&0u32.to_le_bytes()).unwrap(); // no broken locks
        }
        let mut permits = Permits::new();
        permits.load(&directory);
        assert_eq!(permits.bounty, 140, "the rest of the file did not survive");
        assert!(permits.holds_in(home().centre, Office::Sheriff));
        assert!(
            !permits.holds_in((512, -512), Office::Sheriff),
            "the migration kept the old global reach"
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn the_mayor_does_not_outrank_a_front_door() {
        let mut permits = Permits::new();
        permits.take_office(home().centre, Office::Mayor);

        let street = claim_named("THE TOWN");
        assert_eq!(permits.standing(&street), Standing::Owner, "the streets are theirs");

        let theirs = claims_for(&home())
            .into_iter()
            .find(|claim| matches!(claim.owner, Claimant::Resident(_)))
            .expect("the hometown has residents");
        assert_eq!(
            permits.standing(&theirs),
            Standing::Stranger,
            "the mayor let themselves into somebody's house"
        );
    }

    #[test]
    fn open_ground_inside_town_belongs_to_the_mayor() {
        let sites = [home()];
        // Bare ground: inside the town line, on no paving, in no building.
        // This is the case that stops you setting up shop on the green and
        // calling it yours.
        let green = BlockPos::new(20, town::HOME_GROUND_Y + 1, 20);
        let claim = claim_at(&sites, green).expect("the town's ground is claimed");
        assert_eq!(claim.owner, Claimant::Office(Office::Mayor));
        assert_eq!(claim.bounds, None, "bare ground is not a building");
    }

    #[test]
    fn the_paving_is_the_towns_too() {
        // The paved cross is authored, so it is a building claim rather than
        // open ground — but it lands with the same owner either way, which is
        // what makes the rule explainable in one sentence.
        let sites = [home()];
        let path = BlockPos::new(6, town::HOME_GROUND_Y, -6);
        let claim = claim_at(&sites, path).expect("the paving is claimed");
        assert_eq!(claim.owner, Claimant::Office(Office::Mayor));
    }

    #[test]
    fn outside_the_footprint_nothing_is_claimed() {
        let sites = [home()];
        let wilderness = BlockPos::new(400, 70, 400);
        assert!(claim_at(&sites, wilderness).is_none(), "the frontier is free");
    }

    #[test]
    fn a_building_claim_beats_the_open_ground_it_stands_on() {
        let sites = [home()];
        let inside = BlockPos::new(-14, town::HOME_GROUND_Y + 1, 9);
        let claim = claim_at(&sites, inside).expect("the house is claimed");
        assert_eq!(claim.owner, Claimant::Player, "the street swallowed the house");
    }

    #[test]
    fn a_guest_grant_covers_one_building_and_no_other() {
        let mut permits = Permits::new();
        let claims = claims_for(&home());
        let theirs: Vec<Claim> = claims
            .into_iter()
            .filter(|claim| matches!(claim.owner, Claimant::Resident(_)))
            .collect();
        assert!(theirs.len() >= 2, "need two dwellings to tell them apart");

        permits.grant(theirs[0].key);
        assert_eq!(permits.standing(&theirs[0]), Standing::Guest);
        assert_eq!(
            permits.standing(&theirs[1]),
            Standing::Stranger,
            "one grant opened two houses"
        );
    }

    #[test]
    fn every_dwelling_names_the_resident_who_sleeps_in_it() {
        // The roster's home routes are the only record of who lives where, so
        // a claim that cannot find its resident is a bug, not a default.
        for claim in claims_for(&home()) {
            if let Claimant::Resident(index) = claim.owner {
                assert!(index < RESIDENTS, "{} has no resident", claim.label);
            }
        }
    }

    #[test]
    fn a_refusal_is_only_read_back_for_the_block_it_was_about() {
        // Bedrock returns the same error *without* emitting an event, so a
        // stale refusal would otherwise be read as a fresh one and the player
        // would be told bedrock belongs to the mayor.
        let mut permits = Permits::new();
        let wall = BlockPos::new(1, 2, 3);
        permits.refuse(
            wall,
            Refusal {
                label: "THE SHOP".into(),
                owner: Claimant::Office(Office::Mayor),
            },
        );
        assert!(permits.refusal_for(wall).is_some());
        assert!(
            permits.refusal_for(BlockPos::new(9, 9, 9)).is_none(),
            "a stale refusal leaked onto another block"
        );
    }

    #[test]
    fn an_unwitnessed_crime_costs_nothing() {
        let mut permits = Permits::new();
        assert!(!permits.caught(BOUNTY_BREACH, 0));
        assert_eq!(permits.bounty, 0, "the town billed you for a private moment");

        assert!(permits.caught(BOUNTY_BREACH, 1));
        assert_eq!(permits.bounty, BOUNTY_BREACH);
    }

    #[test]
    fn bounty_climbs_to_the_warrant_threshold() {
        let mut permits = Permits::new();
        assert!(!permits.wanted());
        while permits.bounty < WARRANT_THRESHOLD {
            permits.caught(BOUNTY_PRYING, 1);
        }
        assert!(permits.wanted());
    }

    #[test]
    fn breach_progress_survives_looking_away() {
        // Ordinary drilling resets the moment your aim wobbles. A lock is a
        // project you come back to, or tier two is simply unplayable.
        let mut permits = Permits::new();
        let lock = BlockPos::new(-16, 73, 10);
        permits.set_breach(lock, 0.4);
        permits.set_breach(lock, 0.7);
        assert!((permits.breach_progress(lock) - 0.7).abs() < 1e-6);
        assert_eq!(permits.breach_progress(BlockPos::new(0, 0, 0)), 0.0);
    }

    #[test]
    fn a_broken_lock_comes_back_when_the_town_gets_round_to_it() {
        let mut permits = Permits::new();
        let lock = BlockPos::new(-16, 73, 10);
        permits.set_breach(lock, 0.9);
        permits.broke(lock, 1_000);

        assert!(permits.is_broken(lock));
        assert_eq!(permits.breach_progress(lock), 0.0, "breach progress outlived the lock");
        assert!(permits.due_rebuilds(1_000 + REBUILD_TICKS - 1).is_empty());

        let due = permits.due_rebuilds(1_000 + REBUILD_TICKS);
        assert_eq!(due, vec![lock]);
        assert!(!permits.is_broken(lock), "the rebuild was not recorded");
        assert!(
            permits.due_rebuilds(9_999_999).is_empty(),
            "the same lock was rebuilt twice"
        );
    }

    #[test]
    fn permits_survive_a_round_trip_through_disk() {
        let directory = std::env::temp_dir().join(format!(
            "vx-permits-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let mut permits = Permits::new();
        permits.grant(claim_named("THE SUPPLY SHED").key);
        permits.take_office(home().centre, Office::Sheriff);
        permits.caught(BOUNTY_BREACH, 1);
        permits.set_breach(BlockPos::new(-16, 73, 10), 0.55);
        permits.broke(BlockPos::new(1, 73, -16), 4_242);
        permits.save(&directory).unwrap();

        let mut read_back = Permits::new();
        read_back.load(&directory);

        std::fs::write(directory.join("permits.dat"), b"junkjunkjunkjunk").unwrap();
        let mut damaged = Permits::new();
        damaged.caught(BOUNTY_HACK, 1);
        damaged.load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        assert_eq!(read_back, permits);
        assert!(
            read_back.claim_here(BlockPos::new(0, town::HOME_GROUND_Y, 0)).is_none(),
            "derived town data was written to disk"
        );
        assert_eq!(damaged, Permits::new(), "damage did not reset cleanly");
    }

    #[test]
    fn the_panel_renders_deterministically_and_says_where_you_stand() {
        let claims = claims_for(&home());
        let mine = claims.iter().find(|c| c.owner == Claimant::Player).unwrap();
        let theirs = claims
            .iter()
            .find(|c| matches!(c.owner, Claimant::Resident(_)))
            .unwrap();

        let permits = Permits::new();
        let mut yours = PermitPanel::default();
        yours.open_at(BlockPos::new(-16, 73, 10), mine.clone());
        let mut neighbours = PermitPanel::default();
        neighbours.open_at(BlockPos::new(-16, 73, -2), theirs.clone());

        assert_eq!(
            render_permit(&yours, &permits),
            render_permit(&yours, &permits),
            "the panel is not a pure function of its inputs"
        );
        assert_ne!(
            render_permit(&yours, &permits),
            render_permit(&neighbours, &permits),
            "your house and a neighbour's look the same"
        );
    }

    #[test]
    fn the_panel_changes_when_the_badge_does() {
        let claims = claims_for(&home());
        let theirs = claims
            .iter()
            .find(|c| matches!(c.owner, Claimant::Resident(_)))
            .unwrap();
        let mut panel = PermitPanel::default();
        panel.open_at(BlockPos::new(-16, 73, -2), theirs.clone());

        let stranger = Permits::new();
        let mut sheriff = Permits::new();
        sheriff.take_office(home().centre, Office::Sheriff);

        assert_ne!(
            render_permit(&panel, &stranger),
            render_permit(&panel, &sheriff),
            "the badge made no difference on the panel"
        );
    }

    #[test]
    fn every_panel_label_is_drawable() {
        let mut lines: Vec<String> = vec![
            "NO LOCK HERE".into(),
            "EARN THEIR TRUST, PICK IT, OR DRILL IT.".into(),
            "E LEAVES.".into(),
            format!("BOUNTY ON YOU: {}", 25),
        ];
        for standing in [Standing::Owner, Standing::Sheriff, Standing::Guest, Standing::Stranger] {
            lines.push(standing_line(standing).0.into());
        }
        for tier in [None, Some(Tier::One), Some(Tier::Two), Some(Tier::Three)] {
            lines.push(tier_line(tier));
        }
        for owner in [
            Claimant::Player,
            Claimant::Office(Office::Mayor),
            Claimant::Office(Office::Sheriff),
            Claimant::Resident(0),
            Claimant::Resident(1),
            Claimant::Resident(2),
            Claimant::Resident(99),
        ] {
            lines.push(format!("HELD BY {}", owner_line(owner)));
        }
        for claim in claims_for(&home()) {
            lines.push(claim.label.clone());
            lines.push(Refusal { label: claim.label, owner: claim.owner }.line());
        }
        for line in lines {
            for character in line.chars() {
                assert!(
                    vx_render::font::knows(character),
                    "undrawable {character:?} in {line:?}"
                );
            }
        }
    }

    // -- the gate, end to end ------------------------------------------------

    /// A world with the whole hometown generated, ready to be dug at.
    ///
    /// Radius four, not two: the containers reach x = -17, which sits in a
    /// chunk a radius-two disc does not cover, and an unloaded block reads as
    /// air rather than as a wall.
    fn town_world() -> vx_world::World {
        let mut world = vx_world::World::new(2024);
        world.load_around(vx_core::ChunkPos::new(0, 0), 4);
        world
    }

    fn gated(mut permits: Permits) -> (vx_core::EventBus, Shared) {
        permits.set_sites(vec![home()]);
        let shared: Shared = std::rc::Rc::new(std::cell::RefCell::new(permits));
        let mut bus = vx_core::EventBus::new();
        let registry = vx_world::World::new(0).registry().clone();
        install(&mut bus, shared.clone(), &registry);
        (bus, shared)
    }

    #[test]
    fn a_break_inside_a_dwelling_is_refused_through_break_block() {
        // The whole round in one test: an ordinary break, through the ordinary
        // call, refused by a listener the caller knows nothing about.
        let mut world = town_world();
        let (bus, permits) = gated(Permits::new());

        // A wall of the east-door container, which a resident owns.
        let wall = BlockPos::new(-17, town::HOME_GROUND_Y + 1, -1);
        assert!(world.block(wall) != vx_core::BlockId::AIR, "picked a spot with no wall in it");

        let result = vx_world::break_block(&mut world, &bus, wall);
        assert_eq!(result, Err(vx_world::EditError::Cancelled));
        assert!(world.block(wall) != vx_core::BlockId::AIR, "the wall came down anyway");
        assert!(
            permits.borrow().refusal_for(wall).is_some(),
            "the player would never learn why"
        );
    }

    #[test]
    fn the_same_break_goes_through_once_you_are_welcome() {
        let mut world = town_world();
        let wall = BlockPos::new(-17, town::HOME_GROUND_Y + 1, -1);

        let mut permits = Permits::new();
        let claim = claim_at(&[home()], wall).expect("the wall is claimed");
        permits.grant(claim.key);
        let (bus, _) = gated(permits);

        assert!(vx_world::break_block(&mut world, &bus, wall).is_ok());
        assert!(world.block(wall) == vx_core::BlockId::AIR, "the grant did not open the wall");
    }

    #[test]
    fn breaking_your_own_chest_still_works() {
        // Regression for stage 10c: the chest is authored *inside* the claim
        // that covers your house, and the pack-up flow depends on breaking it.
        let mut world = town_world();
        let (bus, _) = gated(Permits::new());
        let chest = town::chest_position(&home());

        assert!(vx_world::break_block(&mut world, &bus, chest).is_ok());
        assert!(world.block(chest) == vx_core::BlockId::AIR, "your own chest refused you");
    }

    #[test]
    fn a_lockbox_itself_is_never_gated() {
        // Attacking a lock is the point of it existing. What governs a breach
        // is the drill, not permission.
        let mut world = town_world();
        let (bus, _) = gated(Permits::new());

        let locks = plan::lockboxes(&home());
        let (theirs, _) = locks
            .iter()
            .find(|(at, _)| {
                claim_at(&[home()], *at)
                    .is_some_and(|claim| matches!(claim.owner, Claimant::Resident(_)))
            })
            .expect("a resident's lock");

        assert!(
            vx_world::break_block(&mut world, &bus, *theirs).is_ok(),
            "the gate refused a lock, so no lock could ever be broken"
        );
    }

    #[test]
    fn the_wilderness_is_still_free() {
        let mut world = vx_world::World::new(2024);
        world.load_around(vx_core::ChunkPos::new(40, 40), 1);
        let (bus, _) = gated(Permits::new());

        // The first real rock under the column — this stretch of frontier
        // happens to be under water, and water is unbreakable by design.
        let stone = world.registry().id_of("engine:stone").unwrap();
        let surface = world.surface_y(640, 640).expect("chunk is loaded");
        let rock = (1..60)
            .map(|depth| BlockPos::new(640, surface - depth, 640))
            .find(|at| world.block(*at) == stone)
            .unwrap_or_else(|| {
                let sample: Vec<String> = (1..12)
                    .map(|d| {
                        let at = BlockPos::new(640, surface - d, 640);
                        world.registry().get_or_air(world.block(at)).name.clone()
                    })
                    .collect();
                panic!("no rock under the column; found {sample:?}")
            });

        assert!(
            vx_world::break_block(&mut world, &bus, rock).is_ok(),
            "the town's law reached the far side of the map"
        );
    }

    #[test]
    fn placing_on_the_towns_ground_is_refused_too() {
        // The consequence worth stating out loud: your chest and your base
        // container cannot go just anywhere in town.
        let mut world = town_world();
        let (bus, permits) = gated(Permits::new());
        let green = BlockPos::new(20, town::HOME_GROUND_Y + 1, 20);

        // Aim at the ground and try to stack a block on it.
        let under = BlockPos::new(green.x, green.y - 1, green.z);
        let hit = vx_world::RayHit {
            block: under,
            face: vx_core::Face::PosY,
            distance: 1.0,
            id: world.block(under),
        };
        let stone = world.registry().id_of("engine:stone").unwrap();
        let result = vx_world::place_block(&mut world, &bus, &hit, stone, |_| false);

        assert_eq!(result, Err(vx_world::EditError::Cancelled));
        assert!(
            permits.borrow().refusal_for(green).is_some(),
            "refused without saying why"
        );
    }

    #[test]
    fn the_sheriff_walks_through_the_gate() {
        let mut world = town_world();
        let mut permits = Permits::new();
        permits.take_office(home().centre, Office::Sheriff);
        let (bus, _) = gated(permits);

        let wall = BlockPos::new(-17, town::HOME_GROUND_Y + 1, -1);
        assert!(
            vx_world::break_block(&mut world, &bus, wall).is_ok(),
            "the badge did not open the door"
        );
    }

    #[test]
    fn a_drone_job_overlapping_a_claim_is_never_dispatched() {
        // The planner's question, asked before a crew is put on the job.
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);

        // A box straddling the mayor's house.
        let refused = permits.blocked_span(
            BlockPos::new(-20, town::HOME_GROUND_Y, -6),
            BlockPos::new(-12, town::HOME_GROUND_Y + 3, 4),
        );
        assert!(refused.is_some(), "a dig through a house was allowed");

        // The same question out past the town line.
        assert!(
            permits
                .blocked_span(
                    BlockPos::new(600, 40, 600),
                    BlockPos::new(620, 60, 620)
                )
                .is_none(),
            "the planner refused a dig in the wilderness"
        );
    }

    #[test]
    fn your_own_excavation_under_your_own_house_is_allowed() {
        // The rule is ownership, not proximity: a claim you hold is not an
        // obstacle to your own machines.
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);
        let house = claims_for(&home())
            .into_iter()
            .find(|claim| claim.owner == Claimant::Player)
            .expect("you have a house");
        let (min, max) = house.bounds.expect("a house has edges");

        assert!(
            permits.blocked_span(min, max).is_none(),
            "the planner refused to dig your own cellar"
        );
    }

    #[test]
    fn breaking_the_lock_opens_the_building_until_it_is_rebuilt() {
        // What a breach actually buys. Without this, destroying a box would be
        // the loudest possible way to achieve nothing at all.
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);

        let wall = BlockPos::new(-17, town::HOME_GROUND_Y + 1, -1);
        let claim = permits.claim_here(wall).expect("the wall is claimed");
        assert!(!permits.may_edit(&claim), "a stranger walked in");

        let lock = claim.lock.expect("a dwelling has a lock");
        permits.broke(lock, 500);

        let claim = permits.claim_here(wall).expect("still a claim");
        assert!(permits.is_open(&claim), "the breach bought nothing");
        assert!(permits.may_edit(&claim), "the wall stayed shut");

        // ...and the window closes when the town gets round to it.
        assert_eq!(permits.due_rebuilds(500 + REBUILD_TICKS), vec![lock]);
        let claim = permits.claim_here(wall).expect("still a claim");
        assert!(!permits.may_edit(&claim), "the door never shut again");
    }

    #[test]
    fn a_broken_lock_opens_its_own_building_and_no_other() {
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);

        let dwellings: Vec<Claim> = claims_for(&home())
            .into_iter()
            .filter(|claim| matches!(claim.owner, Claimant::Resident(_)))
            .collect();
        assert!(dwellings.len() >= 2);

        permits.broke(dwellings[0].lock.expect("a lock"), 0);
        assert!(permits.may_edit(&dwellings[0]));
        assert!(
            !permits.may_edit(&dwellings[1]),
            "one broken lock opened the whole street"
        );
    }

    #[test]
    fn the_streets_have_no_lock_to_force() {
        // Town land can only ever be earned, never broken into — there is no
        // box on a road.
        let permits = {
            let mut permits = Permits::new();
            permits.set_sites(vec![home()]);
            permits
        };
        let street = claim_named("THE TOWN");
        assert_eq!(street.lock, None);
        assert!(!permits.is_open(&street));
    }

    #[test]
    fn a_weak_drill_makes_no_progress_on_a_stronger_lock() {
        // The gate that lets "impossible for a new player" mean something.
        // Drill power is 1.25 fresh and 13.84 fully levelled and upgraded.
        let fresh = 1.25;
        let maxed = 13.84;

        assert!(fresh >= min_power(Tier::One), "a house lock must be reachable");
        assert!(fresh < min_power(Tier::Two), "a shop lock must not be");
        assert!(fresh < min_power(Tier::Three));

        assert!(maxed > min_power(Tier::Two), "levelling must open grade two");
        assert!(maxed > min_power(Tier::Three), "and eventually grade three");

        // And the grades are strictly ordered, so the tiering means something.
        assert!(min_power(Tier::One) < min_power(Tier::Two));
        assert!(min_power(Tier::Two) < min_power(Tier::Three));
    }

    #[test]
    fn every_lock_grade_names_a_registered_block() {
        let world = vx_world::World::new(0);
        for tier in [Tier::One, Tier::Two, Tier::Three] {
            assert!(
                world.registry().id_of(tier.block_name()).is_some(),
                "{tier:?} names a block that does not exist"
            );
        }
    }

    #[test]
    fn the_tier_of_a_block_round_trips() {
        let world = vx_world::World::new(0);
        for tier in [Tier::One, Tier::Two, Tier::Three] {
            let id = world.registry().id_of(tier.block_name()).unwrap();
            assert_eq!(tier_of(world.registry(), id), Some(tier));
        }
        let stone = world.registry().id_of("engine:stone").unwrap();
        assert_eq!(tier_of(world.registry(), stone), None);
    }

    // -- picking locks -------------------------------------------------------

    fn panel_on(claim: &Claim) -> PermitPanel {
        let mut panel = PermitPanel::default();
        panel.open_at(claim.lock.unwrap_or(BlockPos::new(0, 0, 0)), claim.clone());
        panel
    }

    fn a_neighbours_claim() -> Claim {
        claims_for(&home())
            .into_iter()
            .find(|claim| matches!(claim.owner, Claimant::Resident(_)))
            .expect("the hometown has residents")
    }

    #[test]
    fn a_bypass_below_the_tier_minimum_is_refused() {
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);
        let shop = claims_for(&home())
            .into_iter()
            .find(|claim| claim.label == "THE SUPPLY SHED")
            .unwrap();
        let mut panel = panel_on(&shop);

        // Grade two wants twenty levels; a beginner is told so rather than
        // standing there for a minute finding out.
        let outcome = panel.work_bypass(&mut permits, 1, 1.0);
        assert!(
            matches!(outcome, Bypass::Refused(ref why) if why.contains("SECURITY")),
            "got {outcome:?}"
        );
        assert!(panel.bypass.is_none(), "a refused pick left work behind");
    }

    #[test]
    fn a_successful_bypass_grants_one_building_and_leaves_the_lock_standing() {
        // The quiet route: you are in, the box is untouched, and unless
        // somebody watched you do it nobody ever knows.
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);
        let theirs = a_neighbours_claim();
        let mut panel = panel_on(&theirs);

        let lock = theirs.lock.expect("a dwelling has a lock");
        let mut xp_awarded = 0;
        for _ in 0..600 {
            match panel.work_bypass(&mut permits, 10, 0.1) {
                Bypass::Working(_) => {}
                Bypass::Opened { xp } => {
                    xp_awarded = xp;
                    break;
                }
                Bypass::Refused(why) => panic!("refused: {why}"),
            }
        }

        assert!(xp_awarded > 0, "the pick never completed");
        assert!(permits.granted(theirs.key), "no grant was issued");
        assert!(permits.may_edit(&theirs));
        assert!(!permits.is_broken(lock), "picking a lock destroyed it");

        // ...and only that one building.
        let other = claims_for(&home())
            .into_iter()
            .find(|claim| {
                matches!(claim.owner, Claimant::Resident(_)) && claim.key != theirs.key
            })
            .expect("another dwelling");
        assert!(!permits.may_edit(&other), "one pick opened the street");
    }

    #[test]
    fn picking_is_faster_at_a_higher_security_level() {
        // Levelling buys speed, not certainty — there is no roll anywhere in
        // this, which is what keeps it reproducible.
        let quick = crate::skills::bypass_seconds(bypass_base(Tier::One), 60);
        let slow = crate::skills::bypass_seconds(bypass_base(Tier::One), 1);
        assert!(quick < slow, "levelling bought nothing: {slow} then {quick}");
        assert!(quick > 0.0);
    }

    #[test]
    fn a_bypass_is_deterministic() {
        // Same lock, same level, same answer — twice.
        let run = || {
            let mut permits = Permits::new();
            permits.set_sites(vec![home()]);
            let theirs = a_neighbours_claim();
            let mut panel = panel_on(&theirs);
            let mut ticks = 0;
            loop {
                ticks += 1;
                match panel.work_bypass(&mut permits, 12, 0.05) {
                    Bypass::Working(_) => {}
                    Bypass::Opened { .. } => break ticks,
                    Bypass::Refused(why) => panic!("refused: {why}"),
                }
            }
        };
        assert_eq!(run(), run(), "the same pick took a different time twice");
    }

    #[test]
    fn there_is_no_point_picking_a_lock_you_already_hold() {
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);
        let mine = claims_for(&home())
            .into_iter()
            .find(|claim| claim.owner == Claimant::Player)
            .unwrap();
        let mut panel = panel_on(&mine);

        assert!(matches!(
            panel.work_bypass(&mut permits, 99, 1.0),
            Bypass::Refused(_)
        ));
    }

    #[test]
    fn walking_away_loses_the_pick() {
        // A breach you come back to; a pick you do not. That asymmetry is what
        // makes choosing between the loud route and the quiet one a decision.
        let mut permits = Permits::new();
        permits.set_sites(vec![home()]);
        let theirs = a_neighbours_claim();
        let mut panel = panel_on(&theirs);

        panel.work_bypass(&mut permits, 10, 1.0);
        assert!(panel.bypass.is_some(), "no work was done to lose");
        panel.close();
        assert!(panel.bypass.is_none(), "the pick survived walking away");
    }

    #[test]
    fn every_lock_grade_can_eventually_be_picked() {
        // No grade may be a dead end: the endgame ones want most of a career,
        // but the ladder has to reach the top.
        for tier in [Tier::One, Tier::Two, Tier::Three] {
            assert!(min_security(tier) <= crate::skills::MAX_LEVEL);
            assert!(bypass_base(tier) > 0.0);
        }
        assert!(min_security(Tier::One) < min_security(Tier::Two));
        assert!(min_security(Tier::Two) < min_security(Tier::Three));
    }

    #[test]
    fn the_replay_oracle_runs_without_the_gate() {
        // Replay is handed a fresh bus by every caller, which is what keeps
        // recorded edits re-appliable. If somebody ever "helpfully" passes the
        // live bus in, a town break would silently no-op and the rebuild would
        // diverge — this is the tripwire for that.
        let mut world = town_world();
        let bare = vx_core::EventBus::new();
        let wall = BlockPos::new(-17, town::HOME_GROUND_Y + 1, -1);

        assert!(
            vx_world::break_block(&mut world, &bare, wall).is_ok(),
            "a virgin bus refused an edit, so replay cannot be trusted"
        );
    }

    #[test]
    fn every_claim_label_is_drawable() {
        for claim in claims_for(&home()) {
            for character in claim.label.chars() {
                assert!(
                    vx_render::font::knows(character),
                    "undrawable {character:?} in {}",
                    claim.label
                );
            }
        }
    }
}
