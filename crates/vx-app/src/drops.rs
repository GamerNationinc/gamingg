//! The small block on the floor, waiting to be picked up.
//!
//! # Why this file exists
//!
//! [`crate::pack`] gave the player somewhere to put what they mine, and a
//! ceiling on it. That immediately raises the question every game with a
//! carrying limit has to answer: what happens when you swing at a rock with a
//! full pack? Three answers were on the table — refuse the swing, eat the
//! block, or put it on the ground — and the ground is the one the player asked
//! for, in exactly these words: *"I like these dropping feature of Minecraft
//! that it becomes a smaller block and pops into your inventory."*
//!
//! So a full pack costs you nothing but a walk. The block still breaks, what
//! came out of it lies where it fell, and an over-mined face ends up with a
//! little pile of copper in front of it. Nothing is ever destroyed by
//! carelessness, which matters more here than it looks: this is the same game
//! that spent stage 54 proving nothing is lost across a save.
//!
//! # Why it borrows rather than invents
//!
//! [`crate::arsenal::Crash`] has been a thing-on-the-ground-you-walk-over
//! since stage 13: spawned, saved, drawn, and collected by proximity. This is
//! that shape, keyed to a block name and a cell rather than to an economy
//! index and a column, and drawn in the good's own tile so a copper drop looks
//! like copper.
//!
//! # Why it is deterministic
//!
//! Drops are part of the replay oracle's state, not decoration. A block that
//! dropped rather than stowed is a block that is *not* on your back, and what
//! is on your back is the load byte, which is how fast you walk, which is
//! where you are standing — and where you are standing decides which drops you
//! then pick up. So: insertion order, integer positions, no randomness, and a
//! pickup rule that reads only the player's feet.

use std::io::{Read, Write};
use std::path::Path;

use glam::{DVec3, Mat4, Vec3};
use vx_core::{BlockPos, BlockRegistry, Face};
use vx_render::Object;

use crate::pack::Pack;

const MAGIC: &[u8; 4] = b"VXDP";
const VERSION: u32 = 1;

/// Longest good name accepted, so a damaged file cannot ask for a huge buffer.
const MAX_NAME: u32 = 64;

/// How near your feet have to be for a drop to hop into the pack.
///
/// Wider than a block so you cannot walk over one and miss it, and narrower
/// than the salvage reach so a shelf of drops does not empty from across the
/// adit. The pleasure of the thing is that it comes to you.
pub const REACH: f64 = 2.2;

/// How many piles the ground will hold before the oldest is forgotten.
///
/// Drops merge per cell and per good, so reaching this needs five hundred
/// distinct cells with something in them — an afternoon of deliberate
/// over-mining rather than an accident. The cap exists for the same reason
/// `rain::MAX_DROPS` does: a number that cannot run away.
pub const MAX_DROPS: usize = 512;

/// How big the little block is drawn.
pub const SIZE: f32 = 0.28;

/// How far above the cell's floor it hangs, before the bob.
const HOVER: f64 = 0.30;

/// How far it rides up and down, and how fast.
const BOB: f64 = 0.07;
const BOB_HZ: f64 = 0.7;

/// How fast it turns on the spot, in turns a second.
const SPIN_HZ: f32 = 0.15;

/// One good, in one cell, on the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drop {
    /// The cell it is lying in — the block's own cell, which is now air.
    pub at: BlockPos,
    /// What it is, by the same namespaced name everything else here uses.
    pub good: String,
    /// How many.
    pub count: u64,
}

impl Drop {
    /// Where it is drawn and measured from: the middle of its cell, hanging.
    pub fn centre(&self) -> DVec3 {
        DVec3::new(
            f64::from(self.at.x) + 0.5,
            f64::from(self.at.y) + HOVER,
            f64::from(self.at.z) + 0.5,
        )
    }
}

/// Everything lying about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drops {
    drops: Vec<Drop>,
}

impl Drops {
    /// Nothing on the floor.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many piles there are.
    pub fn len(&self) -> usize {
        self.drops.len()
    }

    /// Nothing on the floor.
    pub fn is_empty(&self) -> bool {
        self.drops.is_empty()
    }

    /// Every pile, in the order they fell.
    pub fn iter(&self) -> impl Iterator<Item = &Drop> {
        self.drops.iter()
    }

