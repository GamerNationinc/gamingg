//! The radio network: what the mast at the centre of a town is *for*.
//!
//! # Postings are derived, not stored
//!
//! A town's board is a pure function of that town's seed and its neighbours,
//! exactly as the town itself is a pure function of the world seed and a
//! lattice cell. Walk away and come back and the board reads the same; two
//! players on the same seed see the same work. Nothing about a board is
//! written to disk, and asking a town three kilometres away what it is
//! offering costs a handful of hashes and **loads no chunks** — which is what
//! makes a posting for a town that has never been generated possible at all.
//!
//! # The ledger is the only state
//!
//! What *is* saved is the player's side of it: which postings they took, which
//! they settled, and which towns they have actually stood in. That is player
//! knowledge rather than world truth, so it lives in its own tolerant file
//! beside the map and the wallet — a damaged ledger costs you your contracts,
//! never your world.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;

use vx_world::town::{Speciality, TownSite};

const MAGIC: &[u8; 4] = b"VXPO";
const VERSION: u32 = 1;

/// How many postings a board shows at once.
const MIN_POSTINGS: usize = 2;
const MAX_POSTINGS: usize = 4;

/// How close you have to get to a town centre before you have "found" it.
pub const DISCOVERY_RANGE: i32 = 60;

/// Goods the network hauls, and what it pays per unit.
///
/// Deliberately pays over the shop counter ([`crate::shop::sell_price`]) —
/// a contract is worth the walk. `engine:stone` has no counter price at all,
/// so hauling is the only thing spoil is good for.
const FREIGHT: [(&str, u64); 3] = [
    ("engine:copper_ore", 11),
    ("engine:log", 3),
    ("engine:stone", 2),
];

/// Another town, named and placed, without carrying the whole site around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TownRef {
    pub centre: (i32, i32),
    pub name: String,
}

impl TownRef {
    pub fn of(site: &TownSite) -> Self {
        TownRef {
            centre: site.centre,
            name: site.name.to_string(),
        }
    }
}

/// What a posting asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    /// Register freight with the network and sign for it at the far end.
    Deliver {
        goods: String,
        amount: u64,
        target: TownRef,
    },
    /// Have the flier sweep the sector containing this column.
    Survey { at: (i32, i32) },
}

/// One line of work on a town's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    /// Stable across sessions: hashed from the issuer and the slot, so the
    /// ledger can name a posting without storing the town that made it.
    pub id: u64,
    /// The town whose mast broadcast it.
    pub issuer: (i32, i32),
    pub task: Task,
    pub reward: u64,
}

impl Posting {
    /// One line for the board.
    pub fn title(&self) -> String {
        match &self.task {
            Task::Deliver {
                goods,
                amount,
                target,
            } => format!(
                "HAUL {amount} {} TO {}",
                crate::shop::display_name(goods),
                target.name
            ),
            Task::Survey { at } => format!("SURVEY SECTOR {} {}", at.0, at.1),
        }
    }

    /// Is this settled at the town that posted it, or at the far end?
    pub fn settles_at(&self) -> (i32, i32) {
        match &self.task {
            // Freight is signed for on arrival; that walk is the gameplay.
            Task::Deliver { target, .. } => target.centre,
            // A survey is reported back to whoever asked for it.
            Task::Survey { .. } => self.issuer,
        }
    }
}

