//! The minimap: where you have been, and where everything is right now.
//!
//! # The explored set is the only state
//!
//! Worldgen is a pure function of `(seed, position)`, so the surface of an
//! explored-but-unloaded chunk can be recomputed from the generator's height
//! field on demand. The map therefore stores no terrain thumbnails at all —
//! just the set of chunks you (or the flier) have seen. Accepted consequence:
//! edits and mine holes show only while their chunks are loaded; distant
//! explored land draws as generated.
//!
//! # Player knowledge, not world truth
//!
//! The explored set persists in its own small file beside the region files
//! rather than inside them, because it belongs to the player rather than the
//! world. Losing it is cosmetic, so a damaged file logs and starts empty —
//! it must never take the world down with it.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;

use vx_core::{BlockPos, ChunkPos, CHUNK_SIZE};
use vx_world::town::{self, TownSite};
use vx_world::{World, SEA_LEVEL};

/// Side length of the map image, in pixels.
pub const MAP_SIZE: u32 = 192;

/// Redraw the picture at most this often, in frames. The terrain sampling is
/// the whole cost of the map, and 6 Hz is indistinguishable from live for
/// something that scrolls at walking speed.
pub const REDRAW_INTERVAL: u32 = 10;

const MAGIC: &[u8; 4] = b"VXMP";
const VERSION: u32 = 1;

/// A dot on the map, in world column coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Marker {
    pub x: i32,
    pub z: i32,
    pub colour: [u8; 4],
    /// Half-width in pixels, so important dots survive zooming out.
    pub radius: i32,
}

/// Marker colours, kept together so the legend stays consistent.
pub mod colour {
    pub const PLAYER: [u8; 4] = [255, 255, 255, 255];
    pub const DRONE: [u8; 4] = [25, 25, 30, 255];
    pub const FLIER: [u8; 4] = [120, 200, 255, 255];
    pub const PING: [u8; 4] = [255, 140, 40, 255];
    pub const BASE: [u8; 4] = [80, 255, 170, 255];
    /// A town you have actually stood in.
    pub const TOWN: [u8; 4] = [235, 235, 240, 255];
    /// Where an accepted posting wants you — drawn whether or not the ground
    /// around it has ever been seen.
    pub const CONTRACT: [u8; 4] = [255, 90, 140, 255];
    /// The Ruined City. Drawn from the first frame, over the fog, because
    /// knowing where it is has never been the difficulty — getting there is.
    pub const CITY: [u8; 4] = [255, 205, 90, 255];
    /// A load on the trade network, yours or a town's.
    pub const TRADE: [u8; 4] = [255, 214, 90, 255];
    /// A contact the kestrel reported: where something *was* seen.
    pub const MARK: [u8; 4] = [255, 70, 70, 255];

    /// The mark colour, faded by how old the report is: fresh intelligence
    /// and stale intelligence must not read the same.
    pub fn mark_aged(age: u64) -> [u8; 4] {
        let fade = (age.min(crate::scout::MARK_DECAY) * 160 / crate::scout::MARK_DECAY) as u8;
        let [r, g, b, a] = MARK;
        [
            r.saturating_sub(fade / 2),
            g.saturating_sub(fade / 4),
            b.saturating_sub(fade / 4),
            a.saturating_sub(fade),
        ]
    }
}

/// The map's persistent and per-session state.
#[derive(Debug, Default)]
pub struct MapState {
    /// Chunks the player has been near or the flier has swept.
    explored: HashSet<ChunkPos>,
    pub visible: bool,
    /// Blocks per map pixel: 1, 2 or 4.
    pub zoom: i32,
    /// Frames until the next redraw is allowed.
    cooldown: u32,
}

impl MapState {
    pub fn new() -> Self {
        MapState {
            explored: HashSet::new(),
            visible: true,
            zoom: 2,
            cooldown: 0,
        }
    }