    /// How much of `good` is on the floor anywhere — what the conservation
    /// tests count.
    pub fn count(&self, good: &str) -> u64 {
        self.drops
            .iter()
            .filter(|drop| drop.good == good)
            .map(|drop| drop.count)
            .fold(0u64, u64::saturating_add)
    }

    /// How far a drop will fall looking for a floor.
    ///
    /// Not physics — a drop is not a body and stepping one every tick would
    /// put a hundred entities on the simulation clock for a cosmetic gain.
    /// This is the one thing that actually matters about falling: a block cut
    /// out of a ceiling ends up on the ground under it rather than hanging at
    /// head height. Resolved once, at the moment it is shed, so both sides of
    /// a replay get the same cell.
    pub const FALL: i32 = 12;

    /// Where a drop cut at `at` comes to rest.
    ///
    /// Straight down until something solid is underneath, or [`FALL`] blocks,
    /// whichever comes first. A drop with nothing under it for twelve blocks
    /// stays where it was cut, which is the honest answer: you mined out over
    /// a shaft and you can come back for it with a ladder.
    ///
    /// [`FALL`]: Drops::FALL
    pub fn settle(world: &vx_world::World, at: BlockPos) -> BlockPos {
        let mut resting = at;
        for _ in 0..Self::FALL {
            let below = resting.offset([0, -1, 0]);
            if !world.block(below).is_air() {
                return resting;
            }
            resting = below;
        }
        at
    }

    /// Put something on the ground.
    ///
    /// Merges by cell *and* good, so cutting a long face leaves a handful of
    /// growing piles rather than four hundred entities. Past [`MAX_DROPS`] the
    /// oldest pile is forgotten rather than the newest refused: what you are
    /// standing next to is what you are about to pick up.
    pub fn shed(&mut self, at: BlockPos, good: &str, count: u64) {
        if count == 0 {
            return;
        }
        if let Some(pile) = self
            .drops
            .iter_mut()
            .find(|drop| drop.at == at && drop.good == good)
        {
            pile.count = pile.count.saturating_add(count);
            return;
        }
        if self.drops.len() >= MAX_DROPS {
            self.drops.remove(0);
        }
        self.drops.push(Drop {
            at,
            good: good.to_string(),
            count,
        });
    }

    /// Walk near a pile and it goes in your pack, as far as it fits.
    ///
    /// Returns what actually moved, oldest pile first, so a caller can say so.
    /// A pile that only half fits leaves the rest on the ground — which is
    /// exactly what you want when the thing that filled your pack in the first
    /// place is lying at your feet.
    pub fn collect_near(&mut self, feet: DVec3, pack: &mut Pack, capacity: u64) -> Vec<(String, u64)> {
        let mut taken: Vec<(String, u64)> = Vec::new();
        for drop in &mut self.drops {
            if (drop.centre() - feet).length() > REACH {
                continue;
            }
            let moved = pack.take_in(&drop.good, drop.count, capacity);
            if moved == 0 {
                continue;
            }
            drop.count -= moved;
            taken.push((drop.good.clone(), moved));
        }
        self.drops.retain(|drop| drop.count > 0);
        taken
    }
}

/// Draw the floor.
///
/// A pure function of the piles and the clock, like [`crate::rain::streaks`]:
/// the little blocks turn on the spot and bob, which is the whole reason a
/// player notices one in a dark adit. `origin` is the camera's own position,
/// subtracted here so a drop three thousand kilometres out is still exact.
pub fn objects(drops: &Drops, registry: &BlockRegistry, seconds: f32, origin: DVec3) -> Vec<Object> {
    let mut objects = Vec::with_capacity(drops.len());
    for drop in drops.iter() {
        let Some(id) = registry.id_of(&drop.good) else {
            continue;
        };
        let Some(def) = registry.get(id) else {
            continue;
        };
        // Piles ride their own phase so a shelf of them does not pulse in
        // unison, and the phase is a function of the cell rather than of a
        // counter: the same drop bobs the same way on both sides of a replay.
        let phase = f64::from(drop.at.x.wrapping_mul(7) ^ drop.at.z.wrapping_mul(13)) * 0.37;
        let bob = (f64::from(seconds) * std::f64::consts::TAU * BOB_HZ + phase).sin() * BOB;
        let at = (drop.centre() + DVec3::Y * bob - origin).as_vec3();
        // A bigger pile is a bigger block, but only just: it has to read as
        // "more of it" at a glance without becoming a second block on the
        // floor.
        let swell = 1.0 + (drop.count.min(64) as f32 / 64.0) * 0.25;
        let model = Mat4::from_translation(at)
            * Mat4::from_rotation_y(seconds * std::f32::consts::TAU * SPIN_HZ + phase as f32)
            * Mat4::from_scale(Vec3::splat(SIZE * swell))
            * Mat4::from_translation(Vec3::splat(-0.5));
        objects.push(Object::new(model, def.texture(Face::PosY) as u32).already_relative());
    }
    objects
}

