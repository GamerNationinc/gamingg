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

/// Lattice cell size, in blocks: one candidate town per cell.
pub const CELL: i32 = 512;

/// How far a town's plateau blends out past its flat core.
pub const SKIRT: i32 = 24;

/// Core half-widths a town may take.
pub const MIN_CORE_HALF: i32 = 20;
pub const MAX_CORE_HALF: i32 = 34;

/// The furthest a town can influence a column: the gather margin.
pub const REACH: i32 = MAX_CORE_HALF + SKIRT;

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
}

impl Speciality {
    pub fn name(self) -> &'static str {
        match self {
            Speciality::Depot => "DEPOT",
            Speciality::Mine => "MINE",
            Speciality::Refinery => "REFINERY",
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
    highest - lowest <= MAX_RELIEF
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
    let span = site.core_half + SKIRT;
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
        .any(|site| distance_to_core(site, x, z) < SKIRT as f32)
}

/// The natural height with the nearest overlapping town's plateau blended in.
///
/// Sites are far enough apart that at most one is ever in range of a column —
/// a test pins that down — so "nearest" is really "the only one".
pub fn blend_height(sites: &[TownSite], x: i32, z: i32, natural: i32) -> i32 {
    let Some((site, distance)) = sites
        .iter()
        .map(|site| (site, distance_to_core(site, x, z)))
        .filter(|(_, distance)| *distance < SKIRT as f32)
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return natural;
    };

    if distance <= 0.0 {
        return site.ground;
    }
    let t = distance / SKIRT as f32;
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
                let needed = (2 * REACH) as i64;
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
            assert!(
                spread <= MAX_RELIEF,
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

        let specialities: std::collections::HashSet<Speciality> =
            towns.iter().map(|site| site.speciality).collect();
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
}