    /// Mark every chunk within `radius` of `centre` as explored.
    pub fn explore_around(&mut self, centre: ChunkPos, radius: i32) {
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                self.explored.insert(ChunkPos::new(centre.x + dx, centre.z + dz));
            }
        }
    }

    pub fn explore(&mut self, chunk: ChunkPos) {
        self.explored.insert(chunk);
    }

    pub fn is_explored(&self, chunk: ChunkPos) -> bool {
        self.explored.contains(&chunk)
    }

    pub fn explored_count(&self) -> usize {
        self.explored.len()
    }

    /// Step the zoom through 1, 2, 4 blocks per pixel.
    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom / 2).max(1);
    }

    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom * 2).min(4);
    }

    /// Count down the redraw throttle; true when a redraw is due.
    pub fn should_redraw(&mut self) -> bool {
        if self.cooldown == 0 {
            // Minus one so "every REDRAW_INTERVAL frames" means what it says:
            // one fire per interval of calls, not per interval-plus-one.
            self.cooldown = REDRAW_INTERVAL - 1;
            true
        } else {
            self.cooldown -= 1;
            false
        }
    }

    /// Force the next `should_redraw` to fire — after a zoom change, say.
    pub fn invalidate(&mut self) {
        self.cooldown = 0;
    }

    /// Write the explored set beside the world save.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        // Written atomically enough for a cosmetic file: a torn write is
        // caught by the tolerant loader and simply forgotten.
        let mut file = crate::keeping::begin(directory, "explored.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&(self.explored.len() as u64).to_le_bytes())?;
        // Sorted, so the file is deterministic for a given set.
        let mut chunks: Vec<ChunkPos> = self.explored.iter().copied().collect();
        chunks.sort_by_key(|chunk| (chunk.x, chunk.z));
        for chunk in chunks {
            file.write_all(&chunk.x.to_le_bytes())?;
            file.write_all(&chunk.z.to_le_bytes())?;
        }
        file.commit()
    }

    /// Load the explored set, tolerating absence and damage.
    ///
    /// Anything wrong with the file — missing, truncated, wrong magic — logs
    /// and yields an empty set. The map is player knowledge; a corrupted
    /// memory is forgotten ground, not a failed world.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("explored.dat");
        match read_explored(&path) {
            Ok(Some(chunks)) => {
                log::info!("loaded {} explored chunks", chunks.len());
                self.explored = chunks;
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("could not read {}: {error}; starting unexplored", path.display());
                self.explored.clear();
            }
        }
    }
}

fn read_explored(path: &Path) -> std::io::Result<Option<HashSet<ChunkPos>>> {
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
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != VERSION {
        return Err(std::io::Error::other("unknown version"));
    }
    let mut count_bytes = [0u8; 8];
    file.read_exact(&mut count_bytes)?;
    let count = u64::from_le_bytes(count_bytes);

    let mut chunks = HashSet::new();
    for _ in 0..count {
        let mut x = [0u8; 4];
        let mut z = [0u8; 4];
        file.read_exact(&mut x)?;
        file.read_exact(&mut z)?;
        chunks.insert(ChunkPos::new(i32::from_le_bytes(x), i32::from_le_bytes(z)));
    }
    Ok(Some(chunks))
}