/// The splitmix64 finalizer the ore, flora and town lattices use, so postings
/// are hashed the same way as everything else derived from a seed.
fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn stream(site: &TownSite, salt: u64) -> u64 {
    mix(site.seed
        ^ mix(salt)
        ^ (site.centre.0 as i64 as u64).wrapping_mul(0x2545_f491_4f6c_dd1d)
        ^ (site.centre.1 as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// What a town is short of, and therefore what it pays to have brought in.
fn wants(speciality: Speciality) -> &'static str {
    match speciality {
        // A depot moves anything.
        Speciality::Depot => "engine:copper_ore",
        // A camp burns timber for props and fires.
        Speciality::Mine => "engine:log",
        // A refinery eats aggregate.
        Speciality::Refinery => "engine:stone",
    }
}

fn rate(goods: &str) -> u64 {
    FREIGHT
        .iter()
        .find(|(name, _)| *name == goods)
        .map_or(1, |(_, rate)| *rate)
}

fn distance(a: (i32, i32), b: (i32, i32)) -> i64 {
    let dx = (a.0 - b.0) as i64;
    let dz = (a.1 - b.1) as i64;
    (((dx * dx + dz * dz) as f64).sqrt()) as i64
}

/// The board this town is showing.
///
/// `neighbours` is what the mast can hear — pass the towns within radio range
/// (see [`vx_world::town::towns_near`]); an empty list is fine and yields a
/// board of survey work only, which is exactly what an isolated town would
/// have to offer.
pub fn postings_for(site: &TownSite, neighbours: &[TownSite]) -> Vec<Posting> {
    let reachable: Vec<&TownSite> = neighbours
        .iter()
        .filter(|other| other.centre != site.centre)
        .collect();

    let count = MIN_POSTINGS
        + (stream(site, 0) % (MAX_POSTINGS - MIN_POSTINGS + 1) as u64) as usize;

    let mut postings = Vec::with_capacity(count);
    for slot in 0..count {
        let roll = stream(site, 1 + slot as u64);
        let id = mix(roll ^ 0x5bf0_3635_ca62_9163);

        // Every third slot is survey work, and a town with nobody to trade
        // with posts nothing else.
        let survey = reachable.is_empty() || roll.is_multiple_of(3);
        let task = if survey {
            let angle = (roll >> 8) as f64 / u64::MAX as f64 * std::f64::consts::TAU;
            let far = 320 + ((roll >> 20) % 640) as i32;
            Task::Survey {
                at: (
                    site.centre.0 + (angle.cos() * far as f64) as i32,
                    site.centre.1 + (angle.sin() * far as f64) as i32,
                ),
            }
        } else {
            let target = reachable[(roll >> 12) as usize % reachable.len()];
            let goods = wants(target.speciality).to_string();
            let amount = 20 + ((roll >> 32) % 60);
            Task::Deliver {
                amount,
                goods,
                target: TownRef::of(target),
            }
        };

        let reward = match &task {
            Task::Deliver {
                goods,
                amount,
                target,
            } => amount * rate(goods) + (distance(site.centre, target.centre) / 4) as u64,
            Task::Survey { at } => 120 + (distance(site.centre, *at) / 3) as u64,
        };

        postings.push(Posting {
            id,
            issuer: site.centre,
            task,
            reward,
        });
    }
    postings
}

/// The player's side of the network.
#[derive(Debug, Default)]
pub struct Ledger {
    accepted: Vec<Posting>,
    settled: HashSet<u64>,
    visited: HashSet<(i32, i32)>,
}

impl Ledger {
    pub fn new() -> Self {
        Ledger::default()
    }

    /// Take a posting. Accepting one twice, or re-taking one already settled,
    /// is a no-op rather than a duplicate.
    pub fn accept(&mut self, posting: &Posting) -> bool {
        if self.settled.contains(&posting.id) || self.is_accepted(posting.id) {
            return false;
        }
        self.accepted.push(posting.clone());
        true
    }

    pub fn is_accepted(&self, id: u64) -> bool {
        self.accepted.iter().any(|posting| posting.id == id)
    }

    pub fn is_settled(&self, id: u64) -> bool {
        self.settled.contains(&id)
    }

    pub fn accepted(&self) -> &[Posting] {
        &self.accepted
    }

    pub fn get(&self, id: u64) -> Option<&Posting> {
        self.accepted.iter().find(|posting| posting.id == id)
    }

    /// Mark a posting done and take it off the sheet.
    pub fn settle(&mut self, id: u64) {
        self.accepted.retain(|posting| posting.id != id);
        self.settled.insert(id);
    }

    /// Stand in a town: from here on, the map knows where it is.
    pub fn visit(&mut self, centre: (i32, i32)) -> bool {
        self.visited.insert(centre)
    }

    pub fn knows(&self, centre: (i32, i32)) -> bool {
        self.visited.contains(&centre)
    }

    /// Towns the player has actually been to, in a stable order.
    pub fn visited(&self) -> Vec<(i32, i32)> {
        let mut towns: Vec<(i32, i32)> = self.visited.iter().copied().collect();
        towns.sort();
        towns
    }

    /// Where the outstanding work is, for the map to pin — including targets
    /// in territory the player has never seen. Markers draw over unexplored
    /// ground by design, so a pin in the black is the whole point.
    pub fn pins(&self) -> Vec<(i32, i32)> {
        self.accepted()
            .iter()
            .map(|posting| posting.settles_at())
            .collect()
    }

    /// Postings that can be handed in at this town.
    pub fn due_at(&self, centre: (i32, i32)) -> Vec<&Posting> {
        self.accepted
            .iter()
            .filter(|posting| posting.settles_at() == centre)
            .collect()
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "postings.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;

        file.write_all(&(self.accepted.len() as u32).to_le_bytes())?;
        for posting in &self.accepted {
            write_posting(&mut file, posting)?;
        }

        let mut settled: Vec<u64> = self.settled.iter().copied().collect();
        settled.sort_unstable();
        file.write_all(&(settled.len() as u32).to_le_bytes())?;
        for id in settled {
            file.write_all(&id.to_le_bytes())?;
        }

        let visited = self.visited();
        file.write_all(&(visited.len() as u32).to_le_bytes())?;
        for (x, z) in visited {
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
        }
        file.commit()
    }

    /// Read the ledger back, tolerating absence and damage: a broken sheet is
    /// an empty sheet, logged, never a failed world.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("postings.dat");
        match read_ledger(&path) {
            Ok(Some(ledger)) => *self = ledger,
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting a fresh sheet", path.display());
            }
        }
    }
}

