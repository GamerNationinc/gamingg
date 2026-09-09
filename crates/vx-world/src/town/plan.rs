//! What a town is built of: authored blueprints, stamped at a site.
//!
//! Buildings are ASCII layer grids — the cheapest authoring format a human can
//! edit in place and a test can reason about. Layer `i` sits at
//! `site.ground + i`; layer 0 *replaces* the surface block, so floors and
//! paving land on the ground rather than hovering over it. Rows run +z from a
//! blueprint's `min`, characters run +x, and every offset is **relative to the
//! town centre** — which is what lets one set of `&'static` blueprints stamp at
//! any site on the lattice without allocating.
//!
//! Doors and windows are gaps in the wall grid. Nothing is carved afterwards.

use vx_core::{BlockPos, ChunkPos, LocalPos, CHUNK_SIZE};

use crate::chunk::Chunk;
use crate::gen::TerrainBlocks;
use crate::town::{Speciality, TownSite};

/// What an authored cell is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// Corrugated container wall.
    Metal,
    /// The same, weathered: the frontier has been out here a while.
    Rusted,
    /// Container roof and catwalk decking.
    Grate,
    /// The radio mast's lattice.
    Mast,
    /// The console the network is worked from.
    Beacon,
    /// The trading counter.
    Counter,
    /// Paving.
    Path,
    /// The player's storage chest, inside their house.
    Chest,
    /// The mailbox outside the player's door, where ordered goods land.
    Mailbox,
    /// The lockbox that says who may edit this building.
    Permit(Tier),
    /// The watch box on the office roof.
    Roost,
    /// The bank's deposit box.
    Vault,
    /// A ward cot: the whole of what a hospital is, mechanically.
    Cot,
}

/// How hard a lockbox is to get past.
///
/// The tier is the block, because no per-instance block state exists: three
/// tiers, three registered blocks, three tiles. That is not a workaround — it
/// means you can *see* a lock's grade across the room and decide whether it is
/// worth your afternoon before you start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// A house. Slow with a starter drill, but it will give.
    One,
    /// A shop, the tower, the sheriff's office. Not without real gear.
    Two,
    /// Bunkers and military outposts. Endgame; nothing stamps one yet.
    Three,
}

impl Tier {
    /// The namespaced block that carries this tier.
    pub fn block_name(self) -> &'static str {
        match self {
            Tier::One => "engine:permit_box_i",
            Tier::Two => "engine:permit_box_ii",
            Tier::Three => "engine:permit_box_iii",
        }
    }

    /// The ASCII character the blueprints author it with.
    pub fn glyph(self) -> u8 {
        match self {
            Tier::One => b'1',
            Tier::Two => b'2',
            Tier::Three => b'3',
        }
    }
}

/// What a building is for.
///
/// Purpose and geometry, never ownership — who holds a claim is fiction and
/// lives in `vx-app`, on the far side of the crate boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Somebody lives here.
    Dwelling,
    /// The player's own house.
    PlayerHouse,
    /// The supply counter.
    Shop,
    /// Where the sheriff and the deputies work.
    Security,
    /// The radio tower: the town's own infrastructure.
    Civic,
    /// Paving. Town property, but nothing to lock.
    Paving,
    /// The bank. The one building in town whose whole purpose is holding
    /// other people's things, which is why it carries the heaviest lock the
    /// game has — and the first Tier Three ever stamped anywhere.
    Bank,
    /// The clinic: two cots, a counter of sorts, and the only door on the
    /// frontier that is worth walking a long way to.
    Clinic,
}

impl Role {
    /// The grade of lock this kind of building carries.
    pub fn tier(self) -> Option<Tier> {
        match self {
            Role::Dwelling | Role::PlayerHouse => Some(Tier::One),
            Role::Shop | Role::Security | Role::Civic | Role::Clinic => Some(Tier::Two),
            Role::Bank => Some(Tier::Three),
            Role::Paving => None,
        }
    }
}

/// How deep a building's *strip* footing runs under its load-bearing walls.
///
/// Buildings are founded the way real ones are: a deep strip under whatever
/// carries load, and a shallow slab under the floor between. What sets the
/// depth is not the height of the building but what it is protecting — a
/// bank's strongroom is founded far deeper than a shed, because the strip is
/// the only thing standing between a shovel and everything on deposit.
///
/// Paving gets none. A plaza is a surface, not a structure, and putting four
/// hundred hardness under the whole market square would wall the town's own
/// ground off from anyone who ever wanted to dig a cellar.
pub fn strip_depth(role: Role) -> i32 {
    match role {
        Role::Paving => 0,
        Role::Dwelling | Role::PlayerHouse => 2,
        Role::Shop | Role::Security | Role::Clinic => 3,
        Role::Civic => 4,
        Role::Bank => 5,
    }
}

/// How deep the slab runs under a floor that carries nothing.
const SLAB_DEPTH: i32 = 1;

/// How deep this column's footing runs below the town's grade, if at all.
///
/// A column is founded on a strip when the layer above the floor holds
/// something — a wall, a lockbox, the mast's leg — and on a slab when it is
/// merely floor. Overlapping blueprints take the deeper answer, which is what
/// a builder would do.
pub fn footing_at(site: &TownSite, x: i32, z: i32) -> Option<i32> {
    let (local_x, local_z) = (x - site.centre.0, z - site.centre.1);
    let mut deepest = 0;
    for blueprint in plan_for(site) {
        let (width, depth) = blueprint.extent();
        let col = local_x - blueprint.min.0;
        let row = local_z - blueprint.min.1;
        if col < 0 || row < 0 || col >= width || row >= depth {
            continue;
        }
        let strip = strip_depth(blueprint.role);
        if strip == 0 {
            continue;
        }
        let filled = |layer: usize| -> bool {
            blueprint
                .layers
                .get(layer)
                .and_then(|rows| rows.get(row as usize))
                .and_then(|line| line.as_bytes().get(col as usize))
                .is_some_and(|glyph| *glyph != b'.')
        };
        let here = if filled(1) {
            strip
        } else if filled(0) {
            SLAB_DEPTH
        } else {
            0
        };
        deepest = deepest.max(here);
    }
    (deepest > 0).then_some(deepest)
}

