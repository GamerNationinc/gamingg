//! Block textures, as a GPU texture array.
//!
//! An array rather than an atlas, deliberately. Greedy meshing emits UVs that
//! span the whole merged quad (a 4×2 quad runs 0..4 by 0..2), so the texture
//! has to repeat across it. With an atlas that means doing the wrap by hand in
//! the shader, and the seam where `fract()` rolls over samples the neighbouring
//! tile — the classic atlas bleed. An array layer per tile lets the sampler's
//! `Repeat` address mode do it correctly for free.
//!
//! The textures here are generated procedurally: original placeholder art, so
//! the repository carries no borrowed assets. Replacing this with loaded PNGs
//! later changes nothing above it.

/// Edge length of one tile in pixels.
pub const TILE_SIZE: u32 = 16;

/// Tile slots, matching the texture indices `vx-world` assigns its built-ins.
pub mod slot {
    pub const STONE: u32 = 0;
    pub const DIRT: u32 = 1;
    pub const GRASS_TOP: u32 = 2;
    pub const GRASS_SIDE: u32 = 3;
    pub const SAND: u32 = 4;
    pub const WATER: u32 = 5;
    pub const BEDROCK: u32 = 6;
    pub const COPPER_ORE: u32 = 7;
    pub const CONTAINER: u32 = 8;
    pub const HULL: u32 = 9;
    pub const TREAD: u32 = 10;
    pub const STEEL: u32 = 11;
    pub const CAB: u32 = 12;
    pub const PLANK: u32 = 13;
    pub const ROOF: u32 = 14;
    pub const COUNTER: u32 = 15;
    pub const CLOTH: u32 = 16;
    pub const SKIN: u32 = 17;
    pub const LOG_SIDE: u32 = 18;
    pub const LOG_TOP: u32 = 19;
    pub const LEAVES: u32 = 20;
    pub const TUFT: u32 = 21;
    pub const METAL_WALL: u32 = 22;
    pub const RUSTED_METAL: u32 = 23;
    pub const CATWALK: u32 = 24;
    pub const MAST: u32 = 25;
    pub const BEACON: u32 = 26;
    pub const COPPER_BAR: u32 = 27;
    pub const CHEST: u32 = 28;
    pub const MAILBOX: u32 = 29;
    pub const PERMIT_I: u32 = 30;
    pub const PERMIT_II: u32 = 31;
    pub const PERMIT_III: u32 = 32;
    /// The law's watch box, and the one you can buy for your own roof.
    pub const ROOST: u32 = 33;
    /// The fabricator: raw stock in, anything out.
    pub const PRINTER: u32 = 34;
    /// Total generated tiles.
    /// The bunker's skin: poured, reinforced, and four hundred times slower
    /// to cut than stone.
    pub const BUNKER_SHELL: u32 = 35;
    /// A supply cache, sealed until somebody opens it.
    pub const SUPPLY_CACHE: u32 = 36;

    /// A canister of oxyhydrogen.
    pub const HHO_CELL: u32 = 37;
    /// The electrolyser: electrodes in a water bath.
    pub const ELECTROLYSER: u32 = 38;

    /// A fort's revetted earth.
    pub const RAMPART: u32 = 39;
    /// The bank's deposit box.
    pub const VAULT: u32 = 40;

    /// A building's foundation.
    pub const FOOTING: u32 = 41;

    /// The deep prize, and the only ore that reads as a warning.
    pub const URANIUM_ORE: u32 = 42;
    /// Rock with oil in it.
    pub const OIL_SAND: u32 = 43;
    /// Shale with gas in it.
    pub const GAS_SHALE: u32 = 44;
    /// A barrel of crude: what a well lifts and a town buys.
    pub const OIL_BARREL: u32 = 45;
    /// A canister of well gas — the fuel you do not need a lake for.
    pub const GAS_CELL: u32 = 46;
    /// The wellhead: the machine that stands over a field and waits.
    pub const WELLHEAD: u32 = 47;

    /// The white of an eye.
    pub const EYE: u32 = 48;
    /// A pupil, and the line of a mouth: the two dark marks on a face.
    pub const PUPIL: u32 = 49;
    /// The ward cot: what a hospital is, mechanically.
    pub const COT: u32 = 50;

    /// Spruce bark: darker and finer than the hardwoods'.
    pub const SPRUCE_SIDE: u32 = 51;
    /// Spruce end grain — tighter rings than a hardwood's.
    pub const SPRUCE_TOP: u32 = 52;
    /// Subalpine needles: blue-dark, dense.
    pub const NEEDLES: u32 = 53;
    /// Black-spruce bark: grey, scaly, thin.
    pub const BOG_BARK: u32 = 54;
    /// Bog needles: sparse, olive, half sky showing through.
    pub const BOG_NEEDLES: u32 = 55;
    /// Sphagnum: the moss carpet a peat bog stands on.
    pub const SPHAGNUM: u32 = 56;

    /// An ancient's bark: deep-furrowed, grey with age.
    pub const ANCIENT_BARK: u32 = 57;
    /// Its end grain, which is mostly rings.
    pub const ANCIENT_TOP: u32 = 58;
    /// Prime timber: milled heartwood off an emergent stem.
    pub const PRIME_TIMBER: u32 = 59;

    /// The pump: a housing, a band of ports, and wet metal.
    pub const PUMP: u32 = 60;

    /// Wood alight: the one tile in the game that is meant to be too bright.
    pub const EMBER: u32 = 61;
    /// And what it leaves behind.
    pub const ASH: u32 = 62;

    /// The ground gone white: the top of any snowed block.
    pub const SNOW_TOP: u32 = 63;
    /// The side of a snowed block: the bare ground with a white band draped
    /// over the top rows, the way grass drapes over dirt.
    pub const SNOWY_SIDE: u32 = 64;
    /// Still water frozen over: pale, see-through, a hairline crack or two.
    pub const ICE: u32 = 65;

    /// The drill's holographic cage: a cyan grid on a dark field, the way a
    /// projected line looked to somebody in 1984. Worn by no block — it is
    /// only ever an [`crate::Object`], drawn as thin bars round the block
    /// the bit is aimed at.
    pub const HOLOGRAM: u32 = 66;
    /// The sonar's answer, in the other half of the same palette: magenta on
    /// dark, so an echo is never mistaken for the cage that fired it.
    pub const SONAR: u32 = 67;

    /// What is left of a machine: scorched, buckled plate with the rust
    /// showing through where the paint has gone. Deliberately a *different*
    /// tile from [`HULL`] rather than a darker shade of it — a wreck has to
    /// read as a wreck at fifty blocks, across a hillside, with no label.
    pub const WRECK: u32 = 68;

    /// Total generated tiles.
    pub const COUNT: u32 = 69;
}

