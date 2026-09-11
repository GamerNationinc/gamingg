//! Towns: where the frontier is settled.
//!
//! A town is a flat plot with a plan stamped on it. Sites come from a jittered
//! lattice — the same idiom as [`crate::ore`]'s deposits and [`crate::flora`]'s
//! trees — so **the map of towns is derivable arithmetic, never stored**.
//! Asking "where are the towns within five kilometres" costs a few hundred
//! hashes and touches no chunk, which is what lets a beacon post work at a
//! town that has never been generated.
//!
//! # The height field must not feed itself
//!
//! A site is chosen partly by how flat and how dry its ground is, so siting
//! reads the terrain — and the terrain, once a town exists, has that town's
//! plateau flattened into it. Read the blended field while siting and town N's
//! plateau decides where town N+1 stands; siting stops being a pure function of
//! the seed and starts depending on which town happened to be considered first.
//! That is why [`crate::gen::TerrainGenerator`] carries two height functions and
//! siting may only ever see `natural_height_at`.
//!
//! # The site list is a superset contract
//!
//! Every function here that takes `sites: &[TownSite]` answers for a column
//! *given those sites*. Callers must have gathered over a box containing every
//! column they will ask about — exactly the contract [`crate::ore::ore_at`]
//! has with `deposits_overlapping`. Honour it and the answer for a column is
//! identical no matter which chunk asked; break it and chunk seams disagree.

pub mod plan;

use vx_core::BlockPos;

use crate::gen::SEA_LEVEL;
use crate::seed::{finalise, unit};

/// Lattice cell size, in blocks: one candidate town per cell.
pub const CELL: i32 = 512;

/// How far a town's plateau blends out past its flat core.
pub const SKIRT: i32 = 24;

/// Core half-widths a town may take.
pub const MIN_CORE_HALF: i32 = 20;
pub const MAX_CORE_HALF: i32 = 34;

/// The Ruined City's flat core: a plateau 192 blocks across, about nine times
/// a frontier town's area. It is the one settlement in the world that is not
/// on the tier table, which is the point of it.
pub const CITY_CORE_HALF: i32 = 96;

/// And its skirt, double the usual.
///
/// A plot that big with a frontier town's skirt on it would end in a
/// twenty-four block step all the way round — a mesa rather than a city on a
/// plain. The blend is taken from the site now rather than from the constant,
/// so a big plot eases into the country over twice the distance.
pub const CITY_SKIRT: i32 = 48;

/// The furthest any site can influence a column: the gather margin.
///
/// **This is a contract, not a convenience.** Every caller that gathers sites
/// over a box expands it by this much; a site whose reach exceeds it is a site
/// two neighbouring chunks disagree about, and the seam shows.
///
/// Sized by the widest thing on the lattice, which is not the widest
/// *plateau*: the Ruined City's ancient curtain stands fifty-eight blocks
/// outside its own core and then throws twenty-six-block points off that, with
/// a six-block wall and an eleven-block ditch beyond them — a hundred and
/// twelve past the core all told. `the_gather_margin_covers_the_widest_wall`
/// asserts it against `fort::forts_for` rather than trusting the arithmetic
/// here, and it was worth writing for a reason that predates the city: a
/// six-point trace has always reached about six blocks further than
/// `core_half + SKIRT`, and only `flora::CANOPY_REACH` being folded into the
/// per-chunk gather for an unrelated reason was covering the difference.
pub const REACH: i32 = CITY_CORE_HALF + 112;

/// The hometown's authored plateau and size. Fixed, so every world's starting
/// town is the same one.
pub const HOME_GROUND_Y: i32 = 72;
pub const HOME_CORE_HALF: i32 = 26;

/// Ground must clear the sea by this much for a town to be built on it.
pub const MIN_DRY: i32 = 3;

/// The most a town's plot may rise and fall before it is rejected as too
/// steep. Towns belong in valleys and on plains, not bulldozed into a cliff.
pub const MAX_RELIEF: i32 = 28;

/// Fraction of lattice cells that hold a town at all.
const PRESENCE: f32 = 0.55;

/// What a town is for. Shapes its plan and, later, what its board posts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Speciality {
    /// Freight and storage: the hometown's kind.
    Depot,
    /// A camp built around a hole in the ground.
    Mine,
    /// Tanks and pipework.
    Refinery,
    /// The Ruined City: one per seed, and nothing like the other three.
    ///
    /// A terminal rather than a works. It makes almost nothing, holds a great
    /// deal, and has the money to buy whatever the frontier can carry to it —
    /// which is what turns it into the far end of the network stage 58 built
    /// rather than another stop on it.
    City,
}

impl Speciality {
    pub fn name(self) -> &'static str {
        match self {
            Speciality::Depot => "DEPOT",
            Speciality::Mine => "MINE",
            Speciality::Refinery => "REFINERY",
            Speciality::City => "CITY",
        }
    }
}