/// One building at a site, with the ground it claims.
///
/// Bounds run one below the floor and one above the roof, so nobody tunnels
/// under a wall or drops a lid on a roof and calls it untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Building {
    pub role: Role,
    pub min: BlockPos,
    pub max: BlockPos,
}

/// One authored building, positioned relative to the town centre.
struct Blueprint {
    role: Role,
    min: (i32, i32),
    layers: &'static [&'static [&'static str]],
}

impl Blueprint {
    /// Width in x and depth in z, from layer zero.
    ///
    /// Safe only because every blueprint is rectangular on every layer, which
    /// `a_blueprint_is_rectangular_on_every_layer` exists to keep true —
    /// `cell_at` tolerates raggedness silently, so nothing else would notice.
    fn extent(&self) -> (i32, i32) {
        let depth = self.layers.first().map_or(0, |rows| rows.len()) as i32;
        let width = self
            .layers
            .first()
            .and_then(|rows| rows.first())
            .map_or(0, |line| line.len()) as i32;
        (width, depth)
    }
}

/// The radio tower: the one structure every town on the frontier shares.
///
/// A grated deck, four lattice legs rising well clear of the rooftops, and the
/// beacon console at its foot facing the plaza. `T` mast, `G` grate, `B`
/// console.
const RADIO_TOWER: Blueprint = Blueprint {
    role: Role::Civic,
    min: (-2, -16),
    layers: &[
        // Deck at ground level.
        &["GGGGG", "GGGGG", "GGGGG", "GGGGG", "GGGGG"],
        // The console stands on the deck's south edge, facing the plaza.
        &["T..2T", ".....", ".....", ".....", "T.B.T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        // A service ring part way up.
        &["TGGGT", "G...G", "G...G", "G...G", "TGGGT"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        &["T...T", ".....", ".....", ".....", "T...T"],
        // The head: aerials clustered at the top.
        &["TTTTT", "T...T", "T...T", "T...T", "TTTTT"],
        &[".T.T.", ".....", ".....", ".....", ".T.T."],
        &["..T..", ".....", ".....", ".....", "..T.."],
    ],
};

/// The supply shed: a container with the trading counter along its back wall.
/// `M` metal, `X` rusted, `G` roof decking, `C` counter.
/// The clinic: a ward with two cots against the far wall and a lockbox by
/// the door.
///
/// Same nine-by-seven container shell as the shop, because a frontier
/// hospital *is* a shed with beds in it — and because the shell is already
/// proven against the footing, claim and lock machinery. What makes it a
/// hospital is the two `H` cells, which are the only thing in the building
/// the player can use.
const CLINIC: Blueprint = Blueprint {
    role: Role::Clinic,
    min: (10, 7),
    layers: &[
        &[
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
        ],
        &[
            "MMMM.MMMM",
            "M.......M",
            "X.......X",
            "M.......M",
            "X.......X",
            "M2..H.H.M",
            "MMMXMMMXM",
        ],
        &[
            "MMMMMMMMM",
            "M.......X",
            "M.......M",
            "X.......M",
            "M.......X",
            "M.......M",
            "MXMMMXMMM",
        ],
        &[
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
        ],
    ],
};

const SUPPLY_SHED: Blueprint = Blueprint {
    role: Role::Shop,
    min: (-4, 6),
    layers: &[
        &[
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
        ],
        &[
            "MMM...MMM",
            "M.......M",
            "X.......X",
            "M.CCCCC.M",
            "X.......X",
            "M2......M",
            "MMMXMMMXM",
        ],
        &[
            "MMMM.MMMM",
            "M.......X",
            "M.......M",
            "X.......M",
            "M.......X",
            "M.......M",
            "MXMMMXMMM",
        ],
        &[
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
        ],
    ],
};

/// The bank: a lobby you walk into and a strongroom you do not.
///
/// The vault box sits behind an inner wall with one way through, and the
/// building's lock is Tier Three — the grade that has existed since stage 11
/// and has never had anything worth putting behind it until now. Breaking in
/// is possible, slow, loud and expensive, which is exactly the shape the
/// permits round was built for.
///
/// `M` metal, `X` rusted, `G` decking, `V` the vault box, `3` the lock.
const BANK: Blueprint = Blueprint {
    role: Role::Bank,
    min: (7, -14),
    layers: &[
        &[
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
        ],
        &[
            "MMMXMMMMM",
            "M...M...M",
            "X...M.V.X",
            "M.......M",
            "X...M...X",
            "M3..M...M",
            "MMMMMMMXM",
        ],
        &[
            "MMMMMMMMM",
            "M...M...M",
            "M...M...M",
            "X.......X",
            "M...M...M",
            "M...M...M",
            "MXMMMMMMM",
        ],
        &[
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
            "GGGGGGGGG",
        ],
    ],
};

/// A dwelling: one container, door in the east wall.
const CONTAINER_EAST_DOOR: &[&[&str]] = &[
    &["GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG"],
    &["MMMXMMM", "M1....X", "M......", "X......", "M.....M", "MXMMMXM"],
    // The doorway runs two blocks high, or nothing could walk through it.
    &["MMMMMMM", "X.....M", "M......", "M......", "X.....M", "MMMXMMM"],
    &["GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG"],
];

/// The same, door in the west wall.
const CONTAINER_WEST_DOOR: &[&[&str]] = &[
    &["GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG"],
    &["MMMXMMM", "X....1M", "......M", "......X", "M.....M", "MXMMMXM"],
    &["MMMMMMM", "M.....X", "......M", "......M", "M.....X", "MMMXMMM"],
    &["GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG"],
];

/// The same, door in the north wall — and stacked two containers high, with a
/// catwalk over the lower roof.
const CONTAINER_STACK: &[&[&str]] = &[
    &["GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG"],
    &["MMMXMMM", "M1....M", "X.....X", "M.....M", "M.....X", "MM...MM"],
    &["MMMMMMM", "X.....M", "M.....X", "M.....M", "X.....M", "MM...MM"],
    &["GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG"],
    &["XXXMXXX", "X.....X", "M.....M", "X.....X", "M.....M", "XXXXXXX"],
    &["GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG", "GGGGGGG"],
];

/// Paving: an east-west high street past every door and a north-south lane
/// from the tower down to the shed, crossing at the plaza where you wake up.
/// The player's own house, hometown only: the east-door container's proven
/// shape with the door facing the plaza lane, a chest against the west wall
/// and a mailbox planted outside beside the door. `S` chest, `O` mailbox.
///
/// The eighth column sits outside the walls — no floor, no roof — so the
/// mailbox stands on the town's ground like the street furniture it is.
const PLAYER_HOUSE: Blueprint = Blueprint {
    role: Role::PlayerHouse,
    min: (-17, 6),
    layers: &[
        &[
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
        ],
        &[
            "MMMXMMM.",
            "M.....XO",
            "MS......",
            "X.......",
            "M1....M.",
            "MXMMMXM.",
        ],
        // The doorway runs two blocks high, or nothing could walk through it.
        &[
            "MMMMMMM.",
            "X.....M.",
            "M.......",
            "M.......",
            "X.....M.",
            "MMMXMMM.",
        ],
        &[
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
            "GGGGGGG.",
        ],
    ],
};

/// Where the sheriff and the deputies work.
///
/// The mirror of the player's house across the plaza, door on the west face
/// looking down the high street. Its lockbox is a grade above a dwelling's:
/// the office that answers break-ins is not itself an easy break-in.
const SECURITY_OFFICE: Blueprint = Blueprint {
    role: Role::Security,
    min: (10, 6),
    layers: &[
        &[
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
        ],
        &[
            "MMMXMMM",
            "X....2M",
            "......M",
            "......X",
            "M.....M",
            "MXMMMXM",
        ],
        // The doorway runs two blocks high, or nothing could walk through it.
        &[
            "MMMMMMM",
            "M.....X",
            "......M",
            "......M",
            "M.....X",
            "MMMXMMM",
        ],
        &[
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
            "GGGGGGG",
        ],
        // The roost: a two-by-two watch box on the roof, out of which the
        // town's watcher drone launches when something loud happens. Placed
        // toward the plaza-facing corner so the pop-out is visible from the
        // street. Adding this layer grows the office's claim by one block of
        // height, which is correct — the box is the sheriff's property. Its
        // own block, so you can read it across the plaza and know what it is
        // before you decide to do anything about it.
        &[
            ".......",
            ".......",
            ".......",
            "....RR.",
            "....RR.",
            ".......",
        ],
    ],
};

const PATHS: Blueprint = Blueprint {
    role: Role::Paving,
    min: (-16, -9),
    layers: &[&[
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
        "PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP",
        "PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP",
        "PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP",
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
        "...............PPP...............",
    ]],
};

/// The hometown, and the shape every depot follows.
const DEPOT_TOWN: &[Blueprint] = &[
    RADIO_TOWER,
    BANK,
    SUPPLY_SHED,
    CLINIC,
    Blueprint {
        role: Role::Dwelling,
        min: (-17, -3),
        layers: CONTAINER_EAST_DOOR,
    },
    Blueprint {
        role: Role::Dwelling,
        min: (10, -3),
        layers: CONTAINER_WEST_DOOR,
    },
    Blueprint {
        role: Role::Dwelling,
        min: (-4, -24),
        layers: CONTAINER_STACK,
    },
    PATHS,
];

/// A mining camp: fewer dwellings, stacked bunk containers, same tower.
const MINE_TOWN: &[Blueprint] = &[
    RADIO_TOWER,
    BANK,
    SUPPLY_SHED,
    CLINIC,
    Blueprint {
        role: Role::Dwelling,
        min: (-17, -3),
        layers: CONTAINER_STACK,
    },
    Blueprint {
        role: Role::Dwelling,
        min: (10, -3),
        layers: CONTAINER_STACK,
    },
    PATHS,
];

/// A refinery: rusted tanks in place of half the housing.
const REFINERY_TOWN: &[Blueprint] = &[
    RADIO_TOWER,
    BANK,
    SUPPLY_SHED,
    CLINIC,
    Blueprint {
        role: Role::Dwelling,
        min: (-17, -3),
        layers: CONTAINER_WEST_DOOR,
    },
    Blueprint {
        role: Role::Dwelling,
        min: (10, -3),
        layers: CONTAINER_STACK,
    },
    Blueprint {
        role: Role::Dwelling,
        min: (-4, -24),
        layers: CONTAINER_EAST_DOOR,
    },
    PATHS,
];

/// The hometown: a depot, plus the one building no other town has — yours.
///
/// Kept as its own plan rather than a conditional inside the depot's, so
/// "the player's house is singular" is a property of the data instead of a
/// rule someone has to remember.
const HOME_TOWN: &[Blueprint] = &[
    RADIO_TOWER,
    BANK,
    SUPPLY_SHED,
    CLINIC,
    Blueprint {
        role: Role::Dwelling,
        min: (-17, -3),
        layers: CONTAINER_EAST_DOOR,
    },
    Blueprint {
        role: Role::Dwelling,
        min: (10, -3),
        layers: CONTAINER_WEST_DOOR,
    },
    Blueprint {
        role: Role::Dwelling,
        min: (-4, -24),
        layers: CONTAINER_STACK,
    },
    PLAYER_HOUSE,
    SECURITY_OFFICE,
    PATHS,
];

/// The buildings a site puts up.
///
/// A plan is picked, never generated: `&'static` throughout, so stamping
/// allocates nothing and the plan stays a pure function of the site.
fn plan_for(site: &TownSite) -> &'static [Blueprint] {
    if site.is_home() {
        return HOME_TOWN;
    }
    match site.speciality {
        Speciality::Depot => DEPOT_TOWN,
        Speciality::Mine => MINE_TOWN,
        Speciality::Refinery => REFINERY_TOWN,
    }
}

/// Where this town's counter stands, as an offset from its centre.
pub fn counter_offset(_site: &TownSite) -> (i32, i32) {
    (0, 9)
}

/// The shop's doorway, in its north wall, as an offset from the centre.
///
/// Named since stage 48 for the same reason the house's door was: the counter
/// stands *inside* a building, so anything walking to it on foot — a played
/// session, and one day anything that paths — has to aim at the gap in the
/// wall before it aims at the counter. The doorway is three wide.
pub fn shop_door_offset() -> (i32, i32) {
    (0, 6)
}

/// Where a customer stands to use the counter: inside the shop, on the near
/// side of the counter run, within arm's reach of it.
pub fn counter_stand_offset() -> (i32, i32) {
    (0, 8)
}

/// Where this town's beacon console stands, as an offset from its centre:
/// the south face of the radio tower's deck.
pub fn beacon_offset(_site: &TownSite) -> (i32, i32) {
    (0, -12)
}

/// Where the player's chest stands in the hometown, against the house's west
/// wall. Meaningless for any other site — only the hometown has the house.
pub fn chest_offset() -> (i32, i32) {
    (-16, 8)
}

/// The mailbox outside the player's door.
pub fn mailbox_offset() -> (i32, i32) {
    (-10, 7)
}

/// Where a new player wakes up: inside their house, facing the door.
pub fn spawn_offset() -> (i32, i32) {
    (-14, 9)
}

/// The doorway of the player's house, in its east wall.
///
/// Named since stage 48, because the loop starts by walking through it and
/// anything that wants to leave the house on foot has to aim at the gap
/// rather than at where it is going: the wall either side is two blocks of
/// solid container and a body that sets off on the bearing of somewhere a
/// hundred and seventy blocks away walks straight into it. The doorway is two
/// wide (z 8 and 9) and two high; this names the column in line with the
/// spawn, so leaving is a straight line for the first three blocks.
pub fn door_offset() -> (i32, i32) {
    (-11, 9)
}

/// Your own lockbox, in the corner of your house. Named so the geometry tests
/// and the blueprint cannot drift apart.
pub fn permit_offset_player_house() -> (i32, i32) {
    (-16, 10)
}

/// The authored cell at a world position for one site, if any.
pub fn cell_at(site: &TownSite, x: i32, y: i32, z: i32) -> Option<Cell> {
    let layer = y - site.ground;
    if layer < 0 {
        return None;
    }
    let (local_x, local_z) = (x - site.centre.0, z - site.centre.1);

    for blueprint in plan_for(site) {
        let Some(rows) = blueprint.layers.get(layer as usize) else {
            continue;
        };
        let (row, col) = (local_z - blueprint.min.1, local_x - blueprint.min.0);
        if row < 0 || col < 0 {
            continue;
        }
        let Some(line) = rows.get(row as usize) else {
            continue;
        };
        match line.as_bytes().get(col as usize) {
            Some(b'M') => return Some(Cell::Metal),
            Some(b'X') => return Some(Cell::Rusted),
            Some(b'G') => return Some(Cell::Grate),
            Some(b'T') => return Some(Cell::Mast),
            Some(b'B') => return Some(Cell::Beacon),
            Some(b'C') => return Some(Cell::Counter),
            Some(b'P') => return Some(Cell::Path),
            Some(b'S') => return Some(Cell::Chest),
            Some(b'O') => return Some(Cell::Mailbox),
            Some(b'1') => return Some(Cell::Permit(Tier::One)),
            Some(b'2') => return Some(Cell::Permit(Tier::Two)),
            Some(b'3') => return Some(Cell::Permit(Tier::Three)),
            Some(b'R') => return Some(Cell::Roost),
            Some(b'V') => return Some(Cell::Vault),
            Some(b'H') => return Some(Cell::Cot),
            _ => continue,
        }
    }
    None
}

/// The same across every gathered site, naming the site that owns the cell.
pub fn cell_at_any(
    sites: &[TownSite],
    x: i32,
    y: i32,
    z: i32,
) -> Option<(&TownSite, Cell)> {
    sites
        .iter()
        .find_map(|site| cell_at(site, x, y, z).map(|cell| (site, cell)))
}

/// Every building this site puts up, with the ground each one claims.
///
/// Pure in the site, like everything else here: the same town always yields the
/// same buildings in the same order, which is what lets a claim be *derived*
/// rather than stored.
pub fn buildings(site: &TownSite) -> Vec<Building> {
    plan_for(site)
        .iter()
        .map(|blueprint| {
            let (width, depth) = blueprint.extent();
            Building {
                role: blueprint.role,
                // Down to the bottom of the footing and one above the roof:
                // a claim you can tunnel under is not a claim, and now that
                // there is something real down there to cut, the claim has
                // to reach it.
                min: BlockPos::new(
                    site.centre.0 + blueprint.min.0,
                    site.ground - strip_depth(blueprint.role).max(1),
                    site.centre.1 + blueprint.min.1,
                ),
                max: BlockPos::new(
                    site.centre.0 + blueprint.min.0 + width - 1,
                    site.ground + blueprint.layers.len() as i32,
                    site.centre.1 + blueprint.min.1 + depth - 1,
                ),
            }
        })
        .collect()
}

/// Where this site's lockboxes stand, with the grade of each.
pub fn lockboxes(site: &TownSite) -> Vec<(BlockPos, Tier)> {
    let mut found = Vec::new();
    for blueprint in plan_for(site) {
        for (layer, rows) in blueprint.layers.iter().enumerate() {
            for (row, line) in rows.iter().enumerate() {
                for (col, byte) in line.bytes().enumerate() {
                    let tier = match byte {
                        b'1' => Tier::One,
                        b'2' => Tier::Two,
                        b'3' => Tier::Three,
                        _ => continue,
                    };
                    found.push((
                        BlockPos::new(
                            site.centre.0 + blueprint.min.0 + col as i32,
                            site.ground + layer as i32,
                            site.centre.1 + blueprint.min.1 + row as i32,
                        ),
                        tier,
                    ));
                }
            }
        }
    }
    found
}

/// The tallest authored layer of a site's plan, for the stamping loop's bound.
fn max_layers(site: &TownSite) -> i32 {
    plan_for(site)
        .iter()
        .map(|blueprint| blueprint.layers.len() as i32)
        .max()
        .unwrap_or(0)
}

/// Stamp every gathered town's authored blocks into a freshly generated chunk.
///
/// Pure in `(chunk position, sites)`: the same chunk always receives the same
/// blocks, so regeneration is idempotent and nothing here is ever saved.
pub fn stamp(chunk: &mut Chunk, position: ChunkPos, sites: &[TownSite], blocks: &TerrainBlocks) {
    let origin = position.origin();

    for site in sites {
        // Quick reject: does this chunk overlap the site's buildable square?
        let reach = site.core_half;
        if origin.x > site.centre.0 + reach
            || origin.z > site.centre.1 + reach
            || origin.x + CHUNK_SIZE <= site.centre.0 - reach
            || origin.z + CHUNK_SIZE <= site.centre.1 - reach
        {
            continue;
        }

        let layers = max_layers(site);
        for local_z in 0..CHUNK_SIZE {
            for local_x in 0..CHUNK_SIZE {
                let world_x = origin.x + local_x;
                let world_z = origin.z + local_z;

                // Footings first, below grade: everything above is built on
                // top of them, and a wall poured after its own foundation is
                // the wrong way round in a stamp as well as on a site.
                if let Some(depth) = footing_at(site, world_x, world_z) {
                    for step in 1..=depth {
                        if let Some(cell) = LocalPos::new(local_x, site.ground - step, local_z) {
                            chunk.set(cell, blocks.footing);
                        }
                    }
                }
                for layer in 0..layers {
                    let world_y = site.ground + layer;
                    let Some(cell) = cell_at(site, world_x, world_y, world_z) else {
                        continue;
                    };
                    let block = match cell {
                        Cell::Metal => blocks.metal_wall,
                        Cell::Rusted => blocks.rusted_metal,
                        Cell::Grate => blocks.catwalk,
                        Cell::Mast => blocks.mast,
                        Cell::Beacon => blocks.beacon,
                        Cell::Counter => blocks.counter,
                        Cell::Path => blocks.stone,
                        Cell::Chest => blocks.chest,
                        Cell::Mailbox => blocks.mailbox,
                        Cell::Permit(Tier::One) => blocks.permit_box_i,
                        Cell::Permit(Tier::Two) => blocks.permit_box_ii,
                        Cell::Permit(Tier::Three) => blocks.permit_box_iii,
                        Cell::Roost => blocks.roost,
                        Cell::Vault => blocks.vault,
                        Cell::Cot => blocks.ward_cot,
                    };
                    if let Some(local) = LocalPos::new(local_x, world_y, local_z) {
                        chunk.set(local, block);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::town::{self, HOME_GROUND_Y};

    #[test]
    fn every_wall_stands_on_a_strip_and_every_floor_on_a_slab() {
        // A building founded on dirt is a building you get into with a
        // shovel. Every load-bearing column runs a deep strip; the floor
        // between runs a slab; the plaza runs neither.
        let site = town::home_site();
        for blueprint in plan_for(&site) {
            let (width, depth) = blueprint.extent();
            let strip = strip_depth(blueprint.role);
            for row in 0..depth {
                for col in 0..width {
                    let x = site.centre.0 + blueprint.min.0 + col;
                    let z = site.centre.1 + blueprint.min.1 + row;
                    let bearing = cell_at(&site, x, HOME_GROUND_Y + 1, z).is_some();
                    let found = footing_at(&site, x, z);
                    if strip == 0 {
                        continue;
                    }
                    if bearing {
                        assert!(
                            found.is_some_and(|deep| deep >= strip),
                            "{:?} carries a wall at ({x}, {z}) on {found:?}",
                            blueprint.role
                        );
                    } else if cell_at(&site, x, HOME_GROUND_Y, z).is_some() {
                        assert!(
                            found.is_some(),
                            "{:?} has unfounded floor at ({x}, {z})",
                            blueprint.role
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_plaza_is_not_founded() {
        // Paving is a surface, not a structure. Four hundred hardness under
        // the whole market square would wall the town's own ground off from
        // anybody who ever wanted a cellar — so a paved column that no
        // building also stands on carries nothing below it.
        assert_eq!(strip_depth(Role::Paving), 0);
        let site = town::home_site();
        let mut checked = 0;
        for blueprint in plan_for(&site) {
            if blueprint.role != Role::Paving {
                continue;
            }
            let (width, depth) = blueprint.extent();
            for row in 0..depth {
                for col in 0..width {
                    let x = site.centre.0 + blueprint.min.0 + col;
                    let z = site.centre.1 + blueprint.min.1 + row;
                    // Skip anywhere a real building also stands: that column
                    // is founded by the building, and rightly.
                    let built_on = plan_for(&site).iter().any(|other| {
                        if other.role == Role::Paving {
                            return false;
                        }
                        let (ow, od) = other.extent();
                        let ocol = x - site.centre.0 - other.min.0;
                        let orow = z - site.centre.1 - other.min.1;
                        (0..ow).contains(&ocol) && (0..od).contains(&orow)
                    });
                    if built_on {
                        continue;
                    }
                    assert_eq!(
                        footing_at(&site, x, z),
                        None,
                        "the open plaza is founded at ({x}, {z})"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 50, "only {checked} open paved columns checked");
    }

    #[test]
    fn the_vault_cannot_be_reached_from_underneath() {
        // The whole reason footings exist. Before them the way past a Tier
        // Three lock was a hole in the floor, which would have made every
        // lock in this game decoration.
        let site = town::home_site();
        let vault = (0..4)
            .flat_map(|dy| {
                (-40..40).flat_map(move |dx| {
                    (-40..40).map(move |dz| (dx, dy, dz))
                })
            })
            .map(|(dx, dy, dz)| {
                (
                    site.centre.0 + dx,
                    HOME_GROUND_Y + dy,
                    site.centre.1 + dz,
                )
            })
            .find(|(x, y, z)| cell_at(&site, *x, *y, *z) == Some(Cell::Vault))
            .expect("the hometown has a bank");

        // The box itself, and every column around it at floor level, is
        // founded on the bank's own deep strip.
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let found = footing_at(&site, vault.0 + dx, vault.2 + dz);
            assert!(
                found.is_some(),
                "the ground under the vault at ({dx}, {dz}) is bare dirt"
            );
        }
        let under = footing_at(&site, vault.0, vault.2).unwrap();
        assert!(
            under >= SLAB_DEPTH,
            "the vault stands on nothing but a scrape"
        );

        // And the claim reaches the bottom of that footing, so undermining it
        // is a crime as well as a long afternoon.
        let bank = buildings(&site)
            .into_iter()
            .find(|building| building.role == Role::Bank)
            .expect("the bank is a building");
        assert!(
            bank.min.y <= HOME_GROUND_Y - strip_depth(Role::Bank),
            "the bank's claim stops above its own foundation"
        );
    }

    #[test]
    fn the_counter_stands_inside_the_shop() {
        let site = town::home_site();
        let counter = town::counter_position(&site);
        assert_eq!(
            cell_at(&site, counter.x, counter.y, counter.z),
            Some(Cell::Counter)
        );
        // Container walls surround it at the same height.
        assert!(matches!(
            cell_at(&site, -4, HOME_GROUND_Y + 1, 12),
            Some(Cell::Metal | Cell::Rusted)
        ));
    }

    /// You can get to the counter, and stand somewhere to use it.
    ///
    /// The counter is inside a building, and until stage 48 nothing named the
    /// way in — so a body walking at the counter's own coordinates walked into
    /// the shop's north wall instead. The door and the customer's spot are
    /// named now, and this is what keeps them true.
    #[test]
    fn the_shop_has_a_door_and_somewhere_to_stand_at_the_counter() {
        let site = town::home_site();
        let counter = town::counter_position(&site);
        let (door_x, door_z) = shop_door_offset();
        let (stand_x, stand_z) = counter_stand_offset();

        // A doorway two high in the named column, or nothing could walk
        // through it.
        for y in [HOME_GROUND_Y + 1, HOME_GROUND_Y + 2] {
            assert_eq!(
                cell_at(&site, door_x, y, door_z),
                None,
                "the shop's doorway is blocked at ({door_x},{y},{door_z})"
            );
        }

        // Somewhere to stand inside, clear to head height.
        for y in [HOME_GROUND_Y + 1, HOME_GROUND_Y + 2] {
            assert_eq!(
                cell_at(&site, stand_x, y, stand_z),
                None,
                "no room to stand at the counter at ({stand_x},{y},{stand_z})"
            );
        }

        // And the walk from the door to that spot is a straight line down the
        // same column, with the counter in reach at the end of it.
        for z in door_z..=stand_z {
            assert_eq!(
                cell_at(&site, stand_x, HOME_GROUND_Y + 1, z),
                None,
                "the way in is blocked at z={z}"
            );
        }
        let stand = (
            site.centre.0 + stand_x,
            site.centre.1 + stand_z,
        );
        let reach = ((counter.x - stand.0) as f64).hypot((counter.z - stand.1) as f64);
        assert!(
            reach <= 5.0,
            "the customer's spot is {reach} from the counter, out of arm's reach"
        );
    }

    #[test]
    fn spawn_stays_clear_of_the_furniture() {
        // The player appears at the centre; nothing authored may stand there,
        // and the ground under them is paving rather than a wall.
        let site = town::home_site();
        for layer in 1..max_layers(&site) {
            assert_eq!(cell_at(&site, 0, HOME_GROUND_Y + layer, 0), None, "layer {layer}");
        }
        assert_eq!(cell_at(&site, 0, HOME_GROUND_Y, 0), Some(Cell::Path));
    }

    #[test]
    fn every_authored_cell_sits_inside_its_core() {
        // A building leaking past the flat core would float or drown.
        let site = town::home_site();
        for blueprint in plan_for(&site) {
            for (layer, rows) in blueprint.layers.iter().enumerate() {
                for (row, line) in rows.iter().enumerate() {
                    for (col, byte) in line.bytes().enumerate() {
                        if byte == b'.' {
                            continue;
                        }
                        let x = blueprint.min.0 + col as i32;
                        let z = blueprint.min.1 + row as i32;
                        assert!(
                            x.abs() <= site.core_half && z.abs() <= site.core_half,
                            "cell at ({x},{z}) layer {layer} leaves the core"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_players_house_stands_hollow_with_a_working_door() {
        let site = town::home_site();
        // Interior air across both wall layers.
        let fittings = [chest_offset(), permit_offset_player_house()];
        for x in -16..=-12 {
            for z in 7..=10 {
                if fittings.contains(&(x, z)) {
                    continue;
                }
                for y in [HOME_GROUND_Y + 1, HOME_GROUND_Y + 2] {
                    assert_eq!(cell_at(&site, x, y, z), None, "furniture at ({x},{y},{z})");
                }
            }
        }
        // The doorway: two wide, two high, in the east wall — and where
        // `door_offset` says it is, so the walk out of the house and the
        // plan that draws it cannot drift apart.
        let (door_x, door_z) = door_offset();
        assert!([8, 9].contains(&door_z), "the named door is not in the doorway");
        for z in [8, 9] {
            for y in [HOME_GROUND_Y + 1, HOME_GROUND_Y + 2] {
                assert_eq!(cell_at(&site, door_x, y, z), None, "doorway blocked at z={z} y={y}");
            }
        }
        // A floor underfoot and a roof overhead.
        assert_eq!(cell_at(&site, -14, HOME_GROUND_Y, 9), Some(Cell::Grate));
        assert_eq!(cell_at(&site, -14, HOME_GROUND_Y + 3, 9), Some(Cell::Grate));
    }

    #[test]
    fn the_chest_and_mailbox_stand_where_the_plan_promises() {
        let site = town::home_site();
        let chest = town::chest_position(&site);
        assert_eq!(cell_at(&site, chest.x, chest.y, chest.z), Some(Cell::Chest));
        let mailbox = town::mailbox_position(&site);
        assert_eq!(
            cell_at(&site, mailbox.x, mailbox.y, mailbox.z),
            Some(Cell::Mailbox)
        );
        // The mailbox stands outside the walls and blocks neither door column.
        for z in [8, 9] {
            assert_eq!(cell_at(&site, -10, HOME_GROUND_Y + 1, z), None, "door blocked");
        }
    }

    #[test]
    fn the_spawn_column_inside_the_house_is_clear_and_floored() {
        let site = town::home_site();
        let spawn = town::spawn_position(&site);
        assert_eq!(cell_at(&site, spawn.x, spawn.y - 1, spawn.z), Some(Cell::Grate));
        for y in [spawn.y, spawn.y + 1] {
            assert_eq!(cell_at(&site, spawn.x, y, spawn.z), None, "spawn blocked at y={y}");
        }
    }

    #[test]
    fn the_house_exists_only_in_the_hometown() {
        // Another depot on the lattice gets the plain plan: your house is
        // singular, as a property of the data.
        let elsewhere = TownSite {
            centre: (2048, 2048),
            ..town::home_site()
        };
        assert!(!elsewhere.is_home());
        let (cx, cz) = chest_offset();
        assert_eq!(
            cell_at(&elsewhere, elsewhere.centre.0 + cx, HOME_GROUND_Y + 1, elsewhere.centre.1 + cz),
            None,
            "a stranger's depot grew the player's chest"
        );
    }

    #[test]
    fn a_blueprint_is_rectangular_on_every_layer() {
        // `extent` reads width and depth off layer zero, and `cell_at`
        // tolerates a ragged row in silence — so a stray character would give
        // every claim in the game slightly wrong edges and nothing would say
        // so. This is the guard for that.
        for site in [
            town::home_site(),
            TownSite { centre: (512, 0), speciality: Speciality::Mine, ..town::home_site() },
            TownSite { centre: (0, 512), speciality: Speciality::Refinery, ..town::home_site() },
        ] {
            for blueprint in plan_for(&site) {
                let (width, depth) = blueprint.extent();
                for (layer, rows) in blueprint.layers.iter().enumerate() {
                    assert_eq!(
                        rows.len() as i32,
                        depth,
                        "layer {layer} at {:?} has a different depth",
                        blueprint.min
                    );
                    for (row, line) in rows.iter().enumerate() {
                        assert_eq!(
                            line.len() as i32,
                            width,
                            "row {row} of layer {layer} at {:?} is ragged",
                            blueprint.min
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_building_carries_a_lockbox_of_its_tier() {
        // Paving has nothing to lock; everything else does, and at the grade
        // its role calls for.
        for site in [
            town::home_site(),
            TownSite { centre: (512, 0), speciality: Speciality::Mine, ..town::home_site() },
            TownSite { centre: (0, 512), speciality: Speciality::Refinery, ..town::home_site() },
        ] {
            let boxes = lockboxes(&site);
            for building in buildings(&site) {
                let Some(tier) = building.role.tier() else {
                    continue;
                };
                let found = boxes.iter().find(|(at, _)| {
                    at.x >= building.min.x
                        && at.x <= building.max.x
                        && at.z >= building.min.z
                        && at.z <= building.max.z
                });
                let (_, grade) = found.unwrap_or_else(|| {
                    panic!("{:?} at {:?} has no lockbox", building.role, building.min)
                });
                assert_eq!(*grade, tier, "{:?} carries the wrong grade", building.role);
            }
        }
    }

    #[test]
    fn a_lockbox_never_blocks_a_door_or_a_home_route() {
        // The doorways every plan promises, and the interior points the
        // villagers walk to at night. A box in either would wall somebody in.
        let site = town::home_site();
        let blocked: Vec<BlockPos> = lockboxes(&site).into_iter().map(|(at, _)| at).collect();

        let doorways = [
            BlockPos::new(0, HOME_GROUND_Y + 1, 6),    // the shed
            BlockPos::new(-11, HOME_GROUND_Y + 1, -1), // east-door container
            BlockPos::new(10, HOME_GROUND_Y + 1, -1),  // west-door container
            BlockPos::new(-11, HOME_GROUND_Y + 1, 8),  // the player's house
            BlockPos::new(10, HOME_GROUND_Y + 1, 8),   // the security office
        ];
        for door in doorways {
            assert!(!blocked.contains(&door), "a lockbox blocks the door at {door:?}");
        }

        // The roster's three home routes end inside their containers.
        for bed in [(-14.0, -0.5), (-1.0, -21.0), (13.0, -0.5)] {
            for y in [HOME_GROUND_Y + 1, HOME_GROUND_Y + 2] {
                let at = BlockPos::new(bed.0 as i32, y, bed.1 as i32);
                assert!(!blocked.contains(&at), "a lockbox stands where somebody sleeps");
            }
        }
    }

    #[test]
    fn the_security_office_stands_only_in_the_hometown() {
        let home = town::home_site();
        assert!(buildings(&home).iter().any(|b| b.role == Role::Security));

        let elsewhere = TownSite { centre: (2048, 2048), ..home };
        assert!(
            !buildings(&elsewhere).iter().any(|b| b.role == Role::Security),
            "a stranger's depot grew a sheriff"
        );
    }

    #[test]
    fn building_bounds_cover_every_authored_cell() {
        // A claim is only as honest as its edges: every block a plan actually
        // stamps has to fall inside the box that claims it.
        let site = town::home_site();
        let boxes = buildings(&site);
        for layer in 0..max_layers(&site) {
            let y = site.ground + layer;
            for x in -30..=30 {
                for z in -30..=30 {
                    if cell_at(&site, x, y, z).is_none() {
                        continue;
                    }
                    assert!(
                        boxes.iter().any(|b| {
                            x >= b.min.x && x <= b.max.x
                                && y >= b.min.y && y <= b.max.y
                                && z >= b.min.z && z <= b.max.z
                        }),
                        "authored cell at ({x},{y},{z}) is inside no building"
                    );
                }
            }
        }
    }

    #[test]
    fn a_claim_reaches_under_the_floor_and_over_the_roof() {
        // Otherwise the answer to a locked door is a shovel.
        let site = town::home_site();
        let house = buildings(&site)
            .into_iter()
            .find(|b| b.role == Role::PlayerHouse)
            .expect("the hometown has the player's house");
        assert!(house.min.y < site.ground, "you could tunnel in from below");
        assert!(
            house.max.y > site.ground + 3,
            "you could build a lid on the roof"
        );
    }

    #[test]
    fn doors_exist_where_the_blueprints_promise() {
        let site = town::home_site();
        assert_eq!(cell_at(&site, 0, HOME_GROUND_Y + 1, 6), None, "shed doorway blocked");
        assert_eq!(cell_at(&site, -11, HOME_GROUND_Y + 1, -1), None, "east-door container");
        assert_eq!(cell_at(&site, 10, HOME_GROUND_Y + 1, -1), None, "west-door container");
    }

    #[test]
    fn every_town_has_a_beacon_and_a_counter() {
        // Both are the town's link to the wider game: one to the network, one
        // to the economy. A plan missing either would strand a traveller.
        for speciality in [Speciality::Depot, Speciality::Mine, Speciality::Refinery] {
            let mut site = town::home_site();
            site.speciality = speciality;

            let beacon = town::beacon_position(&site);
            assert_eq!(
                cell_at(&site, beacon.x, beacon.y, beacon.z),
                Some(Cell::Beacon),
                "{speciality:?} has no beacon at its post"
            );

            let counter = town::counter_position(&site);
            assert_eq!(
                cell_at(&site, counter.x, counter.y, counter.z),
                Some(Cell::Counter),
                "{speciality:?} has no counter"
            );
        }
    }

    #[test]
    fn the_radio_tower_stands_well_clear_of_the_rooftops() {
        // It is the landmark you navigate a town by, so it has to clear the
        // containers by a good margin.
        let site = town::home_site();
        let mast_top = (0..40)
            .rev()
            .find(|layer| cell_at(&site, 0, HOME_GROUND_Y + layer, -14) == Some(Cell::Mast))
            .or_else(|| {
                (0..40).rev().find(|layer| {
                    cell_at(&site, -2, HOME_GROUND_Y + layer, -16) == Some(Cell::Mast)
                })
            })
            .expect("no mast anywhere in the hometown");
        assert!(mast_top >= 12, "the mast tops out at only {mast_top} blocks");
    }

    #[test]
    fn a_plan_stamps_the_same_wherever_its_town_sits() {
        // The point of site-relative blueprints: geometry travels with the
        // town rather than being nailed to the origin.
        let home = town::home_site();
        let mut moved = home;
        moved.centre = (1500, -2200);
        moved.ground = 88;

        for (dx, dz, dy) in [(0, 9, 1), (-4, 12, 1), (0, 0, 0), (-11, -1, 1)] {
            let here = cell_at(&home, dx, HOME_GROUND_Y + dy, dz);
            let there = cell_at(&moved, moved.centre.0 + dx, moved.ground + dy, moved.centre.1 + dz);
            assert_eq!(here, there, "offset ({dx},{dz},{dy}) differs between sites");
        }
    }

    #[test]
    fn every_town_has_a_ward_with_two_cots_in_it() {
        // The building is the feature: a plan that stamps a clinic with no
        // cot in it is a hospital you cannot use, and the failure would be
        // invisible until somebody walked in bleeding.
        for site in [
            town::home_site(),
            TownSite {
                speciality: Speciality::Mine,
                ..town::home_site()
            },
            TownSite {
                speciality: Speciality::Depot,
                ..town::home_site()
            },
            TownSite {
                speciality: Speciality::Refinery,
                ..town::home_site()
            },
        ] {
            let mut cots = Vec::new();
            for x in site.centre.0 - 40..site.centre.0 + 40 {
                for z in site.centre.1 - 40..site.centre.1 + 40 {
                    for layer in 0..4 {
                        if cell_at(&site, x, site.ground + layer, z) == Some(Cell::Cot) {
                            cots.push((x, site.ground + layer, z));
                        }
                    }
                }
            }
            assert_eq!(cots.len(), 2, "{:?} ward has {cots:?}", site.speciality);

            // And the ward is a claimed building like any other, so breaking
            // into one is a crime rather than a shortcut.
            let clinic = buildings(&site)
                .into_iter()
                .find(|building| building.role == Role::Clinic)
                .expect("a town with cots and no clinic");
            for (x, y, z) in &cots {
                assert!(
                    *x >= clinic.min.x
                        && *x <= clinic.max.x
                        && *z >= clinic.min.z
                        && *z <= clinic.max.z
                        && *y >= clinic.min.y
                        && *y <= clinic.max.y,
                    "a cot at {x},{y},{z} is outside its own building"
                );
            }
            println!("{:?}: cots at {cots:?}", site.speciality);
        }
    }
}
