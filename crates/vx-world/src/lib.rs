//! Chunk storage, terrain generation and world state.
//!
//! This crate is the simulation half of the engine. It has no dependency on
//! rendering or windowing, so it can be driven headlessly — by tests today, by
//! a dedicated server later.

pub mod bunker;
pub mod caves;
pub mod chunk;
pub mod edit;
pub mod flora;
pub mod fluid;
pub mod forest;
pub mod fort;
pub mod gen;
pub mod hash;
pub mod micro;
pub mod noise;
pub mod ore;
pub mod physics;
pub mod raycast;
pub mod reservoir;
pub mod save;
pub mod season;
pub mod seed;
pub mod sight;
pub mod storage;
pub mod town;
pub mod weather;
pub mod world;

pub use chunk::{BlockView, Chunk, SoloChunkView};
pub use world::Carved;
pub use edit::{break_block, place_at, place_block, BlockBreakEvent, BlockPlaceEvent, EditError};
pub use gen::{TerrainBlocks, TerrainGenerator, SEA_LEVEL};
pub use hash::{chunk_hash, region_hash, world_hash};
pub use noise::Fbm;
pub use ore::{deposits_overlapping, ore_at, Deposit, OreKind};
pub use physics::{
    collides, step_aabb, supported, Aabb, MoveParams, PlayerBody, StepResult, GRAVITY, INSET,
    JUMP_SPEED, SKIN, STEP_HEIGHT, SUBSTEPS, TERMINAL_VELOCITY,
};
pub use raycast::{raycast, raycast_solid, RayHit};
pub use save::{SaveError, WorldSave};
pub use seed::SeedPath;
pub use sight::{obstruction, sees};
pub use storage::{PalettedStorage, StorageError};
pub use town::plan::{Building, Role, Tier};
pub use town::{Speciality, TownName, TownSite};
pub use world::World;