/// Deterministic per-pixel jitter, so tiles look grainy rather than flat.
fn jitter(tile: u32, x: u32, y: u32) -> f32 {
    let mut hash = (tile as u64)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (x as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ (y as u64).wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 32;
    // Centred on zero, spanning roughly -0.5..0.5.
    ((hash >> 40) as f32) / ((1u32 << 24) as f32) - 0.5
}

fn shade(base: [f32; 3], amount: f32) -> [u8; 4] {
    [
        ((base[0] + amount).clamp(0.0, 1.0) * 255.0) as u8,
        ((base[1] + amount).clamp(0.0, 1.0) * 255.0) as u8,
        ((base[2] + amount).clamp(0.0, 1.0) * 255.0) as u8,
        255,
    ]
}

/// The foliage slots the year repaints, and nothing else.
///
/// Everything in the atlas that is *alive*: two hardwood canopies, the two
/// conifer crowns, the ground cover and the moss. Stone does not have a
/// season, so stone is painted once at startup and never touched again —
/// which is why a repaint costs seven kilobytes rather than sixty-three.
pub const FOLIAGE: [u32; 7] = [
    slot::LEAVES,
    slot::NEEDLES,
    slot::BOG_NEEDLES,
    slot::SPHAGNUM,
    slot::TUFT,
    slot::GRASS_TOP,
    slot::GRASS_SIDE,
];

fn mix(from: [f32; 3], to: [f32; 3], amount: f32) -> [f32; 3] {
    let amount = amount.clamp(0.0, 1.0);
    [
        from[0] + (to[0] - from[0]) * amount,
        from[1] + (to[1] - from[1]) * amount,
        from[2] + (to[2] - from[2]) * amount,
    ]
}

/// A living colour walked through the year: green while it is growing, then
/// the turn, then whatever it looks like once the year is done with it.
///
/// Two legs rather than one, because a leaf does not fade from green to brown
/// — it goes *gold* on the way, and losing that is losing the autumn. The
/// hinge is at 0.55 of the turn, which is where a real canopy is at its most
/// colourful rather than halfway between two states.
fn through_the_year(green: [f32; 3], gold: [f32; 3], bare: [f32; 3], turn: f32) -> [f32; 3] {
    if turn < 0.55 {
        mix(green, gold, turn / 0.55)
    } else {
        mix(gold, bare, (turn - 0.55) / 0.45)
    }
}

/// How far a conifer follows the year. A tenth, near enough — a spruce in
/// February is a spruce, and a number much bigger than this makes the whole
/// country turn at once and costs the hardwoods their moment.
const EVERGREEN: f32 = 0.10;

/// Ground cover through the year: green, then straw, then the dun of grass
/// that has been standing dead since November.
///
/// One curve for the sward and the tufts together, because they are the same
/// grass and a lawn whose blades disagreed with it would read as a bug.
fn sward(turn: f32) -> [f32; 3] {
    through_the_year(
        [0.32, 0.55, 0.24],
        [0.58, 0.53, 0.24],
        [0.42, 0.38, 0.27],
        turn,
    )
}

/// Generate the RGBA pixels for one tile at one point in the year.
///
/// `turn` is [`vx_world::season::leaf_turn`]: `0` full green through `1`
/// bare. Every slot but the seven in [`FOLIAGE`] ignores it entirely, and
/// asking for the same slot at the same turn always gives the same bytes —
/// the atlas is as deterministic as everything else here.
pub fn generate_tile(tile: u32, turn: f32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((TILE_SIZE * TILE_SIZE * 4) as usize);

    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            let noise = jitter(tile, x, y);
            let texel = match tile {
                slot::STONE => shade([0.50, 0.50, 0.52], noise * 0.10),
                slot::DIRT => shade([0.45, 0.32, 0.21], noise * 0.12),
                slot::GRASS_TOP => shade(sward(turn), noise * 0.13),
                slot::GRASS_SIDE => {
                    // Dirt, with a band of grass draped over the top few rows.
                    // The band edge wobbles so it does not read as a ruler line.
                    // Only the band takes the season — the dirt under it is
                    // dirt in any month.
                    let wobble = (jitter(tile, x, 0) * 2.0).round() as i32;
                    let band = (3 + wobble).clamp(1, 6) as u32;
                    if y < band {
                        shade(sward(turn), noise * 0.13)
                    } else {
                        shade([0.45, 0.32, 0.21], noise * 0.12)
                    }
                }
                slot::SAND => shade([0.80, 0.74, 0.52], noise * 0.08),
                slot::WATER => {
                    let mut texel = shade([0.16, 0.35, 0.62], noise * 0.06);
                    texel[3] = 160; // see-through, so the bed shows below
                    texel
                }
                slot::BEDROCK => shade([0.18, 0.18, 0.20], noise * 0.22),
                slot::COPPER_ORE => {
                    // Veins of metal through host rock. The metal has to carry
                    // most of the tile: an outcrop is only useful if a player
                    // can pick it out of a hillside at distance, and a few
                    // flecks on grey read as plain stone from more than a
                    // dozen blocks away.
                    if jitter(tile ^ 0x51, x / 2, y / 2) > -0.12 {
                        shade([0.78, 0.40, 0.16], noise * 0.16)
                    } else {
                        shade([0.44, 0.44, 0.47], noise * 0.10)
                    }
                }
                slot::CONTAINER => {
                    // A steel crate: riveted border around brushed panels, so
                    // it reads as built rather than mined even at distance.
                    let edge = x.min(TILE_SIZE - 1 - x).min(y).min(TILE_SIZE - 1 - y);
                    if edge == 0 {
                        shade([0.30, 0.32, 0.36], noise * 0.05)
                    } else if edge == 1 && (x + y) % 3 == 0 {
                        shade([0.72, 0.74, 0.78], 0.0) // rivets
                    } else {
                        shade([0.52, 0.56, 0.62], noise * 0.08)
                    }
                }
                slot::HULL => {
                    // Rust-orange machine plate with a darker weld seam across
                    // the middle, so big flat rig sides do not read as one
                    // slab of paint.
                    if y == TILE_SIZE / 2 {
                        shade([0.55, 0.24, 0.10], noise * 0.05)
                    } else {
                        shade([0.76, 0.34, 0.13], noise * 0.10)
                    }
                }
                slot::TREAD => {
                    // Near-black rubber, ribbed.
                    if x % 4 < 2 {
                        shade([0.10, 0.10, 0.11], noise * 0.05)
                    } else {
                        shade([0.17, 0.17, 0.19], noise * 0.05)
                    }
                }
                slot::STEEL => {
                    // Bright brushed steel: horizontal grain.
                    shade([0.72, 0.74, 0.78], jitter(tile ^ 0x77, 0, y) * 0.18 + noise * 0.04)
                }
                slot::CAB => {
                    // Pale glassy blue-grey with a highlight streak.
                    if x + y < 8 {
                        shade([0.80, 0.88, 0.94], noise * 0.03)
                    } else {
                        shade([0.55, 0.66, 0.76], noise * 0.05)
                    }
                }
                slot::PLANK => {
                    // Warm boards with a dark seam every fourth row and the
                    // odd nail head, so a wall reads as carpentry.
                    if y % 4 == 3 {
                        shade([0.38, 0.26, 0.14], noise * 0.06)
                    } else if y % 4 == 1 && x % 8 == 2 {
                        shade([0.30, 0.28, 0.26], 0.0) // nails
                    } else {
                        shade([0.62, 0.44, 0.24], noise * 0.10)
                    }
                }
                slot::ROOF => {
                    // Staggered shingle courses in a weathered red-brown.
                    let course = y / 4;
                    let shifted = x + course * 2;
                    if y % 4 == 0 || shifted % 8 == 0 {
                        shade([0.30, 0.13, 0.10], noise * 0.06)
                    } else {
                        shade([0.50, 0.22, 0.15], noise * 0.10)
                    }
                }
                slot::COUNTER => {
                    // Dark shop wood under a brass strip: the one block in
                    // town you trade over, so it has to stand apart.
                    if y < 3 {
                        shade([0.72, 0.58, 0.22], noise * 0.06)
                    } else if y % 5 == 4 {
                        shade([0.24, 0.16, 0.10], noise * 0.05)
                    } else {
                        shade([0.40, 0.28, 0.16], noise * 0.08)
                    }
                }
                slot::CLOTH => {
                    // Woven fabric: a faint checked weave in a muted blue.
                    if (x + y) % 2 == 0 {
                        shade([0.28, 0.38, 0.55], noise * 0.05)
                    } else {
                        shade([0.24, 0.33, 0.49], noise * 0.05)
                    }
                }
                slot::SKIN => shade([0.82, 0.62, 0.46], noise * 0.05),
                slot::LOG_SIDE => {
                    // Bark: vertical ridges, the grain running with the trunk.
                    if jitter(tile ^ 0x2f, x, 0) > 0.1 {
                        shade([0.26, 0.17, 0.09], noise * 0.08)
                    } else {
                        shade([0.38, 0.26, 0.14], noise * 0.10)
                    }
                }
                slot::LOG_TOP => {
                    // End grain: rough growth rings around the centre.
                    let cx = x as f32 - 7.5;
                    let cy = y as f32 - 7.5;
                    let ring = (cx * cx + cy * cy).sqrt() as u32;
                    if ring.is_multiple_of(3) {
                        shade([0.44, 0.31, 0.17], noise * 0.06)
                    } else {
                        shade([0.60, 0.44, 0.24], noise * 0.08)
                    }
                }
                slot::LEAVES => {
                    // Mottled two-tone canopy. Opaque on purpose — the
                    // "fast graphics" look — so foliage never joins water in
                    // the unsorted alpha-blend pass.
                    //
                    // This is the tile that does the year's work. A hardwood
                    // cove is the thing you *see* turn, and the two tones
                    // turn at slightly different rates, so a canopy in
                    // October is patchy rather than uniformly orange.
                    if jitter(tile ^ 0x63, x / 2, y / 2) > 0.05 {
                        let colour = through_the_year(
                            [0.16, 0.34, 0.12],
                            [0.62, 0.36, 0.09],
                            [0.29, 0.24, 0.20],
                            turn,
                        );
                        shade(colour, noise * 0.10)
                    } else {
                        let colour = through_the_year(
                            [0.24, 0.46, 0.16],
                            [0.72, 0.51, 0.14],
                            [0.36, 0.30, 0.25],
                            turn * 0.85,
                        );
                        shade(colour, noise * 0.12)
                    }
                }
                slot::SPRUCE_SIDE => {
                    // Conifer bark: finer ridges than a hardwood's, and
                    // darker, so a spruce stand reads black-green at range.
                    if jitter(tile ^ 0x51, x, y / 4) > -0.05 {
                        shade([0.20, 0.14, 0.10], noise * 0.07)
                    } else {
                        shade([0.29, 0.20, 0.13], noise * 0.09)
                    }
                }
                slot::SPRUCE_TOP => {
                    // Tight rings: a slow tree in a short season.
                    let cx = x as f32 - 7.5;
                    let cy = y as f32 - 7.5;
                    let ring = (cx * cx + cy * cy).sqrt() as u32;
                    if ring.is_multiple_of(2) {
                        shade([0.40, 0.30, 0.18], noise * 0.05)
                    } else {
                        shade([0.54, 0.41, 0.25], noise * 0.07)
                    }
                }
                slot::NEEDLES => {
                    // Dense and blue-dark, the colour that makes the high
                    // country read as conifer from a ridge away.
                    //
                    // A spruce barely notices the year — that is what makes
                    // it a spruce, and it is what makes the hardwood cove
                    // legible when it turns beside one. All it does is go a
                    // shade colder and duller through the winter.
                    let evergreen = |green: [f32; 3], winter: [f32; 3]| {
                        mix(green, winter, turn * EVERGREEN)
                    };
                    if jitter(tile ^ 0x53, x / 2, y / 2) > 0.0 {
                        shade(evergreen([0.10, 0.24, 0.16], [0.09, 0.19, 0.18]), noise * 0.09)
                    } else {
                        shade(evergreen([0.14, 0.32, 0.22], [0.12, 0.25, 0.24]), noise * 0.10)
                    }
                }
                slot::BOG_BARK => {
                    // Thin scaly grey: a black spruce is mostly bark.
                    if jitter(tile ^ 0x54, x / 3, y / 3) > 0.0 {
                        shade([0.26, 0.24, 0.21], noise * 0.08)
                    } else {
                        shade([0.34, 0.31, 0.26], noise * 0.09)
                    }
                }
                slot::BOG_NEEDLES => {
                    // Olive and patchy. Opaque, like every other leaf here:
                    // foliage never joins water in the unsorted alpha pass.
                    // A bog crown reads thin because there is so little of
                    // it, not because the texture has holes in it.
                    //
                    // A black spruce is a conifer too, so it moves as little
                    // as its high-country cousin — just towards rust rather
                    // than towards blue, because that is what a bog does.
                    let evergreen = |green: [f32; 3], winter: [f32; 3]| {
                        mix(green, winter, turn * EVERGREEN)
                    };
                    if jitter(tile ^ 0x55, x, y) > 0.35 {
                        shade(evergreen([0.13, 0.19, 0.11], [0.17, 0.17, 0.11]), noise * 0.09)
                    } else if jitter(tile ^ 0x56, x / 2, y / 2) > 0.0 {
                        shade(evergreen([0.18, 0.28, 0.14], [0.24, 0.25, 0.14]), noise * 0.10)
                    } else {
                        shade(evergreen([0.26, 0.34, 0.18], [0.34, 0.30, 0.18]), noise * 0.11)
                    }
                }
                slot::SPHAGNUM => {
                    // The moss carpet: pale green, wet, faintly hummocked —
                    // and reddening hard as the year closes, which is a real
                    // thing sphagnum does and is the best colour in the game
                    // for about a week in October.
                    let hummock = ((x / 4) + (y / 4)) % 2 == 0;
                    if hummock {
                        let colour = through_the_year(
                            [0.44, 0.55, 0.32],
                            [0.52, 0.36, 0.27],
                            [0.44, 0.33, 0.28],
                            turn,
                        );
                        shade(colour, noise * 0.10)
                    } else {
                        let colour = through_the_year(
                            [0.34, 0.47, 0.30],
                            [0.43, 0.31, 0.24],
                            [0.36, 0.29, 0.25],
                            turn,
                        );
                        shade(colour, noise * 0.11)
                    }
                }
                slot::ANCIENT_BARK => {
                    // Deep furrows running the height of the trunk, grey
                    // where the ridges have weathered.
                    let furrow = jitter(tile ^ 0x57, x, y / 6) > 0.15;
                    if furrow {
                        shade([0.17, 0.14, 0.11], noise * 0.06)
                    } else if x % 5 == 0 {
                        shade([0.42, 0.38, 0.32], noise * 0.08)
                    } else {
                        shade([0.31, 0.26, 0.20], noise * 0.09)
                    }
                }
                slot::ANCIENT_TOP => {
                    // Rings, and a lot of them. That is the whole story of
                    // the block.
                    let cx = x as f32 - 7.5;
                    let cy = y as f32 - 7.5;
                    let ring = (cx * cx + cy * cy).sqrt() as u32;
                    if ring.is_multiple_of(2) {
                        shade([0.35, 0.26, 0.15], noise * 0.05)
                    } else {
                        shade([0.52, 0.40, 0.24], noise * 0.06)
                    }
                }
                slot::PRIME_TIMBER => {
                    // Heartwood: straight grain, no bark, and pale.
                    if y % 4 == 0 {
                        shade([0.48, 0.34, 0.19], noise * 0.06)
                    } else {
                        shade([0.66, 0.50, 0.30], noise * 0.08)
                    }
                }
                slot::PUMP => {
                    // A squat housing with a band of intake ports round its
                    // middle, wet enough to read as a water machine.
                    let band = (6..10).contains(&y);
                    if band && x % 4 < 2 {
                        shade([0.10, 0.20, 0.26], noise * 0.06)
                    } else if band {
                        shade([0.32, 0.46, 0.52], noise * 0.08)
                    } else if !(2..=13).contains(&y) {
                        shade([0.24, 0.26, 0.28], noise * 0.07)
                    } else {
                        shade([0.42, 0.45, 0.48], noise * 0.09)
                    }
                }
                slot::EMBER => {
                    // Char with fire in the cracks. Bright on purpose: a
                    // stand alight has to read from the next ridge.
                    let crack = jitter(tile ^ 0x61, x / 2, y / 2);
                    if crack > 0.28 {
                        shade([0.98, 0.72, 0.18], noise * 0.10)
                    } else if crack > 0.05 {
                        shade([0.82, 0.30, 0.07], noise * 0.12)
                    } else {
                        shade([0.16, 0.10, 0.08], noise * 0.08)
                    }
                }
                slot::ASH => {
                    // Grey, soft, and faintly warm where it has not cooled.
                    if jitter(tile ^ 0x62, x, y) > 0.42 {
                        shade([0.30, 0.27, 0.26], noise * 0.09)
                    } else {
                        shade([0.20, 0.18, 0.18], noise * 0.10)
                    }
                }
                slot::SNOW_TOP => {
                    // Near-white with a cool grain, and the odd texel of blue
                    // shadow where the surface is not quite flat — which is
                    // what stops a snowfield reading as a sheet of paper.
                    if jitter(tile ^ 0x63, x / 2, y / 2) > 0.38 {
                        shade([0.82, 0.86, 0.92], noise * 0.04)
                    } else {
                        shade([0.93, 0.95, 0.97], noise * 0.03)
                    }
                }
                slot::SNOWY_SIDE => {
                    // The grass side's dirt with snow draped over the top
                    // rows, using the same wobble the grass band uses so the
                    // two blocks sit beside each other without a ruler line.
                    let wobble = (jitter(slot::GRASS_SIDE, x, 0) * 2.0).round() as i32;
                    let band = (3 + wobble).clamp(1, 6) as u32;
                    if y < band {
                        shade([0.92, 0.94, 0.97], noise * 0.03)
                    } else {
                        shade([0.45, 0.32, 0.21], noise * 0.12)
                    }
                }
                slot::HOLOGRAM | slot::SONAR => {
                    // Two tiles, one painter, because they are the same idea
                    // in two colours: a lit grid line on a nearly-black
                    // field. The cage is cyan, the echo magenta.
                    //
                    // Both are *opaque*. The obvious way to draw a hologram
                    // is a translucent shell, and it is the wrong way here:
                    // the object pipeline writes depth and blends unsorted
                    // (see `lib.rs`), so a clear box round a block z-fights
                    // its own faces and occludes whatever translucent thing
                    // draws after it. Thin opaque bars in a bright tile give
                    // the same read with none of that, and the glow comes
                    // from `Object::light` pushed past 1.0 instead.
                    let lit = if tile == slot::HOLOGRAM {
                        [0.20, 0.95, 0.98]
                    } else {
                        [0.98, 0.24, 0.86]
                    };
                    // A grid every four texels, with the scanline rows of a
                    // tube that never quite settled.
                    let grid = x % 4 == 0 || y % 4 == 0;
                    let scan = y % 2 == 0;
                    if grid {
                        shade(lit, noise * 0.05)
                    } else if scan {
                        shade([lit[0] * 0.45, lit[1] * 0.45, lit[2] * 0.45], noise * 0.04)
                    } else {
                        shade([lit[0] * 0.12, lit[1] * 0.12, lit[2] * 0.12], noise * 0.02)
                    }
                }
                slot::WRECK => {
                    // Scorched plate. The machine's own rust-orange, burnt
                    // most of the way to charcoal, with the odd texel of
                    // unburnt paint left where a panel folded and sheltered
                    // it — so the eye reads "this used to be a digger"
                    // rather than "this is a dark rock".
                    let scorch = jitter(tile ^ 0x5a, x / 2, y / 2);
                    if y == TILE_SIZE / 2 || x % 7 == 3 {
                        // The buckled seams, blackest of all.
                        shade([0.09, 0.07, 0.06], noise * 0.03)
                    } else if scorch > 0.36 {
                        shade([0.42, 0.19, 0.08], noise * 0.06)
                    } else {
                        shade([0.19, 0.14, 0.12], noise * 0.05)
                    }
                }
                slot::ICE => {
                    // Pale blue-white and see-through, with a hairline crack
                    // or two: solid enough to walk on, thin enough to read
                    // the lake bed through.
                    let crack = jitter(tile ^ 0x65, x / 3, y) > 0.44 && (x + y) % 5 == 0;
                    let mut texel = if crack {
                        shade([0.62, 0.74, 0.86], noise * 0.03)
                    } else {
                        shade([0.80, 0.90, 0.97], noise * 0.03)
                    };
                    texel[3] = 200;
                    texel
                }
                slot::TUFT => {
                    // A few grass blades on a transparent field; drawn as a
                    // crossed pair of quads in the world, not a cube.
                    let sway = (jitter(tile ^ 0x11, x, 0) * 3.0) as i32;
                    let blade = x % 5 == 2 && (y as i32) > 4 + sway && jitter(tile ^ 0x19, x, 0) > -0.3;
                    if blade {
                        shade(sward(turn), noise * 0.12)
                    } else {
                        [0, 0, 0, 0]
                    }
                }
                slot::METAL_WALL => {
                    // Corrugated container flank: vertical ribbing catching
                    // the light, so a wall reads as pressed steel rather than
                    // a flat panel.
                    let rib = x % 4;
                    let lift = match rib {
                        0 => 0.06,
                        1 => 0.02,
                        2 => -0.04,
                        _ => -0.02,
                    };
                    shade([0.55, 0.58, 0.60], lift + noise * 0.05)
                }
                slot::RUSTED_METAL => {
                    // The same ribbing, weathered. Patches of rust bloom
                    // through where the paint has given up.
                    let rib = x % 4;
                    let lift = if rib < 2 { 0.04 } else { -0.03 };
                    if jitter(tile ^ 0x3d, x / 3, y / 3) > 0.05 {
                        shade([0.55, 0.30, 0.16], lift + noise * 0.10)
                    } else {
                        shade([0.48, 0.47, 0.45], lift + noise * 0.06)
                    }
                }
                slot::CATWALK => {
                    // Open steel grating, drawn opaque: the mesher has no
                    // sorted-transparency pass, so a see-through walkway would
                    // cost far more than it looks.
                    if x % 4 == 0 || y % 4 == 0 {
                        shade([0.38, 0.40, 0.43], noise * 0.05)
                    } else {
                        shade([0.16, 0.17, 0.19], noise * 0.08)
                    }
                }
                slot::MAST => {
                    // Galvanised lattice: diagonal bracing over dark sky-grey,
                    // which reads as a tower from a distance without needing
                    // a single transparent texel.
                    let diagonal = (x + y) % 6 < 2 || (x + TILE_SIZE - y) % 6 < 2;
                    if diagonal {
                        shade([0.62, 0.65, 0.68], noise * 0.06)
                    } else {
                        shade([0.22, 0.24, 0.27], noise * 0.05)
                    }
                }
                slot::BEACON => {
                    // A console face: dark housing, an amber strip that reads
                    // as powered even at night.
                    if (5..9).contains(&y) && (2..14).contains(&x) {
                        shade([0.95, 0.62, 0.16], noise * 0.06)
                    } else if y < 3 {
                        shade([0.30, 0.32, 0.36], noise * 0.05)
                    } else {
                        shade([0.18, 0.19, 0.22], noise * 0.06)
                    }
                }
                slot::COPPER_BAR => {
                    // Stacked ingots: bright bar faces with dark seams between
                    // them, so a pallet of them reads as many rather than one.
                    let seam = y % 5 == 0 || (x + (y / 5) * 3) % 8 == 0;
                    if seam {
                        shade([0.28, 0.16, 0.10], noise * 0.05)
                    } else {
                        // Lighter along the top of each bar, for a rounded look.
                        let lift = if y % 5 == 1 { 0.10 } else { 0.0 };
                        shade(
                            [0.85 + lift, 0.48 + lift * 0.6, 0.22 + lift * 0.3],
                            noise * 0.07,
                        )
                    }
                }
                slot::CHEST => {
                    // A strongbox: plank body, a darker iron strap across the
                    // middle with a clasp — the strap is what says "chest"
                    // rather than "crate" at a glance.
                    let strap = (7..10).contains(&y);
                    let clasp = strap && (6..10).contains(&x);
                    if clasp {
                        shade([0.72, 0.70, 0.62], noise * 0.05)
                    } else if strap || !(1..TILE_SIZE - 1).contains(&x) {
                        shade([0.24, 0.20, 0.16], noise * 0.06)
                    } else {
                        shade([0.55, 0.40, 0.24], noise * 0.10)
                    }
                }
                slot::MAILBOX => {
                    // A postal panel: pale housing with one dark letter slot
                    // near the top. One strong horizontal is all it takes.
                    if (3..6).contains(&y) && (3..13).contains(&x) {
                        shade([0.10, 0.11, 0.13], noise * 0.03)
                    } else if y >= TILE_SIZE - 2 {
                        shade([0.35, 0.37, 0.40], noise * 0.05)
                    } else {
                        shade([0.70, 0.72, 0.75], noise * 0.06)
                    }
                }
                slot::PERMIT_I | slot::PERMIT_II | slot::PERMIT_III => {
                    // A lockbox: dark housing, a recessed panel, and a row of
                    // status pips whose count *is* the grade. Reading a lock's
                    // tier across the room is the point — you should be able to
                    // decide whether it is worth your afternoon before you
                    // start drilling.
                    let pips = match tile {
                        slot::PERMIT_I => 1,
                        slot::PERMIT_II => 2,
                        _ => 3,
                    };
                    // Housing tints upward with grade, so tiers differ at a
                    // glance even before the pips resolve.
                    let housing = [
                        0.20 + pips as f32 * 0.03,
                        0.21 + pips as f32 * 0.02,
                        0.25 + pips as f32 * 0.04,
                    ];
                    let lit = [0.35, 0.95, 0.55];
                    let panel = (4..12).contains(&x) && (3..9).contains(&y);
                    let pip_row = (11..13).contains(&y);
                    let pip_index = (x.saturating_sub(4)) / 3;
                    if pip_row && x >= 4 && pip_index < pips && (x - 4) % 3 < 2 {
                        shade(lit, noise * 0.04)
                    } else if panel {
                        shade([0.13, 0.15, 0.18], noise * 0.05)
                    } else {
                        shade(housing, noise * 0.06)
                    }
                }
                slot::ROOST => {
                    // A watch box: dark shuttered housing with a pale seam
                    // across the lid and one cold lens. It should read as
                    // "something lives in there and it opens", which is the
                    // only warning the town gives you before it does.
                    let seam = (7..9).contains(&y);
                    let lens = (10..14).contains(&x) && (3..7).contains(&y);
                    if lens {
                        shade([0.45, 0.75, 0.95], noise * 0.04)
                    } else if seam {
                        shade([0.58, 0.60, 0.64], noise * 0.05)
                    } else if y < 8 {
                        shade([0.22, 0.24, 0.28], noise * 0.06)
                    } else {
                        shade([0.17, 0.18, 0.22], noise * 0.06)
                    }
                }
                slot::FOOTING => {
                    // Poured footing: coarse aggregate, a shuttering seam and
                    // the odd tie rod. Darker and heavier than the rampart
                    // above it, which is what a foundation looks like when
                    // you find one — you are meant to know you have hit
                    // something you should not have started.
                    let seam = y % 9 == 0;
                    let aggregate = ((x * 11 + y * 7) % 23) < 5;
                    let tie = (x % 13 == 0) && (4..12).contains(&y);
                    if tie {
                        shade([0.40, 0.30, 0.22], noise * 0.05)
                    } else if seam {
                        shade([0.22, 0.22, 0.24], noise * 0.04)
                    } else if aggregate {
                        shade([0.40, 0.40, 0.42], noise * 0.07)
                    } else {
                        shade([0.30, 0.30, 0.32], noise * 0.06)
                    }
                }
                slot::RAMPART => {
                    // Packed earth behind a stone revetment: courses of block
                    // work low down, spilling earth above. It should read as
                    // something raised in a hurry and meant to stop shot.
                    let course = y % 5 == 0;
                    let joint = (x + (y / 5) * 7) % 11 == 0;
                    let earth = ((x * 3 + y * 5) % 29) < 6;
                    if course || joint {
                        shade([0.34, 0.32, 0.29], noise * 0.05)
                    } else if earth {
                        shade([0.44, 0.36, 0.26], noise * 0.07)
                    } else {
                        shade([0.55, 0.50, 0.42], noise * 0.06)
                    }
                }
                slot::VAULT => {
                    // A strongbox door: a heavy plate, a ring of bolts and a
                    // wheel in the middle. Unmistakable across a dark room,
                    // which is the point of the one block in town that holds
                    // everything a player owns.
                    let cx = x as i32 - TILE_SIZE as i32 / 2;
                    let cy = y as i32 - TILE_SIZE as i32 / 2;
                    let radius = ((cx * cx + cy * cy) as f32).sqrt();
                    let wheel = radius < 3.5;
                    let spoke = radius < 6.0 && (cx.abs() < 1 || cy.abs() < 1);
                    let bolts = (5.5..7.0).contains(&radius);
                    let plate = radius < 11.0;
                    if wheel || spoke {
                        shade([0.78, 0.66, 0.26], noise * 0.04)
                    } else if bolts {
                        shade([0.62, 0.63, 0.66], noise * 0.05)
                    } else if plate {
                        shade([0.36, 0.38, 0.42], noise * 0.05)
                    } else {
                        shade([0.24, 0.25, 0.28], noise * 0.06)
                    }
                }
                slot::HHO_CELL => {
                    // A pressure canister: pale steel, a bright banded collar
                    // at the neck and a sight glass showing the gas. It has to
                    // read as "full of something that wants out".
                    let neck = (2..5).contains(&y) && (5..11).contains(&x);
                    let glass = (7..12).contains(&y) && (6..10).contains(&x);
                    let body = (1..TILE_SIZE - 1).contains(&x) && y >= 5;
                    if glass {
                        shade([0.55, 0.78, 0.92], noise * 0.05)
                    } else if neck {
                        shade([0.82, 0.66, 0.20], noise * 0.05)
                    } else if body {
                        shade([0.62, 0.64, 0.68], noise * 0.06)
                    } else {
                        shade([0.28, 0.29, 0.32], noise * 0.05)
                    }
                }
                slot::ELECTROLYSER => {
                    // A bath with two plates in it and bubbles coming off
                    // them: the picture of the thing it does, which is the
                    // only tutorial a block gets.
                    let frame = !(1..TILE_SIZE - 1).contains(&x) || !(1..TILE_SIZE - 1).contains(&y);
                    let plate = (4..6).contains(&x) || (TILE_SIZE - 6..TILE_SIZE - 4).contains(&x);
                    let bath = y >= 6;
                    let bubble = bath && ((x * 5 + y * 11) % 37) < 3;
                    if frame {
                        shade([0.30, 0.31, 0.34], noise * 0.05)
                    } else if bubble {
                        shade([0.86, 0.92, 0.98], noise * 0.04)
                    } else if plate && bath {
                        shade([0.72, 0.48, 0.22], noise * 0.05)
                    } else if bath {
                        shade([0.20, 0.42, 0.52], noise * 0.06)
                    } else {
                        shade([0.42, 0.44, 0.47], noise * 0.06)
                    }
                }
                slot::BUNKER_SHELL => {
                    // Poured concrete with the formwork seams still on it and
                    // rebar showing where the face has spalled. It has to read
                    // as "somebody built this to survive something", and — at
                    // four hundred hardness — as not worth your afternoon.
                    let seam = y % 8 == 0 || (x % 12 == 0 && y % 8 > 4);
                    let spall = ((x * 7 + y * 13) % 61) < 4;
                    let rebar = spall && (x + y) % 3 == 0;
                    if rebar {
                        shade([0.35, 0.24, 0.18], noise * 0.05)
                    } else if spall {
                        shade([0.44, 0.43, 0.41], noise * 0.07)
                    } else if seam {
                        shade([0.33, 0.33, 0.34], noise * 0.05)
                    } else {
                        shade([0.52, 0.52, 0.51], noise * 0.06)
                    }
                }
                slot::SUPPLY_CACHE => {
                    // A strapped crate with a stencilled band. Bright enough
                    // to spot down a dark corridor by lamplight, which is the
                    // whole job of the texture.
                    let band = (6..10).contains(&y);
                    let strap = (2..4).contains(&x) || (TILE_SIZE - 4..TILE_SIZE - 2).contains(&x);
                    let edge = !(1..TILE_SIZE - 1).contains(&x) || !(1..TILE_SIZE - 1).contains(&y);
                    if edge {
                        shade([0.24, 0.20, 0.14], noise * 0.05)
                    } else if strap {
                        shade([0.30, 0.31, 0.33], noise * 0.05)
                    } else if band {
                        shade([0.78, 0.62, 0.16], noise * 0.05)
                    } else {
                        shade([0.42, 0.33, 0.20], noise * 0.07)
                    }
                }
                slot::PRINTER => {
                    // A fabricator: a lit build chamber behind a frame, with
                    // a gantry rail across the top. The window is the tell —
                    // it should look like something is being made in there,
                    // because that is the whole promise of the machine.
                    let frame = !(2..TILE_SIZE - 2).contains(&x)
                        || !(2..TILE_SIZE - 2).contains(&y);
                    let rail = (3..5).contains(&y) && (2..TILE_SIZE - 2).contains(&x);
                    let bed = y >= TILE_SIZE - 5 && (4..TILE_SIZE - 4).contains(&x);
                    if frame {
                        shade([0.30, 0.32, 0.36], noise * 0.06)
                    } else if rail {
                        shade([0.55, 0.57, 0.60], noise * 0.05)
                    } else if bed {
                        shade([0.75, 0.78, 0.82], noise * 0.05)
                    } else {
                        // The chamber glow: warm, because something in there
                        // is hot.
                        shade([0.20, 0.17, 0.12], noise * 0.05)
                    }
                }
                slot::URANIUM_ORE => {
                    // Dark host rock with pitchblende in it and a sick green
                    // bloom around the grains. Nothing else in the palette is
                    // green-on-black, so a face of it in lamplight is
                    // unmistakable — which is the point, because this is the
                    // one ore that is also a warning sign.
                    let grain = jitter(tile ^ 0x9d, x / 2, y / 2);
                    if grain > 0.34 {
                        shade([0.30, 0.62, 0.26], noise * 0.10)
                    } else if grain > 0.12 {
                        shade([0.16, 0.26, 0.16], noise * 0.08)
                    } else {
                        shade([0.22, 0.21, 0.24], noise * 0.09)
                    }
                }
                slot::OIL_SAND => {
                    // Saturated sandstone: pale grains gone black where the
                    // crude has soaked in, in bands, because that is how it
                    // lies in the ground.
                    let band = ((y / 3) % 2) == 0;
                    let soaked = jitter(tile ^ 0x11, x, y / 2) > (if band { -0.3 } else { 0.25 });
                    if soaked {
                        shade([0.12, 0.10, 0.08], noise * 0.05)
                    } else {
                        shade([0.46, 0.40, 0.29], noise * 0.08)
                    }
                }
                slot::GAS_SHALE => {
                    // Laminated grey shale with a bluish sheen along the
                    // partings. Flat and layered, so it never gets confused
                    // with oil sand at a glance.
                    if y % 4 == 0 {
                        shade([0.34, 0.42, 0.48], noise * 0.05)
                    } else if y % 4 == 2 && (x + y) % 5 == 0 {
                        shade([0.44, 0.54, 0.60], noise * 0.04)
                    } else {
                        shade([0.28, 0.30, 0.33], noise * 0.07)
                    }
                }
                slot::OIL_BARREL => {
                    // A drum: two hoops around a dark cylinder, stacked
                    // upright, with a lid ring on top.
                    let hoop = (5..7).contains(&y) || (TILE_SIZE - 7..TILE_SIZE - 5).contains(&y);
                    let edge = !(2..TILE_SIZE - 2).contains(&x);
                    if edge {
                        shade([0.10, 0.10, 0.11], noise * 0.04)
                    } else if hoop {
                        shade([0.42, 0.36, 0.20], noise * 0.05)
                    } else {
                        shade([0.20, 0.18, 0.16], noise * 0.07)
                    }
                }
                slot::GAS_CELL => {
                    // A pressure bottle, upright, with a shoulder and a valve
                    // — the same silhouette as the oxyhydrogen canister but
                    // in field yellow, because mixing the two up at the pump
                    // should be impossible.
                    let bottle = (4..TILE_SIZE - 4).contains(&x) && y >= 4;
                    let valve = (7..9).contains(&x) && y < 4;
                    let highlight = bottle && x == 6;
                    if valve {
                        shade([0.55, 0.57, 0.60], noise * 0.04)
                    } else if highlight {
                        shade([0.95, 0.86, 0.40], noise * 0.03)
                    } else if bottle {
                        shade([0.72, 0.60, 0.14], noise * 0.06)
                    } else {
                        shade([0.24, 0.24, 0.26], noise * 0.05)
                    }
                }
                slot::WELLHEAD => {
                    // A christmas tree: the stack of valves and flanges that
                    // sits on a hole in the ground, painted the red that
                    // every wellhead on earth is painted.
                    let stack = (6..10).contains(&x);
                    let flange = stack && (y % 5 == 0);
                    let deck = y >= TILE_SIZE - 3;
                    if deck {
                        shade([0.34, 0.35, 0.38], noise * 0.05)
                    } else if flange {
                        shade([0.60, 0.60, 0.62], noise * 0.04)
                    } else if stack {
                        shade([0.62, 0.16, 0.12], noise * 0.06)
                    } else {
                        shade([0.28, 0.29, 0.31], noise * 0.06)
                    }
                }
                slot::EYE => {
                    // Off-white with the faintest warm cast, and no grain
                    // worth the name: an eye that looks like weathered
                    // stone reads as a hole in somebody's head.
                    shade([0.90, 0.89, 0.85], noise * 0.02)
                }
                slot::PUPIL => {
                    // Near-black with one bright texel of catchlight. The
                    // catchlight is the whole trick — it is what turns two
                    // dark squares into something that is looking at you.
                    if x == 4 && y == 4 {
                        shade([0.85, 0.88, 0.92], 0.0)
                    } else {
                        shade([0.07, 0.07, 0.09], noise * 0.03)
                    }
                }
                slot::COT => {
                    // A bed: pale sheeting with a folded blanket across the
                    // foot and a rail at the head. It has to read as
                    // "somewhere to lie down" from the doorway, because that
                    // is the whole instruction the building gives.
                    let rail = y < 3;
                    let blanket = y >= TILE_SIZE - 5;
                    let seam = blanket && (y == TILE_SIZE - 5 || x % 6 == 0);
                    if rail {
                        shade([0.62, 0.64, 0.68], noise * 0.04)
                    } else if seam {
                        shade([0.28, 0.42, 0.46], noise * 0.03)
                    } else if blanket {
                        shade([0.36, 0.54, 0.58], noise * 0.05)
                    } else {
                        shade([0.86, 0.87, 0.88], noise * 0.03)
                    }
                }
                // Unknown slots get magenta, the universal "missing texture".
                _ => [255, 0, 255, 255],
            };
            pixels.extend_from_slice(&texel);
        }
    }
    pixels
}

