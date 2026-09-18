//! Sealed rooms: which open space is enclosed, and by how much.
//!
//! The room graph from `ATMOSPHERE.md`, part A. No gas yet — this is the
//! *honest experiment* the note asks for first: label open space per
//! sixteen-block section, stitch sections across their faces, and say for
//! any block whether it stands in a sealed room or in the weather.
//!
//! # Derived, never stored
//!
//! Everything here is a pure function of the blocks. Nothing is journalled
//! and nothing is saved: a room is recomputed from the ground the way a
//! chunk's mesh is, so the replay oracle has no opinion about it — a world
//! that hashes the same has the same rooms.
//!
//! # A room is a walk, not a union-find
//!
//! The note sketches a global union-find over `(section, label)` pairs with
//! incremental splits. Splits are the awkward half of that — you cannot
//! un-union — so this does the simpler thing with the same cost bound: the
//! per-section labels are cached and relabelled one section at a time, and
//! a room is built on demand by walking face arrays from the section-label
//! you asked about, under [`SEALED_VOLUME_MAX`]. A sealed room is kept,
//! keyed by its lowest open block, until an edit touches one of its
//! sections; an outdoors verdict is never kept, because the budget makes
//! that walk cheap and forgetting it removes every invalidation case that
//! involves chunk loading.

pub mod label;
pub mod rooms;

pub use rooms::{Room, Rooms, Verdict};

use vx_core::{BlockId, BlockPos, BlockRegistry, ChunkPos, Face, CHUNK_HEIGHT, CHUNK_SIZE};

/// Open volume beyond which a region stops being a room and becomes weather.
/// A hangar is large. A valley is not a hangar.
pub const SEALED_VOLUME_MAX: u32 = 32_768;

/// Blocks to a side of a section. Sixteen, so a section is a cube and a
/// column is sixteen of them — an index local to this module, not a storage
/// format.
pub const SECTION: i32 = CHUNK_SIZE;

/// Sections in a column.
pub const SECTIONS_PER_COLUMN: i32 = CHUNK_HEIGHT / SECTION;

/// One 16³ section of one chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SectionPos {
    pub chunk: ChunkPos,
    /// Which sixteen blocks of the column, from the bottom.
    pub y: i32,
}

impl SectionPos {
    pub fn of(pos: BlockPos) -> Self {
        SectionPos {
            chunk: pos.chunk(),
            y: pos.y.div_euclid(SECTION),
        }
    }

    /// The section across a face. `None` above the world or below it.
    pub fn neighbour(self, face: Face) -> Option<Self> {
        let [dx, dy, dz] = face.offset();
        let y = self.y + dy;
        if !(0..SECTIONS_PER_COLUMN).contains(&y) {
            return None;
        }
        Some(SectionPos {
            chunk: ChunkPos::new(self.chunk.x + dx, self.chunk.z + dz),
            y,
        })
    }

    /// The lowest block of the section.
    pub fn origin(self) -> BlockPos {
        let corner = self.chunk.origin();
        BlockPos::new(corner.x, self.y * SECTION, corner.z)
    }
}

/// Which block ids hold pressure, read once off the registry so the
/// labeller never asks it per voxel.
#[derive(Debug, Clone)]
pub struct Sealing(Vec<bool>);

impl Sealing {
    pub fn of(registry: &BlockRegistry) -> Self {
        Sealing(
            (0..registry.len())
                .map(|id| registry.get_or_air(BlockId(id as u16)).sealed)
                .collect(),
        )
    }

    /// Does this block hold pressure? Unknown ids do not: a block whose mod
    /// is gone reads as air everywhere else too.
    pub fn seals(&self, id: BlockId) -> bool {
        self.0.get(id.0 as usize).copied().unwrap_or(false)
    }
}
