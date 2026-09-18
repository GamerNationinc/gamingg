//! The room graph: section labels stitched across faces, walked on demand.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use vx_core::{BlockPos, BlockRegistry, Face};

use super::label::{self, Label, Labels};
use super::{SectionPos, Sealing, SEALED_VOLUME_MAX, SECTIONS_PER_COLUMN};
use crate::world::World;

/// An enclosed volume of open space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    /// The lowest open block by `(x, y, z)` order. Deterministic,
    /// recomputable from the blocks alone, and what a saved room will be
    /// keyed by when there is something to save.
    pub anchor: BlockPos,
    /// Open blocks.
    pub volume: u32,
    /// The bounds, inclusive. Free from labelling; what a fog volume or an
    /// audio bus would ask for.
    pub min: BlockPos,
    pub max: BlockPos,
    /// Every section-label the room is made of, so an edit in any of them
    /// can drop it.
    members: Vec<(SectionPos, Label)>,
}

/// What stands at a block, as far as the air is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict<'a> {
    Sealed(&'a Room),
    /// Open space that reaches the weather: past the budget, into an
    /// unloaded chunk, or out of the top of the column.
    Outdoors,
    /// A sealing block, or ground that is not loaded.
    Solid,
}

/// The graph, and its caches.
#[derive(Debug)]
pub struct Rooms {
    sealing: Sealing,
    sections: BTreeMap<SectionPos, Labels>,
    rooms: BTreeMap<BlockPos, Room>,
    /// Which room each section-label belongs to, for the lookup.
    claimed: BTreeMap<(SectionPos, Label), BlockPos>,
    /// How many chunks were loaded at the last refresh, so the section cache
    /// is only swept when residency moved.
    loaded: usize,
}

impl Rooms {
    pub fn new(registry: &BlockRegistry) -> Self {
        Rooms {
            sealing: Sealing::of(registry),
            sections: BTreeMap::new(),
            rooms: BTreeMap::new(),
            claimed: BTreeMap::new(),
            loaded: 0,
        }
    }

    /// Sealed rooms currently known.
    pub fn known(&self) -> usize {
        self.rooms.len()
    }

    /// Sections currently labelled.
    pub fn labelled(&self) -> usize {
        self.sections.len()
    }

    /// Catch up with what changed: relabel every edited section and forget
    /// every room an edit could have reached.
    pub fn refresh(&mut self, world: &mut World) {
        let edits = world.take_edits();
        let mut touched: BTreeSet<SectionPos> = BTreeSet::new();
        for pos in edits {
            let section = SectionPos::of(pos);
            self.sections.remove(&section);
            // A room next door may now extend into this section, and its
            // members would not say so: forget the neighbours' rooms too.
            touched.insert(section);
            touched.extend(Face::ALL.iter().filter_map(|face| section.neighbour(*face)));
        }
        if !touched.is_empty() {
            self.forget_rooms_touching(&touched);
        }

        // Chunks that went away take their sections and rooms with them.
        if world.loaded_chunk_count() != self.loaded {
            self.loaded = world.loaded_chunk_count();
            let gone: BTreeSet<SectionPos> = self
                .sections
                .keys()
                .filter(|section| !world.is_loaded(section.chunk))
                .copied()
                .collect();
            if !gone.is_empty() {
                self.sections.retain(|section, _| !gone.contains(section));
                self.forget_rooms_touching(&gone);
            }
        }
    }

    fn forget_rooms_touching(&mut self, sections: &BTreeSet<SectionPos>) {
        let dropped: Vec<BlockPos> = self
            .rooms
            .values()
            .filter(|room| room.members.iter().any(|(section, _)| sections.contains(section)))
            .map(|room| room.anchor)
            .collect();
        for anchor in dropped {
            if let Some(room) = self.rooms.remove(&anchor) {
                for member in room.members {
                    self.claimed.remove(&member);
                }
            }
        }
    }

    /// The labels of a section, computing them on first sight. `None` when
    /// its chunk is not loaded.
    fn labels(&mut self, world: &World, section: SectionPos) -> Option<&Labels> {
        if !self.sections.contains_key(&section) {
            let chunk = world.chunk(section.chunk)?;
            let labels = label::label_section(chunk, section.y, &self.sealing);
            self.sections.insert(section, labels);
        }
        self.sections.get(&section)
    }