/// The block texture array and its sampler, ready to bind.
///
/// # The year is a repaint, not a shader
///
/// The atlas has always been procedural — sixty-three sixteen-pixel tiles
/// generated on the CPU at startup and uploaded once. That makes a season
/// almost free: the seven living slots are re-run at a new point in the year
/// and written back over their own layers. Nothing else in the pipeline
/// learns that seasons exist. No new uniform, no branch in the fragment
/// shader, no second atlas taking a second bind group, and no per-vertex
/// anything.
///
/// The cost is seven kilobytes uploaded on the day the year moves on, which
/// is twenty-eight times in a year and never in the same frame as anything
/// else.
pub struct TileTextures {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    pub layer_count: u32,
    /// Kept so the year can write back over it.
    texture: wgpu::Texture,
    /// The point in the year the foliage layers are currently painted for,
    /// quantised the same way [`Self::repaint`] quantises. `None` until the
    /// first repaint, so the first one always lands.
    painted: Option<i32>,
}

/// How finely the year is quantised for the repaint: a hundredth of the turn.
///
/// Fine enough that the canopy never visibly steps — a hundred stops across a
/// year is one every few minutes of play, and each is a fraction of a shade —
/// and coarse enough that an unchanged day does no work at all.
const TURN_STEPS: f32 = 100.0;

