//! Breaking and placing blocks.
//!
//! Every edit is announced on the event bus before it happens, through
//! [`EventBus::emit_cancellable`], so a handler can veto it. Nothing uses that
//! yet — the point is that mods will, in M3, without any of these call sites
//! changing.
//!
//! Edits go through [`World::set_block`], which already marks neighbouring
//! chunks dirty when a block sits on a seam, so the mesher picks up the change
//! on both sides.

use vx_core::{BlockId, BlockPos, Cancellable, EventBus, Face};

use crate::raycast::RayHit;
use crate::world::World;

/// Announced before a block is removed.
#[derive(Debug, Clone)]
pub struct BlockBreakEvent {
    pub position: BlockPos,
    /// The block about to be removed.
    pub block: BlockId,
    cancelled: bool,
}

impl BlockBreakEvent {
    pub fn new(position: BlockPos, block: BlockId) -> Self {
        BlockBreakEvent {
            position,
            block,
            cancelled: false,
        }
    }
}

impl Cancellable for BlockBreakEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

/// Announced before a block is placed.
#[derive(Debug, Clone)]
pub struct BlockPlaceEvent {
    pub position: BlockPos,
    /// The block about to be placed. A handler may swap it for another.
    pub block: BlockId,
    /// The existing block being built against.
    pub against: BlockPos,
    pub face: Face,
    cancelled: bool,
}

impl BlockPlaceEvent {
    pub fn new(position: BlockPos, block: BlockId, against: BlockPos, face: Face) -> Self {
        BlockPlaceEvent {
            position,
            block,
            against,
            face,
            cancelled: false,
        }
    }
}

impl Cancellable for BlockPlaceEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

/// Why an edit did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("the target position is outside the world or its chunk is not loaded")]
    OutOfRange,
    #[error("there is nothing there to break")]
    NothingThere,
    #[error("that space is already occupied")]
    Occupied,
    #[error("something is standing in the way")]
    Obstructed,
    #[error("a handler cancelled the edit")]
    Cancelled,
}

/// Remove the block at `position`, returning what was removed.
pub fn break_block(
    world: &mut World,
    events: &EventBus,
    position: BlockPos,
) -> Result<BlockId, EditError> {
    let existing = world.block(position);
    if existing.is_air() {
        return Err(EditError::NothingThere);
    }
    // Unbreakable blocks (bedrock) have no hardness.
    if world
        .registry()
        .get(existing)
        .is_some_and(|def| def.hardness.is_none())
    {
        return Err(EditError::Cancelled);
    }

    let mut event = BlockBreakEvent::new(position, existing);
    if !events.emit_cancellable(&mut event) {
        return Err(EditError::Cancelled);
    }

    world
        .set_block(position, BlockId::AIR)
        .ok_or(EditError::OutOfRange)?;
    Ok(existing)
}

/// Place `block` against the face reported by `hit`.
///
/// `is_obstructed` is asked whether the target position is occupied by
/// something the world does not know about — in practice the player's own
/// bounding box. Without it you can seal yourself inside a block, which is the
/// classic way to get stuck in a voxel game.
pub fn place_block(
    world: &mut World,
    events: &EventBus,
    hit: &RayHit,
    block: BlockId,
    is_obstructed: impl Fn(BlockPos) -> bool,
) -> Result<BlockPos, EditError> {
    place_at(
        world,
        events,
        hit.placement(),
        block,
        hit.block,
        hit.face,
        is_obstructed,
    )
}

