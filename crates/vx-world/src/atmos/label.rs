//! Per-section connected components of open space.
//!
//! Pure in the section's blocks: the same bytes in give the same labels
//! out, whatever order sections were built in. The neighbours only ever
//! read the six face arrays, never the voxels behind them.

use vx_core::{BlockPos, Face, LocalPos};

use super::{SectionPos, Sealing, SECTION};
use crate::chunk::Chunk;

/// Voxels in a section.
pub const VOXELS: usize = (SECTION * SECTION * SECTION) as usize;

/// Cells on one face of a section.
pub const FACE_CELLS: usize = (SECTION * SECTION) as usize;

/// A local component id. `0` is sealing; components count from one.
///
/// Sixteen bits rather than the note's eight: a section can in principle
/// hold up to two thousand one-voxel pockets, and a labeller that silently
/// merged the two hundred and fifty-sixth into the last would be the kind of
/// bug that only shows up in a checkerboard somebody built on purpose.
pub type Label = u16;

/// One section, labelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    /// Component per voxel, indexed `x + 16 z + 256 y`.
    pub labels: Vec<Label>,
    /// Per component (index `label - 1`): open voxels, the lowest voxel by
    /// `(x, y, z)` order, and the bounds — all in section-local coordinates.
    pub components: Vec<Component>,
    /// The label on each cell of each face, indexed by `Face as usize`.
    /// The cell index runs over the two axes the face does not, low axis
    /// fastest: `(y, z)` for the X faces, `(x, z)` for the Y faces, `(x, y)`
    /// for the Z faces.
    pub faces: Vec<[Label; FACE_CELLS]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    pub size: u16,
    pub lowest: [u8; 3],
    pub min: [u8; 3],
    pub max: [u8; 3],
}

fn index(x: i32, y: i32, z: i32) -> usize {
    (x + SECTION * z + SECTION * SECTION * y) as usize
}

/// The cell a voxel occupies on a face, if it lies on that face.
fn face_cell(face: Face, x: i32, y: i32, z: i32) -> Option<usize> {
    let last = SECTION - 1;
    let on = match face {
        Face::NegX => x == 0,
        Face::PosX => x == last,
        Face::NegY => y == 0,
        Face::PosY => y == last,
        Face::NegZ => z == 0,
        Face::PosZ => z == last,
    };
    if !on {
        return None;
    }
    Some(match face {
        Face::NegX | Face::PosX => (y + SECTION * z) as usize,
        Face::NegY | Face::PosY => (x + SECTION * z) as usize,
        Face::NegZ | Face::PosZ => (x + SECTION * y) as usize,
    })
}

/// Label one section of a chunk.
pub fn label_section(chunk: &Chunk, section_y: i32, sealing: &Sealing) -> Labels {
    let base_y = section_y * SECTION;
    let open: Vec<bool> = (0..VOXELS)
        .map(|voxel| {
            let voxel = voxel as i32;
            let (x, z, y) = (
                voxel % SECTION,
                (voxel / SECTION) % SECTION,
                voxel / (SECTION * SECTION),
            );
            let local = LocalPos::new(x, base_y + y, z).expect("a section voxel is in its chunk");
            !sealing.seals(chunk.get(local))
        })
        .collect();

    let mut labels = vec![0 as Label; VOXELS];
    let mut components = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..VOXELS {
        if !open[start] || labels[start] != 0 {
            continue;
        }
        let label = components.len() as Label + 1;
        let mut component = Component {
            size: 0,
            lowest: [u8::MAX; 3],
            min: [u8::MAX; 3],
            max: [0; 3],
        };
        labels[start] = label;
        stack.push(start);
        while let Some(voxel) = stack.pop() {
            let v = voxel as i32;
            let (x, z, y) = (v % SECTION, (v / SECTION) % SECTION, v / (SECTION * SECTION));
            component.size += 1;
            let here = [x as u8, y as u8, z as u8];
            if here < component.lowest {
                component.lowest = here;
            }
            for ((low, high), &at) in component
                .min
                .iter_mut()
                .zip(component.max.iter_mut())
                .zip(here.iter())
            {
                *low = (*low).min(at);
                *high = (*high).max(at);
            }
            for face in Face::ALL {
                let [dx, dy, dz] = face.offset();
                let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                if !(0..SECTION).contains(&nx)
                    || !(0..SECTION).contains(&ny)
                    || !(0..SECTION).contains(&nz)
                {
                    continue;
                }
                let next = index(nx, ny, nz);
                if open[next] && labels[next] == 0 {
                    labels[next] = label;
                    stack.push(next);
                }
            }
        }
        components.push(component);
    }

    let mut faces = vec![[0 as Label; FACE_CELLS]; 6];
    for (voxel, &label) in labels.iter().enumerate() {
        if label == 0 {
            continue;
        }
        let v = voxel as i32;
        let (x, z, y) = (v % SECTION, (v / SECTION) % SECTION, v / (SECTION * SECTION));
        for face in Face::ALL {
            if let Some(cell) = face_cell(face, x, y, z) {
                faces[face as usize][cell] = label;
            }
        }
    }

    Labels {
        labels,
        components,
        faces,
    }
}

impl Labels {
    /// The label at a section-local voxel.
    pub fn at(&self, x: i32, y: i32, z: i32) -> Label {
        self.labels[index(x, y, z)]
    }

    pub fn component(&self, label: Label) -> &Component {
        &self.components[label as usize - 1]
    }
}

/// A section-local voxel as a world position.
pub fn world_pos(section: SectionPos, local: [u8; 3]) -> BlockPos {
    let origin = section.origin();
    BlockPos::new(
        origin.x + i32::from(local[0]),
        origin.y + i32::from(local[1]),
        origin.z + i32::from(local[2]),
    )
}