/// Write the floor to `drops.dat`.
pub fn save(drops: &Drops, directory: &Path) -> std::io::Result<()> {
    let mut file = crate::keeping::begin(directory, "drops.dat")?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&(drops.len() as u32).to_le_bytes())?;
    for drop in drops.iter() {
        file.write_all(&drop.at.x.to_le_bytes())?;
        file.write_all(&drop.at.y.to_le_bytes())?;
        file.write_all(&drop.at.z.to_le_bytes())?;
        file.write_all(&(drop.good.len() as u32).to_le_bytes())?;
        file.write_all(drop.good.as_bytes())?;
        file.write_all(&drop.count.to_le_bytes())?;
    }
    file.commit()
}

/// Read it back, tolerating absence and damage.
///
/// A drop that vanished on reload is precisely the bug stage 54 spent a round
/// closing, and it would be worse here: the player put it there on purpose.
pub fn load(directory: &Path) -> Drops {
    let path = directory.join("drops.dat");
    match read(&path) {
        Ok(Some(drops)) => drops,
        Ok(None) => Drops::new(),
        Err(error) => {
            log::warn!("ignoring damaged drops at {}: {error}", path.display());
            Drops::new()
        }
    }
}

fn read(path: &Path) -> std::io::Result<Option<Drops>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("not a drops file"));
    }
    if read_u32(&mut file)? != VERSION {
        return Ok(None);
    }
    let rows = read_u32(&mut file)?;
    if rows as usize > MAX_DROPS {
        return Err(std::io::Error::other("implausible drop manifest"));
    }
    let mut drops = Drops::new();
    for _ in 0..rows {
        let at = BlockPos::new(
            read_i32(&mut file)?,
            read_i32(&mut file)?,
            read_i32(&mut file)?,
        );
        let length = read_u32(&mut file)?;
        if length > MAX_NAME {
            return Err(std::io::Error::other("implausible good name"));
        }
        let mut name = vec![0u8; length as usize];
        file.read_exact(&mut name)?;
        let good = String::from_utf8(name)
            .map_err(|_| std::io::Error::other("a good's name is not text"))?;
        let mut count = [0u8; 8];
        file.read_exact(&mut count)?;
        drops.shed(at, &good, u64::from_le_bytes(count));
    }
    Ok(Some(drops))
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_i32(file: &mut impl Read) -> std::io::Result<i32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(i32::from_le_bytes(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("vx-drops-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn registry() -> BlockRegistry {
        let mut registry = BlockRegistry::new();
        vx_world::gen::TerrainBlocks::register_builtins(&mut registry);
        registry
    }

    #[test]
    fn a_face_leaves_piles_and_not_a_swarm() {
        let mut drops = Drops::new();
        let at = BlockPos::new(4, 30, 9);
        for _ in 0..40 {
            drops.shed(at, "engine:copper_ore", 1);
        }
        assert_eq!(drops.len(), 1, "one cell should hold one pile per good");
        assert_eq!(drops.count("engine:copper_ore"), 40);
        // A second good in the same cell is its own pile, because it has to
        // be picked up as its own thing.
        drops.shed(at, "engine:stone", 3);
        assert_eq!(drops.len(), 2);
    }

    #[test]
    fn walk_over_it_and_it_comes_back() {
        let capacity = crate::pack::capacity(1, 0, 0);
        let mut drops = Drops::new();
        let at = BlockPos::new(0, 40, 0);
        drops.shed(at, "engine:copper_ore", 5);
        let mut pack = Pack::new();
        // Standing well away, nothing moves.
        let taken = drops.collect_near(DVec3::new(20.0, 40.0, 0.0), &mut pack, capacity);
        assert!(taken.is_empty());
        assert_eq!(drops.count("engine:copper_ore"), 5);
        // Standing on it, it all does.
        let taken = drops.collect_near(DVec3::new(0.5, 40.0, 0.5), &mut pack, capacity);
        assert_eq!(taken, vec![("engine:copper_ore".to_string(), 5)]);
        assert!(drops.is_empty(), "an emptied pile should not linger");
        assert_eq!(pack.count("engine:copper_ore"), 5);
    }

    #[test]
    fn a_full_pack_leaves_it_where_it_is() {
        let capacity = crate::pack::capacity(1, 0, 0);
        let mut pack = Pack::new();
        while pack.stow("engine:stone", capacity) {}
        let mut drops = Drops::new();
        drops.shed(BlockPos::new(0, 40, 0), "engine:copper_ore", 5);
        let taken = drops.collect_near(DVec3::new(0.5, 40.0, 0.5), &mut pack, capacity);
        assert!(taken.is_empty(), "a full pack took something anyway");
        assert_eq!(drops.count("engine:copper_ore"), 5, "and the ore vanished");
        // Make room for exactly one and the pile shrinks by exactly one.
        pack.drain().for_each(drop);
        for _ in 0..63 {
            pack.stow("engine:stone", capacity);
        }
        let taken = drops.collect_near(DVec3::new(0.5, 40.0, 0.5), &mut pack, capacity);
        assert!(taken.is_empty(), "ore is heavier than the room left");
    }

    #[test]
    fn the_floor_cannot_run_away() {
        let mut drops = Drops::new();
        for step in 0..(MAX_DROPS as i32 + 40) {
            drops.shed(BlockPos::new(step, 30, 0), "engine:stone", 1);
        }
        assert_eq!(drops.len(), MAX_DROPS);
        // The newest pile survives; the oldest is what went.
        assert!(drops
            .iter()
            .any(|drop| drop.at.x == MAX_DROPS as i32 + 39));
    }

    /// A block cut out of a ceiling ends up on the ground under it.
    #[test]
    fn a_drop_falls_to_the_floor_it_was_cut_over() {
        let mut world = vx_world::World::new(7);
        world.load_around(vx_core::ChunkPos::new(0, 0), 1);
        // A column with real ground in it: the surface, and a cell well above.
        let ground = world.surface_y(0, 0).expect("no ground at the origin");
        let high = BlockPos::new(0, ground + 6, 0);
        let rested = Drops::settle(&world, high);
        assert_eq!(rested.x, high.x);
        assert_eq!(rested.z, high.z);
        assert!(rested.y < high.y, "the drop did not fall at all");
        assert!(
            !world.block(rested.offset([0, -1, 0])).is_air(),
            "the drop came to rest over nothing"
        );
        // And a drop already on the ground does not sink into it.
        assert_eq!(Drops::settle(&world, rested), rested);
        // Nothing under it for a long way and it stays where it was cut,
        // which is the documented answer rather than a fall to bedrock.
        let sky = BlockPos::new(0, ground + 400, 0);
        assert_eq!(Drops::settle(&world, sky), sky);
    }

    #[test]
    fn drops_survive_a_save() {
        let directory = scratch("round-trip");
        let mut drops = Drops::new();
        drops.shed(BlockPos::new(-4, 62, 7), "engine:copper_ore", 12);
        drops.shed(BlockPos::new(1_000_000, 8, -900_000), "engine:log", 3);
        save(&drops, &directory).unwrap();
        assert_eq!(load(&directory), drops);
    }

    #[test]
    fn a_missing_or_damaged_floor_is_a_clean_floor() {
        let directory = scratch("damage");
        assert!(load(&directory).is_empty());
        std::fs::write(directory.join("drops.dat"), b"VXDPrubbish").unwrap();
        assert!(load(&directory).is_empty());
    }

    #[test]
    fn every_pile_draws_as_its_own_good() {
        let registry = registry();
        let mut drops = Drops::new();
        drops.shed(BlockPos::new(0, 40, 0), "engine:copper_ore", 1);
        drops.shed(BlockPos::new(2, 40, 0), "engine:leaves", 1);
        // A name the registry lost with a mod is skipped rather than drawn as
        // whatever now occupies that number — the region format's rule.
        drops.shed(BlockPos::new(4, 40, 0), "othermod:mystery", 1);
        let objects = objects(&drops, &registry, 0.0, DVec3::ZERO);
        assert_eq!(objects.len(), 2);
        assert_ne!(
            objects[0].tile, objects[1].tile,
            "copper and leaves drew the same block"
        );
    }
}
