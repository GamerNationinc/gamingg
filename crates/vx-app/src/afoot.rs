//! The ground a *body* can cross, as opposed to the ground a drone can drive.
//!
//! # Why the drones' field is the wrong field
//!
//! [`vx_agent::FlowField`] has been the game's pathfinder since stage 5 and it
//! is a good one, but it answers a different question: it labels the cells a
//! machine that occupies one block and manages a one-block step can reach.
//! Stage 50's haul asked it about a mountainside and it said, correctly, that
//! almost nothing there was reachable — 2,673 cells out of a box of 218,000,
//! and not one of them closer to the next town than the ledge the walker was
//! already stood on.
//!
//! A person is not a ground drone. A body is two blocks tall, mantles a ledge
//! more than twice a drone's step ([`crate::movement::MANTLE_MAX`]), and steps
//! off things a drone would refuse because falling is free and landing is
//! cheap. On the slope that stopped the haul, that difference is the whole
//! difference between a wall and a staircase.
//!
//! So this is the same breadth-first sweep with a body's rules: two blocks of
//! headroom, a climb of two, and a drop of [`DROP`]. It is used only when the
//! legs have run out of ideas — see [`crate::session::Session::walk_to`] —
//! because the game deliberately has no route planner for the player and this
//! is not one: it is a sweep of what can be *seen from here*, which is what a
//! person gets when they walk up to a ridge and look along it.

use vx_core::BlockPos;
use vx_world::World;

/// Blocks of ledge a body will pull itself up. The mantle is 2.2, and the
/// fraction is reach, not standing room.
pub const CLIMB: i32 = 2;

/// Blocks a body will step off without thinking about it. Deeper than this is
/// a fall worth avoiding even though the legs would survive it, and a sweep
/// that treats a cliff edge as an ordinary step plans routes down cliffs.
pub const DROP: i32 = 6;

/// Can a body stand here? Floor beneath, and two blocks of it clear.
pub fn standable(world: &World, pos: BlockPos) -> bool {
    if !pos.in_vertical_bounds() || pos.y <= 1 {
        return false;
    }
    !world.is_solid(pos)
        && !world.is_solid(pos.offset([0, 1, 0]))
        && world.is_solid(pos.offset([0, -1, 0]))
}

/// The cell a body at `pos` ends up in once gravity has had its say.
pub fn settle(world: &World, pos: BlockPos) -> BlockPos {
    let mut at = pos;
    while at.y > 1 && !world.is_solid(at.offset([0, -1, 0])) {
        at = at.offset([0, -1, 0]);
    }
    at
}

/// The footing reachable in one step from `pos` in one horizontal direction.
///
/// Scanning downward from the highest cell a mantle reaches means a step up is
/// preferred to a step down when both are possible, which is what a person
/// walking a slope does and also what stops a route oscillating across a kerb.
fn footing(world: &World, pos: BlockPos, dx: i32, dz: i32) -> Option<BlockPos> {
    (-DROP..=CLIMB).rev().find_map(|dy| {
        let next = pos.offset([dx, dy, dz]);
        standable(world, next).then_some(next)
    })
}

/// A breadth-first sweep of the walkable ground around a place.
///
/// Holds the predecessor of every reached cell, so a route back out of it is a
/// walk up the chain rather than a second search.
pub struct Ground {
    min: BlockPos,
    size: [i32; 3],
    /// Index of the cell this one was reached from; `usize::MAX` for cells
    /// nothing reached, and its own index for the cell the sweep started at.
    came: Vec<usize>,
}

/// The marker for a cell no route reaches.
const UNTOUCHED: usize = usize::MAX;

