//! Block definitions and the runtime registry that owns them.
//!
//! Blocks are registered at startup: first by the engine's built-ins, then by
//! mods. A [`BlockId`] is a dense index into the registry, small enough to sit
//! in chunk storage cheaply. IDs are assigned in registration order, so they
//! are *not* stable across runs when the mod set changes — anything persisted
//! to disk must be keyed by the string name, not the numeric id.

use std::collections::HashMap;

use crate::face::Face;

/// A dense runtime handle for a block type.
///
/// Not stable across runs; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u16);

impl BlockId {
    /// Empty space. Always id 0, always present in a registry.
    pub const AIR: BlockId = BlockId(0);

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn is_air(self) -> bool {
        self.0 == BlockId::AIR.0
    }
}

/// How a block's geometry is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    /// The ordinary cube, greedy-meshed with its neighbours.
    #[default]
    Cube,
    /// Two crossed diagonal quads — the classic plant. Never culls or merges;
    /// always non-solid and non-opaque.
    Cross,
}

/// How a block is drawn and how it behaves physically.
// Not `Eq`: `hardness` is a float.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDef {
    /// Namespaced unique name, e.g. `engine:stone` or `mymod:copper_ore`.
    /// This is the identity that persists to disk.
    pub name: String,
    /// Shown to players.
    pub display_name: String,
    /// Blocks movement.
    pub solid: bool,
    /// Fully hides the neighbouring face, letting the mesher cull it. Glass is
    /// solid but not opaque; air is neither.
    pub opaque: bool,
    /// True when the block holds pressure. Independent of `solid`: glass and a
    /// closed hatch are sealing, a grate and a ladder are not, and a wounded
    /// block is sealing only while its damage mask still covers the face.
    /// See `ATMOSPHERE.md`.
    pub sealed: bool,
    /// Atlas tile index per face, indexed by `Face as usize`.
    pub textures: [u16; 6],
    /// Time multiplier to break. `None` means unbreakable.
    pub hardness: Option<f32>,
    /// Geometry: cube by default.
    pub shape: Shape,
}

impl BlockDef {
    /// A block with the same texture on all six faces.
    pub fn uniform(name: impl Into<String>, texture: u16) -> Self {
        let name = name.into();
        let display_name = default_display_name(&name);
        BlockDef {
            name,
            display_name,
            solid: true,
            opaque: true,
            sealed: true,
            textures: [texture; 6],
            hardness: Some(1.0),
            shape: Shape::Cube,
        }
    }

    /// A block textured like grass: distinct top, bottom and sides.
    pub fn columnar(name: impl Into<String>, top: u16, side: u16, bottom: u16) -> Self {
        let mut def = BlockDef::uniform(name, side);
        def.textures[Face::PosY as usize] = top;
        def.textures[Face::NegY as usize] = bottom;
        def
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }

    pub fn with_hardness(mut self, hardness: Option<f32>) -> Self {
        self.hardness = hardness;
        self
    }

    /// Mark as see-through: still solid, but does not cull neighbouring faces.
    pub fn translucent(mut self) -> Self {
        self.opaque = false;
        self
    }

    /// Mark as passable: no collision. Air passes too, unless
    /// [`BlockDef::sealed`] says otherwise afterwards.
    pub fn non_solid(mut self) -> Self {
        self.solid = false;
        self.sealed = false;
        self
    }

    /// Say outright whether the block holds pressure, where `solid` gets it
    /// wrong: a grate is solid and does not, water is not solid and does.
    pub fn sealed(mut self, sealed: bool) -> Self {
        self.sealed = sealed;
        self
    }

    /// Crossed-quad plant geometry. Cross blocks cannot block movement or
    /// hide a neighbour's face, so this implies non-solid and non-opaque.
    pub fn cross(mut self) -> Self {
        self.shape = Shape::Cross;
        self.solid = false;
        self.opaque = false;
        self
    }

    pub fn texture(&self, face: Face) -> u16 {
        self.textures[face as usize]
    }
}

/// Derive `engine:oak_log` -> `Oak Log` so mods get something reasonable for free.
fn default_display_name(name: &str) -> String {
    let bare = name.split_once(':').map_or(name, |(_, rest)| rest);
    let mut out = String::with_capacity(bare.len());
    for (i, word) in bare.split('_').filter(|w| !w.is_empty()).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        bare.to_string()
    } else {
        out
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("block '{0}' is already registered")]
    DuplicateName(String),
    #[error("block registry is full (limit {0} blocks)")]
    Full(usize),
}

/// Owns every known [`BlockDef`] and maps names to ids.
#[derive(Debug, Clone)]
pub struct BlockRegistry {
    defs: Vec<BlockDef>,
    by_name: HashMap<String, BlockId>,
}

impl BlockRegistry {
    /// Maximum registrable blocks, bounded by `BlockId`'s u16.
    pub const CAPACITY: usize = u16::MAX as usize + 1;