fn write_posting(file: &mut impl Write, posting: &Posting) -> std::io::Result<()> {
    file.write_all(&posting.id.to_le_bytes())?;
    file.write_all(&posting.issuer.0.to_le_bytes())?;
    file.write_all(&posting.issuer.1.to_le_bytes())?;
    file.write_all(&posting.reward.to_le_bytes())?;
    match &posting.task {
        Task::Deliver {
            goods,
            amount,
            target,
        } => {
            file.write_all(&[0u8])?;
            write_string(file, goods)?;
            file.write_all(&amount.to_le_bytes())?;
            file.write_all(&target.centre.0.to_le_bytes())?;
            file.write_all(&target.centre.1.to_le_bytes())?;
            write_string(file, &target.name)?;
        }
        Task::Survey { at } => {
            file.write_all(&[1u8])?;
            file.write_all(&at.0.to_le_bytes())?;
            file.write_all(&at.1.to_le_bytes())?;
        }
    }
    Ok(())
}

fn write_string(file: &mut impl Write, text: &str) -> std::io::Result<()> {
    file.write_all(&(text.len() as u32).to_le_bytes())?;
    file.write_all(text.as_bytes())
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_i32(file: &mut impl Read) -> std::io::Result<i32> {
    Ok(read_u32(file)? as i32)
}

fn read_u64(file: &mut impl Read) -> std::io::Result<u64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

fn read_string(file: &mut impl Read) -> std::io::Result<String> {
    let length = read_u32(file)? as usize;
    // A damaged length must not ask for a gigabyte before it is rejected.
    if length > 256 {
        return Err(std::io::Error::other("implausible string length"));
    }
    let mut bytes = vec![0u8; length];
    file.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("name is not text"))
}

fn read_posting(file: &mut impl Read) -> std::io::Result<Posting> {
    let id = read_u64(file)?;
    let issuer = (read_i32(file)?, read_i32(file)?);
    let reward = read_u64(file)?;
    let mut tag = [0u8; 1];
    file.read_exact(&mut tag)?;
    let task = match tag[0] {
        0 => {
            let goods = read_string(file)?;
            let amount = read_u64(file)?;
            let centre = (read_i32(file)?, read_i32(file)?);
            let name = read_string(file)?;
            Task::Deliver {
                goods,
                amount,
                target: TownRef { centre, name },
            }
        }
        1 => Task::Survey {
            at: (read_i32(file)?, read_i32(file)?),
        },
        other => return Err(std::io::Error::other(format!("unknown task kind {other}"))),
    };
    Ok(Posting {
        id,
        issuer,
        task,
        reward,
    })
}

