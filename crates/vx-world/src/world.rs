//! World state: the set of loaded chunks and block access across them.
//!
//! `World` owns simulation state only. It never references the renderer or the
//! windowing layer, which is what keeps the client/server split available: the
//! same type can later run headless behind a socket without changes.

use std::collections::HashMap;

use vx_core::{BlockId, BlockPos, BlockRegistry, ChunkPos, Face};

use crate::chunk::{BlockView, Chunk};
use crate::gen::{TerrainBlocks, TerrainGenerator};
use crate::town::TownSite;

/// Loaded chunks plus the generator that fills in missing ones.
pub struct World {
    registry: BlockRegistry,
    generator: TerrainGenerator,
    chunks: HashMap<ChunkPos, Chunk>,
    /// Bumped on every effective block change. Cheap cache invalidation for
    /// anything derived from world contents — a consumer stores the count it
    /// built against and rebuilds when it moves. Monotonic, never reset.
    edit_count: u64,
    /// Chunks something is actively working in, and how many things.
    ///
    /// A drone reads the world through [`World::block`], which reports
    /// unloaded ground as air. That is deliberately conservative — a machine
    /// will not drive onto ground it cannot see — but it means a drone's
    /// decisions depend on which chunks happen to be resident, and residency
    /// follows the camera. Two runs of the same excavation could diverge
    /// purely because the player stood somewhere different.
    ///
    /// Pinning closes that: an operation declares the span it will work in,
    /// that span is loaded up front, and streaming may not evict it while the
    /// work lasts. Counted rather than a flag, so overlapping operations
    /// cannot unpin each other's ground.
    pinned: HashMap<ChunkPos, u32>,
    /// Towns that are not on the lattice: the ones somebody founded.
    ///
    /// A charter is a fact the world is *told*, the same way an edit is,
    /// never one it derives — the generator stays pure in `(seed, pos)` and
    /// knows nothing of these. What the world does with them is answer
    /// [`World::towns_near`] with the lattice's towns and these together, so
    /// everything downstream that asks "what towns are here" sees a founded
    /// town without knowing it was founded.
    charters: Vec<TownSite>,
}

impl World {
    /// A world with the engine's built-in blocks registered.
    pub fn new(seed: u64) -> Self {
        let mut registry = BlockRegistry::new();
        let blocks = TerrainBlocks::register_builtins(&mut registry);
        World {
            registry,
            generator: TerrainGenerator::new(seed, blocks),
            chunks: HashMap::new(),
            edit_count: 0,
            pinned: HashMap::new(),
            charters: Vec::new(),
        }
    }

    /// Tell the world about a founded town.
    ///
    /// Idempotent on the centre, so a charter loaded from disk and the same
    /// charter replayed from the journal do not double up.
    pub fn found(&mut self, site: TownSite) {
        if !self.charters.iter().any(|known| known.centre == site.centre) {
            self.charters.push(site);
        }
    }

    /// The founded towns, in the order they were founded.
    pub fn charters(&self) -> &[TownSite] {
        &self.charters
    }

    /// The Ruined City. Pure in the seed, and cheap enough to ask per frame.
    pub fn city(&self) -> TownSite {
        self.generator.city()
    }

    /// Every town within `radius` of `at`, nearest first — the lattice's
    /// towns and the founded ones together.
    ///
    /// This, not the generator's own `towns_near`, is what the game asks:
    /// the generator answers for the seed, the world answers for the seed
    /// *and* what has been done in it.
    pub fn towns_near(&self, at: (i32, i32), radius: i32) -> Vec<TownSite> {
        let mut towns = self.generator.towns_near(at, radius);
        let reach = radius as i64 * radius as i64;
        for site in &self.charters {
            let dx = (site.centre.0 - at.0) as i64;
            let dz = (site.centre.1 - at.1) as i64;
            if dx * dx + dz * dz <= reach {
                towns.push(*site);
            }
        }
        towns.sort_by_key(|site| {
            let dx = (site.centre.0 - at.0) as i64;
            let dz = (site.centre.1 - at.1) as i64;
            dx * dx + dz * dz
        });
        towns
    }