/// The colour class of one column, before height shading.
///
/// `sites` is the towns overlapping the whole picture, gathered once by
/// [`render_map`]. Passing them in matters: [`vx_world::gen::TerrainGenerator::height_at`]
/// runs the town lattice for every column it is asked about, and a map redraw
/// asks about tens of thousands of them.
fn column_colour(
    world: &World,
    state: &MapState,
    sites: &[TownSite],
    x: i32,
    z: i32,
) -> Option<([f32; 3], i32)> {
    let chunk = BlockPos::new(x, 0, z).chunk();

    if world.is_loaded(chunk) {
        let clear = world.surface_y(x, z)?;
        let top = clear - 1;
        let name = world
            .registry()
            .get(world.block(BlockPos::new(x, top, z)))
            .map(|def| def.name.as_str())
            .unwrap_or("");
        let colour = if name.ends_with("_ore") {
            [0.90, 0.48, 0.18]
        } else {
            match name {
                "engine:water" => [0.24, 0.36, 0.65],
                "engine:ice" => [0.70, 0.82, 0.92],
                "engine:snowy_grass" | "engine:snowy_sphagnum" | "engine:snowy_sand" => {
                    [0.90, 0.93, 0.96]
                }
                "engine:grass" => [0.28, 0.52, 0.24],
                "engine:sand" => [0.78, 0.72, 0.51],
                "engine:dirt" => [0.44, 0.32, 0.22],
                "engine:container" => [0.63, 0.67, 0.73],
                "engine:plank" | "engine:counter" => [0.62, 0.45, 0.26],
                "engine:roof" => [0.52, 0.24, 0.17],
                "engine:log" => [0.38, 0.27, 0.16],
                "engine:leaves" | "engine:tall_grass" => [0.20, 0.42, 0.16],
                _ => [0.51, 0.51, 0.53],
            }
        };
        return Some((colour, top));
    }

    if state.is_explored(chunk) {
        // Unloaded but seen: recompute the generated surface. Edits there are
        // not shown — the accepted trade for storing nothing.
        let height = world.generator().height_with_sites(x, z, sites);
        if town::core_contains(sites, x, z).is_some() {
            // A levelled plot reads as metal from the air, the way it looks
            // from the ground.
            return Some(([0.63, 0.67, 0.73], height));
        }
        let colour = if height < SEA_LEVEL {
            [0.24, 0.36, 0.65]
        } else if height < SEA_LEVEL + 2 {
            [0.78, 0.72, 0.51]
        } else {
            [0.28, 0.52, 0.24]
        };
        return Some((colour, height.max(SEA_LEVEL)));
    }

    None
}

/// Render the map image: `MAP_SIZE` square, centred on `centre`, one column
/// sampled per pixel at `zoom` blocks per pixel. Markers are stamped on top.
pub fn render_map(
    world: &World,
    state: &MapState,
    centre: (i32, i32),
    markers: &[Marker],
) -> Vec<u8> {
    render_map_sized(world, state, centre, state.zoom.max(1), MAP_SIZE, markers)
}

/// The same picture at any size and zoom.
///
/// Panels want a smaller map than the corner minimap does, and a trade console
/// wants a zoom that fits two towns rather than the player's chosen one. The
/// fog and marker passes are shared with [`render_map`] — which matters,
/// because the behaviour a paper map wants is already theirs: unexplored ground
/// draws as a dark pane and markers stamp over it with no visibility test, so a
/// pin in the black costs nothing.
pub fn render_map_sized(
    world: &World,
    state: &MapState,
    centre: (i32, i32),
    zoom: i32,
    edge: u32,
    markers: &[Marker],
) -> Vec<u8> {
    let size = edge as i32;
    let zoom = zoom.max(1);
    let mut pixels = vec![0u8; (edge * edge * 4) as usize];

    // One lattice gather for the whole picture rather than one per pixel.
    let half = size / 2 * zoom;
    let sites = world.towns_overlapping(
        (centre.0 - half, centre.1 - half),
        (centre.0 + half, centre.1 + half),
    );

    for py in 0..size {
        for px in 0..size {
            let x = centre.0 + (px - size / 2) * zoom;
            let z = centre.1 + (py - size / 2) * zoom;
            let at = ((py * size + px) * 4) as usize;

            match column_colour(world, state, &sites, x, z) {
                Some((base, height)) => {
                    // Height shading: high ground bright, valleys dark, so
                    // relief reads at a glance.
                    let shade = 0.55 + 0.45 * ((height - 40).clamp(0, 100) as f32 / 100.0);
                    pixels[at] = (base[0] * shade * 255.0) as u8;
                    pixels[at + 1] = (base[1] * shade * 255.0) as u8;
                    pixels[at + 2] = (base[2] * shade * 255.0) as u8;
                    pixels[at + 3] = 235;
                }
                None => {
                    // Unexplored: a dark pane, translucent enough to hint at
                    // the world behind the map.
                    pixels[at] = 10;
                    pixels[at + 1] = 12;
                    pixels[at + 2] = 16;
                    pixels[at + 3] = 170;
                }
            }
        }
    }

    for marker in markers {
        let px = (marker.x - centre.0) / zoom + size / 2;
        let py = (marker.z - centre.1) / zoom + size / 2;
        for dy in -marker.radius..=marker.radius {
            for dx in -marker.radius..=marker.radius {
                let (sx, sy) = (px + dx, py + dy);
                if (0..size).contains(&sx) && (0..size).contains(&sy) {
                    let at = ((sy * size + sx) * 4) as usize;
                    pixels[at..at + 4].copy_from_slice(&marker.colour);
                }
            }
        }
    }

    pixels
}

