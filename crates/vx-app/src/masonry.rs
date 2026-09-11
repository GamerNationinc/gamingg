//! Masonry: the buildings a prospering town has actually put up.
//!
//! # Why growth is an *edit* and not worldgen
//!
//! [`vx_world::town::plan::stamp`] lays down a town's authored plan and
//! nothing else, and it is pure in `(seed, chunk position, sites)`. That
//! purity is what lets `save.rs` write only modified chunks — untouched
//! terrain costs the save nothing because it regenerates identically — and it
//! is what the world hash is a hash *of*.
//!
//! A town that grew by changing what worldgen produced would break both. Every
//! chunk generated before the town got rich would hold the old town; every
//! chunk regenerated after it went broke would lose the new one; and the
//! terrain under a bigger plan would have to be levelled differently, which
//! moves `core_half`, which moves the plateau, which moves the hash.
//!
//! So a growth building reaches the world the way a founded town's chunks do
//! and the way anything the player builds does: as blocks written into loaded
//! chunks, which mark themselves modified and save. Worldgen is told nothing
//! and keeps nothing.
//!
//! # Which means it has to be deferred
//!
//! [`vx_world::world::World::set_block`] needs the chunk to be resident, and a
//! town three thousand blocks away is not. So this file records how many
//! growth steps each town has actually had *built*, which is deliberately not
//! the same number as its [`crate::economy::Market::growth`]: a town grows on
//! its books wherever it is, and the sheds go up the next time somebody is
//! near enough for there to be somewhere to put them.
//!
//! That gap is the level of detail, and it is the same rule the freight
//! network already follows with its `reachable` list. Nothing is lost by
//! waiting — the ledger is the authority and the blocks catch up.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use vx_world::gen::TerrainBlocks;
use vx_world::town::{plan, TownSite};
use vx_world::world::World;

const MAGIC: &[u8; 4] = b"VXMS";
const VERSION: u32 = 1;

/// An upper bound on towns in the ledger, so a corrupt count cannot make us
/// allocate wildly before failing.
const MAX_TOWNS: u32 = 100_000;

/// How many growth steps each town has had built, keyed on its centre.
///
/// Keyed on the centre rather than on an index for the reason every persisted
/// thing in this game is name-keyed: a town's position is what it *is*, and
/// the lattice's ordering is not a promise.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Masonry {
    built: BTreeMap<(i32, i32), u8>,
}

/// What one pass of [`Masonry::raise_due`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Raised {
    /// The towns that put a building up, and what step it was.
    pub put_up: Vec<(TownSite, usize)>,
    /// Towns that have earned a building and are waiting for their ground to
    /// be loaded. Not a failure — see the module docs.
    pub waiting: usize,
}

impl Masonry {
    pub fn new() -> Self {
        Masonry::default()
    }

    /// How many growth steps this town has standing.
    pub fn built(&self, centre: (i32, i32)) -> u8 {
        self.built.get(&centre).copied().unwrap_or(0)
    }

    /// How many towns have anything built at all — the census line.
    pub fn towns(&self) -> usize {
        self.built.len()
    }

    /// Build everything these towns have earned and have the ground for.
    ///
    /// `earned` answers how many steps each site's books say it has grown. The
    /// caller owns the books; this file owns the blocks, and the two meet
    /// here.
    ///
    /// Idempotent by construction: the ledger says what is standing, and
    /// [`plan::growth_blocks`] is pure in `(site, step)`, so running this
    /// twice at the same growth writes nothing the second time.
    pub fn raise_due(
        &mut self,
        world: &mut World,
        sites: &[TownSite],
        earned: impl Fn(&TownSite) -> u8,
    ) -> Raised {
        let mut raised = Raised::default();
        for site in sites {
            let want = earned(site).min(plan::growth_steps(site) as u8);
            while self.built(site.centre) < want {
                let step = self.built(site.centre) as usize;
                if !self.raise_one(world, site, step) {
                    raised.waiting += 1;
                    break;
                }
                self.built.insert(site.centre, step as u8 + 1);
                raised.put_up.push((*site, step));
            }
        }
        raised
    }