/// Head words for town names.
const HEADS: [&str; 16] = [
    "RIDGE", "IRON", "DUST", "SALT", "COLD", "RED", "BLACK", "PALE", "STONE", "COPPER", "LONG",
    "DRY", "NEW", "FAR", "GRIT", "ASH",
];

/// Tail words for town names.
const TAILS: [&str; 16] = [
    "WATCH", "HOLLOW", "GATE", "REACH", "FORK", "CROSS", "STAND", "BEND", "POINT", "MILE", "SPUR",
    "YARD", "HAVEN", "CAMP", "LANDING", "WELL",
];

/// A town's name, as two indices into small word tables.
///
/// Two bytes and `Copy`, so a name rides along on the worldgen path without
/// allocating. Names are deliberately **not** globally unique — nothing keys
/// off them; a town is identified by its centre, and the board shows the
/// coordinates beside the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TownName {
    head: u8,
    tail: u8,
}

impl TownName {
    pub fn head(self) -> &'static str {
        HEADS[self.head as usize % HEADS.len()]
    }

    pub fn tail(self) -> &'static str {
        TAILS[self.tail as usize % TAILS.len()]
    }

    /// A name from two words out of the book, for a town somebody founds.
    ///
    /// Case-insensitive, because it is typed at a terminal; `None` if either
    /// word is not in the book, because a founded town is named from the
    /// same sixteen-by-sixteen vocabulary every other town is — nothing
    /// downstream can tell which kind it is, and that is the point.
    pub fn from_words(head: &str, tail: &str) -> Option<TownName> {
        let head = HEADS.iter().position(|word| word.eq_ignore_ascii_case(head.trim()))?;
        let tail = TAILS.iter().position(|word| word.eq_ignore_ascii_case(tail.trim()))?;
        Some(TownName {
            head: head as u8,
            tail: tail as u8,
        })
    }

    /// The two indices, for the wire.
    pub fn indices(self) -> (u8, u8) {
        (self.head, self.tail)
    }

    /// A name back off the wire. Any byte is a name; the tables wrap.
    pub fn from_indices(head: u8, tail: u8) -> TownName {
        TownName { head, tail }
    }
}

/// The book every town is named from: the head words and the tail words.
pub fn name_book() -> (&'static [&'static str], &'static [&'static str]) {
    (&HEADS, &TAILS)
}

impl std::fmt::Display for TownName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.head(), self.tail())
    }
}

/// One settlement: a flat plot, a plan, and an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TownSite {
    /// The column the town is centred on.
    pub centre: (i32, i32),
    /// The plateau its plot is levelled to.
    pub ground: i32,
    /// Half-width of the flat core.
    pub core_half: i32,
    pub speciality: Speciality,
    pub name: TownName,
    /// A hash stream of this town's own, for anything derived from it.
    pub seed: u64,
}

impl TownSite {
    /// Is this the town every world starts in?
    pub fn is_home(&self) -> bool {
        self.centre == (0, 0)
    }

    /// Is this the Ruined City?
    ///
    /// Asked of the speciality rather than the centre, because unlike the
    /// hometown the city's position is the seed's business and nothing
    /// downstream should be made to know it.
    pub fn is_city(&self) -> bool {
        self.speciality == Speciality::City
    }

    /// How far this site's plateau blends out past its flat core.
    pub fn skirt(&self) -> i32 {
        if self.is_city() {
            CITY_SKIRT
        } else {
            SKIRT
        }
    }
}

/// The hometown: pinned at the origin, authored, and byte-identical in every
/// seed. Returned before any hashing happens, so it is seed-*independent* by
/// construction rather than merely seed-stable.
pub fn home_site() -> TownSite {
    TownSite {
        centre: (0, 0),
        ground: HOME_GROUND_Y,
        core_half: HOME_CORE_HALF,
        speciality: Speciality::Depot,
        name: TownName { head: 8, tail: 12 }, // STONEHAVEN
        seed: 0,
    }
}