/// The compass point and distance from one column to another: `"NE 1420M"`.
///
/// North is -Z, matching the camera's convention, so the eight points read the
/// way they do on the ground rather than the way they do in the array.
pub fn bearing(from: (i32, i32), to: (i32, i32)) -> String {
    let (dx, dz) = ((to.0 - from.0) as f32, (to.1 - from.1) as f32);
    let distance = (dx * dx + dz * dz).sqrt().round() as i64;
    if distance == 0 {
        return "HERE".to_string();
    }

    // Angle clockwise from north, which is -Z.
    let angle = dx.atan2(-dz).to_degrees().rem_euclid(360.0);
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    // Half a sector of offset, so due north spans 337.5..22.5 rather than
    // starting at zero and calling anything slightly west of north "NW".
    let point = POINTS[(((angle + 22.5) / 45.0) as usize) % 8];
    format!("{point} {distance}M")
}

/// Paste a square map into a larger panel at `(left, top)`.
///
/// Composited source-over rather than copied. Unexplored ground is drawn
/// *translucent* on purpose — on the corner minimap it hints at the world
/// behind it — but a panel already has its own background, and copying that
/// translucency straight in would punch a hole through the panel and show the
/// game through it. Blending puts the dark pane over the panel's own dark
/// instead, which is what a paper map should look like.
pub fn blit(
    panel: &mut [u8],
    panel_width: u32,
    map: &[u8],
    edge: u32,
    left: i32,
    top: i32,
) {
    let panel_height = panel.len() as i32 / (panel_width as i32 * 4);
    for row in 0..edge as i32 {
        let y = top + row;
        if !(0..panel_height).contains(&y) {
            continue;
        }
        for column in 0..edge as i32 {
            let x = left + column;
            if !(0..panel_width as i32).contains(&x) {
                continue;
            }
            let from = ((row * edge as i32 + column) * 4) as usize;
            let to = ((y * panel_width as i32 + x) * 4) as usize;
            let alpha = map[from + 3] as u32;
            for channel in 0..3 {
                let over = map[from + channel] as u32 * alpha;
                let under = panel[to + channel] as u32 * (255 - alpha);
                panel[to + channel] = ((over + under) / 255) as u8;
            }
            // The panel keeps its own opacity; the map never makes it see-through.
            panel[to + 3] = panel[to + 3].max(map[from + 3]);
        }
    }
}

/// Draw a hairline frame around a blitted map.
///
/// Unexplored ground and a panel's own background are very nearly the same
/// colour, so without this an inset map reads as a hole rather than a map.
pub fn frame(
    panel: &mut [u8],
    panel_width: u32,
    edge: u32,
    left: i32,
    top: i32,
    colour: [u8; 4],
) {
    let panel_height = panel.len() as i32 / (panel_width as i32 * 4);
    let edge = edge as i32;
    for step in -1..=edge {
        for (x, y) in [
            (left + step, top - 1),
            (left + step, top + edge),
            (left - 1, top + step),
            (left + edge, top + step),
        ] {
            if (0..panel_width as i32).contains(&x) && (0..panel_height).contains(&y) {
                let at = ((y * panel_width as i32 + x) * 4) as usize;
                panel[at..at + 4].copy_from_slice(&colour);
            }
        }
    }
}