    /// Every town whose footprint reaches into the box `min..=max`, the
    /// lattice's and the founded ones together.
    pub fn towns_overlapping(&self, min: (i32, i32), max: (i32, i32)) -> Vec<TownSite> {
        let mut towns = self.generator.towns_overlapping(min, max);
        for site in &self.charters {
            let reach = site.core_half + crate::town::SKIRT;
            let reaches = site.centre.0 + reach >= min.0
                && site.centre.0 - reach <= max.0
                && site.centre.1 + reach >= min.1
                && site.centre.1 - reach <= max.1;
            if reaches {
                towns.push(*site);
            }
        }
        towns
    }

    /// Raise a founded town out of the ground.
    ///
    /// Every chunk the town reaches is loaded and then **regenerated among
    /// the lattice's towns plus this one** — the same code that builds every
    /// other town, asked for the chunk as if the charter had always been on
    /// the map. Blended plateau, paths, buildings, lockboxes, the wall, the
    /// flora and cave masks: all of it, deterministic in `(seed, site)`,
    /// which is what lets the journal replay a founding to the same ground.
    ///
    /// The plot is levelled, and that means **anything dug on it is
    /// filled**. The regenerated chunks are marked modified so they save, and
    /// dirty so they remesh. Returns how many chunks were raised.
    pub fn raise(&mut self, site: TownSite) -> usize {
        self.found(site);
        let reach = crate::town::REACH;
        let size = vx_core::CHUNK_SIZE;
        let min = ChunkPos::new(
            (site.centre.0 - reach).div_euclid(size),
            (site.centre.1 - reach).div_euclid(size),
        );
        let max = ChunkPos::new(
            (site.centre.0 + reach).div_euclid(size),
            (site.centre.1 + reach).div_euclid(size),
        );
        let mut raised = 0;
        for z in min.z..=max.z {
            for x in min.x..=max.x {
                let pos = ChunkPos::new(x, z);
                let origin = pos.origin();
                // The same gather the generator does for itself, with the
                // charters folded in — so a founded town next to another
                // founded town blends against it too.
                let margin = crate::flora::CANOPY_REACH;
                let sites = self.towns_overlapping(
                    (origin.x - margin, origin.z - margin),
                    (origin.x + size - 1 + margin, origin.z + size - 1 + margin),
                );
                let mut chunk = self.generator.generate_among(pos, &sites);
                chunk.mark_modified();
                chunk.mark_dirty();
                self.chunks.insert(pos, chunk);
                self.edit_count += 1;
                raised += 1;
            }
        }
        raised
    }

    pub fn registry(&self) -> &BlockRegistry {
        &self.registry
    }

    /// How many effective block edits this world has seen.
    ///
    /// The number itself means nothing; only "has it changed since I looked"
    /// does. No-op writes (setting a block to what it already is) do not count,
    /// matching the dirty-marking rule above.
    /// How many wounded blocks the loaded world is carrying — the debug
    /// panel's honesty check on micro-on-damage's "rare by construction".
    pub fn composite_count(&self) -> usize {
        self.chunks.values().map(|chunk| chunk.composite_count()).sum()
    }

    pub fn edit_count(&self) -> u64 {
        self.edit_count
    }

    pub fn generator(&self) -> &TerrainGenerator {
        &self.generator
    }

    pub fn seed(&self) -> u64 {
        self.generator.seed()
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    pub fn chunk_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        self.chunks.get_mut(&pos)
    }