    /// Lay one building, or report that its ground is not loaded.
    ///
    /// All or nothing: the chunks are checked before a single block is
    /// written, because half a warehouse standing in the air while the rest of
    /// it waits for a chunk is worse than no warehouse at all.
    fn raise_one(&self, world: &mut World, site: &TownSite, step: usize) -> bool {
        // Read off the live registry rather than cached, for the reason
        // `save.rs` gives about ids: a block's number is an index into
        // registration order, and holding one across a registry change is how
        // a warehouse gets built out of dirt.
        let Some(blocks) = TerrainBlocks::from_registry(world.registry()) else {
            return false;
        };
        let laid = plan::growth_blocks(site, step, &blocks);
        if laid.is_empty() {
            return false;
        }
        if laid.iter().any(|(at, _)| !world.is_loaded(at.chunk())) {
            return false;
        }
        for (at, block) in laid {
            world.set_block(at, block);
        }
        true
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "masonry.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.built.len() as u32).to_le_bytes())?;
        for ((x, z), steps) in &self.built {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&[*steps])?;
        }
        file.commit()
    }

    /// Read it back, tolerating absence and damage.
    ///
    /// A lost ledger is a frontier that has not built anything yet, which
    /// costs the player their sheds and nothing else: the books still say what
    /// each town has earned, so every one of them goes back up the next time
    /// the player walks past. Losing this file is the cheapest loss in the
    /// game, which is exactly why it is allowed to be its own file.
    pub fn load(&mut self, directory: &Path) {
        match read(&directory.join("masonry.dat")) {
            Ok(Some(masonry)) => *self = masonry,
            Ok(None) => {}
            Err(error) => log::warn!("ignoring damaged masonry ledger: {error}"),
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Masonry>> {
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
    if u32::from_le_bytes(word) != VERSION {
        return Err(std::io::Error::other("unknown version"));
    }
    file.read_exact(&mut word)?;
    let rows = u32::from_le_bytes(word);
    if rows > MAX_TOWNS {
        return Err(std::io::Error::other("more towns than there are"));
    }
    let mut built = BTreeMap::new();
    for _ in 0..rows {
        file.read_exact(&mut word)?;
        let x = i32::from_le_bytes(word);
        file.read_exact(&mut word)?;
        let z = i32::from_le_bytes(word);
        let mut steps = [0u8; 1];
        file.read_exact(&mut steps)?;
        // A ledger claiming more buildings than are authored would have the
        // town skip straight past the ones it never built.
        if steps[0] as usize > crate::economy::MAX_GROWTH as usize {
            return Err(std::io::Error::other("a town built past what is authored"));
        }
        built.insert((x, z), steps[0]);
    }
    Ok(Some(Masonry { built }))
}

/// The towns do business, and build what they have earned.
///
/// **The one copy.** `Session::advance` and the journal's `Advance` arm both
/// call it, which is what makes the books and the sheds part of the replay
/// oracle instead of a second implementation of them sitting beside it. Two
/// hand-written copies of a tick is how the last three of these drifted.
///
/// Once a dispatch window, not once a tick: `window` is the last one run, and
/// is updated here. Returns the player-owned loads that landed, so the caller
/// can pay for them — this file knows nothing about wallets either.
pub fn tick_the_network(
    economy: &mut crate::economy::Economy,
    masonry: &mut Masonry,
    world: &mut World,
    window: &mut u64,
    column: (i32, i32),
    now: u64,
) -> Vec<crate::economy::Shipment> {
    let due = now / crate::economy::DISPATCH_EVERY;
    if due == *window {
        return Vec::new();
    }
    *window = due;

    let reachable = world.towns_near(column, crate::RADIO_RANGE);
    let landed = economy.run(&reachable, now);

    // What each town's books say it has grown to, read out before the world is
    // borrowed mutably — reading a market brings it up to date, so the books
    // need `&mut` and cannot be held across `raise_due`.
    let earned: Vec<u8> = reachable
        .iter()
        .map(|site| economy.market(site, now).growth())
        .collect();
    masonry.raise_due(world, &reachable, |site| {
        reachable
            .iter()
            .position(|other| other.centre == site.centre)
            .map_or(0, |index| earned[index])
    });
    landed
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_world::town;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("vx-masonry-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// The hometown with its ground pulled in, which is the only state in
    /// which anything can be built.
    fn town_in_view() -> (World, TownSite) {
        let site = town::home_site();
        let mut world = World::new(2024);
        let centre = vx_core::BlockPos::new(site.centre.0, site.ground, site.centre.1).chunk();
        world.load_around(centre, 3);
        (world, site)
    }

    /// A town builds what its books say it has earned, and then stops.
    #[test]
    fn a_town_builds_what_it_has_earned_and_no_more() {
        let (mut world, site) = town_in_view();
        let mut masonry = Masonry::new();

        let raised = masonry.raise_due(&mut world, std::slice::from_ref(&site), |_| 2);
        assert_eq!(raised.put_up.len(), 2, "the town built the wrong number");
        assert_eq!(raised.waiting, 0, "a loaded town was left waiting");
        assert_eq!(masonry.built(site.centre), 2);

        // And the blocks are actually there.
        let blocks = TerrainBlocks::from_registry(world.registry()).unwrap();
        for step in 0..2 {
            for (at, block) in plan::growth_blocks(&site, step, &blocks) {
                assert_eq!(world.block(at), block, "step {step} did not land at {at:?}");
            }
        }
        // The third is authored and was not asked for.
        assert!(plan::growth_building(&site, 2).is_some());
        let third = plan::growth_blocks(&site, 2, &blocks);
        assert!(
            third.iter().any(|(at, block)| world.block(*at) != *block),
            "a building nobody paid for went up"
        );
    }

    /// Stamping twice at the same growth writes nothing the second time.
    ///
    /// The ledger is the authority and `growth_blocks` is pure in the site, so
    /// this falls out of the design — which is exactly why it is worth a test:
    /// the day it stops falling out, a town starts rebuilding its own sheds on
    /// every dispatch window.
    #[test]
    fn raising_the_same_town_twice_changes_nothing() {
        let (mut world, site) = town_in_view();
        let mut masonry = Masonry::new();
        masonry.raise_due(&mut world, std::slice::from_ref(&site), |_| 3);
        let edits = world.edit_count();

        let again = masonry.raise_due(&mut world, std::slice::from_ref(&site), |_| 3);
        assert!(again.put_up.is_empty(), "a shed was built twice");
        assert_eq!(world.edit_count(), edits, "a second pass wrote blocks");
        assert_eq!(masonry.built(site.centre), 3);
    }

    /// A town cannot be asked for more than has been authored for it, however
    /// rich its books get.
    #[test]
    fn a_town_cannot_build_past_its_own_table() {
        let (mut world, site) = town_in_view();
        let mut masonry = Masonry::new();
        masonry.raise_due(&mut world, std::slice::from_ref(&site), |_| 200);
        assert_eq!(
            masonry.built(site.centre) as usize,
            plan::growth_steps(&site)
        );
    }

    /// **A town grows wherever it is, and builds when you arrive.**
    ///
    /// The level of detail, as a test. A town three thousand blocks away has
    /// no chunks resident, so there is nowhere to put a building — and nothing
    /// is lost by that, because the books are the authority and the blocks
    /// catch up. Getting this wrong in the other direction is worse than it
    /// looks: a half-stamped warehouse standing in unloaded air.
    #[test]
    fn a_town_with_its_ground_unloaded_waits_and_builds_later() {
        let site = town::home_site();
        let mut world = World::new(2024);
        let mut masonry = Masonry::new();

        let waited = masonry.raise_due(&mut world, std::slice::from_ref(&site), |_| 1);
        assert!(waited.put_up.is_empty(), "a building went up in empty air");
        assert_eq!(waited.waiting, 1, "the town was not recorded as waiting");
        assert_eq!(masonry.built(site.centre), 0);
        assert_eq!(world.edit_count(), 0, "a waiting town wrote a block anyway");

        // Now walk over there.
        let centre = vx_core::BlockPos::new(site.centre.0, site.ground, site.centre.1).chunk();
        world.load_around(centre, 3);
        let arrived = masonry.raise_due(&mut world, std::slice::from_ref(&site), |_| 1);
        assert_eq!(arrived.put_up.len(), 1, "the town never caught up");
        assert_eq!(masonry.built(site.centre), 1);
    }

    #[test]
    fn the_ledger_round_trips() {
        let directory = scratch("round-trip");
        let mut masonry = Masonry::new();
        masonry.built.insert((0, 0), 2);
        masonry.built.insert((-4_096, 8_192), 3);
        masonry.save(&directory).unwrap();

        let mut read = Masonry::new();
        read.load(&directory);
        assert_eq!(read, masonry);
        assert_eq!(read.built((-4_096, 8_192)), 3, "a far town lost its sheds");
        assert_eq!(read.towns(), 2);
        std::fs::remove_dir_all(&directory).ok();
    }

    /// A missing ledger is a frontier that has not built yet, and a damaged
    /// one is the same — the books still say what every town has earned.
    #[test]
    fn a_missing_or_damaged_ledger_is_a_frontier_that_has_not_built_yet() {
        let directory = scratch("damaged");
        let mut fresh = Masonry::new();
        fresh.load(&directory);
        assert_eq!(fresh, Masonry::new());

        std::fs::write(directory.join("masonry.dat"), b"not a ledger at all").unwrap();
        let mut torn = Masonry::new();
        torn.load(&directory);
        assert_eq!(torn, Masonry::new());
        std::fs::remove_dir_all(&directory).ok();
    }

    /// A ledger claiming a town built more than is authored is refused, rather
    /// than letting a town skip the buildings it never put up.
    #[test]
    fn a_ledger_past_the_authored_table_is_refused() {
        let directory = scratch("overgrown");
        let mut wrong = Masonry::new();
        wrong.built.insert((0, 0), crate::economy::MAX_GROWTH + 1);
        wrong.save(&directory).unwrap();

        let mut read = Masonry::new();
        read.load(&directory);
        assert_eq!(read, Masonry::new(), "an impossible ledger was accepted");
        std::fs::remove_dir_all(&directory).ok();
    }
}