    /// What the air knows about a block.
    pub fn room_at(&mut self, world: &World, pos: BlockPos) -> Verdict<'_> {
        if !pos.in_vertical_bounds() {
            return Verdict::Solid;
        }
        let section = SectionPos::of(pos);
        let origin = section.origin();
        let (x, y, z) = (pos.x - origin.x, pos.y - origin.y, pos.z - origin.z);
        let Some(labels) = self.labels(world, section) else {
            return Verdict::Solid;
        };
        let label = labels.at(x, y, z);
        if label == 0 {
            return Verdict::Solid;
        }
        let key = (section, label);
        let anchor = match self.claimed.get(&key) {
            Some(anchor) => *anchor,
            None => match self.walk(world, key) {
                Some(room) => {
                    let anchor = room.anchor;
                    for member in &room.members {
                        self.claimed.insert(*member, anchor);
                    }
                    self.rooms.insert(anchor, room);
                    anchor
                }
                None => return Verdict::Outdoors,
            },
        };
        Verdict::Sealed(&self.rooms[&anchor])
    }

    /// Walk the section graph from one section-label, over face arrays only.
    /// `None` is outdoors: the walk crossed the budget, reached an unloaded
    /// chunk, or reached the top of the column.
    fn walk(&mut self, world: &World, start: (SectionPos, Label)) -> Option<Room> {
        let mut seen: BTreeSet<(SectionPos, Label)> = BTreeSet::new();
        let mut queue: VecDeque<(SectionPos, Label)> = VecDeque::new();
        seen.insert(start);
        queue.push_back(start);

        let mut volume = 0u32;
        let mut anchor: Option<BlockPos> = None;
        let mut min = BlockPos::new(i32::MAX, i32::MAX, i32::MAX);
        let mut max = BlockPos::new(i32::MIN, i32::MIN, i32::MIN);
        let mut members = Vec::new();

        while let Some((section, label)) = queue.pop_front() {
            let component = *self.labels(world, section)?.component(label);
            volume += u32::from(component.size);
            if volume > SEALED_VOLUME_MAX {
                return None;
            }
            let lowest = label::world_pos(section, component.lowest);
            if anchor.is_none_or(|held| lowest < held) {
                anchor = Some(lowest);
            }
            let low = label::world_pos(section, component.min);
            let high = label::world_pos(section, component.max);
            min = BlockPos::new(min.x.min(low.x), min.y.min(low.y), min.z.min(low.z));
            max = BlockPos::new(max.x.max(high.x), max.y.max(high.y), max.z.max(high.z));
            members.push((section, label));

            for face in Face::ALL {
                let mine = self.sections[&section].faces[face as usize];
                if !mine.contains(&label) {
                    continue;
                }
                let Some(next) = section.neighbour(face) else {
                    // Below the world is bedrock; above it is the sky.
                    if face == Face::PosY && section.y == SECTIONS_PER_COLUMN - 1 {
                        return None;
                    }
                    continue;
                };
                let theirs = self.labels(world, next)?.faces[face.opposite() as usize];
                for cell in 0..label::FACE_CELLS {
                    if mine[cell] == label && theirs[cell] != 0 {
                        let key = (next, theirs[cell]);
                        if seen.insert(key) {
                            queue.push_back(key);
                        }
                    }
                }
            }
        }

        Some(Room {
            anchor: anchor?,
            volume,
            min,
            max,
            members,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{BlockId, ChunkPos};

    fn world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 2);
        world
    }

    fn block(world: &World, name: &str) -> BlockId {
        world.registry().id_of(name).expect(name)
    }

    /// A hollow box of metal wall from `min` to `max` inclusive, its inside
    /// cleared. Returns the open volume it encloses.
    fn shell(world: &mut World, min: BlockPos, max: BlockPos) -> u32 {
        let metal = block(world, "engine:metal_wall");
        for y in min.y..=max.y {
            for z in min.z..=max.z {
                for x in min.x..=max.x {
                    let wall = x == min.x
                        || x == max.x
                        || y == min.y
                        || y == max.y
                        || z == min.z
                        || z == max.z;
                    let at = BlockPos::new(x, y, z);
                    world.set_block(at, if wall { metal } else { BlockId::AIR });
                }
            }
        }
        ((max.x - min.x - 1) * (max.y - min.y - 1) * (max.z - min.z - 1)) as u32
    }

    /// A hut in the air over the hometown, where nothing else is, and
    /// wholly inside one section.
    const HUT_MIN: BlockPos = BlockPos::new(2, 200, 2);
    const HUT_MAX: BlockPos = BlockPos::new(8, 204, 8);

    fn sealed_volume(rooms: &mut Rooms, world: &World, at: BlockPos) -> Option<(BlockPos, u32)> {
        match rooms.room_at(world, at) {
            Verdict::Sealed(room) => Some((room.anchor, room.volume)),
            _ => None,
        }
    }

    #[test]
    fn a_sealed_hut_is_one_room_of_its_own_volume() {
        let mut world = world();
        let volume = shell(&mut world, HUT_MIN, HUT_MAX);
        assert_eq!(volume, 75);
        let mut rooms = Rooms::new(world.registry());
        rooms.refresh(&mut world);

        let inside = HUT_MIN.offset([1, 1, 1]);
        let Verdict::Sealed(room) = rooms.room_at(&world, inside) else {
            panic!("the hut is not a room");
        };
        assert_eq!(room.volume, 75);
        assert_eq!(room.anchor, inside, "the anchor is not the lowest open block");
        assert_eq!(room.min, inside);
        assert_eq!(room.max, HUT_MAX.offset([-1, -1, -1]));
        assert_eq!(rooms.known(), 1);

        // The same room from its far corner, and nothing from its wall or
        // from the air outside it.
        let far = HUT_MAX.offset([-1, -1, -1]);
        assert_eq!(sealed_volume(&mut rooms, &world, far), Some((inside, 75)));
        assert_eq!(rooms.room_at(&world, HUT_MIN), Verdict::Solid);
        assert_eq!(rooms.room_at(&world, HUT_MAX.offset([2, 0, 0])), Verdict::Outdoors);
        assert_eq!(rooms.known(), 1, "the outdoors was kept");
    }

    #[test]
    fn opening_one_block_in_the_wall_makes_it_outdoors_and_closing_it_brings_the_room_back() {
        let mut world = world();
        shell(&mut world, HUT_MIN, HUT_MAX);
        let mut rooms = Rooms::new(world.registry());
        rooms.refresh(&mut world);
        let inside = HUT_MIN.offset([1, 1, 1]);
        assert_eq!(sealed_volume(&mut rooms, &world, inside), Some((inside, 75)));

        let hole = BlockPos::new(HUT_MAX.x, HUT_MIN.y + 2, HUT_MIN.z + 3);
        let metal = world.block(hole);
        world.set_block(hole, BlockId::AIR);
        rooms.refresh(&mut world);
        assert_eq!(rooms.room_at(&world, inside), Verdict::Outdoors, "a hole did not vent");
        assert_eq!(rooms.known(), 0, "a vented room was kept");

        world.set_block(hole, metal);
        rooms.refresh(&mut world);
        assert_eq!(sealed_volume(&mut rooms, &world, inside), Some((inside, 75)));
    }

    #[test]
    fn a_room_spanning_section_faces_is_one_room() {
        // Straddles chunks (-1, -1)..(0, 0) and sections 11 and 12.
        let mut world = world();
        let (min, max) = (BlockPos::new(-3, 188, -3), BlockPos::new(3, 196, 3));
        let volume = shell(&mut world, min, max);
        assert_eq!(volume, 5 * 7 * 5);
        let mut rooms = Rooms::new(world.registry());
        rooms.refresh(&mut world);
        let inside = min.offset([1, 1, 1]);
        let Verdict::Sealed(room) = rooms.room_at(&world, inside) else {
            panic!("the straddling hut is not a room");
        };
        assert_eq!(room.volume, volume);
        assert_eq!(room.anchor, inside);
        assert!(room.members.len() >= 8, "only {} section-labels", room.members.len());
        // Every corner of the interior answers with the same room.
        for corner in [
            BlockPos::new(2, 195, 2),
            BlockPos::new(-2, 189, 2),
            BlockPos::new(2, 189, -2),
        ] {
            assert_eq!(sealed_volume(&mut rooms, &world, corner), Some((inside, volume)));
        }
        assert_eq!(rooms.known(), 1);
    }

    #[test]
    fn labelling_is_pure_in_the_sections_blocks() {
        let mut world = world();
        shell(&mut world, HUT_MIN, HUT_MAX);
        let sealing = Sealing::of(world.registry());
        let chunk = world.chunk(ChunkPos::new(0, 0)).unwrap();
        let once = label::label_section(chunk, 200 / 16, &sealing);
        let twice = label::label_section(chunk, 200 / 16, &sealing);
        assert_eq!(once, twice);
        // The hut's interior is one component of seventy-five, and the air
        // round it is another that reaches every face.
        let hut = once.at(3, 200 % 16 + 1, 3);
        assert_ne!(hut, 0);
        assert_eq!(once.component(hut).size, 75);
        let sky = once.at(0, 0, 0);
        assert_ne!(sky, hut);
        for face in Face::ALL {
            assert!(once.faces[face as usize].contains(&sky), "the sky misses a face");
            assert!(!once.faces[face as usize].contains(&hut), "the hut reaches a face");
        }
    }

    #[test]
    fn relabelling_one_section_matches_a_full_rebuild() {
        let mut world = world();
        shell(&mut world, HUT_MIN, HUT_MAX);
        let mut rooms = Rooms::new(world.registry());
        rooms.refresh(&mut world);
        let inside = HUT_MIN.offset([1, 1, 1]);
        rooms.room_at(&world, inside);

        // Partition the hut with a wall down the middle, then take one block
        // of it back out: two rooms, then one again, incrementally.
        let metal = block(&world, "engine:metal_wall");
        for y in HUT_MIN.y + 1..HUT_MAX.y {
            for z in HUT_MIN.z + 1..HUT_MAX.z {
                world.set_block(BlockPos::new(5, y, z), metal);
            }
        }
        rooms.refresh(&mut world);
        let west = sealed_volume(&mut rooms, &world, inside).expect("west half vanished");
        let east =
            sealed_volume(&mut rooms, &world, BlockPos::new(6, 201, 3)).expect("east half");
        assert_eq!((west.1, east.1), (30, 30));
        assert_ne!(west.0, east.0);
        world.set_block(BlockPos::new(5, 202, 5), BlockId::AIR);
        rooms.refresh(&mut world);
        let joined = sealed_volume(&mut rooms, &world, inside).expect("the halves did not join");
        assert_eq!(joined.1, 61);

        let mut fresh = Rooms::new(world.registry());
        fresh.refresh(&mut world);
        let rebuilt = sealed_volume(&mut fresh, &world, inside).unwrap();
        assert_eq!(joined, rebuilt, "the incremental graph disagrees with a rebuild");
        assert_eq!(rooms.known(), fresh.known());
    }

    #[test]
    fn a_room_that_grows_past_the_budget_becomes_outdoors() {
        let mut world = world();
        // Thirty-three to a side inside: past the thirty-two cube budget.
        let (min, max) = (BlockPos::new(-17, 100, -17), BlockPos::new(17, 134, 17));
        let volume = shell(&mut world, min, max);
        assert!(volume > SEALED_VOLUME_MAX);
        let mut rooms = Rooms::new(world.registry());
        rooms.refresh(&mut world);
        assert_eq!(rooms.room_at(&world, min.offset([1, 1, 1])), Verdict::Outdoors);
        assert_eq!(rooms.known(), 0);
    }

    #[test]
    fn a_grate_does_not_seal_and_water_does() {
        let mut world = world();
        shell(&mut world, HUT_MIN, HUT_MAX);
        let inside = HUT_MIN.offset([1, 1, 1]);
        let hole = BlockPos::new(HUT_MAX.x, HUT_MIN.y + 2, HUT_MIN.z + 3);
        let mut rooms = Rooms::new(world.registry());

        world.set_block(hole, block(&world, "engine:catwalk"));
        rooms.refresh(&mut world);
        assert_eq!(rooms.room_at(&world, inside), Verdict::Outdoors, "a grate held air");

        world.set_block(hole, block(&world, "engine:water"));
        rooms.refresh(&mut world);
        assert_eq!(sealed_volume(&mut rooms, &world, inside), Some((inside, 75)));
        // And a flooded block inside is not room air.
        world.set_block(BlockPos::new(4, 201, 4), block(&world, "engine:water"));
        rooms.refresh(&mut world);
        assert_eq!(sealed_volume(&mut rooms, &world, inside), Some((inside, 74)));
    }

    /// The finding stage B has to answer with a hatch block: a bunker's
    /// entry stair is open to the sky, so the whole works is outdoors.
    #[test]
    fn a_bunker_is_outdoors_through_its_own_stair() {
        let mut world = World::new(2024);
        let height = |x: i32, z: i32| world.generator().height_at(x, z);
        let site = crate::bunker::bunkers_near(2024, (0, 0), crate::bunker::CELL * 4, &height)
            .into_iter()
            .next()
            .expect("no bunker near the hometown");
        let plan = crate::bunker::layout(&site);
        let (&(x, y, z), _) = plan
            .cells()
            .find(|(_, cell)| **cell == crate::bunker::Cell::Air)
            .expect("a bunker with no air in it");
        let centre = BlockPos::new(site.centre.0, y, site.centre.1).chunk();
        world.load_around(centre, site.reach() / 16 + 2);
        let mut rooms = Rooms::new(world.registry());
        rooms.refresh(&mut world);
        assert_eq!(rooms.room_at(&world, BlockPos::new(x, y, z)), Verdict::Outdoors);
    }
}