    /// Every chunk currently resident, in unspecified order.
    pub fn loaded_chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.chunks.keys().copied()
    }

    pub fn is_loaded(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos)
    }

    /// Insert an already-built chunk, replacing anything at its position.
    ///
    /// Used to bring a chunk in from disk rather than generating it.
    pub fn insert_chunk(&mut self, chunk: Chunk) {
        self.chunks.insert(chunk.pos(), chunk);
    }

    /// Load `pos`, generating it if it is not already resident.
    pub fn load_chunk(&mut self, pos: ChunkPos) -> &Chunk {
        self.chunks
            .entry(pos)
            .or_insert_with(|| self.generator.generate(pos))
    }

    /// Drop every loaded chunk, whatever its state.
    ///
    /// The deliberate reset [`World::unload_beyond`] cannot express: that one
    /// keeps modified and pinned chunks by design, because it is the
    /// streamer's evictor and evicting an unsaved edit would lose it. This is
    /// for the other case — a world about to be *re-read from disk*, where
    /// keeping a generated chunk is exactly the harm.
    ///
    /// It exists because of a bug worth naming. `Session::load_from` opened a
    /// world (generating terrain), then tried to clear it with
    /// `unload_beyond(pos, i32::MAX)` — whose radius is squared into a limit
    /// every chunk is inside, so it retained everything — and then asked the
    /// save for each chunk, which returned early because the chunk was already
    /// loaded. The result was a reload that quietly served **generated ground
    /// in place of the ground on disk**: a dig came back filled in, and the
    /// crew standing in it was entombed and reported stuck.
    pub fn unload_all(&mut self) -> usize {
        let dropped = self.chunks.len();
        self.chunks.clear();
        dropped
    }

    /// Load every chunk within `radius` of `centre`, returning how many were
    /// newly generated.
    pub fn load_around(&mut self, centre: ChunkPos, radius: i32) -> usize {
        let mut generated = 0;
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let pos = ChunkPos::new(centre.x + dx, centre.z + dz);
                if pos.distance_squared(centre) > (radius as i64) * (radius as i64) {
                    continue;
                }
                if !self.is_loaded(pos) {
                    self.load_chunk(pos);
                    generated += 1;
                }
            }
        }
        generated
    }

    /// Drop chunks further than `radius` from `centre`, returning how many
    /// were unloaded.
    ///
    /// Chunks still carrying unsaved edits are kept, so walking away from
    /// something you built cannot discard it before it reaches disk.
    pub fn unload_beyond(&mut self, centre: ChunkPos, radius: i32) -> usize {
        let limit = (radius as i64) * (radius as i64);
        let before = self.chunks.len();
        let pinned = &self.pinned;
        self.chunks.retain(|pos, chunk| {
            pos.distance_squared(centre) <= limit
                || chunk.is_modified()
                || pinned.contains_key(pos)
        });
        before - self.chunks.len()
    }

    /// Load every chunk overlapping a block box and hold it against eviction.
    ///
    /// Returns the chunks pinned, which is also the set
    /// [`World::unpin_span`] must be given to release them. Call this before
    /// dispatching anything that will read the world inside `min..=max`: what
    /// it buys is that the work reads real ground rather than the air an
    /// unloaded chunk reports, and therefore behaves the same however the
    /// player wandered while it ran.
    pub fn pin_span(&mut self, min: BlockPos, max: BlockPos) -> Vec<ChunkPos> {
        let span: Vec<ChunkPos> = crate::hash::chunks_overlapping(min, max).collect();
        for pos in &span {
            self.load_chunk(*pos);
            *self.pinned.entry(*pos).or_insert(0) += 1;
        }
        span
    }

    /// Release a pin taken by [`World::pin_span`]. Unbalanced calls are
    /// ignored rather than underflowing — losing a pin is recoverable, a panic
    /// in the middle of a mine is not.
    pub fn unpin_span(&mut self, span: &[ChunkPos]) {
        for pos in span {
            if let Some(count) = self.pinned.get_mut(pos) {
                *count -= 1;
                if *count == 0 {
                    self.pinned.remove(pos);
                }
            }
        }
    }

    pub fn is_pinned(&self, pos: ChunkPos) -> bool {
        self.pinned.contains_key(&pos)
    }

    pub fn pinned_count(&self) -> usize {
        self.pinned.len()
    }

    /// Block at a world position. Unloaded chunks and out-of-bounds heights
    /// read as air.
    pub fn block(&self, pos: BlockPos) -> BlockId {
        let Some(local) = pos.local() else {
            return BlockId::AIR;
        };
        self.chunks
            .get(&pos.chunk())
            .map_or(BlockId::AIR, |chunk| chunk.get(local))
    }

    /// Write a block, returning the previous one. Fails if the position is
    /// outside the world height or its chunk is not loaded.
    pub fn set_block(&mut self, pos: BlockPos, block: BlockId) -> Option<BlockId> {
        let local = pos.local()?;
        let chunk = self.chunks.get_mut(&pos.chunk())?;
        let previous = chunk.set(local, block);

        // A block on a chunk edge changes what the neighbour's seam faces look
        // like, so that chunk needs remeshing too.
        if previous != block {
            self.dirty_touching_neighbours(pos);
            self.edit_count += 1;
        }
        Some(previous)
    }

    /// This block's wound, if violence has given it one.
    pub fn mask(&self, pos: BlockPos) -> Option<crate::micro::Mask> {
        let local = pos.local()?;
        self.chunks.get(&pos.chunk())?.mask(local)
    }

    /// Take a shape out of a block, and say what became of it.
    ///
    /// The block gains an interior the first time it is hit and loses
    /// exactly the cells the shape removed; when too little is left it
    /// becomes air, which is the only outcome the rest of the game already
    /// knew how to have. Everything here is integer arithmetic on one
    /// register, so two runs fed the same carves agree bit for bit — which
    /// is what lets wounds ride the replay oracle without the journal ever
    /// learning they exist.
    pub fn carve(&mut self, pos: BlockPos, shape: crate::micro::Mask) -> Carved {
        let Some(local) = pos.local() else {
            return Carved::Nothing;
        };
        let Some(chunk) = self.chunks.get_mut(&pos.chunk()) else {
            return Carved::Nothing;
        };
        if chunk.get(local).is_air() {
            return Carved::Nothing;
        }
        let before = chunk.mask(local).unwrap_or(crate::micro::FULL);
        let after = crate::micro::carve(before, shape);
        if after == before {
            return Carved::Nothing;
        }
        if crate::micro::dead(after) {
            // What is left is rubble, not cover. The block goes, by the same
            // path every other break takes.
            let previous = chunk.set(local, BlockId::AIR);
            self.dirty_touching_neighbours(pos);
            self.edit_count += 1;
            return Carved::Broke(previous);
        }
        chunk.set_mask(local, after);
        // A wound on a chunk edge changes what the neighbour draws along the
        // seam, exactly as a broken block does.
        self.dirty_touching_neighbours(pos);
        self.edit_count += 1;
        Carved::Wounded(after)
    }

    /// Set a block's mask outright.
    ///
    /// [`World::carve`] only ever takes cells away, which is right for
    /// damage: a wound does not heal. A fill level *rises*, so the fluid
    /// needs the other direction — and it needs it to mark the same
    /// neighbours dirty, or a shoreline on a chunk edge would only redraw on
    /// one side of the seam.
    ///
    /// Returns false when the chunk is not loaded, which is the same answer
    /// [`World::set_block`] gives and the same contract every edit here has.
    pub fn set_mask(&mut self, pos: BlockPos, mask: crate::micro::Mask) -> bool {
        let Some(local) = pos.local() else {
            return false;
        };
        let Some(chunk) = self.chunks.get_mut(&pos.chunk()) else {
            return false;
        };
        if chunk.get(local).is_air() {
            return false;
        }
        chunk.set_mask(local, mask);
        self.dirty_touching_neighbours(pos);
        self.edit_count += 1;
        true
    }

    /// Mark neighbouring chunks dirty when `pos` sits on a shared edge.
    fn dirty_touching_neighbours(&mut self, pos: BlockPos) {
        let own = pos.chunk();
        for face in Face::ALL {
            let neighbour_chunk = pos.neighbour(face).chunk();
            if neighbour_chunk != own {
                if let Some(chunk) = self.chunks.get_mut(&neighbour_chunk) {
                    chunk.mark_dirty();
                }
            }
        }
    }

    /// Chunks needing a mesh rebuild.
    pub fn dirty_chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.chunks
            .iter()
            .filter(|(_, chunk)| chunk.is_dirty())
            .map(|(pos, _)| *pos)
    }

    pub fn clear_dirty(&mut self, pos: ChunkPos) {
        if let Some(chunk) = self.chunks.get_mut(&pos) {
            chunk.clear_dirty();
        }
    }

    /// True when the block at `pos` blocks movement.
    pub fn is_solid(&self, pos: BlockPos) -> bool {
        self.registry.is_solid(self.block(pos))
    }

    /// A safe standing height above `(x, z)`: one block above the surface.
    /// Returns `None` if the column's chunk is not loaded.
    pub fn surface_y(&self, x: i32, z: i32) -> Option<i32> {
        let chunk_pos = BlockPos::new(x, 0, z).chunk();
        let chunk = self.chunks.get(&chunk_pos)?;
        let local = BlockPos::new(x, 0, z).local()?;
        chunk
            .height_at(local.x(), local.z())
            .map(|top| top + 1)
    }
}