impl Ground {
    /// Sweep outward from `from`, `look` blocks each way and `rise` up and
    /// down.
    ///
    /// `from` is settled first: a walker mid-vault is momentarily standing on
    /// nothing, and a sweep rooted in the air labels nothing at all.
    pub fn sweep(world: &World, from: BlockPos, look: i32, rise: i32) -> Ground {
        let from = settle(world, from);
        let min = BlockPos::new(from.x - look, from.y - rise, from.z - look);
        let size = [look * 2 + 1, rise * 2 + 1, look * 2 + 1];
        let mut ground = Ground {
            min,
            size,
            came: vec![UNTOUCHED; (size[0] * size[1] * size[2]) as usize],
        };
        let Some(root) = ground.index(from) else {
            return ground;
        };
        if !standable(world, from) {
            return ground;
        }
        ground.came[root] = root;

        let mut queue = std::collections::VecDeque::new();
        queue.push_back(from);
        while let Some(at) = queue.pop_front() {
            let here = ground.index(at).expect("queued cell is in bounds");
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let Some(next) = footing(world, at, dx, dz) else {
                    continue;
                };
                let Some(index) = ground.index(next) else {
                    continue;
                };
                if ground.came[index] != UNTOUCHED {
                    continue;
                }
                ground.came[index] = here;
                queue.push_back(next);
            }
        }
        ground
    }

    fn index(&self, pos: BlockPos) -> Option<usize> {
        let (x, y, z) = (pos.x - self.min.x, pos.y - self.min.y, pos.z - self.min.z);
        if x < 0 || y < 0 || z < 0 || x >= self.size[0] || y >= self.size[1] || z >= self.size[2] {
            return None;
        }
        Some(((y * self.size[2] + z) * self.size[0] + x) as usize)
    }

    fn at(&self, index: usize) -> BlockPos {
        let width = self.size[0];
        let depth = self.size[2];
        let x = index as i32 % width;
        let z = (index as i32 / width) % depth;
        let y = index as i32 / (width * depth);
        BlockPos::new(self.min.x + x, self.min.y + y, self.min.z + z)
    }

    /// Every cell the sweep reached.
    pub fn reachable(&self) -> impl Iterator<Item = BlockPos> + '_ {
        self.came
            .iter()
            .enumerate()
            .filter(|(_, came)| **came != UNTOUCHED)
            .map(|(index, _)| self.at(index))
    }

    /// The walk from the sweep's root to `pos`, root first and `pos` last.
    pub fn route_to(&self, pos: BlockPos) -> Option<Vec<BlockPos>> {
        let mut index = self.index(pos)?;
        if self.came[index] == UNTOUCHED {
            return None;
        }
        let mut route = vec![pos];
        while self.came[index] != index {
            index = self.came[index];
            route.push(self.at(index));
        }
        route.reverse();
        Some(route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockId, ChunkPos};

    /// A flat world: stone up to `y = 64`, air above it, so the standable
    /// layer is 65.
    fn flats() -> World {
        let mut world = World::new(7);
        world.load_around(ChunkPos::new(0, 0), 2);
        let stone = world.registry().id_of("engine:stone").expect("no stone");
        for x in -40..40 {
            for z in -40..40 {
                for y in 40..=64 {
                    world.set_block(BlockPos::new(x, y, z), stone);
                }
                for y in 65..80 {
                    world.set_block(BlockPos::new(x, y, z), BlockId::AIR);
                }
            }
        }
        world
    }

    fn wall(world: &mut World, x: i32, from_z: i32, to_z: i32, height: i32) {
        let stone = world.registry().id_of("engine:stone").expect("no stone");
        for z in from_z..=to_z {
            for y in 0..height {
                world.set_block(BlockPos::new(x, 65 + y, z), stone);
            }
        }
    }

    /// The floor of the test world is standable and the stone in it is not.
    #[test]
    fn a_body_stands_on_the_floor_and_not_in_it() {
        let world = flats();
        assert!(standable(&world, BlockPos::new(0, 65, 0)));
        assert!(!standable(&world, BlockPos::new(0, 64, 0)), "stood inside the floor");
        assert!(!standable(&world, BlockPos::new(0, 70, 0)), "stood in mid air");
    }

    /// Two blocks of headroom, not one. A body is 1.8 tall and a sweep that
    /// forgets it plans routes through crawlspaces it cannot use.
    #[test]
    fn a_body_does_not_fit_under_a_one_block_ceiling() {
        let mut world = flats();
        let stone = world.registry().id_of("engine:stone").expect("no stone");
        world.set_block(BlockPos::new(3, 66, 0), stone);
        assert!(!standable(&world, BlockPos::new(3, 65, 0)));
    }

    /// The difference from the drones' field, stated: a two-block ledge is a
    /// wall to a drone and a mantle to a body.
    #[test]
    fn a_body_climbs_what_a_drone_cannot() {
        let mut world = flats();
        wall(&mut world, 4, -8, 8, 2);
        // Fill in behind the step, so there is ground on top of it to stand on.
        let stone = world.registry().id_of("engine:stone").expect("no stone");
        for z in -8..=8 {
            for x in 4..=8 {
                world.set_block(BlockPos::new(x, 65, z), stone);
                world.set_block(BlockPos::new(x, 66, z), stone);
            }
        }
        let start = BlockPos::new(0, 65, 0);
        assert!(!vx_agent::FlowField::build(
            &world,
            vx_agent::VoxelAabb::new(
                BlockPos::new(-10, 60, -10),
                BlockPos::new(10, 75, 10)
            ),
            [start],
        )
        .is_reachable(BlockPos::new(6, 67, 0)));

        let ground = Ground::sweep(&world, start, 10, 10);
        assert!(
            ground.route_to(BlockPos::new(6, 67, 0)).is_some(),
            "a body could not get up a two-block step"
        );
    }

    /// The point of the sweep: a wall with a gap in it is walked round, and
    /// the route says where the gap was.
    #[test]
    fn the_sweep_finds_the_way_round_a_wall() {
        let mut world = flats();
        wall(&mut world, 4, -12, 5, 4);
        wall(&mut world, 4, 7, 12, 4);

        let ground = Ground::sweep(&world, BlockPos::new(0, 65, 0), 16, 12);
        let far = BlockPos::new(8, 65, 0);
        assert!(ground.route_to(far).is_some(), "the far side was never reached");
        let route = ground.route_to(far).expect("no route to the far side");
        assert_eq!(route.first(), Some(&BlockPos::new(0, 65, 0)));
        assert_eq!(route.last(), Some(&far));
        assert!(
            route.iter().any(|cell| cell.x == 4 && cell.z == 6),
            "the route did not go through the gap: {route:?}"
        );
        // Every step of it is somewhere a body can stand.
        for cell in &route {
            assert!(standable(&world, *cell), "the route stands in {cell:?}");
        }
    }

    /// A body sealed in a room reaches the room and nothing else, and the
    /// sweep says so rather than pretending.
    #[test]
    fn a_sealed_room_reaches_only_itself() {
        let mut world = flats();
        wall(&mut world, 2, -2, 2, 4);
        wall(&mut world, -2, -2, 2, 4);
        let stone = world.registry().id_of("engine:stone").expect("no stone");
        for x in -2..=2 {
            for y in 0..4 {
                world.set_block(BlockPos::new(x, 65 + y, 2), stone);
                world.set_block(BlockPos::new(x, 65 + y, -2), stone);
            }
        }
        let ground = Ground::sweep(&world, BlockPos::new(0, 65, 0), 16, 12);
        // Three by three of floor, and no way over a four-block wall.
        assert_eq!(ground.reachable().count(), 9, "the walls leaked");
        assert!(ground.route_to(BlockPos::new(8, 65, 0)).is_none());
    }

    /// The sweep is the same every run: it is used inside the replayed
    /// simulation, where a route that wandered would make the oracle
    /// meaningless.
    #[test]
    fn the_same_ground_gives_the_same_route() {
        let mut world = flats();
        wall(&mut world, 4, -12, 5, 4);
        let far = BlockPos::new(8, 65, 0);
        let once = Ground::sweep(&world, BlockPos::new(0, 65, 0), 16, 12).route_to(far);
        let twice = Ground::sweep(&world, BlockPos::new(0, 65, 0), 16, 12).route_to(far);
        assert_eq!(once, twice);
    }
}