    /// A registry containing only air.
    pub fn new() -> Self {
        let air = BlockDef {
            name: "engine:air".to_string(),
            display_name: "Air".to_string(),
            solid: false,
            opaque: false,
            sealed: false,
            textures: [0; 6],
            hardness: None,
            shape: Shape::Cube,
        };
        let mut registry = BlockRegistry {
            defs: Vec::new(),
            by_name: HashMap::new(),
        };
        registry
            .register(air)
            .expect("air always registers into an empty registry");
        registry
    }

    /// Append a block, returning its freshly assigned id.
    pub fn register(&mut self, def: BlockDef) -> Result<BlockId, RegistryError> {
        if self.by_name.contains_key(&def.name) {
            return Err(RegistryError::DuplicateName(def.name));
        }
        if self.defs.len() >= Self::CAPACITY {
            return Err(RegistryError::Full(Self::CAPACITY));
        }
        let id = BlockId(self.defs.len() as u16);
        self.by_name.insert(def.name.clone(), id);
        self.defs.push(def);
        Ok(id)
    }

    pub fn get(&self, id: BlockId) -> Option<&BlockDef> {
        self.defs.get(id.index())
    }

    /// Look up a block, falling back to air for an out-of-range id rather than
    /// panicking — chunk data from disk or a mod can carry a stale id.
    pub fn get_or_air(&self, id: BlockId) -> &BlockDef {
        self.defs
            .get(id.index())
            .unwrap_or_else(|| &self.defs[BlockId::AIR.index()])
    }

    pub fn id_of(&self, name: &str) -> Option<BlockId> {
        self.by_name.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        // Air is always present, so this never reports true; kept for clippy
        // and API symmetry with `len`.
        self.defs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (BlockId, &BlockDef)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, def)| (BlockId(i as u16), def))
    }

    /// True when the block hides whatever is behind it, so the mesher can cull
    /// the touching face. Unknown ids are treated as transparent.
    pub fn is_opaque(&self, id: BlockId) -> bool {
        self.get(id).is_some_and(|def| def.opaque)
    }

    pub fn is_solid(&self, id: BlockId) -> bool {
        self.get(id).is_some_and(|def| def.solid)
    }
}

impl Default for BlockRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_is_id_zero_in_a_fresh_registry() {
        let registry = BlockRegistry::new();
        assert_eq!(registry.id_of("engine:air"), Some(BlockId::AIR));
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_opaque(BlockId::AIR));
        assert!(!registry.is_solid(BlockId::AIR));
    }

    #[test]
    fn ids_are_assigned_in_registration_order() {
        let mut registry = BlockRegistry::new();
        let stone = registry.register(BlockDef::uniform("engine:stone", 1)).unwrap();
        let dirt = registry.register(BlockDef::uniform("engine:dirt", 2)).unwrap();
        assert_eq!(stone, BlockId(1));
        assert_eq!(dirt, BlockId(2));
        assert_eq!(registry.id_of("engine:stone"), Some(stone));
        assert_eq!(registry.get(dirt).unwrap().name, "engine:dirt");
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let mut registry = BlockRegistry::new();
        registry.register(BlockDef::uniform("mymod:ore", 1)).unwrap();
        let err = registry.register(BlockDef::uniform("mymod:ore", 7)).unwrap_err();
        assert_eq!(err, RegistryError::DuplicateName("mymod:ore".to_string()));
        // The failed registration must not have consumed an id.
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn unknown_ids_resolve_to_air_rather_than_panicking() {
        let registry = BlockRegistry::new();
        let stale = BlockId(9999);
        assert!(registry.get(stale).is_none());
        assert_eq!(registry.get_or_air(stale).name, "engine:air");
        assert!(!registry.is_opaque(stale));
    }

    #[test]
    fn columnar_blocks_texture_each_face_correctly() {
        let def = BlockDef::columnar("engine:grass", 10, 11, 12);
        assert_eq!(def.texture(Face::PosY), 10);
        assert_eq!(def.texture(Face::NegY), 12);
        for face in [Face::NegX, Face::PosX, Face::NegZ, Face::PosZ] {
            assert_eq!(def.texture(face), 11);
        }
    }

    #[test]
    fn display_names_are_derived_from_the_namespaced_name() {
        assert_eq!(BlockDef::uniform("engine:oak_log", 0).display_name, "Oak Log");
        assert_eq!(BlockDef::uniform("engine:stone", 0).display_name, "Stone");
        assert_eq!(BlockDef::uniform("bare_name", 0).display_name, "Bare Name");
    }

    #[test]
    fn translucent_blocks_stay_solid_but_stop_culling() {
        let glass = BlockDef::uniform("engine:glass", 3).translucent();
        assert!(glass.solid);
        assert!(!glass.opaque);
    }
}