impl BlockView for World {
    fn block_at(&self, x: i32, y: i32, z: i32) -> BlockId {
        self.block(BlockPos::new(x, y, z))
    }

    fn mask_at(&self, x: i32, y: i32, z: i32) -> Option<crate::micro::Mask> {
        self.mask(BlockPos::new(x, y, z))
    }
}

/// What a carve did to a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carved {
    /// Nothing there to damage, or the shape took nothing this block still had.
    Nothing,
    /// Still standing, with this much of it left.
    Wounded(crate::micro::Mask),
    /// Too little left to be a block: it is air now, and this is what it was.
    Broke(BlockId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{CHUNK_HEIGHT, CHUNK_SIZE};

    #[test]
    fn a_new_world_has_no_chunks_loaded() {
        let world = World::new(1);
        assert_eq!(world.loaded_chunk_count(), 0);
        assert!(!world.is_loaded(ChunkPos::new(0, 0)));
        // Reading unloaded space is air, not a panic.
        assert!(world.block(BlockPos::new(0, 64, 0)).is_air());
    }

    #[test]
    fn loading_a_chunk_generates_it_once() {
        let mut world = World::new(42);
        world.load_chunk(ChunkPos::new(0, 0));
        assert_eq!(world.loaded_chunk_count(), 1);

        world.load_chunk(ChunkPos::new(0, 0));
        assert_eq!(world.loaded_chunk_count(), 1, "reloading must not duplicate");
    }

    #[test]
    fn load_around_fills_a_disc_and_reports_new_chunks() {
        let mut world = World::new(3);
        let generated = world.load_around(ChunkPos::new(0, 0), 2);

        assert_eq!(generated, world.loaded_chunk_count());
        assert!(world.is_loaded(ChunkPos::new(0, 0)));
        assert!(world.is_loaded(ChunkPos::new(2, 0)));
        // Corners fall outside the radius.
        assert!(!world.is_loaded(ChunkPos::new(2, 2)));

        // Loading again generates nothing new.
        assert_eq!(world.load_around(ChunkPos::new(0, 0), 2), 0);
    }

    #[test]
    fn unloading_drops_only_distant_chunks() {
        let mut world = World::new(3);
        world.load_around(ChunkPos::new(0, 0), 3);
        let before = world.loaded_chunk_count();

        let dropped = world.unload_beyond(ChunkPos::new(0, 0), 1);

        assert!(dropped > 0);
        assert_eq!(world.loaded_chunk_count(), before - dropped);
        assert!(world.is_loaded(ChunkPos::new(0, 0)));
        assert!(world.is_loaded(ChunkPos::new(1, 0)));
        assert!(!world.is_loaded(ChunkPos::new(3, 0)));
    }

    #[test]
    fn writing_a_block_reads_back_and_reports_the_previous_value() {
        let mut world = World::new(7);
        world.load_chunk(ChunkPos::new(0, 0));
        let stone = world.registry().id_of("engine:stone").unwrap();
        let pos = BlockPos::new(4, 200, 4); // well above terrain, so it is air

        let previous = world.set_block(pos, stone);

        assert_eq!(previous, Some(BlockId::AIR));
        assert_eq!(world.block(pos), stone);
        assert!(world.is_solid(pos));
    }

    #[test]
    fn writing_into_unloaded_or_out_of_bounds_space_fails_cleanly() {
        let mut world = World::new(7);
        world.load_chunk(ChunkPos::new(0, 0));
        let stone = world.registry().id_of("engine:stone").unwrap();

        // Chunk not loaded.
        assert_eq!(world.set_block(BlockPos::new(1000, 64, 1000), stone), None);
        // Above and below the world.
        assert_eq!(world.set_block(BlockPos::new(0, CHUNK_HEIGHT, 0), stone), None);
        assert_eq!(world.set_block(BlockPos::new(0, -1, 0), stone), None);
    }

    #[test]
    fn editing_a_chunk_edge_dirties_the_neighbouring_chunk() {
        // Without this, seam faces go stale and you get holes between chunks.
        let mut world = World::new(11);
        world.load_chunk(ChunkPos::new(0, 0));
        world.load_chunk(ChunkPos::new(-1, 0));
        for pos in [ChunkPos::new(0, 0), ChunkPos::new(-1, 0)] {
            world.clear_dirty(pos);
        }

        let stone = world.registry().id_of("engine:stone").unwrap();
        world.set_block(BlockPos::new(0, 200, 4), stone);

        let dirty: Vec<_> = world.dirty_chunks().collect();
        assert!(dirty.contains(&ChunkPos::new(0, 0)), "edited chunk must be dirty");
        assert!(
            dirty.contains(&ChunkPos::new(-1, 0)),
            "chunk across the seam must be dirty too"
        );
    }

    #[test]
    fn editing_the_middle_of_a_chunk_leaves_neighbours_clean() {
        let mut world = World::new(11);
        world.load_around(ChunkPos::new(0, 0), 1);
        let positions: Vec<_> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| ChunkPos::new(x, z)))
            .collect();
        for pos in &positions {
            world.clear_dirty(*pos);
        }

        let stone = world.registry().id_of("engine:stone").unwrap();
        world.set_block(BlockPos::new(8, 200, 8), stone);

        let dirty: Vec<_> = world.dirty_chunks().collect();
        assert_eq!(dirty, vec![ChunkPos::new(0, 0)]);
    }

    #[test]
    fn a_redundant_write_does_not_dirty_anything() {
        let mut world = World::new(11);
        world.load_chunk(ChunkPos::new(0, 0));
        world.clear_dirty(ChunkPos::new(0, 0));

        // Rewriting air over air changes nothing.
        world.set_block(BlockPos::new(0, 200, 0), BlockId::AIR);

        assert_eq!(world.dirty_chunks().count(), 0);
    }

    #[test]
    fn block_view_reads_across_chunk_boundaries() {
        let mut world = World::new(5);
        world.load_chunk(ChunkPos::new(0, 0));
        world.load_chunk(ChunkPos::new(-1, 0));
        let stone = world.registry().id_of("engine:stone").unwrap();

        // Last column of the chunk to the west.
        world.set_block(BlockPos::new(-1, 200, 0), stone);

        assert_eq!(world.block_at(-1, 200, 0), stone);
        assert_eq!(world.block_at(0, 200, 0), BlockId::AIR);
        // Beyond loaded chunks reads as air rather than panicking.
        assert!(world.block_at(9999, 200, 9999).is_air());
    }

    #[test]
    fn surface_y_lands_just_above_the_terrain() {
        let mut world = World::new(2468);
        world.load_chunk(ChunkPos::new(0, 0));

        for x in 0..CHUNK_SIZE {
            let spawn = world.surface_y(x, 0).unwrap();
            assert!(!world.is_solid(BlockPos::new(x, spawn, 0)), "spawn is inside a block");
            assert!(
                world.block(BlockPos::new(x, spawn - 1, 0)) != BlockId::AIR,
                "spawn is floating above the surface"
            );
        }

        assert_eq!(world.surface_y(500, 500), None, "unloaded columns have no surface");
    }

    #[test]
    fn pinned_ground_survives_the_player_walking_away() {
        // The whole point of pinning: an operation's ground must not be
        // evicted just because the camera left. Without this a drone would
        // start reading air where it had been digging rock.
        let mut world = World::new(2024);
        let span = world.pin_span(BlockPos::new(0, 0, 0), BlockPos::new(31, 0, 31));
        assert_eq!(span.len(), 4, "a 32x32 box spans four chunks");
        assert!(span.iter().all(|pos| world.is_loaded(*pos)));

        // Stream far away. Ordinarily every one of these would go.
        world.unload_beyond(ChunkPos::new(500, 500), 2);
        assert!(
            span.iter().all(|pos| world.is_loaded(*pos)),
            "streaming evicted ground an operation was working in"
        );

        world.unpin_span(&span);
        assert_eq!(world.pinned_count(), 0);
        world.unload_beyond(ChunkPos::new(500, 500), 2);
        assert!(
            span.iter().all(|pos| !world.is_loaded(*pos)),
            "released ground was still held"
        );
    }

    #[test]
    fn overlapping_operations_cannot_unpin_each_other() {
        // Counted, not a flag. Two mines sharing a chunk, one finishing, must
        // not pull the ground out from under the other.
        let mut world = World::new(2024);
        let first = world.pin_span(BlockPos::new(0, 0, 0), BlockPos::new(15, 0, 15));
        let second = world.pin_span(BlockPos::new(8, 0, 8), BlockPos::new(20, 0, 20));
        let shared = ChunkPos::new(0, 0);
        assert!(world.is_pinned(shared));

        world.unpin_span(&first);
        assert!(world.is_pinned(shared), "the surviving operation lost its ground");

        world.unpin_span(&second);
        assert!(!world.is_pinned(shared));
    }

    #[test]
    fn an_unbalanced_release_is_ignored_rather_than_underflowing() {
        // Losing a pin is recoverable; a panic in the middle of a mine is not.
        let mut world = World::new(2024);
        let span = world.pin_span(BlockPos::new(0, 0, 0), BlockPos::new(1, 0, 1));
        world.unpin_span(&span);
        world.unpin_span(&span);
        assert_eq!(world.pinned_count(), 0);
    }
}