/// The shared splitmix64 finaliser, mapped to `0..1`. One stream per property
/// via `salt`.
fn hash01(seed: u64, salt: u64, cell_x: i32, cell_z: i32) -> f32 {
    crate::seed::unit(crate::seed::finalise(
        seed ^ salt
            ^ (cell_x as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (cell_z as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f),
    ))
}

/// How many lattice cells out the Ruined City sits, as a ring radius.
///
/// Seven cells, which is three to four kilometres once where-in-the-cell is
/// counted: pinned on the map from the first frame, and a trip you plan rather
/// than a walk you take. The frontier towns stay the everyday economy and the
/// city stays the thing you work up to.
pub const CITY_RING: i32 = 7;

/// What the city will level to get its plot.
///
/// Looser than [`MAX_RELIEF`] because the fiction is different: a frontier town
/// picks flat ground because it has shovels, and whoever raised the great star
/// moved whatever was in the way. Not unbounded, though — the search below
/// still prefers the flattest ground on offer, so the city lands on a plain
/// where there is one.
pub const CITY_RELIEF: i32 = 44;

/// Candidate centres tried inside the city's cell, as fractions of the cell.
///
/// A plot 192 blocks across is a far harder thing to find than a 40-block one,
/// so unlike a town the city gets to look around before it settles. Nine fixed
/// offsets, walked in order, first buildable wins — deterministic, bounded, and
/// paid only by the one cell in the world that is the city's.
const CITY_TRIES: [(f32, f32); 9] = [
    (0.50, 0.50),
    (0.35, 0.35),
    (0.65, 0.35),
    (0.35, 0.65),
    (0.65, 0.65),
    (0.50, 0.30),
    (0.50, 0.70),
    (0.30, 0.50),
    (0.70, 0.50),
];

/// Which lattice cell the Ruined City stands in, for this seed.
///
/// One bearing off one hash, and the cell nearest that bearing at
/// [`CITY_RING`] cells out. Exactly one city exists in a world and this is the
/// whole of why: the answer is a function of the seed alone, so no search, no
/// list and no tie-break is ever needed, and nothing is written down.
///
/// A **round** ring rather than a square one, which matters more than it
/// sounds: walking the cells of a square ring puts a corner city half again as
/// far out as an edge one — 5.1 km against 3.6 — and "three to four
/// kilometres" would then have meant "somewhere between three and five,
/// depending on a hash nobody can see".
pub fn city_cell(seed: u64) -> (i32, i32) {
    // The cells that actually lie on the ring, gathered rather than rounded
    // to. Taking a bearing and rounding each axis independently looks like the
    // same thing and is not: it lands anywhere from 6.4 to 7.6 cells out
    // depending on where the two roundings happen to fall, which turns "three
    // to four kilometres" into "somewhere between three and four and a half,
    // depending on a hash nobody can see". The band below is what makes the
    // promise a promise.
    let mut ring: Vec<(i32, i32)> = Vec::new();
    for dz in -(CITY_RING + 1)..=(CITY_RING + 1) {
        for dx in -(CITY_RING + 1)..=(CITY_RING + 1) {
            let out = (((dx * dx + dz * dz) as f32).sqrt() - CITY_RING as f32).abs();
            if out <= RING_BAND {
                ring.push((dx, dz));
            }
        }
    }
    // Scanned in a fixed order and indexed by one hash, so this is as pure as
    // any other lattice answer — and the origin is not on the ring, so the
    // hometown's cell is never in the list to begin with.
    let pick = (unit(finalise(seed ^ 0x0c17_0000_0000_0001)) * ring.len() as f32) as usize;
    ring[pick.min(ring.len() - 1)]
}

/// How far off the ring a cell may be and still count as on it.
const RING_BAND: f32 = 0.4;

/// The Ruined City for this seed, sited.
///
/// One cell and at most nine terrain probes, which is what makes it cheap
/// enough to ask every frame — and `towns_near` at four kilometres, which is
/// the only other way to find it, is two hundred and fifty cells of hashing.
/// The map pin needs an answer before the player has been anywhere near it,
/// so it needs this one.
pub fn city(seed: u64, natural_height_at: &impl Fn(i32, i32) -> i32) -> TownSite {
    let (cell_x, cell_z) = city_cell(seed);
    city_in_cell(seed, cell_x, cell_z, natural_height_at)
        .expect("the city is always in its own cell")
}

/// The Ruined City, if this is its cell.
///
/// Short-circuited before the presence hash the way the hometown is, so the
/// city owns its cell outright and no ordinary town can share it.
fn city_in_cell(
    seed: u64,
    cell_x: i32,
    cell_z: i32,
    natural_height_at: &impl Fn(i32, i32) -> i32,
) -> Option<TownSite> {
    if (cell_x, cell_z) != city_cell(seed) {
        return None;
    }

    // Walk the candidates, keeping the flattest as a fallback: the city is
    // going to stand somewhere in this cell whatever the ground says, so the
    // question is only which part of it.
    let mut best: Option<((i32, i32), i32, i32)> = None; // centre, ground, relief
    for (fx, fz) in CITY_TRIES {
        let centre = (
            cell_x * CELL + (fx * CELL as f32) as i32,
            cell_z * CELL + (fz * CELL as f32) as i32,
        );
        let ground = natural_height_at(centre.0, centre.1);
        if ground <= SEA_LEVEL + MIN_DRY {
            continue;
        }
        let relief = plot_relief(natural_height_at, centre, CITY_CORE_HALF);
        if relief <= CITY_RELIEF {
            best = Some((centre, ground, relief));
            break;
        }
        if best.is_none_or(|(_, _, worst)| relief < worst) {
            best = Some((centre, ground, relief));
        }
    }
    // Every candidate was in the sea. Take the middle of the cell and raise it
    // out of the water: a world with no city in it is a worse answer than a
    // city on a causeway, and the ring is wide enough that this is rare.
    let (centre, ground, _) = best.unwrap_or_else(|| {
        let centre = (cell_x * CELL + CELL / 2, cell_z * CELL + CELL / 2);
        (centre, SEA_LEVEL + MIN_DRY + 1, 0)
    });

    Some(TownSite {
        centre,
        ground: ground.clamp(SEA_LEVEL + MIN_DRY + 1, 140),
        core_half: CITY_CORE_HALF,
        speciality: Speciality::City,
        // Always the same two words, because the city is a proper noun in a
        // world where every other settlement is a pair of them: RUINEDGATE
        // reads as a place rather than as a roll.
        name: TownName { head: 13, tail: 2 },
        seed: seed
            ^ (cell_x as i64 as u64).wrapping_mul(0x1656_67b1_9e37_79f9)
            ^ (cell_z as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
    })
}

/// How much a plot rises and falls across its corners.
fn plot_relief(
    natural_height_at: &impl Fn(i32, i32) -> i32,
    centre: (i32, i32),
    core_half: i32,
) -> i32 {
    let ground = natural_height_at(centre.0, centre.1);
    let mut lowest = ground;
    let mut highest = ground;
    for (dx, dz) in [
        (-core_half, -core_half),
        (core_half, -core_half),
        (-core_half, core_half),
        (core_half, core_half),
    ] {
        let corner = natural_height_at(centre.0 + dx, centre.1 + dz);
        lowest = lowest.min(corner);
        highest = highest.max(corner);
    }
    highest - lowest
}

/// The town in one lattice cell, or nothing.
///
/// `natural_height_at` **must** be the pre-town height field; see the module
/// docs. Gates are ordered by cost: the cell test and the presence hash reject
/// almost everything before any terrain is sampled.
fn site_in_cell(
    seed: u64,
    cell_x: i32,
    cell_z: i32,
    natural_height_at: &impl Fn(i32, i32) -> i32,
) -> Option<TownSite> {
    // The hometown owns its cell, decided before a single hash runs — which
    // is what makes it seed-*independent* rather than merely seed-stable.
    if cell_x == 0 && cell_z == 0 {
        return Some(home_site());
    }

    // And the Ruined City owns its own, for the same reason and by the same
    // trick: one cell, decided by one hash on the seed, before the presence
    // roll that would otherwise put an ordinary town in it.
    if let Some(city) = city_in_cell(seed, cell_x, cell_z, natural_height_at) {
        return Some(city);
    }

    let key = |salt: u64| hash01(seed, salt, cell_x, cell_z);

    // Cheapest gate first: most cells hold nothing, and rejecting them costs
    // one hash and no terrain sampling at all.
    if key(0x01) > PRESENCE {
        return None;
    }

    // Jitter inside the middle half of the cell. That clamp is what keeps
    // towns apart: two neighbours are at least CELL/2 apart, against a
    // maximum reach of 2 * REACH, so no cross-cell rejection pass is needed
    // and siting never has to consult a neighbour's decision.
    let quarter = CELL / 4;
    let centre = (
        cell_x * CELL + quarter + (key(0x02) * quarter as f32 * 2.0) as i32,
        cell_z * CELL + quarter + (key(0x03) * quarter as f32 * 2.0) as i32,
    );

    // Dry land only.
    let ground = natural_height_at(centre.0, centre.1);
    if ground <= SEA_LEVEL + MIN_DRY {
        return None;
    }

    let core_half = match (key(0x04) * 3.0) as u32 {
        0 => MIN_CORE_HALF,
        1 => HOME_CORE_HALF,
        _ => MAX_CORE_HALF,
    };

    // Buildable: a town levels its plot, so it may not be asked to level a
    // mountainside. Probing the corners costs four noise evaluations, paid
    // only by candidates that got this far.
    if !buildable(natural_height_at, centre, core_half) {
        return None;
    }

    let speciality = match (key(0x05) * 3.0) as u32 {
        0 => Speciality::Depot,
        1 => Speciality::Mine,
        _ => Speciality::Refinery,
    };

    Some(TownSite {
        centre,
        ground: ground.clamp(SEA_LEVEL + MIN_DRY + 1, 140),
        core_half,
        speciality,
        name: TownName {
            head: (key(0x06) * HEADS.len() as f32) as u8,
            tail: (key(0x07) * TAILS.len() as f32) as u8,
        },
        seed: seed
            ^ (cell_x as i64 as u64).wrapping_mul(0x1656_67b1_9e37_79f9)
            ^ (cell_z as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
    })
}

/// Could a town level this plot?
///
/// The four corners of the core against the centre, within [`MAX_RELIEF`].
/// The lattice asks it of every candidate cell; a founded town is asked the
/// same question of the ground the player is standing on, so nobody gets to
/// charter a mountainside the lattice would have refused.
pub fn buildable(
    natural_height_at: &impl Fn(i32, i32) -> i32,
    centre: (i32, i32),
    core_half: i32,
) -> bool {
    plot_relief(natural_height_at, centre, core_half) <= MAX_RELIEF
}

/// Every town whose core or skirt could reach the column box `min..=max`.
///
/// Gather once per chunk and reuse — calling this per column would pay the
/// lattice 256 times over for an answer that cannot change.
pub fn towns_overlapping(
    seed: u64,
    min: (i32, i32),
    max: (i32, i32),
    natural_height_at: &impl Fn(i32, i32) -> i32,
) -> Vec<TownSite> {
    let lo_x = (min.0 - REACH).div_euclid(CELL);
    let hi_x = (max.0 + REACH).div_euclid(CELL);
    let lo_z = (min.1 - REACH).div_euclid(CELL);
    let hi_z = (max.1 + REACH).div_euclid(CELL);

    let mut found = Vec::new();
    for cell_x in lo_x..=hi_x {
        for cell_z in lo_z..=hi_z {
            if let Some(site) = site_in_cell(seed, cell_x, cell_z, natural_height_at) {
                // A cell's town is jittered inside it, so a site from a cell
                // in the window may still be too far to matter.
                if reaches_box(&site, min, max) {
                    found.push(site);
                }
            }
        }
    }
    found
}

/// Could this site's plateau touch the column box?
fn reaches_box(site: &TownSite, min: (i32, i32), max: (i32, i32)) -> bool {
    let span = site.core_half + site.skirt();
    site.centre.0 + span >= min.0
        && site.centre.0 - span <= max.0
        && site.centre.1 + span >= min.1
        && site.centre.1 - span <= max.1
}

/// Towns whose centre lies within `radius` of a column, nearest first.
///
/// What the beacon board and the map pins enumerate over — and it loads
/// nothing, which is the whole point.
pub fn towns_near(
    seed: u64,
    at: (i32, i32),
    radius: i32,
    natural_height_at: &impl Fn(i32, i32) -> i32,
) -> Vec<TownSite> {
    let lo_x = (at.0 - radius).div_euclid(CELL);
    let hi_x = (at.0 + radius).div_euclid(CELL);
    let lo_z = (at.1 - radius).div_euclid(CELL);
    let hi_z = (at.1 + radius).div_euclid(CELL);

    let reach = (radius as i64) * (radius as i64);
    let mut found: Vec<TownSite> = Vec::new();
    for cell_x in lo_x..=hi_x {
        for cell_z in lo_z..=hi_z {
            let Some(site) = site_in_cell(seed, cell_x, cell_z, natural_height_at) else {
                continue;
            };
            if distance_squared(site.centre, at) <= reach {
                found.push(site);
            }
        }
    }
    found.sort_by_key(|site| distance_squared(site.centre, at));
    found
}

fn distance_squared(a: (i32, i32), b: (i32, i32)) -> i64 {
    let dx = (a.0 - b.0) as i64;
    let dz = (a.1 - b.1) as i64;
    dx * dx + dz * dz
}

/// Euclidean distance from a column to a site's flat core. Zero inside.
///
/// Euclidean rather than Chebyshev so the skirt wraps corners in smooth arcs
/// instead of creased diagonals.
fn distance_to_core(site: &TownSite, x: i32, z: i32) -> f32 {
    let dx = ((x - site.centre.0).abs() - site.core_half).max(0) as f32;
    let dz = ((z - site.centre.1).abs() - site.core_half).max(0) as f32;
    (dx * dx + dz * dz).sqrt()
}

/// The site whose flat core this column stands on, if any.
pub fn core_contains(sites: &[TownSite], x: i32, z: i32) -> Option<&TownSite> {
    sites
        .iter()
        .find(|site| distance_to_core(site, x, z) <= 0.0)
}

/// Is this column inside any gathered site's core or skirt?
pub fn footprint_contains(sites: &[TownSite], x: i32, z: i32) -> bool {
    sites
        .iter()
        .any(|site| distance_to_core(site, x, z) < site.skirt() as f32)
}

/// The natural height with the nearest overlapping town's plateau blended in.
///
/// Sites are far enough apart that at most one is ever in range of a column —
/// a test pins that down — so "nearest" is really "the only one".
pub fn blend_height(sites: &[TownSite], x: i32, z: i32, natural: i32) -> i32 {
    let Some((site, distance)) = sites
        .iter()
        .map(|site| (site, distance_to_core(site, x, z)))
        .filter(|(site, distance)| *distance < site.skirt() as f32)
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return natural;
    };

    if distance <= 0.0 {
        return site.ground;
    }
    let t = distance / site.skirt() as f32;
    let smooth = t * t * (3.0 - 2.0 * t);
    site.ground + ((natural - site.ground) as f32 * smooth).round() as i32
}

/// Where this town's beacon console stands.
pub fn beacon_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::beacon_offset(site);
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

/// Where this town's trading counter stands.
pub fn counter_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::counter_offset(site);
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

/// Where the player's chest stands. Only the hometown has the house.
pub fn chest_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::chest_offset();
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

/// The mailbox outside the player's door.
pub fn mailbox_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::mailbox_offset();
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

/// The shop's doorway — the gap you go in through to reach the counter.
pub fn shop_door_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::shop_door_offset();
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

/// Where a customer stands to trade at this town's counter.
pub fn counter_stand_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::counter_stand_offset();
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

/// The doorway of the player's house — the gap you leave through.
pub fn door_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::door_offset();
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

/// Where a new player wakes up: on their house's floor, inside.
pub fn spawn_position(site: &TownSite) -> BlockPos {
    let (x, z) = plan::spawn_offset();
    BlockPos::new(site.centre.0 + x, site.ground + 1, site.centre.1 + z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(_: i32, _: i32) -> i32 {
        90
    }

    #[test]
    fn the_home_town_is_the_same_in_every_seed() {
        // The whole promise of the starting town: one hometown, every world.
        let a = towns_near(1, (0, 0), 100, &flat);
        let b = towns_near(2024, (0, 0), 100, &flat);
        let c = towns_near(u64::MAX, (0, 0), 100, &flat);
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a.first().map(|site| site.centre), Some((0, 0)));
        assert!(a[0].is_home());
        assert_eq!(a[0].ground, HOME_GROUND_Y);
    }

    #[test]
    fn the_core_is_flat_and_the_far_field_is_untouched() {
        let sites = [home_site()];
        for (x, z) in [(0, 0), (26, 26), (-26, 13), (9, -26)] {
            assert_eq!(blend_height(&sites, x, z, 95), HOME_GROUND_Y, "core ({x},{z})");
        }
        let far = HOME_CORE_HALF + SKIRT;
        for (x, z) in [(far, 0), (0, -far), (far + 40, far + 40), (-400, 4)] {
            for natural in [1, 62, 72, 140] {
                assert_eq!(blend_height(&sites, x, z, natural), natural, "wild ({x},{z})");
            }
        }
    }

    #[test]
    fn the_skirt_blends_without_cliffs() {
        let sites = [home_site()];
        for natural in [30, 95, 140] {
            let mut previous = blend_height(&sites, HOME_CORE_HALF, 0, natural);
            for x in HOME_CORE_HALF + 1..HOME_CORE_HALF + SKIRT + 4 {
                let here = blend_height(&sites, x, 0, natural);
                assert!(
                    (here - previous).abs() <= 5,
                    "cliff of {} at x={x} toward {natural}",
                    (here - previous).abs()
                );
                let (lo, hi) = if natural < HOME_GROUND_Y {
                    (natural, HOME_GROUND_Y)
                } else {
                    (HOME_GROUND_Y, natural)
                };
                assert!((lo..=hi).contains(&here), "overshoot at x={x}");
                previous = here;
            }
        }
    }

    #[test]
    fn the_footprint_covers_core_and_skirt_only() {
        let sites = [home_site()];
        assert!(footprint_contains(&sites, 0, 0));
        assert!(footprint_contains(&sites, HOME_CORE_HALF + SKIRT - 1, 0));
        assert!(!footprint_contains(&sites, HOME_CORE_HALF + SKIRT, 0));
        assert!(core_contains(&sites, 0, 0).is_some());
        assert!(core_contains(&sites, HOME_CORE_HALF + 1, 0).is_none());
    }

    #[test]
    fn the_clip_window_is_exact() {
        // The forest's test shape: a small window agrees with a wide sweep in
        // both directions.
        let wide = towns_overlapping(7, (-2000, -2000), (2000, 2000), &flat);
        let window = towns_overlapping(7, (0, 0), (15, 15), &flat);
        for site in &window {
            assert!(wide.contains(site), "window found a town the sweep missed");
        }
        for site in &wide {
            let reaches = reaches_box(site, (0, 0), (15, 15));
            assert_eq!(reaches, window.contains(site), "clip window disagrees on {site:?}");
        }
    }

    /// A real generator's natural field, for tests that need honest terrain.
    fn generator(seed: u64) -> crate::gen::TerrainGenerator {
        let mut registry = vx_core::BlockRegistry::new();
        let blocks = crate::gen::TerrainBlocks::register_builtins(&mut registry);
        crate::gen::TerrainGenerator::new(seed, blocks)
    }

    #[test]
    fn the_frontier_is_neither_empty_nor_crowded() {
        let generator = generator(2024);
        let towns = generator.towns_near((0, 0), 4000);
        assert!(towns.len() > 8, "only {} towns in 4 km", towns.len());
        assert!(towns.len() < 250, "{} towns in 4 km is a suburb", towns.len());
    }

    #[test]
    fn towns_never_overlap_each_other() {
        // Separation falls out of the jitter clamp rather than a rejection
        // pass, so it is worth asserting rather than trusting.
        let generator = generator(2024);
        let towns = generator.towns_near((0, 0), 4000);
        for (index, a) in towns.iter().enumerate() {
            for b in &towns[index + 1..] {
                let gap = distance_squared(a.centre, b.centre);
                // Each site's own footprint, not twice the global margin:
                // `REACH` is the *gather* contract and is sized for the
                // largest thing on the lattice, so measuring two hamlets
                // against it asks them to be as far apart as two cities.
                let needed = (a.core_half + a.skirt() + b.core_half + b.skirt()) as i64;
                assert!(
                    gap > needed * needed,
                    "{} and {} are {gap} apart squared, closer than two footprints",
                    a.name,
                    b.name
                );
            }
        }
    }

    #[test]
    fn a_column_belongs_to_at_most_one_town() {
        let generator = generator(7);
        let sites = generator.towns_overlapping((-2000, -2000), (2000, 2000));
        for site in &sites {
            let inside = sites
                .iter()
                .filter(|other| {
                    distance_to_core(other, site.centre.0, site.centre.1) < SKIRT as f32
                })
                .count();
            assert_eq!(inside, 1, "{} shares its centre with another town", site.name);
        }
    }

    #[test]
    fn towns_stay_out_of_the_sea_and_off_the_cliffs() {
        let generator = generator(31337);
        for site in generator.towns_near((0, 0), 4000) {
            assert!(
                site.ground > SEA_LEVEL + MIN_DRY,
                "{} has its feet in the water at y={}",
                site.name,
                site.ground
            );
            let natural = |x: i32, z: i32| generator.natural_height_at(x, z);
            let half = site.core_half;
            let corners = [
                natural(site.centre.0 - half, site.centre.1 - half),
                natural(site.centre.0 + half, site.centre.1 - half),
                natural(site.centre.0 - half, site.centre.1 + half),
                natural(site.centre.0 + half, site.centre.1 + half),
            ];
            let spread = corners.iter().max().unwrap() - corners.iter().min().unwrap();
            // The city levels more than a town will: it is a plot nine times
            // the area and whoever raised the great star had a state behind
            // them. See `CITY_RELIEF`.
            let allowed = if site.is_city() { CITY_RELIEF } else { MAX_RELIEF };
            assert!(
                spread <= allowed,
                "{} was built across {spread} blocks of relief",
                site.name
            );
        }
    }

    #[test]
    fn the_frontier_is_varied() {
        let generator = generator(2024);
        let towns = generator.towns_near((0, 0), 4000);
        let names: std::collections::HashSet<String> =
            towns.iter().map(|site| site.name.to_string()).collect();
        assert!(names.len() > 5, "only {} distinct names", names.len());

        // The three frontier trades. The city is not one of them — it is not
        // a trade, it is the far end of the network — and whether it happens
        // to be inside this radius is the seed's business.
        let specialities: std::collections::HashSet<Speciality> = towns
            .iter()
            .filter(|site| !site.is_city())
            .map(|site| site.speciality)
            .collect();
        assert_eq!(specialities.len(), 3, "not every trade is represented");

        let sizes: std::collections::HashSet<i32> =
            towns.iter().map(|site| site.core_half).collect();
        assert!(sizes.len() > 1, "every town is the same size");
    }

    #[test]
    fn siting_reads_the_natural_field_not_its_own_output() {
        // The circularity guard: a town's ground must equal the *natural*
        // height at its centre, never the blended height its own plateau
        // produces. If this ever fails, siting has started feeding itself.
        let generator = generator(555);
        for site in generator.towns_near((0, 0), 3000) {
            if site.is_home() {
                continue;
            }
            let natural = generator.natural_height_at(site.centre.0, site.centre.1);
            assert_eq!(
                site.ground,
                natural.clamp(SEA_LEVEL + MIN_DRY + 1, 140),
                "{} was sited against terrain it had already flattened",
                site.name
            );
        }
    }

    #[test]
    fn finding_towns_loads_no_chunks() {
        // The whole reason a beacon can name a town nobody has visited.
        let world = crate::World::new(2024);
        let before = world.loaded_chunks().count();
        let towns = world.generator().towns_near((0, 0), 4000);
        assert!(!towns.is_empty());
        assert_eq!(
            world.loaded_chunks().count(),
            before,
            "enumerating towns generated terrain"
        );
    }

    #[test]
    fn names_are_stable_and_drawable() {
        // The bitmap font has no lower case; a name it cannot draw would show
        // as placeholder boxes.
        let site = home_site();
        assert_eq!(site.name.to_string(), site.name.to_string());
        for character in site.name.to_string().chars() {
            assert!(
                character.is_ascii_uppercase(),
                "name has a character the font cannot draw: {character:?}"
            );
        }
    }
    /// **There is exactly one Ruined City, and it is where the seed says.**
    ///
    /// The load-bearing claim of the whole round. A world with two of them is
    /// a world where the word "the" is a lie; a world with none is one where
    /// the map pins a place that is not there.
    #[test]
    fn every_world_has_exactly_one_city_and_it_is_a_long_way_out() {
        let ground = |_: i32, _: i32| 96;
        for seed in [1, 7, 99, 909, 2024, 4242, 60_003, u64::MAX] {
            let cities: Vec<TownSite> = towns_near(seed, (0, 0), CELL * (CITY_RING + 4), &ground)
                .into_iter()
                .filter(|site| site.is_city())
                .collect();
            assert_eq!(cities.len(), 1, "seed {seed} has {} cities", cities.len());

            let city = cities[0];
            assert!(!city.is_home(), "the city took the hometown's cell");
            assert_eq!(city.core_half, CITY_CORE_HALF);

            // On the ring, and therefore a trip: three to four kilometres,
            // which is the distance this round was asked for.
            let out = ((city.centre.0 as f64).hypot(city.centre.1 as f64)) as i32;
            // Three to four kilometres, every time. The cell is on the ring
            // to within [`RING_BAND`]; the remaining spread is where inside
            // that cell the ground lets the city settle, which is worth a
            // quarter of a kilometre either way — see `CITY_TRIES`.
            assert!(
                (2_900..=4_200).contains(&out),
                "seed {seed} put the city {out} blocks out, off the ring"
            );
        }
    }

    /// The city is derived, like everything else on the lattice: same seed,
    /// same city, however often it is asked and from wherever.
    #[test]
    fn the_city_is_the_same_city_however_it_is_asked_for() {
        let ground = |x: i32, z: i32| 90 + ((x / 97 + z / 89) % 17);
        let cell = city_cell(2024);
        let here = (cell.0 * CELL + CELL / 2, cell.1 * CELL + CELL / 2);
        let near = towns_near(2024, here, CELL, &ground)
            .into_iter()
            .find(|site| site.is_city())
            .expect("no city in its own cell");
        for probe in [
            (here.0 - 400, here.1 - 400),
            (here.0 + 400, here.1 + 400),
            (here.0, here.1 + 300),
        ] {
            let again = towns_overlapping(2024, probe, probe, &ground)
                .into_iter()
                .find(|site| site.is_city());
            if let Some(again) = again {
                assert_eq!(again, near, "two answers for one city");
            }
        }
    }

    /// **The gather margin covers the widest wall any site can produce.**
    ///
    /// Every caller expands its box by `REACH` before gathering, so a site
    /// that reaches further is a site two neighbouring chunks disagree about.
    /// Worth an assertion rather than a comment: a six-point trace has always
    /// reached further than `core_half + SKIRT` and it was `flora`'s canopy
    /// margin, folded in for an unrelated reason, that was covering it.
    #[test]
    fn the_gather_margin_covers_the_widest_wall() {
        let ground = |_: i32, _: i32| 96;
        for seed in [7, 2024, 4242] {
            for site in towns_near(seed, (0, 0), CELL * (CITY_RING + 4), &ground) {
                let widest = crate::fort::forts_for(&site)
                    .map(|wall| wall.reach())
                    .max()
                    .unwrap_or(0);
                assert!(
                    widest <= REACH,
                    "{} reaches {widest}, past the {REACH} block gather margin",
                    site.name
                );
                assert!(
                    site.core_half + site.skirt() <= REACH,
                    "{}'s plateau reaches past the gather margin",
                    site.name
                );
            }
        }
    }

    /// A big plot eases into the country rather than ending in a step.
    #[test]
    fn the_citys_plateau_is_flat_inside_and_blends_all_the_way_out() {
        let ground = |x: i32, z: i32| 88 + ((x.rem_euclid(211) + z.rem_euclid(173)) / 12);
        let cell = city_cell(2024);
        let here = (cell.0 * CELL + CELL / 2, cell.1 * CELL + CELL / 2);
        let city = towns_near(2024, here, CELL, &ground)
            .into_iter()
            .find(|site| site.is_city())
            .expect("no city");
        let sites = [city];

        // Flat inside.
        for step in [-CITY_CORE_HALF, -40, 0, 40, CITY_CORE_HALF] {
            let at = blend_height(&sites, city.centre.0 + step, city.centre.1, 0);
            assert_eq!(at, city.ground, "the plot is not level at {step}");
        }
        // And monotone out to the far edge of the skirt, rather than a cliff
        // at the core's own line.
        let mut last = city.ground;
        for out in CITY_CORE_HALF..CITY_CORE_HALF + CITY_SKIRT {
            let x = city.centre.0 + out;
            let natural = ground(x, city.centre.1);
            let blended = blend_height(&sites, x, city.centre.1, natural);
            assert!(
                (blended - last).abs() <= 4,
                "the skirt steps {} blocks at {out}",
                blended - last
            );
            last = blended;
        }
        assert_eq!(
            blend_height(
                &sites,
                city.centre.0 + CITY_CORE_HALF + CITY_SKIRT,
                city.centre.1,
                123
            ),
            123,
            "the plateau never lets go of the country"
        );
    }

}