/// Put `block` at `position`, with no ray involved.
///
/// # Why this exists
///
/// [`place_block`] takes a [`RayHit`] because for fifty-five stages the only
/// thing in this game that ever built anything was a player pointing at a
/// block. Stage 56 gives the drone crew a spoil heap to stack, and a drone has
/// no ray: it stands next to a cell it has been told to fill and puts what it
/// is carrying into it.
///
/// So this is the real placement rule and `place_block` is now the caller that
/// unpacks a hit into it — done in that direction on purpose, so that every
/// check the player's build has always gone through is exactly the one the
/// crew goes through, rather than a second placer beside the first that drifts.
/// In particular the crew inherits the cancellable [`BlockPlaceEvent`] and
/// therefore the town's permit veto, which already subscribes to it.
///
/// `against` and `face` describe what the block is being built onto. A ray
/// knows them for free; a caller without one passes the cell it is building
/// from and the direction it is reaching.
#[allow(clippy::too_many_arguments)]
pub fn place_at(
    world: &mut World,
    events: &EventBus,
    position: BlockPos,
    block: BlockId,
    against: BlockPos,
    face: Face,
    is_obstructed: impl Fn(BlockPos) -> bool,
) -> Result<BlockPos, EditError> {
    if !position.in_vertical_bounds() {
        return Err(EditError::OutOfRange);
    }
    // Only empty space can be built into. Water counts as replaceable, since
    // building into a lake should work.
    let existing = world.block(position);
    if !existing.is_air() && world.registry().is_solid(existing) {
        return Err(EditError::Occupied);
    }
    if is_obstructed(position) {
        return Err(EditError::Obstructed);
    }

    let mut event = BlockPlaceEvent::new(position, block, against, face);
    if !events.emit_cancellable(&mut event) {
        return Err(EditError::Cancelled);
    }
    // A handler may have substituted a different block.
    let block = event.block;

    world
        .set_block(position, block)
        .ok_or(EditError::OutOfRange)?;
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use glam::Vec3;
    use vx_core::ChunkPos;

    use crate::raycast::raycast_solid;

    /// A world with one chunk loaded and a flat-ish surface to work on.
    fn world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 1);
        world
    }

    fn stone(world: &World) -> BlockId {
        world.registry().id_of("engine:stone").unwrap()
    }

    /// Look straight down at the surface from above.
    fn hit_surface(world: &World, x: i32, z: i32) -> RayHit {
        let top = world.surface_y(x, z).expect("column is loaded");
        raycast_solid(
            world,
            world.registry(),
            glam::DVec3::new(x as f64 + 0.5, top as f64 + 5.0, z as f64 + 0.5),
            Vec3::NEG_Y,
            20.0,
        )
        .expect("the surface should be beneath the camera")
    }

    #[test]
    fn breaking_removes_the_block_and_reports_what_it_was() {
        let mut world = world();
        let events = EventBus::new();
        let hit = hit_surface(&world, 4, 4);

        let removed = break_block(&mut world, &events, hit.block).unwrap();

        assert!(!removed.is_air());
        assert!(world.block(hit.block).is_air());
    }

    #[test]
    fn breaking_empty_space_fails() {
        let mut world = world();
        let events = EventBus::new();
        let sky = BlockPos::new(4, 250, 4);

        assert_eq!(
            break_block(&mut world, &events, sky),
            Err(EditError::NothingThere)
        );
    }

    #[test]
    fn bedrock_cannot_be_broken() {
        let mut world = world();
        let events = EventBus::new();
        // The world floor is bedrock, which has no hardness.
        let floor = BlockPos::new(4, 0, 4);
        assert_eq!(world.block(floor), world.registry().id_of("engine:bedrock").unwrap());

        assert_eq!(
            break_block(&mut world, &events, floor),
            Err(EditError::Cancelled)
        );
        assert!(!world.block(floor).is_air(), "bedrock was removed anyway");
    }

    #[test]
    fn placing_puts_the_block_against_the_face_that_was_hit() {
        let mut world = world();
        let events = EventBus::new();
        let hit = hit_surface(&world, 4, 4);
        let stone = stone(&world);

        let placed = place_block(&mut world, &events, &hit, stone, |_| false).unwrap();

        // Hit the top face, so the block goes one above.
        assert_eq!(hit.face, Face::PosY);
        assert_eq!(placed, hit.block.neighbour(Face::PosY));
        assert_eq!(world.block(placed), stone);
    }

    #[test]
    fn placing_into_an_occupied_space_fails() {
        let mut world = world();
        let events = EventBus::new();
        let hit = hit_surface(&world, 4, 4);
        let stone = stone(&world);

        place_block(&mut world, &events, &hit, stone, |_| false).unwrap();
        // The same hit now targets a filled space.
        assert_eq!(
            place_block(&mut world, &events, &hit, stone, |_| false),
            Err(EditError::Occupied)
        );
    }

    #[test]
    fn placing_where_something_is_standing_fails() {
        // The guard against sealing yourself inside a block.
        let mut world = world();
        let events = EventBus::new();
        let hit = hit_surface(&world, 4, 4);
        let stone = stone(&world);
        let target = hit.placement();

        assert_eq!(
            place_block(&mut world, &events, &hit, stone, |pos| pos == target),
            Err(EditError::Obstructed)
        );
        assert!(world.block(target).is_air(), "the block was placed anyway");
    }

    #[test]
    fn a_handler_can_veto_a_break() {
        let mut world = world();
        let mut events = EventBus::new();
        events.subscribe("guard", |event: &mut BlockBreakEvent| event.cancel());

        let hit = hit_surface(&world, 4, 4);
        let before = world.block(hit.block);

        assert_eq!(
            break_block(&mut world, &events, hit.block),
            Err(EditError::Cancelled)
        );
        assert_eq!(world.block(hit.block), before, "the veto was ignored");
    }

    #[test]
    fn a_handler_can_veto_a_place() {
        let mut world = world();
        let mut events = EventBus::new();
        events.subscribe("guard", |event: &mut BlockPlaceEvent| event.cancel());

        let hit = hit_surface(&world, 4, 4);
        let stone = stone(&world);

        assert_eq!(
            place_block(&mut world, &events, &hit, stone, |_| false),
            Err(EditError::Cancelled)
        );
        assert!(world.block(hit.placement()).is_air());
    }

    #[test]
    fn a_handler_can_substitute_a_different_block() {
        // This is what lets a mod turn one placement into another, and is why
        // the event carries a mutable block rather than just reporting it.
        let mut world = world();
        let dirt = world.registry().id_of("engine:dirt").unwrap();

        let mut events = EventBus::new();
        events.subscribe("swapper", move |event: &mut BlockPlaceEvent| {
            event.block = dirt;
        });

        let hit = hit_surface(&world, 4, 4);
        let stone = stone(&world);
        let placed = place_block(&mut world, &events, &hit, stone, |_| false).unwrap();

        assert_eq!(world.block(placed), dirt, "the substitution was ignored");
    }

    #[test]
    fn handlers_observe_the_position_and_block_being_edited() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_inner = Rc::clone(&seen);

        let mut world = world();
        let mut events = EventBus::new();
        events.subscribe("observer", move |event: &mut BlockBreakEvent| {
            seen_inner.borrow_mut().push((event.position, event.block));
        });

        let hit = hit_surface(&world, 6, 6);
        let removed = break_block(&mut world, &events, hit.block).unwrap();

        assert_eq!(*seen.borrow(), vec![(hit.block, removed)]);
    }

    #[test]
    fn editing_marks_the_chunk_for_remeshing() {
        let mut world = world();
        let events = EventBus::new();
        for pos in [ChunkPos::new(0, 0), ChunkPos::new(-1, 0)] {
            world.clear_dirty(pos);
        }

        let hit = hit_surface(&world, 4, 4);
        break_block(&mut world, &events, hit.block).unwrap();

        let dirty: Vec<_> = world.dirty_chunks().collect();
        assert!(dirty.contains(&ChunkPos::new(0, 0)), "the edited chunk is not dirty");
    }

    #[test]
    fn placing_above_the_world_ceiling_fails() {
        let mut world = world();
        let events = EventBus::new();
        let stone = stone(&world);

        // A hit on the topmost block, looking to build above it.
        let hit = RayHit {
            block: BlockPos::new(4, vx_core::CHUNK_HEIGHT - 1, 4),
            face: Face::PosY,
            distance: 1.0,
            id: stone,
        };

        assert_eq!(
            place_block(&mut world, &events, &hit, stone, |_| false),
            Err(EditError::OutOfRange)
        );
    }

    #[test]
    fn breaking_then_placing_restores_the_original_state() {
        let mut world = world();
        let events = EventBus::new();
        let hit = hit_surface(&world, 7, 3);

        let removed = break_block(&mut world, &events, hit.block).unwrap();
        assert!(world.block(hit.block).is_air());

        // Look down again; the ray now reaches the block underneath.
        let next = hit_surface(&world, 7, 3);
        let restored = place_block(&mut world, &events, &next, removed, |_| false).unwrap();

        assert_eq!(restored, hit.block);
        assert_eq!(world.block(hit.block), removed);
    }
}