#[cfg(test)]
mod charter_tests {
    use super::*;
    use crate::town::{self, Speciality, TownName, TownSite};

    /// A charter site a full cell east of the hometown, on ground the
    /// lattice left empty.
    fn charter(world: &World) -> TownSite {
        let centre = (town::CELL + 60, 40);
        let ground = world.generator().natural_height_at(centre.0, centre.1);
        TownSite {
            centre,
            ground: ground.clamp(crate::gen::SEA_LEVEL + town::MIN_DRY + 1, 140),
            core_half: town::HOME_CORE_HALF,
            speciality: Speciality::Depot,
            name: TownName::from_words("iron", "reach").unwrap(),
            seed: 0x5eed_c4a7_7e55,
        }
    }

    /// The world answers for the seed and for what was done in it: a founded
    /// town is in `towns_near` the moment the world is told, and the lattice
    /// still does not know it.
    #[test]
    fn a_founded_town_is_in_the_worlds_answer_but_not_the_lattices() {
        let mut world = World::new(2024);
        let site = charter(&world);
        let before = world.towns_near(site.centre, 100);
        assert!(before.iter().all(|known| known.centre != site.centre));

        world.found(site);
        world.found(site);
        assert_eq!(world.charters().len(), 1, "telling the world twice doubled it");

        let after = world.towns_near(site.centre, 100);
        assert_eq!(after.first().map(|s| s.centre), Some(site.centre));
        assert!(world.generator().towns_near(site.centre, 100).is_empty());
        assert!(
            world
                .towns_overlapping((site.centre.0 - 8, site.centre.1 - 8), (site.centre.0 + 8, site.centre.1 + 8))
                .iter()
                .any(|s| s.centre == site.centre)
        );
    }