/// Every chunk covered by a scan sector, for marking swept ground explored.
pub fn sector_chunks(sector: vx_agent::Sector) -> impl Iterator<Item = ChunkPos> {
    let (min_x, min_z) = sector.min_column();
    let chunks = vx_agent::SECTOR_SIZE / CHUNK_SIZE;
    let base = ChunkPos::new(min_x.div_euclid(CHUNK_SIZE), min_z.div_euclid(CHUNK_SIZE));
    (0..chunks).flat_map(move |dx| (0..chunks).map(move |dz| ChunkPos::new(base.x + dx, base.z + dz)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;

    fn world_with_origin() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 2);
        world
    }

    #[test]
    fn exploring_grows_and_never_shrinks() {
        let mut state = MapState::new();
        state.explore_around(ChunkPos::new(0, 0), 2);
        let first = state.explored_count();
        assert_eq!(first, 25);

        state.explore_around(ChunkPos::new(1, 1), 2);
        assert!(state.explored_count() >= first, "exploring lost ground");
        // Re-exploring the same ground adds nothing.
        let again = state.explored_count();
        state.explore_around(ChunkPos::new(0, 0), 2);
        assert_eq!(state.explored_count(), again);
    }

    #[test]
    fn the_explored_set_round_trips_through_disk() {
        let directory = std::env::temp_dir().join(format!("vx-map-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut state = MapState::new();
        state.explore(ChunkPos::new(3, -7));
        state.explore(ChunkPos::new(-100, 42));
        state.save(&directory).unwrap();

        let mut loaded = MapState::new();
        loaded.load(&directory);
        assert_eq!(loaded.explored_count(), 2);
        assert!(loaded.is_explored(ChunkPos::new(3, -7)));
        assert!(loaded.is_explored(ChunkPos::new(-100, 42)));

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_corrupt_explored_file_yields_empty_not_failure() {
        let directory = std::env::temp_dir().join(format!("vx-map-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("explored.dat"), b"garbage").unwrap();

        let mut state = MapState::new();
        state.explore(ChunkPos::new(1, 1));
        state.load(&directory);
        assert_eq!(state.explored_count(), 0, "corrupt file should reset, not crash");

        std::fs::remove_dir_all(&directory).ok();
    }

    /// The sibling of the water probe below: snow reads white and ice reads
    /// a paler blue than the lake it was.
    #[test]
    fn snow_reads_white_and_ice_reads_pale_blue() {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(12, 12), 2);
        let mut state = MapState::new();
        state.explore_around(ChunkPos::new(12, 12), 2);
        let size = MAP_SIZE as i32;
        let zoom = state.zoom;
        let column = |px: i32, py: i32| (200 + (px - size / 2) * zoom, 200 + (py - size / 2) * zoom);
        let ice = world.registry().id_of("engine:ice").unwrap();
        let snow = world.registry().id_of("engine:snowy_grass").unwrap();
        let water = world.registry().id_of("engine:water").unwrap();
        let mut crown = |px: i32, py: i32, block: vx_core::BlockId| {
            let (x, z) = column(px, py);
            let top = world.surface_y(x, z).unwrap() - 1;
            world.set_block(vx_core::BlockPos::new(x, top, z), block);
        };
        // Three columns a few pixels apart, all inside the explored ring.
        crown(96, 96, ice);
        crown(90, 96, snow);
        crown(96, 90, water);

        let pixels = render_map(&world, &state, (200, 200), &[]);
        let read = |px: i32, py: i32| {
            let at = ((py * size + px) * 4) as usize;
            (pixels[at], pixels[at + 1], pixels[at + 2])
        };
        let (r, g, b) = read(96, 96);
        assert!(b > r && b >= g, "ice pixel is not blue: {r},{g},{b}");
        let (wr, wg, wb) = read(96, 90);
        assert!(wb > wr && wb > wg, "water pixel is not blue: {wr},{wg},{wb}");
        assert!(r > wr && g > wg, "ice is not paler than water: {r},{g},{b} vs {wr},{wg},{wb}");
        let (r, g, b) = read(90, 96);
        let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
        assert!(lo > 120 && hi - lo < 40, "snow pixel is not white: {r},{g},{b}");
    }

    #[test]
    fn loaded_terrain_colours_match_their_surface_blocks() {
        // The map must tell the truth about ground it can see directly.
        // Centred in the wilderness: the starting village's paving around the
        // origin is deliberately none of the classifiable colours below.
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(12, 12), 2);
        let mut state = MapState::new();
        state.explore_around(ChunkPos::new(12, 12), 2);

        let pixels = render_map(&world, &state, (200, 200), &[]);
        let size = MAP_SIZE as i32;
        let zoom = state.zoom;

        let mut checked = 0;
        for (px, py) in [(96, 96), (60, 96), (96, 60), (120, 120)] {
            let x = 200 + (px - size / 2) * zoom;
            let z = 200 + (py - size / 2) * zoom;
            let Some(clear) = world.surface_y(x, z) else { continue };
            let name = world
                .registry()
                .get(world.block(vx_core::BlockPos::new(x, clear - 1, z)))
                .map(|def| def.name.clone())
                .unwrap_or_default();
            let at = ((py * size + px) * 4) as usize;
            let (r, g, b) = (pixels[at], pixels[at + 1], pixels[at + 2]);

            match name.as_str() {
                "engine:water" => assert!(b > r && b > g, "water pixel is not blue: {r},{g},{b}"),
                "engine:grass" => assert!(g > r && g > b, "grass pixel is not green: {r},{g},{b}"),
                "engine:sand" => assert!(r > b && g > b, "sand pixel is not sandy: {r},{g},{b}"),
                _ => continue,
            }
            checked += 1;
        }
        assert!(checked > 0, "no classifiable columns landed under the probes");
    }

    #[test]
    fn unexplored_ground_is_dark_and_explored_ground_is_not() {
        let world = world_with_origin();
        let mut state = MapState::new();
        state.explore(ChunkPos::new(0, 0)); // only the origin chunk

        let pixels = render_map(&world, &state, (8, 8), &[]);
        let size = MAP_SIZE as i32;
        let zoom = state.zoom;

        // Centre of the map is over the explored origin chunk.
        let centre_at = ((size / 2 * size + size / 2) * 4) as usize;
        assert!(pixels[centre_at + 3] > 200, "explored ground is see-through");

        // A far corner maps to columns hundreds of blocks away: unexplored.
        let corner_x = 8 + (0 - size / 2) * zoom;
        assert!(!state.is_explored(vx_core::BlockPos::new(corner_x, 0, corner_x).chunk()));
        let corner_at = 0;
        assert!(
            pixels[corner_at] < 30 && pixels[corner_at + 1] < 30,
            "unexplored ground is not dark"
        );
    }

    #[test]
    fn a_marker_lands_on_its_column_and_moves_with_it() {
        let world = world_with_origin();
        let mut state = MapState::new();
        state.explore_around(ChunkPos::new(0, 0), 3);

        let size = MAP_SIZE as i32;
        let zoom = state.zoom;
        let dot = |pixels: &[u8], px: i32, py: i32| -> [u8; 4] {
            let at = ((py * size + px) * 4) as usize;
            [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
        };

        let marker = |x: i32, z: i32| Marker {
            x,
            z,
            colour: colour::DRONE,
            radius: 1,
        };

        let here = render_map(&world, &state, (0, 0), &[marker(8, 8)]);
        assert_eq!(dot(&here, size / 2 + 8 / zoom, size / 2 + 8 / zoom), colour::DRONE);

        // Move the drone: the dot moves, the old spot does not keep it.
        let moved = render_map(&world, &state, (0, 0), &[marker(16, 8)]);
        assert_eq!(dot(&moved, size / 2 + 16 / zoom, size / 2 + 8 / zoom), colour::DRONE);
        assert_ne!(dot(&moved, size / 2 + 8 / zoom, size / 2 + 8 / zoom), colour::DRONE);
    }

    #[test]
    fn the_redraw_throttle_fires_once_per_interval() {
        let mut state = MapState::new();
        assert!(state.should_redraw(), "the first frame should draw");
        let mut fired = 0;
        for _ in 0..REDRAW_INTERVAL {
            if state.should_redraw() {
                fired += 1;
            }
        }
        assert_eq!(fired, 1, "the throttle fired {fired} times in one interval");
    }

    #[test]
    fn a_marker_draws_over_unexplored_ground() {
        // The contract pin depends on this: an accepted posting can name a
        // town in territory that has never been seen, and the pin has to show
        // there or the player has nothing to walk towards.
        let world = World::new(2024);
        let state = MapState::new();
        assert!(!state.is_explored(ChunkPos::new(64, 64)));

        let blank = render_map(&world, &state, (1_024, 1_024), &[]);
        let pinned = render_map(
            &world,
            &state,
            (1_024, 1_024),
            &[Marker {
                x: 1_024,
                z: 1_024,
                colour: colour::CONTRACT,
                radius: 3,
            }],
        );
        assert_ne!(blank, pinned, "a pin in the black did not draw");

        let centre = ((MAP_SIZE / 2 * MAP_SIZE + MAP_SIZE / 2) * 4) as usize;
        assert_eq!(&pinned[centre..centre + 4], &colour::CONTRACT);
    }

    #[test]
    fn a_town_reads_as_metal_on_explored_ground() {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 1);
        let mut state = MapState::new();
        // Explored but unloaded: the recomputed-surface path, which is where
        // the town lattice has to be consulted.
        state.explore_around(ChunkPos::new(0, 0), 8);
        let sites = world.towns_overlapping((-128, -128), (128, 128));
        assert!(!sites.is_empty(), "the hometown vanished");

        let (plot, _) = column_colour(&world, &state, &sites, 0, 0).expect("the plot is unmapped");
        // Explored, but well outside the hometown's skirt.
        let (wild, _) =
            column_colour(&world, &state, &sites, 120, 0).expect("open ground is unmapped");
        assert_ne!(plot, wild, "a levelled town plot looks like open country");
    }

    #[test]
    fn a_small_map_agrees_with_a_big_one_about_what_is_explored() {
        // The panel maps and the corner minimap must not disagree about where
        // the black starts, or a pin would sit in a different place on each.
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 1);
        let mut state = MapState::new();
        state.explore_around(ChunkPos::new(0, 0), 2);

        let big = render_map_sized(&world, &state, (0, 0), 4, 96, &[]);
        let small = render_map_sized(&world, &state, (0, 0), 4, 48, &[]);

        // The centre pixel of each covers the same column, so it must match.
        let centre_of = |pixels: &[u8], edge: u32| {
            let at = (((edge / 2) * edge + edge / 2) * 4) as usize;
            [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
        };
        assert_eq!(centre_of(&big, 96), centre_of(&small, 48));
    }

    #[test]
    fn a_pin_draws_over_unexplored_ground_at_panel_size_too() {
        // The paper-map behaviour, exercised through the size the console uses.
        let world = World::new(2024);
        let state = MapState::new();
        let edge = 96;
        let blank = render_map_sized(&world, &state, (4_000, 4_000), 8, edge, &[]);
        let pinned = render_map_sized(
            &world,
            &state,
            (4_000, 4_000),
            8,
            edge,
            &[Marker {
                x: 4_000,
                z: 4_000,
                colour: colour::CONTRACT,
                radius: 2,
            }],
        );
        assert_ne!(blank, pinned, "a pin in the black did not draw");
        let at = (((edge / 2) * edge + edge / 2) * 4) as usize;
        assert_eq!(&pinned[at..at + 4], &colour::CONTRACT);
    }

    #[test]
    fn bearings_read_the_way_they_do_on_the_ground() {
        // North is -Z, matching the camera.
        assert!(bearing((0, 0), (0, -100)).starts_with("N "));
        assert!(bearing((0, 0), (100, 0)).starts_with("E "));
        assert!(bearing((0, 0), (0, 100)).starts_with("S "));
        assert!(bearing((0, 0), (-100, 0)).starts_with("W "));
        assert!(bearing((0, 0), (100, -100)).starts_with("NE "));
        assert!(bearing((0, 0), (100, 100)).starts_with("SE "));
        assert!(bearing((0, 0), (-100, 100)).starts_with("SW "));
        assert!(bearing((0, 0), (-100, -100)).starts_with("NW "));

        // Slightly west of due north is still north, not north-west.
        assert!(bearing((0, 0), (-5, -100)).starts_with("N "));

        // Distance is Euclidean and rounded.
        assert_eq!(bearing((0, 0), (300, -400)), "NE 500M");
        assert_eq!(bearing((10, -10), (10, -10)), "HERE");
    }

    #[test]
    fn every_bearing_is_something_the_font_can_draw() {
        // Panels print these, and a character the font does not know draws as
        // a filled box.
        for x in [-1000, -37, 0, 37, 1000] {
            for z in [-1000, -37, 0, 37, 1000] {
                let line = bearing((0, 0), (x, z));
                for character in line.chars() {
                    assert!(
                        vx_render::font::knows(character),
                        "the font cannot draw {character:?} in {line:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn blitting_lands_the_map_where_it_was_asked_and_clips_the_rest() {
        let panel_width = 32u32;
        let mut panel = vec![0u8; (panel_width * 32 * 4) as usize];
        let edge = 8u32;

        // An opaque source composites to a straight copy.
        let mut solid = vec![200u8; (edge * edge * 4) as usize];
        for texel in solid.chunks_exact_mut(4) {
            texel[3] = 255;
        }
        blit(&mut panel, panel_width, &solid, edge, 4, 6);
        let at = ((6 * panel_width + 4) * 4) as usize;
        assert_eq!(panel[at], 200, "the map did not land at its corner");
        assert_eq!(panel[at + 3], 255);
        // Just outside it is untouched.
        let outside = ((5 * panel_width + 4) * 4) as usize;
        assert_eq!(panel[outside], 0);

        // Hanging off the edge clips rather than panicking.
        blit(&mut panel, panel_width, &solid, edge, -3, 28);
        blit(&mut panel, panel_width, &solid, edge, 30, 2);
    }

    #[test]
    fn unexplored_ground_blends_into_the_panel_instead_of_holing_it() {
        // The map draws unexplored ground translucent so the corner minimap
        // hints at the world behind it. Copying that into a panel would punch
        // a hole and show the game through the middle of a menu, so the blit
        // composites and keeps the panel's own opacity.
        let width = 8u32;
        let mut panel = vec![0u8; (width * width * 4) as usize];
        for texel in panel.chunks_exact_mut(4) {
            texel.copy_from_slice(&[10, 12, 16, 235]);
        }

        let edge = 4u32;
        let mut fog = vec![0u8; (edge * edge * 4) as usize];
        for texel in fog.chunks_exact_mut(4) {
            // What `render_map_sized` writes for ground nobody has walked.
            texel.copy_from_slice(&[10, 12, 16, 170]);
        }

        blit(&mut panel, width, &fog, edge, 0, 0);
        let at = 0;
        assert_eq!(&panel[at..at + 3], &[10, 12, 16], "the fog changed colour");
        assert_eq!(
            panel[at + 3], 235,
            "the map made the panel see-through"
        );
    }
}