impl TileTextures {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layer_count = slot::COUNT;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("block tiles"),
            size: wgpu::Extent3d {
                width: TILE_SIZE,
                height: TILE_SIZE,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // sRGB: the shader's lighting maths wants linear values, and the
            // hardware converts on read.
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        for layer in 0..layer_count {
            // Painted green: a fresh save opens in spring, and the first
            // frame's `repaint` corrects it before anything is drawn if it
            // does not.
            let pixels = generate_tile(layer, 0.0);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(TILE_SIZE * 4),
                    rows_per_image: Some(TILE_SIZE),
                },
                wgpu::Extent3d {
                    width: TILE_SIZE,
                    height: TILE_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("block tiles view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block tiles sampler"),
            // Repeat is what makes merged-quad UVs tile correctly.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            // Nearest keeps the blocky look and avoids blurring at edges.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tiles layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tiles bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        TileTextures {
            view,
            sampler,
            bind_group_layout,
            bind_group,
            layer_count,
            texture,
            painted: None,
        }
    }

    /// Move the atlas on to where the year has got to.
    ///
    /// `turn` is [`vx_world::season::leaf_turn`] at the current tick. Cheap
    /// to call every frame and meant to be: it quantises, compares against
    /// what is already on the card, and returns having done nothing at all
    /// unless the year has actually moved. When it has moved it rewrites the
    /// seven layers in [`FOLIAGE`] and leaves the other fifty-six alone.
    ///
    /// Returns whether it painted, which the capture fixtures want so they
    /// can be sure the frame they grab is the season they asked for.
    pub fn repaint(&mut self, queue: &wgpu::Queue, turn: f32) -> bool {
        let step = (turn.clamp(0.0, 1.0) * TURN_STEPS).round() as i32;
        if self.painted == Some(step) {
            return false;
        }
        self.painted = Some(step);
        let turn = step as f32 / TURN_STEPS;
        for layer in FOLIAGE {
            let pixels = generate_tile(layer, turn);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(TILE_SIZE * 4),
                    rows_per_image: Some(TILE_SIZE),
                },
                wgpu::Extent3d {
                    width: TILE_SIZE,
                    height: TILE_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tile_generates_a_full_rgba_image() {
        for tile in 0..slot::COUNT {
            let pixels = generate_tile(tile, 0.0);
            assert_eq!(pixels.len(), (TILE_SIZE * TILE_SIZE * 4) as usize);
        }
    }

    #[test]
    fn tile_generation_is_deterministic() {
        // Terrain and textures must look the same on every machine and run.
        assert_eq!(generate_tile(slot::STONE, 0.0), generate_tile(slot::STONE, 0.0));
    }

    #[test]
    fn different_tiles_look_different() {
        assert_ne!(generate_tile(slot::STONE, 0.0), generate_tile(slot::DIRT, 0.0));
        assert_ne!(
            generate_tile(slot::GRASS_TOP, 0.0),
            generate_tile(slot::GRASS_SIDE, 0.0)
        );
    }

    #[test]
    fn tiles_are_textured_rather_than_flat_colour() {
        // A constant tile would mean the jitter is broken.
        let pixels = generate_tile(slot::STONE, 0.0);
        let first = &pixels[0..3];
        assert!(
            pixels.chunks_exact(4).any(|texel| texel[0..3] != *first),
            "stone tile is a flat colour"
        );
    }

    #[test]
    fn only_water_ice_and_tufts_are_transparent() {
        for tile in 0..slot::COUNT {
            let pixels = generate_tile(tile, 0.0);
            let alphas: Vec<u8> = pixels.chunks_exact(4).map(|texel| texel[3]).collect();
            match tile {
                slot::WATER => {
                    assert!(alphas.iter().all(|&a| a < 255), "water should be see-through")
                }
                slot::ICE => {
                    // See-through like water, and more solid than it: the
                    // lake bed shows, the floor reads as a floor.
                    assert!(alphas.iter().all(|&a| a < 255 && a > 128), "ice is the wrong kind of clear")
                }
                slot::TUFT => {
                    // Blades on empty air: some texels fully clear, some
                    // fully solid, nothing half-and-half to blend badly.
                    assert!(alphas.contains(&0), "tuft has no clear texels");
                    assert!(alphas.contains(&255), "tuft has no blades");
                    assert!(alphas.iter().all(|&a| a == 0 || a == 255));
                }
                _ => assert!(alphas.iter().all(|&a| a == 255), "tile {tile} should be opaque"),
            }
        }
    }

    #[test]
    fn grass_side_has_grass_above_dirt() {
        // The top row should be greener than the bottom row, or the block
        // reads upside down in game.
        let pixels = generate_tile(slot::GRASS_SIDE, 0.0);
        let row = |y: u32| -> (u32, u32) {
            let mut green = 0;
            let mut red = 0;
            for x in 0..TILE_SIZE {
                let at = ((y * TILE_SIZE + x) * 4) as usize;
                red += pixels[at] as u32;
                green += pixels[at + 1] as u32;
            }
            (red, green)
        };

        let (top_red, top_green) = row(0);
        let (bottom_red, bottom_green) = row(TILE_SIZE - 1);
        assert!(top_green > top_red, "top of grass_side is not green");
        assert!(bottom_red > bottom_green, "bottom of grass_side is not dirt");
    }

    /// The year repaints the living slots and nothing else. Stone is stone in
    /// February, and if that ever stopped being true the repaint would be
    /// rewriting sixty-three layers a day for no reason.
    #[test]
    fn only_the_foliage_takes_a_season() {
        for tile in 0..slot::COUNT {
            let green = generate_tile(tile, 0.0);
            let bare = generate_tile(tile, 1.0);
            if FOLIAGE.contains(&tile) {
                assert_ne!(green, bare, "living tile {tile} did not turn");
            } else {
                assert_eq!(green, bare, "tile {tile} has a season it should not");
            }
        }
    }

    /// The atlas is as deterministic as everything else: the same slot at the
    /// same point in the year is the same bytes, on any machine and any run.
    #[test]
    fn the_same_point_in_the_year_paints_the_same_atlas() {
        for turn in [0.0, 0.33, 0.55, 0.9, 1.0] {
            for tile in FOLIAGE {
                assert_eq!(generate_tile(tile, turn), generate_tile(tile, turn));
            }
        }
    }

    /// The canopy really does go green, then gold, then bare — measured off
    /// the pixels rather than off the constants that made them, because the
    /// look is the deliverable and a lerp that never reached the texture
    /// would pass every other test in this file.
    #[test]
    fn the_hardwood_canopy_goes_green_then_gold_then_bare() {
        let mean = |turn: f32| {
            let pixels = generate_tile(slot::LEAVES, turn);
            let mut totals = [0u32; 3];
            for texel in pixels.chunks_exact(4) {
                for channel in 0..3 {
                    totals[channel] += texel[channel] as u32;
                }
            }
            let count = (TILE_SIZE * TILE_SIZE) as f32;
            [
                totals[0] as f32 / count,
                totals[1] as f32 / count,
                totals[2] as f32 / count,
            ]
        };
        let summer = mean(0.0);
        let autumn = mean(0.55);
        let winter = mean(1.0);

        assert!(summer[1] > summer[0] * 1.4, "the summer canopy is not green");
        assert!(autumn[0] > autumn[1] * 1.2, "the autumn canopy is not gold");
        assert!(
            autumn[0] > summer[0] * 1.8,
            "autumn is no warmer than summer"
        );
        // Winter drains: the colour that was in it goes out, and what is
        // left is darker than the blaze that preceded it. Not grey — one
        // tone lags the other on purpose, so a canopy in December still has
        // the last of the gold caught in it, which is what a real one does.
        let spread = |colour: [f32; 3]| {
            colour[0].max(colour[1]).max(colour[2]) - colour[0].min(colour[1]).min(colour[2])
        };
        assert!(
            spread(winter) < spread(autumn) * 0.7,
            "the bare canopy is as colourful as the autumn one: {winter:?}"
        );
        assert!(winter[0] < autumn[0], "winter is brighter than autumn");
    }

    /// A spruce barely notices. That is the point of a spruce, and it is what
    /// makes the cove beside it legible when it turns.
    #[test]
    fn the_conifers_barely_move_and_the_hardwoods_do_the_work() {
        let travelled = |tile: u32| {
            let green = generate_tile(tile, 0.0);
            let bare = generate_tile(tile, 1.0);
            let sum: u32 = green
                .iter()
                .zip(bare.iter())
                .map(|(a, b)| a.abs_diff(*b) as u32)
                .sum();
            sum as f32 / green.len() as f32
        };
        let hardwood = travelled(slot::LEAVES);
        for conifer in [slot::NEEDLES, slot::BOG_NEEDLES] {
            assert!(
                travelled(conifer) < hardwood / 4.0,
                "conifer {conifer} moved {:.1} against the hardwood's {hardwood:.1}",
                travelled(conifer)
            );
            assert!(travelled(conifer) > 0.0, "conifer {conifer} ignores the year entirely");
        }
    }

    /// The blades and the sward are the same grass, so they turn together.
    /// A lawn whose tufts disagreed with the ground under them would read as
    /// a bug from the first step outside.
    #[test]
    fn the_tufts_match_the_ground_they_stand_in() {
        for turn in [0.0, 0.5, 1.0] {
            let ground = generate_tile(slot::GRASS_TOP, turn);
            let blades = generate_tile(slot::TUFT, turn);
            let mean = |pixels: &[u8], only_solid: bool| {
                let mut totals = [0u32; 3];
                let mut count = 0u32;
                for texel in pixels.chunks_exact(4) {
                    if only_solid && texel[3] == 0 {
                        continue;
                    }
                    for channel in 0..3 {
                        totals[channel] += texel[channel] as u32;
                    }
                    count += 1;
                }
                [
                    totals[0] as f32 / count as f32,
                    totals[1] as f32 / count as f32,
                    totals[2] as f32 / count as f32,
                ]
            };
            let ground = mean(&ground, false);
            let blades = mean(&blades, true);
            for channel in 0..3 {
                assert!(
                    (ground[channel] - blades[channel]).abs() < 30.0,
                    "at turn {turn} the tufts are a different grass: {ground:?} against {blades:?}"
                );
            }
        }
    }

    /// Nothing the atlas gained can make a foliage tile blend: they were
    /// opaque before the year existed and they stay opaque, or the canopy
    /// joins water in the unsorted alpha pass and the woods start flickering.
    #[test]
    fn a_season_never_makes_a_leaf_see_through() {
        for turn in [0.0, 0.4, 0.7, 1.0] {
            for tile in FOLIAGE {
                let pixels = generate_tile(tile, turn);
                for texel in pixels.chunks_exact(4) {
                    match tile {
                        slot::TUFT => assert!(texel[3] == 0 || texel[3] == 255),
                        _ => assert_eq!(texel[3], 255, "tile {tile} went see-through in the year"),
                    }
                }
            }
        }
    }
}