    /// Day one: raising a charter puts the whole town in the ground — the
    /// beacon, the counter, the vault, the cots, the wall — and it is the
    /// same ground twice, because it is the same generator asked the same
    /// question.
    #[test]
    fn raising_a_charter_puts_a_whole_town_in_the_ground_deterministically() {
        let build = || {
            let mut world = World::new(2024);
            let site = charter(&world);
            let centre = vx_core::BlockPos::new(site.centre.0, 0, site.centre.1).chunk();
            world.load_around(centre, 5);
            // Dig a hole where the plaza will be: the charter fills it.
            let dug = vx_core::BlockPos::new(site.centre.0, site.ground, site.centre.1);
            world.set_block(dug, BlockId::AIR);
            let raised = world.raise(site);
            (world, site, raised, dug)
        };
        let (world, site, raised, dug) = build();
        assert!(raised >= 36, "only {raised} chunks were raised");

        // The plot was levelled and the hole is gone.
        assert!(!world.block(dug).is_air(), "the charter did not level the plot");

        // The fixtures every other system reaches for are where they say.
        let registry = world.registry();
        let name_at = |at: vx_core::BlockPos| registry.get_or_air(world.block(at)).name.clone();
        assert!(name_at(town::beacon_position(&site)).contains("beacon"), "no beacon: {}", name_at(town::beacon_position(&site)));
        assert!(name_at(town::counter_position(&site)).contains("counter"), "no counter");
        let buildings = town::plan::buildings(&site);
        assert!(buildings.iter().any(|b| b.role == town::plan::Role::Bank), "no bank in the plan");
        assert!(!town::plan::lockboxes(&site).is_empty(), "no lockboxes");
        for (lock, _) in town::plan::lockboxes(&site) {
            assert!(name_at(lock).contains("permit"), "lockbox missing at {lock:?}: {}", name_at(lock));
        }
        // And the wall: somewhere on the fort's reach there is a rampart.
        let fort = crate::fort::fort_for(&site);
        let mut walled = false;
        for x in site.centre.0 - fort.reach()..=site.centre.0 + fort.reach() {
            for y in site.ground..site.ground + 6 {
                let at = vx_core::BlockPos::new(x, y, site.centre.1);
                if name_at(at).contains("rampart") || name_at(at).contains("wall") {
                    walled = true;
                }
            }
        }
        assert!(walled, "no wall was raised");

        // Byte-identical the second time.
        let (again, _, _, _) = build();
        let span = (
            vx_core::BlockPos::new(site.centre.0 - 64, 0, site.centre.1 - 64),
            vx_core::BlockPos::new(site.centre.0 + 64, 200, site.centre.1 + 64),
        );
        assert_eq!(
            crate::region_hash(&world, span.0, span.1),
            crate::region_hash(&again, span.0, span.1),
            "raising the same charter twice gave two different towns"
        );

        // Raised chunks save and remesh.
        let centre = vx_core::BlockPos::new(site.centre.0, 0, site.centre.1).chunk();
        assert!(world.chunk(centre).unwrap().is_modified());
        assert!(world.chunk(centre).unwrap().is_dirty());
    }

    /// The book: a founded town is named from the same sixteen-by-sixteen
    /// vocabulary as every other town, case-insensitively, and a word that
    /// is not in it is refused.
    #[test]
    fn a_founded_town_is_named_from_the_book() {
        let (heads, tails) = town::name_book();
        assert_eq!(heads.len(), 16);
        assert_eq!(tails.len(), 16);
        let name = TownName::from_words("Iron", "REACH").unwrap();
        assert_eq!(name.to_string(), "IRONREACH");
        let (head, tail) = name.indices();
        assert_eq!(TownName::from_indices(head, tail), name);
        assert!(TownName::from_words("harvest", "moon").is_none());
        assert!(TownName::from_words("iron", "moon").is_none());
    }

    /// The lattice's own siting rule, asked of arbitrary ground.
    #[test]
    fn buildable_is_the_lattices_relief_rule() {
        let flat = |_: i32, _: i32| 80;
        assert!(town::buildable(&flat, (0, 0), town::HOME_CORE_HALF));
        let cliff = |x: i32, _: i32| if x > 0 { 80 + town::MAX_RELIEF + 1 } else { 80 };
        assert!(!town::buildable(&cliff, (0, 0), town::HOME_CORE_HALF));
    }
}