fn read_ledger(path: &Path) -> std::io::Result<Option<Ledger>> {
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
    if read_u32(&mut file)? != VERSION {
        return Err(std::io::Error::other("unknown version"));
    }

    let mut ledger = Ledger::new();
    let accepted = read_u32(&mut file)?;
    for _ in 0..accepted {
        ledger.accepted.push(read_posting(&mut file)?);
    }
    let settled = read_u32(&mut file)?;
    for _ in 0..settled {
        ledger.settled.insert(read_u64(&mut file)?);
    }
    let visited = read_u32(&mut file)?;
    for _ in 0..visited {
        ledger.visited.insert((read_i32(&mut file)?, read_i32(&mut file)?));
    }
    Ok(Some(ledger))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_world::town;

    /// Towns from a flat world, so the fixtures do not depend on the noise.
    fn frontier() -> Vec<TownSite> {
        town::towns_near(7, (0, 0), 2_000, &|_, _| 90)
    }

    fn home_and_neighbours() -> (TownSite, Vec<TownSite>) {
        let sites = frontier();
        let home = town::home_site();
        (home, sites)
    }

    #[test]
    fn a_board_reads_the_same_on_every_visit() {
        let (home, neighbours) = home_and_neighbours();
        let first = postings_for(&home, &neighbours);
        let second = postings_for(&home, &neighbours);
        assert_eq!(first, second, "a board changed between two visits");
        assert!(
            (MIN_POSTINGS..=MAX_POSTINGS).contains(&first.len()),
            "a board of {} postings", first.len()
        );
        // Ids are distinct, or the ledger could not tell two jobs apart.
        let mut ids: Vec<u64> = first.iter().map(|posting| posting.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), first.len(), "two postings share an id");
    }

    #[test]
    fn different_towns_post_different_work() {
        let neighbours = frontier();
        let boards: Vec<Vec<Posting>> = neighbours
            .iter()
            .take(4)
            .map(|site| postings_for(site, &neighbours))
            .collect();
        assert!(boards.len() >= 2, "the fixture frontier is too small to compare");
        assert!(
            boards.windows(2).any(|pair| pair[0] != pair[1]),
            "every town posted the same board"
        );
        // Every posting is attributed to the town that made it.
        for (site, board) in neighbours.iter().take(4).zip(&boards) {
            for posting in board {
                assert_eq!(posting.issuer, site.centre);
                assert!(posting.reward > 0, "unpaid work");
            }
        }
    }

    #[test]
    fn a_town_never_posts_freight_to_itself() {
        let neighbours = frontier();
        for site in &neighbours {
            for posting in postings_for(site, &neighbours) {
                if let Task::Deliver { target, .. } = &posting.task {
                    assert_ne!(
                        target.centre, site.centre,
                        "{} posted a haul to itself", site.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_town_with_nobody_to_talk_to_still_has_work() {
        let home = town::home_site();
        let alone = postings_for(&home, &[]);
        assert!(!alone.is_empty());
        assert!(
            alone.iter().all(|posting| matches!(posting.task, Task::Survey { .. })),
            "an isolated town posted freight with nowhere to send it"
        );
    }

    #[test]
    fn freight_settles_at_the_far_end_and_surveys_at_home() {
        let (home, neighbours) = home_and_neighbours();
        for posting in postings_for(&home, &neighbours) {
            match &posting.task {
                Task::Deliver { target, .. } => {
                    assert_eq!(posting.settles_at(), target.centre);
                    assert_ne!(
                        posting.settles_at(),
                        posting.issuer,
                        "a delivery you could sign for without moving"
                    );
                }
                Task::Survey { .. } => assert_eq!(posting.settles_at(), posting.issuer),
            }
        }
    }

    #[test]
    fn accepting_twice_is_a_no_op_and_settled_work_cannot_be_retaken() {
        let (home, neighbours) = home_and_neighbours();
        let posting = &postings_for(&home, &neighbours)[0];
        let mut ledger = Ledger::new();

        assert!(ledger.accept(posting));
        assert!(!ledger.accept(posting), "the same job was taken twice");
        assert_eq!(ledger.accepted().len(), 1);
        assert!(ledger.is_accepted(posting.id));

        ledger.settle(posting.id);
        assert!(ledger.is_settled(posting.id));
        assert!(ledger.accepted().is_empty());
        assert!(!ledger.accept(posting), "a finished job came back on the sheet");
    }

    #[test]
    fn the_sheet_pins_where_the_work_is_and_lists_what_is_due_here() {
        let (home, neighbours) = home_and_neighbours();
        let board = postings_for(&home, &neighbours);
        let mut ledger = Ledger::new();
        for posting in &board {
            ledger.accept(posting);
        }

        let pins = ledger.pins();
        assert_eq!(pins.len(), board.len());
        for posting in &board {
            assert!(pins.contains(&posting.settles_at()));
            assert!(
                ledger.due_at(posting.settles_at()).iter().any(|due| due.id == posting.id),
                "a posting is not due where it settles"
            );
        }
        // Nothing is due at a town with no work pointing at it.
        assert!(ledger.due_at((999_999, 999_999)).is_empty());
    }

    #[test]
    fn discovery_is_remembered_and_is_not_the_same_as_having_work_there() {
        let mut ledger = Ledger::new();
        assert!(!ledger.knows((512, -512)));
        assert!(ledger.visit((512, -512)), "the first visit did not register");
        assert!(!ledger.visit((512, -512)), "a second visit counted as new");
        assert!(ledger.knows((512, -512)));
        assert_eq!(ledger.visited(), vec![(512, -512)]);
        assert!(ledger.pins().is_empty(), "visiting a town invented a contract");
    }

    #[test]
    fn the_ledger_round_trips_and_tolerates_damage() {
        let directory = std::env::temp_dir().join(format!("vx-beacon-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let (home, neighbours) = home_and_neighbours();
        let board = postings_for(&home, &neighbours);
        let mut ledger = Ledger::new();
        for posting in &board {
            ledger.accept(posting);
        }
        ledger.settle(board[0].id);
        ledger.visit((0, 0));
        ledger.visit((-512, 1_024));
        ledger.save(&directory).unwrap();

        let mut read = Ledger::new();
        read.load(&directory);
        assert_eq!(read.accepted(), ledger.accepted(), "contracts did not survive the trip");
        assert!(read.is_settled(board[0].id));
        assert_eq!(read.visited(), vec![(-512, 1_024), (0, 0)]);

        std::fs::write(directory.join("postings.dat"), b"NOT A LEDGER AT ALL").unwrap();
        let mut damaged = Ledger::new();
        damaged.load(&directory);
        assert!(damaged.accepted().is_empty(), "a damaged sheet invented contracts");
        assert!(damaged.visited().is_empty());

        std::fs::remove_dir_all(&directory).ok();
        let mut missing = Ledger::new();
        missing.load(&directory);
        assert!(missing.accepted().is_empty());
    }

    #[test]
    fn enumerating_the_whole_frontier_loads_no_chunks() {
        // The point of deriving towns: a beacon can name a place three
        // kilometres away that has never existed as a single block.
        let probes = std::cell::Cell::new(0usize);
        let sites = town::towns_near(11, (0, 0), 3_000, &|_, _| {
            probes.set(probes.get() + 1);
            90
        });
        assert!(sites.len() > 4, "a 3 km sweep found only {} towns", sites.len());
        for site in &sites {
            assert!(!postings_for(site, &sites).is_empty());
        }
        // Height probes are arithmetic, not chunk loads; the count is bounded
        // by the number of lattice cells, not by the world.
        // The cost is bounded by the number of lattice cells swept, not by
        // the area: a few probes per cell to site and level a candidate. The
        // region itself holds 36 million columns.
        let cells = ((3_000 + 3_000) / town::CELL + 2).pow(2);
        let probes = probes.get();
        assert!(
            probes <= cells as usize * 8,
            "{probes} height probes over {cells} cells is not arithmetic any more"
        );
    }
}
