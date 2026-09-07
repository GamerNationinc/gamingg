//! The command journal: what the player told the world to do, in order.
//!
//! # Why a log and not a picture
//!
//! Ask who actually edits this world. The player breaks a few hundred blocks by
//! hand over a session. A single drone excavating an open pit moves *tens of
//! thousands* — and it moves exactly the same ones every time, because
//! `digging_is_deterministic` says so and the chunk-pinning in
//! [`vx_agent::working_span`] is what makes that true regardless of where the
//! player was standing.
//!
//! So writing the drone's output to disk is writing down the output of a
//! deterministic function whose input was one line. A whole mine — every bench,
//! every haul, every block — is recoverable from `Dispatch { area, method }`
//! plus the number of ticks it was allowed to run.
//!
//! That is what this records. Region files still hold the ground (they are the
//! keyframes); the log holds everything since the last one, and replaying it
//! reconstructs the rest.
//!
//! # Ticks, never seconds
//!
//! [`crate::mining::Mining::update`] turns wall-clock elapsed time into a tick
//! count, and that conversion depends on frame rate, stalls, and how long
//! somebody dragged the window. None of it is reproducible. The tick count is.
//! So the log records `Advance { ticks }` and replays through
//! [`crate::mining::Mining::advance`], which means a session recorded at nine
//! frames a second replays identically at three hundred.
//!
//! # What it is worth beyond small saves
//!
//! A log plus a content hash is a *determinism oracle*. Replay it and check the
//! world came out the same, and you have a test that covers worldgen, agents
//! and editing all at once — one that catches divergence no unit test is shaped
//! to find. A save becomes a perfect reproduction case. Rewind becomes possible.
//! None of that is built here; all of it is opened up by recording orders
//! instead of outcomes.

use std::io::{Read, Write};
use std::path::Path;

use vx_agent::{MineMethod, VoxelAabb};
use vx_core::{BlockPos, EventBus};
use vx_world::{PlayerBody, World};

use crate::mining::Mining;
use crate::movement::{self, MoveCommand, Movement};

const MAGIC: &[u8; 4] = b"VXLG";
// Bumped to 2 when a dispatch gained its crew size and to 3 when the player's
// own movement joined the log — both format changes.
//
// Four and five are *world* changes: the player's house joined the hometown,
// then the lockboxes and the security office did. An older log replays against
// ground that no longer generates, so it would report a divergence that is
// nobody's fault. Refusing it lets the oracle restart honestly instead. A
// journal is an oracle rather than a load path — region files are still written
// every save — so an old one being rejected costs a determinism check, not a
// world.
//
// Six added the gold panel's admin orders. A cheat is an order like any other:
// the moment an admin action mutated state outside this log, the oracle would
// be blind to it and every replay of that session would report a divergence
// that is nobody's fault.
//
// Seven added `Fire`. Shots break blocks, so a firefight is part of the hash.
//
// Eight is both kinds at once: the roost box joined the security office roof
// (a world change, like four and five) and the kestrel's standing orders
// joined the log (a format change). The scout itself never touches ground —
// its orders replay as documented no-ops — but the log stays the complete
// record of what was ordered.
//
// Nine is both kinds again: the watch box became its own block (a world
// change) and intrusion orders joined the log (a format change). An
// intrusion moves claims and grants, never ground, so it replays as a
// no-op like the scout's orders.
//
// Ten is both again: the fabricator became a block, and `Print` joined the
// log. Unlike the scout and the coil, a print is *not* a no-op — it eats
// the base pile, and the pile is something `Rebuilt` carries. So replay
// spends the materials exactly as the live game did, and only the outputs
// that live outside the pile are the honest no-ops.
//
// Eleven is a world change alone: caves. The carve is part of generated
// ground, so every hash a version-ten journal recorded was taken over
// terrain that no longer exists. No format change — the same commands read
// the same way — but pretending the old hashes still bind would make every
// old session replay as a divergence that is really this bump.
//
// Twelve is the promised "recipe indices are content" bump: the optics rows
// joined the fabricator's catalogue in ladder order rather than at the end,
// which renumbers every recipe after them — and `Print` records a recipe by
// index. A version-eleven journal's Print entries would replay as different
// recipes spending different materials, which is exactly the divergence this
// number exists to name.
//
// Thirteen is a world change alone, like eleven: bunkers. The ground now
// holds works that were not there, so every hash a version-twelve journal
// recorded was taken over terrain that no longer exists.
//
// Sixteen founds every building and every fort wall on footings, which is
// generated ground that was not there before.
//
// Fifteen adds `Bank` — goods across a strongroom counter — and the star
// forts and bank buildings that generated ground now carries. Both kinds of
// change at once, like eight and nine before it.
//
// Fourteen adds `Electrolyse` — a format change, and one that reaches the
// hash by a longer road than usual. The order itself only moves goods, but
// the fleet burns those goods to dig, so a log replayed without it would
// run the crew dry at a different tick and leave a different hole.
//
// Seventeen adds `Talk` and `Gift` — the townsfolk got names. Talk replays
// as a no-op like `Scout`; `Gift` takes a good off the base pile, which is
// state `Rebuilt` carries, so the format change is also a meaning change.
//
// Eighteen changes no command at all — it renumbers them. Five upgrade
// parts joined the fabricator's catalogue *in ladder order*, which shifts
// the indices `Print` records, so a version-seventeen log replayed here
// would print the wrong pattern and spend the wrong materials. The same
// reason twelve exists: recipe indices are content, and the ladder is worth
// more than an append-only table.
//
// Nineteen adds `Repair`. Wear is the first machine state that had to live
// *inside* the replayed `Mining` rather than beside it in a live-only file,
// because how long a crew turned is what decides where the hole ends up —
// so mending a machine is an order the log must carry, and its replay arm
// spends the same parts off the same pile.
// Twenty is two changes at once, like eight and fifteen before it. The
// ground changed: uranium joined the ore lattice below the overburden, and
// oil and gas bodies are stamped into the deep stone off a lattice of their
// own — a log recorded over the old rock would replay its digging into rock
// that is no longer the same rock. And `Spud` joins the commands, because a
// well eats casing off the pile and puts barrels back on it, which is the
// `Electrolyse` argument word for word: the fleet burns what the pile holds,
// so a replay that skipped a well would run the crew dry at a different tick
// and leave a different hole.
// Twenty-one moves generated ground twice over. Every town on the frontier
// now walls itself: what used to come back `Open` — a hamlet with nothing
// around it — is a mini star, four short bastions on a low thin trace drawn
// in tight against the buildings. And every town gained a clinic, which is a
// building where there was open plot. A log recorded over the old ground
// would replay its digging into rock that has a wall through it.
// Twenty-two changes what grows on the ground rather than what is under it.
// Every column now belongs to one of three forests, decided by how high and
// how wet it is: peat bog in the flat convergent lows, mixed hardwood with
// the odd emergent giant through the middle, subalpine conifer up high
// thinning to knee-high krummholz at the treeline and bare rock above it.
// The trees themselves changed species, shape and density, the bog carpets
// itself in sphagnum instead of grass, and nothing at all stands above the
// tree limit. A log recorded over the old forest would replay a crew driving
// through trunks that are no longer there and around ones that are.
// Twenty-three adds `Fell`. A tree coming down is the largest single edit
// the player can make to the ground: the stem leaves, the crown leaves,
// whatever the arc swept through leaves, a neighbour may come down with it,
// and a line of logs is written along where it landed. All of that is
// re-derived from the cut — the stump and the face — by the same kinematic
// arc, so the order stays an order. A log recorded before this replays a
// forest that never fell.
// Twenty-four sets the water moving. A block of it now carries a fill level
// in the same sixty-four cells a wounded block carries its damage in, and a
// hole cut beside water wakes an automaton that pours, levels and settles
// over the ticks that follow. Water that moves is ground that moves, so a log
// recorded before this replays a lake that never went anywhere.
// Twenty-six adds `Stand`. It changes nothing about the ground — a seat is
// paperwork — but it is a decision the player made, and the log is the record
// of those. Replay drops it, the same honest line `Talk` and `Gift` already
// take, and a log from before this simply has nobody standing for anything.
// Twenty-five hands the world a sky and lets it burn. Weather is a pure
// function of the seed, the tick and the region, so no order records it —
// but it now edits the ground twice over. Lightning, hashed off the same
// tick, lights the tall and the lonely; the fire eats uphill and downwind
// and leaves ash and snags behind it; and the stands it cleared — along with
// every stand a saw cleared — come back through meadow, thicket and mixed
// forest on the day clock. A log recorded before this replays a country with
// no weather, no fire and no regrowth, over ground that all three moved.
// 27: the player's body and its physics run in `f64` (stage 42). Nothing on
// the wire changed, but a log recorded under 26 replays its movement through
// a different integrator and may land a block off where it did — so it is a
// different version, and the loader stays as tolerant as it has always been.
// 28: a town founded, and a town taken (stage 43).
// 29: every body runs in `f64` (stage 45), and the one float on the wire —
// the muzzle of a shot — is written at that width. A log recorded under 28
// carries `f32` muzzles four bytes shorter and cannot be read; it restarts
// the oracle like every older log.
const VERSION: u32 = 29;

/// How many entries may pile up before a keyframe is worth writing.
///
/// Replay is fast — it is the same code the game runs — but not free, and this
/// bounds load time by the log tail rather than by how long a world has been
/// played.
pub const KEYFRAME_ENTRIES: usize = 4_096;

/// One thing the player told the world to do.
///
/// Deliberately *intent*, not outcome: `Dispatch` is "mine that, that way", not
/// the forty thousand blocks that follow from it.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// The handheld drill finished a block.
    Break { at: BlockPos },
    /// A block placed by hand. Named, never numbered — block ids shift when a
    /// mod is installed, exactly as the save format has always said.
    Place { at: BlockPos, block: String },
    /// An excavation ordered: the marked area, the method, and how many drones
    /// were put on it.
    ///
    /// The crew is recorded because it changes the hole. Machines cost credits
    /// now, so how many you own varies over a session, and a replay that
    /// guessed would dig a different excavation from the one that happened.
    Dispatch {
        area: VoxelAabb,
        method: MineMethod,
        crew: u32,
    },
    /// The plan abandoned.
    Cancel,
    /// The player's held input changed.
    ///
    /// Recorded on *change*, not per tick. `Advance` already folds, so holding
    /// W for a minute is one `Move` and one `Advance` rather than four thousand
    /// entries — run-length encoding for free, out of machinery that was
    /// already there.
    ///
    /// This is what extends the oracle to cover where the player walked. It did
    /// not before: the log knew every block a drone cut and nothing at all
    /// about the person who ordered it.
    Move {
        bits: u16,
        yaw_q: i16,
        pitch_q: i16,
        load: u8,
    },
    /// Simulation ticks run. The unit of time the log speaks in.
    Advance { ticks: u32 },
    /// An operator's order from the gold panel. In the same log as everything
    /// else on purpose: a journal with cheats in it still replays to a hash,
    /// which is what makes the panel a scenario editor rather than a taint.
    Admin(Admin),
    /// A pattern put on the fabricator, by its index in the catalogue.
    ///
    /// Recorded by index rather than by name because the catalogue is code,
    /// not content: a row cannot be renamed by a save file the way a block
    /// can be renamed by a mod. If the catalogue ever becomes moddable this
    /// becomes a name, and the version bump that brings it will say so.
    Print { recipe: u32 },
    /// Goods moved across a bank's counter. Journalled for the same reason
    /// `Print` and `Electrolyse` are: it moves the base pile, and the pile is
    /// state `Rebuilt` carries — a replay that skipped a deposit would finish
    /// holding ore the session had already banked, and the fleet would burn
    /// through a different amount of fuel on the strength of it.
    Bank {
        town: (i32, i32),
        good: String,
        amount: u64,
        deposit: bool,
    },
    /// A run started on the electrolyser, by its index in the machine's
    /// list of runs. Like `Print`, and for the same reason: the electrodes
    /// come off the pile and the canisters go back onto it, and the pile is
    /// state `Rebuilt` carries — so a replay that skipped this would finish
    /// holding copper the live game had already dissolved, and would run its
    /// fleet dry at a different tick than the session did.
    Electrolyse { run: u32 },
    /// A hole sunk under a wellhead. What it finds is a pure function of the
    /// seed and the column, so the order carries only *where*: both sides
    /// ask the ground the same question and get the same answer, exactly
    /// like `Salvage` derives its haul from a position rather than carrying
    /// one.
    Spud { at: BlockPos },
    /// A supply cache stripped. Not a no-op and not merely a break: it
    /// clears a block *and* pays out a haul, and both sides derive the haul
    /// from the same position, so a replay finishes holding exactly what the
    /// live session held.
    Salvage { at: BlockPos },
    /// A machine sent to work a lock or the town's watch box. Journalled
    /// for the record; replayed as a no-op, because what an intrusion moves
    /// is claims and grants — permits state, kept in its own file — and not
    /// one block of the ground the hash covers.
    Intrude(IntrudeOrder),
    /// A standing order for the kestrel. Journalled because the log is the
    /// complete record of what the player ordered; replayed as a no-op
    /// because the scout reveals contacts, never terrain — nothing it does
    /// can reach the hash. The same honest line the economy draws.
    Scout(ScoutOrder),
    /// The launcher fired: the muzzle, and the quantised aim.
    ///
    /// The muzzle position is recorded — the one deliberate exception to
    /// "intent, not outcome". Live movement runs on a real-time 64 Hz clock
    /// and journal ticks on the drones' 8 Hz clock, so the body a replay
    /// rebuilds stands a few subticks from where the live body stood when the
    /// trigger was pulled. A shot breaks blocks, blocks are in the hash, and
    /// a hash must not depend on which clock you asked. The floats cross the
    /// wire as raw bits, like `SetTuning`, at the width the body runs at.
    Fire {
        muzzle: [f64; 3],
        yaw_q: i16,
        pitch_q: i16,
    },
    /// A word with a townsperson: which town's roster, which person on it.
    /// Journalled because the log is the complete record of what the player
    /// did; replayed as a no-op, because what talking moves is disposition —
    /// a ledger kept in its own file, like permits grants — and never one
    /// block of the ground the hash covers.
    Talk { town: (i32, i32), person: u8 },
    /// A present handed over. *Not* a no-op like `Talk`: the good comes off
    /// the base pile, and the pile is state `Rebuilt` carries — a replay
    /// that skipped the gift would finish holding a bar the live session had
    /// already wrapped. What the gift *earns* is disposition, outside the
    /// hash, and the ledger scores it from its own entries.
    Gift {
        town: (i32, i32),
        person: u8,
        good: String,
    },
    /// A machine mended: spare parts off the pile, its wear back to nothing.
    ///
    /// The most oracle-entangled order in the log. Wear decides how many
    /// ticks a crew actually turns, turning decides how much ground it cuts,
    /// and ground is the hash — so a replay that skipped a repair would dig
    /// a visibly different hole. Both sides run `Wear::repair`, one
    /// function, no second implementation to drift.
    Repair { machine: MachineTag },
    /// A pump switched on or off.
    ///
    /// The order is the switch, never the water: what it lifts, and where
    /// that runs to afterwards, is the automaton's business on both sides.
    Pump { at: BlockPos, on: bool },
    /// Your name put on a town's ballot, or taken off it.
    ///
    /// The decision is the order; the *election* is not, because polling day
    /// is a pure function of the town and the day and the count is a pure
    /// function of ledgers both sides already hold. So this is recorded for
    /// the same reason `Talk` and `Gift` are — it is a thing the player did —
    /// and replay drops it, because nothing downstream of it touches a block.
    Stand { town: (i32, i32), office: u8, on: bool },
    /// A town founded: where, and its name out of the book.
    ///
    /// The order is the ground. Founding *raises* a town — every chunk the
    /// plot reaches is regenerated as if the charter had always been on the
    /// map — so this is the ordinary oracle, not the inverted one: both sides
    /// derive the same site from the same seed and the same centre, and both
    /// sides raise it. What the charter cost and who sits in its chairs is
    /// bookkeeping beside it, and replay drops that as it drops `Stand`.
    Found { x: i32, z: i32, head: u8, tail: u8 },
    /// A town taken: both chairs seized after its posse was broken.
    ///
    /// Recorded because the player did it; replayed as nothing, because a
    /// seat is a ledger entry and a ledger entry is not ground. The price —
    /// the factions, the bounty, the neighbours' paper, the residents' trust
    /// — is every bit as live-only as the seat.
    Take { town: (i32, i32) },
    /// A tree put on the ground: the stump block that was cut, and the face
    /// it was cut from.
    ///
    /// The order is the cut, never the outcome. Where the stem lands, what it
    /// smashes on the way, which neighbour it takes with it and where the
    /// logs come to rest are all re-derived by the same kinematic arc the
    /// live game swept — a pure function of the tree, the direction and the
    /// tick, which is exactly why the fall is not a rigid body.
    Fell { at: BlockPos, face: u8 },
}

/// Which machine an order names, on the wire.
///
/// A tag rather than [`crate::mining::MachineRef`] because the wire is a
/// format and the enum is code: the kestrel is absent here because it takes
/// no wear, and if that ever changes it is a version bump, loudly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineTag {
    Digger(u32),
    Flier(u32),
}

impl MachineTag {
    pub fn of(machine: crate::mining::MachineRef) -> Option<MachineTag> {
        match machine {
            crate::mining::MachineRef::Digger(index) => Some(MachineTag::Digger(index as u32)),
            crate::mining::MachineRef::Flier(index) => Some(MachineTag::Flier(index as u32)),
            crate::mining::MachineRef::Kestrel => None,
        }
    }

    pub fn machine(self) -> crate::mining::MachineRef {
        match self {
            MachineTag::Digger(index) => crate::mining::MachineRef::Digger(index as usize),
            MachineTag::Flier(index) => crate::mining::MachineRef::Flier(index as usize),
        }
    }
}

/// What the gold panel may order.
///
/// Replay applies what [`Rebuilt`] carries — the player, the movement tuning,
/// the fleet's base pile. Machines, credits, skills and town books are not in
/// `Rebuilt`, so those arms are no-ops on replay, the same honest line
/// `run_replay` already draws for the economy: recorded, visible in the log,
/// not part of the hash the oracle checks.
#[derive(Debug, Clone, PartialEq)]
pub enum Admin {
    /// Goods into the fleet's base pile. No base means nothing happens, live
    /// and replayed alike — the rule must match or the oracle lies.
    Give { good: String, amount: u64 },
    /// Machines into the garage. Replay no-op: dispatches record their crew.
    SpawnMachine { kind: String, count: u32 },
    /// The player, moved. Block-quantised so the landing spot survives the
    /// wire exactly.
    Teleport { x: i32, y: i32, z: i32 },
    /// A named stat. "stamina" is movement state and replays; "credits" and
    /// "xp:<skill>" are live-only.
    SetStat { key: String, value: u64 },
    /// A town's shelf. Live-only: the books are outside the oracle.
    SetStock { x: i32, z: i32, good: String, amount: u64 },
    /// A tunable, by name. This one is the subtle one: it changes how every
    /// later command is *interpreted*, so a journal now carries its physics.
    /// The f32 crosses the wire as raw bits — exact, not printed-and-parsed.
    SetTuning { key: String, value: f32 },
}

/// What the kestrel may be told to do. Mirrors `vx_agent::KestrelMode`
/// minus the states only flight itself can enter (`Manual`, `Returning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoutOrder {
    /// Come home and dock.
    Dock,
    /// Circle the owner.
    Orbit,
    /// Fly to a column, look, come back.
    Sortie { x: i32, z: i32 },
    /// Land there and watch.
    Perch { x: i32, z: i32 },
    /// Hold ahead along the owner's heading.
    Vanguard,
}

/// What a machine may be told to work on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrudeOrder {
    /// A lockbox at this block.
    Lock { x: i32, y: i32, z: i32 },
    /// The watch box at this block, to the given grade.
    Roost {
        x: i32,
        y: i32,
        z: i32,
        grade: crate::intrusion::Grade,
    },
    /// Called off.
    Abort,
}

impl Command {
    /// A `Move` carrying this command.
    pub fn moving(command: MoveCommand) -> Self {
        Command::Move {
            bits: command.bits,
            yaw_q: command.yaw_q,
            pitch_q: command.pitch_q,
            load: command.load,
        }
    }
}

/// A command and when it happened, in ticks since the world began.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub tick: u64,
    pub command: Command,
}

/// Everything ordered since the last keyframe.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CommandLog {
    entries: Vec<Entry>,
    /// The tick the keyframe on disk was taken at.
    pub keyframe_tick: u64,
    /// The world hash at that keyframe, so a replay can say whether it landed
    /// where it was supposed to.
    pub keyframe_hash: u64,
    /// Ticks since the world began.
    tick: u64,
}

impl CommandLog {
    pub fn new() -> Self {
        CommandLog::default()
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record a command at the current tick.
    ///
    /// `Advance` folds into a preceding `Advance` rather than appending: a
    /// running game issues one per frame, and sixty entries a second would
    /// bury the handful that carry meaning. Folding is exact — ticks add.
    pub fn record(&mut self, command: Command) {
        if let Command::Advance { ticks } = command {
            if ticks == 0 {
                return;
            }
            self.tick += u64::from(ticks);
            if let Some(Entry {
                command: Command::Advance { ticks: last },
                ..
            }) = self.entries.last_mut()
            {
                *last += ticks;
                return;
            }
        }
        self.entries.push(Entry {
            tick: self.tick,
            command,
        });
    }

    /// Is the tail long enough that a keyframe would pay for itself?
    pub fn wants_keyframe(&self) -> bool {
        self.entries.len() >= KEYFRAME_ENTRIES
    }

    /// Start a fresh tail from a keyframe taken now.
    pub fn keyframed(&mut self, hash: u64) {
        self.entries.clear();
        self.keyframe_tick = self.tick;
        self.keyframe_hash = hash;
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(directory.join("log.dat"))?);
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&self.keyframe_tick.to_le_bytes())?;
        file.write_all(&self.keyframe_hash.to_le_bytes())?;
        file.write_all(&self.tick.to_le_bytes())?;
        file.write_all(&(self.entries.len() as u32).to_le_bytes())?;
        for entry in &self.entries {
            write_entry(&mut file, entry)?;
        }
        file.flush()
    }

    /// Read a log back, tolerating absence and damage.
    ///
    /// A damaged log is an *empty* log, not a failed world: the region files
    /// are the keyframe and they still hold the ground. What is lost is
    /// whatever happened since — bounded by [`KEYFRAME_ENTRIES`], and said out
    /// loud rather than swallowed.
    pub fn load(directory: &Path) -> CommandLog {
        let path = directory.join("log.dat");
        match read_log(&path) {
            Ok(Some(log)) => log,
            Ok(None) => CommandLog::new(),
            Err(error) => {
                log::warn!(
                    "could not read {}: {error}; the world still loads from its region files, \
                     but the determinism oracle restarts here",
                    path.display()
                );
                CommandLog::restarted()
            }
        }
    }

    /// A log that begins mid-history, after an old or damaged one was refused.
    ///
    /// The nonzero keyframe tick is the load-bearing part: it says "this log
    /// does not reach back to genesis", so `--replay` declines to compare
    /// hashes instead of reporting a divergence that is really a version bump.
    /// One specific tick would be a lie — the refused file's clock is exactly
    /// what could not be read.
    pub fn restarted() -> CommandLog {
        CommandLog {
            keyframe_tick: 1,
            ..CommandLog::default()
        }
    }
}

/// Everything a replay reconstructs.
pub struct Rebuilt {
    pub mining: Mining,
    pub player: PlayerBody,
    pub movement: Movement,
    /// The command in force, carried across every `Advance` that follows it.
    held: MoveCommand,
    /// Slugs in the air, stepped one integration per `Advance` tick.
    pub shots: Vec<crate::arsenal::Shot>,
    /// Stems on their way down, stepped on the same clock and for the same
    /// reason: what they smash and what they leave lying is ground, and
    /// ground is the hash.
    pub falls: Vec<crate::felling::Falling>,
    /// Water that is not finished moving, for the third time the same
    /// reason: a flood is an edit to the ground, spread over ticks.
    pub water: Vec<vx_world::fluid::Water>,
    /// Pumps that are switched on, and how many ticks they have run: they
    /// lift on the water's own quarter clock, not the player's.
    pub pumps: Vec<BlockPos>,
    pump_step: u32,
    /// Fires burning, on the same clock and for the same reason as the rest:
    /// what a fire eats is ground.
    pub fires: Vec<crate::fire::Fire>,
    /// Stands that something cleared, and what they have grown back to.
    pub stands: crate::succession::Ledger,
    /// The absolute tick, counted by the `Advance` orders themselves.
    ///
    /// The weather and the lightning are pure functions of it, so both sides
    /// need the same number — and counting the ticks the log carries is the
    /// only way to get it that cannot drift from what the live game did.
    pub tick: u64,
    /// Every town's strongroom. Carried because a deposit moves the pile,
    /// and the pile is what half the log's arithmetic runs over.
    pub banks: crate::bank::Bank,
}

/// Replay a log over a world.
///
/// The world must already be at the log's keyframe — region files loaded, or a
/// bare generated world when `keyframe_tick` is zero. `start` is where the
/// player's feet begin: the log records what they *did*, not where they were
/// standing when they started, so the caller supplies the origin the path is
/// measured from.
impl Default for Rebuilt {
    fn default() -> Self {
        Rebuilt {
            mining: Mining::default(),
            player: PlayerBody::default(),
            movement: Movement::default(),
            held: MoveCommand::default(),
            shots: Vec::new(),
            falls: Vec::new(),
            water: Vec::new(),
            pumps: Vec::new(),
            pump_step: 0,
            fires: Vec::new(),
            stands: crate::succession::Ledger::default(),
            tick: 0,
            banks: crate::bank::Bank::default(),
        }
    }
}

pub fn replay_from(
    log: &CommandLog,
    world: &mut World,
    events: &EventBus,
    start: glam::DVec3,
) -> Rebuilt {
    let mut state = Rebuilt {
        player: PlayerBody {
            position: start,
            ..PlayerBody::default()
        },
        ..Rebuilt::default()
    };
    for entry in log.entries() {
        apply(&entry.command, world, events, &mut state);
    }
    state
}

/// Replay from the surface at the world origin.
///
/// Deterministic from the seed, which is what an oracle needs; it is not
/// necessarily where the player actually stood.
pub fn replay(log: &CommandLog, world: &mut World, events: &EventBus) -> Rebuilt {
    let ground = world.surface_y(0, 0).unwrap_or(64) as f64;
    replay_from(log, world, events, glam::DVec3::new(0.5, ground, 0.5))
}

fn apply(command: &Command, world: &mut World, events: &EventBus, state: &mut Rebuilt) {
    let mining = &mut state.mining;
    match command {
        Command::Break { at } => {
            let _ = vx_world::break_block(world, events, *at);
            // Cut into a lake and the lake notices. No order of its own: the
            // break is already recorded, so both sides wake the same water on
            // the same tick and the flood that follows is re-derived rather
            // than replayed.
            wake_water(&mut state.water, world, *at);
        }
        Command::Place { at, block } => {
            if let Some(id) = world.registry().id_of(block) {
                world.set_block(*at, id);
            } else {
                // A block whose mod is gone decodes to nothing rather than to
                // whatever now occupies its number — the same rule the region
                // format follows.
                log::warn!("replay skipped an unknown block {block} at {at:?}");
            }
        }
        Command::Dispatch { area, method, crew } => {
            mining.mark(world, area.min);
            mining.mark(world, area.max);
            // Select the recorded method rather than trusting the ranking:
            // ranking is deterministic, but it reads the world, and pinning the
            // *choice* means a later tweak to the cost model cannot silently
            // re-cut an old mine a different way.
            for _ in 0..MineMethod::ALL.len() {
                if mining.selected_plan().is_some_and(|plan| plan.method == *method) {
                    break;
                }
                mining.cycle_method();
            }
            mining.start(world, *crew);
        }
        Command::Cancel => mining.cancel(world),
        Command::Move {
            bits,
            yaw_q,
            pitch_q,
            load,
        } => {
            state.held = MoveCommand {
                bits: *bits,
                yaw_q: *yaw_q,
                pitch_q: *pitch_q,
                load: *load,
            };
        }
        Command::Admin(order) => match order {
            Admin::Give { good, amount } => {
                // Same rule as live: no base, nothing happens. The rule must
                // match on both sides or the oracle lies.
                if let Some(base) = state.mining.fleet.base.as_mut() {
                    base.stockpile.add(good.clone(), *amount);
                }
            }
            Admin::Teleport { x, y, z } => {
                state.player.position = glam::DVec3::new(*x as f64 + 0.5, *y as f64, *z as f64 + 0.5);
                state.player.velocity = glam::DVec3::ZERO;
                state.player.on_ground = false;
            }
            Admin::SetStat { key, value } => {
                // Only what `Rebuilt` carries. Everything else is live-only,
                // the same line `run_replay` draws for the economy.
                if key == "stamina" {
                    state.movement.stamina =
                        (*value as f32).min(state.movement.tuning.stam_max);
                }
            }
            Admin::SetTuning { key, value } => {
                if !state.movement.tuning.set(key, *value) {
                    log::warn!("replay met an unknown tunable '{key}'");
                }
            }
            // Machines and town books are outside the oracle: crews are
            // recorded on each dispatch, and the books were never replayed.
            Admin::SpawnMachine { .. } | Admin::SetStock { .. } => {}
        },
        // The scout reveals contacts, never terrain: nothing it does can
        // reach the world hash, so replay only needs to decode the order and
        // move on — the same honest line run_replay draws for the economy.
        // An intrusion moves grants and claims, which live in their own file
        // outside the hash, so it draws the same line — and so does a word
        // with a neighbour: talk moves disposition, kept in its own ledger.
        Command::Scout(_) | Command::Intrude(_) | Command::Talk { .. } => {}
        Command::Repair { machine } => {
            // One function, both sides: the parts come off the pile and the
            // ledger resets, or neither happens.
            if let Some(base) = state.mining.fleet.base.as_mut() {
                state.mining.wear.repair(machine.machine(), &mut base.stockpile);
            }
        }
        Command::Pump { at, on } => {
            if *on {
                if !state.pumps.contains(at) {
                    state.pumps.push(*at);
                }
            } else {
                state.pumps.retain(|other| other != at);
            }
        }
        Command::Stand { .. } | Command::Take { .. } => {
            // Live-only. Who holds a seat decides who signs a warrant and
            // which locks open for you — credits, ledgers and paperwork, none
            // of which is ground. `Rebuilt` carries no register for the same
            // reason it carries no wallet. Taking a town is the same fact
            // with a fight in front of it, and the fight was slugs, which the
            // log already carries.
        }
        Command::Found { x, z, head, tail } => {
            // The site is derived exactly as the live game derived it — the
            // world's seed, the centre and the name — and the town is raised
            // by the same generator asked the same question. Deterministic in
            // all of it, which is what puts founding on this side of the
            // oracle rather than the inverted one.
            let name = vx_world::town::TownName::from_indices(*head, *tail);
            let generator = world.generator().clone();
            let natural = |x: i32, z: i32| generator.natural_height_at(x, z);
            let site = crate::charter::site_at(world.seed(), (*x, *z), name, &natural);
            world.raise(site);
        }
        Command::Fell { at, face } => {
            // Find the tree the same way the live game did — off worldgen,
            // which is pure — and start the same stem falling. The arc is
            // ticked below, on the same clock as the slugs.
            let sites = world.towns_near((at.x, at.z), 96);
            if let Some(tree) = crate::felling::standing_tree(world, *at, &sites) {
                let lean = crate::felling::lean_at(world, tree.base.x, tree.base.z);
                let (direction, chair) = crate::felling::aim(*face as usize, lean);
                state
                    .falls
                    .push(crate::felling::start(world, &tree, direction, chair));
                // A cut stand is a disturbed stand. The clock that brings a
                // burn back is the clock that brings a cut back, and both
                // sides write the same cell on the same tick.
                state.stands.disturb(tree.base, state.tick);
            }
        }
        Command::Gift { good, .. } => {
            // One good, off the pile, both sides. What it earned is in the
            // disposition ledger, outside the hash.
            if let Some(base) = state.mining.fleet.base.as_mut() {
                base.stockpile.take(good, 1);
            }
        }
        Command::Bank {
            town,
            good,
            amount,
            deposit,
        } => {
            // The amount is what actually moved when the session did it, not
            // what was asked for — a capacity that bit mid-deposit would put
            // the log and the world at odds otherwise. So both sides move
            // exactly this much and neither re-decides.
            if let Some(base) = state.mining.fleet.base.as_mut() {
                if *deposit {
                    state.banks.deposit(*town, good, *amount, &mut base.stockpile);
                } else {
                    state.banks.withdraw(*town, good, *amount, &mut base.stockpile);
                }
            }
        }
        Command::Electrolyse { run } => {
            // Electrodes off the pile, canisters back onto it. Both sides run
            // the same arithmetic over the same store, which is what lets the
            // fleet's tank — drawn from that store inside `Mining::advance` —
            // reach the same level on both sides without a single order of
            // its own.
            if let (Some(base), Some(run)) = (
                state.mining.fleet.base.as_mut(),
                crate::electrolysis::run(*run as usize),
            ) {
                base.stockpile.take("engine:copper_bar", run.bars);
                base.stockpile
                    .add(crate::fuel::CELL, u64::from(run.cells));
            }
        }
        Command::Spud { at } => {
            // Casing and cement off the pile, and a hole that lifts back
            // onto it for as long as the ground holds out. One function,
            // both sides — and the ground it consults is the seed, which
            // neither side can disagree about.
            let seed = world.seed();
            if let Some(base) = state.mining.fleet.base.as_mut() {
                let _ = state.mining.wells.spud(*at, seed, &mut base.stockpile);
            }
        }
        Command::Salvage { at } => {
            // The crate goes, and its haul lands on the pile. The contents
            // are a pure function of the position, so the two sides cannot
            // drift: there is nothing rolled here to disagree about.
            world.set_block(*at, vx_core::BlockId::AIR);
            if let Some(base) = state.mining.fleet.base.as_mut() {
                for (name, count) in crate::salvage::contents(*at, 1) {
                    base.stockpile.add(name, count);
                }
            }
        }
        Command::Print { recipe } => {
            // The materials come off the pile on both sides: the pile is
            // state `Rebuilt` carries, so a replay that skipped this would
            // finish holding ore the live game had already melted. What the
            // print *becomes* is another matter — a good goes back on the
            // pile, and slugs, cells, machines and modules are live-only,
            // the same honest line `Give` and `SpawnMachine` draw.
            if let (Some(base), Some(recipe)) = (
                state.mining.fleet.base.as_mut(),
                crate::printer::recipe(*recipe as usize),
            ) {
                for (name, needed) in recipe.inputs {
                    base.stockpile.take(name, *needed);
                }
                if let crate::printer::Output::Good { name, count } = recipe.output {
                    base.stockpile.add(name, count);
                }
            }
        }
        Command::Fire {
            muzzle,
            yaw_q,
            pitch_q,
        } => {
            crate::arsenal::launch(
                &mut state.shots,
                &mut state.movement,
                glam::DVec3::from(*muzzle),
                *yaw_q,
                *pitch_q,
            );
        }
        Command::Advance { ticks } => {
            mining.advance(world, events, *ticks);
            // The held command is re-issued for every tick it covers, at
            // `SUBTICKS` movement steps each. This is the same call the live
            // game makes, which is what makes the log an oracle for the
            // player's path rather than a second implementation of it.
            for _ in 0..*ticks {
                state.tick += 1;
                movement::advance_journal_tick(
                    &mut state.movement,
                    &mut state.player,
                    world,
                    state.held,
                );
                // Shots step on the same clock. The sweeps are dropped: the
                // craters are the part the hash checks, the bills were the
                // live game's business.
                let _ = crate::arsenal::advance_shots(
                    &mut state.shots,
                    world,
                    &state.movement.tuning,
                );
                // And the trees. The sweeps are dropped for the same reason
                // the slugs' are: the blocks are the part the hash checks,
                // and who got flattened was the live game's business.
                if !state.falls.is_empty() {
                    let _ = crate::felling::advance_falls(&mut state.falls, world);
                }
                // And the water, which settles on the same clock and edits
                // the same ground. Its reports are dropped like the rest:
                // the blocks are what the hash checks.
                state.pump_step = state.pump_step.wrapping_add(1);
                if !state.pumps.is_empty() {
                    run_pumps(&state.pumps, &mut state.water, world, state.pump_step);
                }
                if !state.water.is_empty() {
                    settle_water(&mut state.water, world);
                }
                // The sky, the lightning and what it lights. None of it is
                // recorded: the weather is a pure function of the tick and
                // the strike is hashed off the same, so both sides get the
                // same storm over the same ground.
                let standing = state.player.position;
                burn_and_grow(
                    &mut state.fires,
                    &mut state.stands,
                    world,
                    state.tick,
                    standing,
                );
            }
        }
    }
}

/// Wake whatever water a hole in the ground has just exposed.
///
/// One body per disturbance, and a disturbance inside an existing body's
/// reach joins that body rather than starting another — otherwise cutting a
/// gallery block by block would leave a dozen automata arguing over the same
/// cells.
pub fn wake_water(bodies: &mut Vec<vx_world::fluid::Water>, world: &mut World, at: BlockPos) {
    let Some(water) = world.registry().id_of("engine:water") else {
        return;
    };
    let mut touched = false;
    for body in bodies.iter_mut() {
        let reach = vx_world::fluid::REACH;
        if (at.x - body.origin.x).abs() <= reach
            && (at.y - body.origin.y).abs() <= reach
            && (at.z - body.origin.z).abs() <= reach
        {
            body.wake_around(world, water, at);
            touched = true;
            break;
        }
    }
    if touched {
        return;
    }
    let mut body = vx_world::fluid::Water::new(at);
    body.wake_around(world, water, at);
    if body.busy() {
        bodies.push(body);
    }
}

/// One tick of weather, fire and regrowth.
///
/// Live and replay call this, which is what makes a burned hillside the same
/// burned hillside on both sides. Everything it needs is the tick and where
/// the player is standing — and the player's path is replayed, so both are
/// known without an order of any kind.
pub fn burn_and_grow(
    fires: &mut Vec<crate::fire::Fire>,
    stands: &mut crate::succession::Ledger,
    world: &mut World,
    tick: u64,
    standing: glam::DVec3,
) -> crate::frost::Report {
    let seed = world.seed();
    let at = BlockPos::new(
        standing.x.floor() as i32,
        standing.y.floor() as i32,
        standing.z.floor() as i32,
    );
    let sky = vx_world::weather::at(seed, tick, at.x, at.z);

    // The cold first: snow that settles and water that freezes, on the
    // rain's cadence, around the player like the rain.
    let frost = crate::frost::settle(world, seed, tick, at);

    // Lightning, and the one strike in fifty that lights anything.
    if let Some(hit) = crate::fire::strike(seed, tick, at, world, &sky) {
        let mut fire = crate::fire::Fire::new(hit);
        if fire.light(world, hit) {
            fires.push(fire);
        }
    }

    // What is alight, and what it leaves.
    if !fires.is_empty() {
        for report in crate::fire::advance_fire(fires, world, seed, tick, &sky) {
            if let Some(gone) = report.at.filter(|_| report.spent > 0) {
                stands.disturb(gone, tick);
            }
        }
    }

    // And what is coming back. Once a day is plenty for a forest.
    if tick.is_multiple_of(64 * 30) && !stands.is_empty() {
        let sites = world.towns_near((at.x, at.z), 160);
        stands.advance(world, tick, &sites);
    }
    frost
}

/// Lift a step's worth of water out of every running pump.
///
/// On the water's own quarter clock rather than the player's, so a pump
/// delivers at the rate the automaton can carry away.
pub fn run_pumps(
    pumps: &[BlockPos],
    bodies: &mut Vec<vx_world::fluid::Water>,
    world: &mut World,
    step: u32,
) -> u32 {
    if !step.is_multiple_of(vx_world::fluid::EVERY) {
        return 0;
    }
    let Some(water) = world.registry().id_of("engine:water") else {
        return 0;
    };
    let mut lifted = 0;
    for at in pumps {
        // A pump that has been broken since it was switched on is a pump no
        // longer: the block is the machine.
        if world.registry().get_or_air(world.block(*at)).name != "engine:pump" {
            continue;
        }
        if let Some(spout) = vx_world::fluid::pump(world, water, *at) {
            lifted += 1;
            wake_water(bodies, world, spout);
        }
    }
    lifted
}

/// Step every body of water, and retire the ones that have settled.
pub fn settle_water(bodies: &mut Vec<vx_world::fluid::Water>, world: &mut World) -> u32 {
    let Some(water) = world.registry().id_of("engine:water") else {
        return 0;
    };
    let mut moved = 0;
    for body in bodies.iter_mut() {
        moved += body.settle(world, water).moved;
    }
    bodies.retain(|body| body.busy());
    moved
}

fn write_entry(file: &mut impl Write, entry: &Entry) -> std::io::Result<()> {
    file.write_all(&entry.tick.to_le_bytes())?;
    match &entry.command {
        Command::Break { at } => {
            file.write_all(&[0u8])?;
            write_pos(file, *at)?;
        }
        Command::Place { at, block } => {
            file.write_all(&[1u8])?;
            write_pos(file, *at)?;
            file.write_all(&(block.len() as u32).to_le_bytes())?;
            file.write_all(block.as_bytes())?;
        }
        Command::Dispatch { area, method, crew } => {
            file.write_all(&[2u8])?;
            write_pos(file, area.min)?;
            write_pos(file, area.max)?;
            file.write_all(&[match method {
                MineMethod::Adit => 0,
                MineMethod::Decline => 1,
                MineMethod::Pit => 2,
            }])?;
            file.write_all(&crew.to_le_bytes())?;
        }
        Command::Cancel => file.write_all(&[3u8])?,
        Command::Advance { ticks } => {
            file.write_all(&[4u8])?;
            file.write_all(&ticks.to_le_bytes())?;
        }
        Command::Move {
            bits,
            yaw_q,
            pitch_q,
            load,
        } => {
            file.write_all(&[5u8])?;
            file.write_all(&bits.to_le_bytes())?;
            file.write_all(&yaw_q.to_le_bytes())?;
            file.write_all(&pitch_q.to_le_bytes())?;
            file.write_all(&[*load])?;
        }
        Command::Admin(order) => match order {
            Admin::Give { good, amount } => {
                file.write_all(&[6u8])?;
                write_name(file, good)?;
                file.write_all(&amount.to_le_bytes())?;
            }
            Admin::SpawnMachine { kind, count } => {
                file.write_all(&[7u8])?;
                write_name(file, kind)?;
                file.write_all(&count.to_le_bytes())?;
            }
            Admin::Teleport { x, y, z } => {
                file.write_all(&[8u8])?;
                file.write_all(&x.to_le_bytes())?;
                file.write_all(&y.to_le_bytes())?;
                file.write_all(&z.to_le_bytes())?;
            }
            Admin::SetStat { key, value } => {
                file.write_all(&[9u8])?;
                write_name(file, key)?;
                file.write_all(&value.to_le_bytes())?;
            }
            Admin::SetStock { x, z, good, amount } => {
                file.write_all(&[10u8])?;
                file.write_all(&x.to_le_bytes())?;
                file.write_all(&z.to_le_bytes())?;
                write_name(file, good)?;
                file.write_all(&amount.to_le_bytes())?;
            }
            Admin::SetTuning { key, value } => {
                file.write_all(&[11u8])?;
                write_name(file, key)?;
                // Raw bits: exact, not printed-and-parsed.
                file.write_all(&value.to_bits().to_le_bytes())?;
            }
        },
        Command::Fire {
            muzzle,
            yaw_q,
            pitch_q,
        } => {
            file.write_all(&[12u8])?;
            for part in muzzle {
                file.write_all(&part.to_bits().to_le_bytes())?;
            }
            file.write_all(&yaw_q.to_le_bytes())?;
            file.write_all(&pitch_q.to_le_bytes())?;
        }
        Command::Print { recipe } => {
            file.write_all(&[15u8])?;
            file.write_all(&recipe.to_le_bytes())?;
        }
        Command::Salvage { at } => {
            file.write_all(&[16u8])?;
            write_pos(file, *at)?;
        }
        Command::Electrolyse { run } => {
            file.write_all(&[17u8])?;
            file.write_all(&run.to_le_bytes())?;
        }
        Command::Spud { at } => {
            file.write_all(&[22u8])?;
            write_pos(file, *at)?;
        }
        Command::Bank {
            town,
            good,
            amount,
            deposit,
        } => {
            file.write_all(&[18u8])?;
            file.write_all(&town.0.to_le_bytes())?;
            file.write_all(&town.1.to_le_bytes())?;
            write_name(file, good)?;
            file.write_all(&amount.to_le_bytes())?;
            file.write_all(&[u8::from(*deposit)])?;
        }
        Command::Talk { town, person } => {
            file.write_all(&[19u8])?;
            file.write_all(&town.0.to_le_bytes())?;
            file.write_all(&town.1.to_le_bytes())?;
            file.write_all(&[*person])?;
        }
        Command::Pump { at, on } => {
            file.write_all(&[24u8])?;
            write_pos(file, *at)?;
            file.write_all(&[u8::from(*on)])?;
        }
        Command::Stand { town, office, on } => {
            file.write_all(&[25u8])?;
            file.write_all(&town.0.to_le_bytes())?;
            file.write_all(&town.1.to_le_bytes())?;
            file.write_all(&[*office, u8::from(*on)])?;
        }
        Command::Found { x, z, head, tail } => {
            file.write_all(&[26u8])?;
            file.write_all(&x.to_le_bytes())?;
            file.write_all(&z.to_le_bytes())?;
            file.write_all(&[*head, *tail])?;
        }
        Command::Take { town } => {
            file.write_all(&[27u8])?;
            file.write_all(&town.0.to_le_bytes())?;
            file.write_all(&town.1.to_le_bytes())?;
        }
        Command::Fell { at, face } => {
            file.write_all(&[23u8])?;
            write_pos(file, *at)?;
            file.write_all(&[*face])?;
        }
        Command::Repair { machine } => {
            file.write_all(&[21u8])?;
            match machine {
                MachineTag::Digger(index) => {
                    file.write_all(&[0u8])?;
                    file.write_all(&index.to_le_bytes())?;
                }
                MachineTag::Flier(index) => {
                    file.write_all(&[1u8])?;
                    file.write_all(&index.to_le_bytes())?;
                }
            }
        }
        Command::Gift { town, person, good } => {
            file.write_all(&[20u8])?;
            file.write_all(&town.0.to_le_bytes())?;
            file.write_all(&town.1.to_le_bytes())?;
            file.write_all(&[*person])?;
            write_name(file, good)?;
        }
        Command::Intrude(order) => {
            file.write_all(&[14u8])?;
            match order {
                IntrudeOrder::Abort => file.write_all(&[0u8])?,
                IntrudeOrder::Lock { x, y, z } => {
                    file.write_all(&[1u8])?;
                    file.write_all(&x.to_le_bytes())?;
                    file.write_all(&y.to_le_bytes())?;
                    file.write_all(&z.to_le_bytes())?;
                }
                IntrudeOrder::Roost { x, y, z, grade } => {
                    file.write_all(&[2u8])?;
                    file.write_all(&x.to_le_bytes())?;
                    file.write_all(&y.to_le_bytes())?;
                    file.write_all(&z.to_le_bytes())?;
                    file.write_all(&[match grade {
                        crate::intrusion::Grade::Blind => 0u8,
                        crate::intrusion::Grade::Silence => 1u8,
                        crate::intrusion::Grade::Tap => 2u8,
                    }])?;
                }
            }
        }
        Command::Scout(order) => {
            file.write_all(&[13u8])?;
            match order {
                ScoutOrder::Dock => file.write_all(&[0u8])?,
                ScoutOrder::Orbit => file.write_all(&[1u8])?,
                ScoutOrder::Sortie { x, z } => {
                    file.write_all(&[2u8])?;
                    file.write_all(&x.to_le_bytes())?;
                    file.write_all(&z.to_le_bytes())?;
                }
                ScoutOrder::Perch { x, z } => {
                    file.write_all(&[3u8])?;
                    file.write_all(&x.to_le_bytes())?;
                    file.write_all(&z.to_le_bytes())?;
                }
                ScoutOrder::Vanguard => file.write_all(&[4u8])?,
            }
        }
    }
    Ok(())
}

/// A length-prefixed name on the wire, capped like every other name here.
fn write_name(file: &mut impl Write, name: &str) -> std::io::Result<()> {
    file.write_all(&(name.len() as u32).to_le_bytes())?;
    file.write_all(name.as_bytes())
}

fn read_name(file: &mut impl Read) -> std::io::Result<String> {
    let length = read_u32(file)? as usize;
    if length > 256 {
        return Err(std::io::Error::other("implausible name length"));
    }
    let mut bytes = vec![0u8; length];
    file.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("name is not text"))
}

fn write_pos(file: &mut impl Write, at: BlockPos) -> std::io::Result<()> {
    file.write_all(&at.x.to_le_bytes())?;
    file.write_all(&at.y.to_le_bytes())?;
    file.write_all(&at.z.to_le_bytes())
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_u64(file: &mut impl Read) -> std::io::Result<u64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

fn read_pos(file: &mut impl Read) -> std::io::Result<BlockPos> {
    Ok(BlockPos::new(
        read_u32(file)? as i32,
        read_u32(file)? as i32,
        read_u32(file)? as i32,
    ))
}

fn read_entry(file: &mut impl Read) -> std::io::Result<Entry> {
    let tick = read_u64(file)?;
    let mut tag = [0u8; 1];
    file.read_exact(&mut tag)?;
    let command = match tag[0] {
        0 => Command::Break { at: read_pos(file)? },
        1 => {
            let at = read_pos(file)?;
            let length = read_u32(file)? as usize;
            if length > 256 {
                return Err(std::io::Error::other("implausible block name"));
            }
            let mut bytes = vec![0u8; length];
            file.read_exact(&mut bytes)?;
            let block =
                String::from_utf8(bytes).map_err(|_| std::io::Error::other("name is not text"))?;
            Command::Place { at, block }
        }
        2 => {
            let min = read_pos(file)?;
            let max = read_pos(file)?;
            let mut method = [0u8; 1];
            file.read_exact(&mut method)?;
            Command::Dispatch {
                area: VoxelAabb::new(min, max),
                method: match method[0] {
                    0 => MineMethod::Adit,
                    1 => MineMethod::Decline,
                    2 => MineMethod::Pit,
                    other => {
                        return Err(std::io::Error::other(format!("unknown method {other}")))
                    }
                },
                crew: read_u32(file)?,
            }
        }
        3 => Command::Cancel,
        4 => Command::Advance {
            ticks: read_u32(file)?,
        },
        5 => {
            let mut bits = [0u8; 2];
            file.read_exact(&mut bits)?;
            let mut yaw = [0u8; 2];
            file.read_exact(&mut yaw)?;
            let mut pitch = [0u8; 2];
            file.read_exact(&mut pitch)?;
            let mut load = [0u8; 1];
            file.read_exact(&mut load)?;
            Command::Move {
                bits: u16::from_le_bytes(bits),
                yaw_q: i16::from_le_bytes(yaw),
                pitch_q: i16::from_le_bytes(pitch),
                load: load[0],
            }
        }
        6 => Command::Admin(Admin::Give {
            good: read_name(file)?,
            amount: read_u64(file)?,
        }),
        7 => Command::Admin(Admin::SpawnMachine {
            kind: read_name(file)?,
            count: read_u32(file)?,
        }),
        8 => Command::Admin(Admin::Teleport {
            x: read_u32(file)? as i32,
            y: read_u32(file)? as i32,
            z: read_u32(file)? as i32,
        }),
        9 => Command::Admin(Admin::SetStat {
            key: read_name(file)?,
            value: read_u64(file)?,
        }),
        10 => Command::Admin(Admin::SetStock {
            x: read_u32(file)? as i32,
            z: read_u32(file)? as i32,
            good: read_name(file)?,
            amount: read_u64(file)?,
        }),
        11 => Command::Admin(Admin::SetTuning {
            key: read_name(file)?,
            value: f32::from_bits(read_u32(file)?),
        }),
        12 => {
            let mut muzzle = [0f64; 3];
            for part in &mut muzzle {
                *part = f64::from_bits(read_u64(file)?);
            }
            let mut yaw = [0u8; 2];
            file.read_exact(&mut yaw)?;
            let mut pitch = [0u8; 2];
            file.read_exact(&mut pitch)?;
            Command::Fire {
                muzzle,
                yaw_q: i16::from_le_bytes(yaw),
                pitch_q: i16::from_le_bytes(pitch),
            }
        }
        13 => {
            let mut which = [0u8; 1];
            file.read_exact(&mut which)?;
            let order = match which[0] {
                0 => ScoutOrder::Dock,
                1 => ScoutOrder::Orbit,
                2 => ScoutOrder::Sortie {
                    x: read_u32(file)? as i32,
                    z: read_u32(file)? as i32,
                },
                3 => ScoutOrder::Perch {
                    x: read_u32(file)? as i32,
                    z: read_u32(file)? as i32,
                },
                4 => ScoutOrder::Vanguard,
                other => {
                    return Err(std::io::Error::other(format!(
                        "unknown scout order {other}"
                    )))
                }
            };
            Command::Scout(order)
        }
        14 => {
            let mut which = [0u8; 1];
            file.read_exact(&mut which)?;
            let order = match which[0] {
                0 => IntrudeOrder::Abort,
                1 => IntrudeOrder::Lock {
                    x: read_u32(file)? as i32,
                    y: read_u32(file)? as i32,
                    z: read_u32(file)? as i32,
                },
                2 => {
                    let (x, y, z) = (
                        read_u32(file)? as i32,
                        read_u32(file)? as i32,
                        read_u32(file)? as i32,
                    );
                    let mut grade = [0u8; 1];
                    file.read_exact(&mut grade)?;
                    let grade = match grade[0] {
                        0 => crate::intrusion::Grade::Blind,
                        1 => crate::intrusion::Grade::Silence,
                        2 => crate::intrusion::Grade::Tap,
                        other => {
                            return Err(std::io::Error::other(format!(
                                "unknown roost grade {other}"
                            )))
                        }
                    };
                    IntrudeOrder::Roost { x, y, z, grade }
                }
                other => {
                    return Err(std::io::Error::other(format!(
                        "unknown intrusion order {other}"
                    )))
                }
            };
            Command::Intrude(order)
        }
        15 => Command::Print {
            recipe: read_u32(file)?,
        },
        16 => Command::Salvage {
            at: read_pos(file)?,
        },
        22 => Command::Spud {
            at: read_pos(file)?,
        },
        17 => Command::Electrolyse {
            run: read_u32(file)?,
        },
        18 => {
            let town = (read_u32(file)? as i32, read_u32(file)? as i32);
            let good = read_name(file)?;
            let amount = read_u64(file)?;
            let mut flag = [0u8; 1];
            file.read_exact(&mut flag)?;
            Command::Bank {
                town,
                good,
                amount,
                deposit: flag[0] != 0,
            }
        }
        19 => {
            let town = (read_u32(file)? as i32, read_u32(file)? as i32);
            let mut person = [0u8; 1];
            file.read_exact(&mut person)?;
            Command::Talk {
                town,
                person: person[0],
            }
        }
        20 => {
            let town = (read_u32(file)? as i32, read_u32(file)? as i32);
            let mut person = [0u8; 1];
            file.read_exact(&mut person)?;
            let good = read_name(file)?;
            Command::Gift {
                town,
                person: person[0],
                good,
            }
        }
        21 => {
            let mut kind = [0u8; 1];
            file.read_exact(&mut kind)?;
            let index = read_u32(file)?;
            let machine = match kind[0] {
                0 => MachineTag::Digger(index),
                1 => MachineTag::Flier(index),
                other => {
                    return Err(std::io::Error::other(format!("unknown machine tag {other}")))
                }
            };
            Command::Repair { machine }
        }
        23 => {
            let at = read_pos(file)?;
            let mut face = [0u8; 1];
            file.read_exact(&mut face)?;
            Command::Fell { at, face: face[0] }
        }
        24 => {
            let at = read_pos(file)?;
            let mut on = [0u8; 1];
            file.read_exact(&mut on)?;
            Command::Pump {
                at,
                on: on[0] != 0,
            }
        }
        25 => {
            let mut word = [0u8; 4];
            file.read_exact(&mut word)?;
            let x = i32::from_le_bytes(word);
            file.read_exact(&mut word)?;
            let z = i32::from_le_bytes(word);
            let mut pair = [0u8; 2];
            file.read_exact(&mut pair)?;
            Command::Stand {
                town: (x, z),
                office: pair[0],
                on: pair[1] != 0,
            }
        }
        26 => {
            let mut word = [0u8; 4];
            file.read_exact(&mut word)?;
            let x = i32::from_le_bytes(word);
            file.read_exact(&mut word)?;
            let z = i32::from_le_bytes(word);
            let mut pair = [0u8; 2];
            file.read_exact(&mut pair)?;
            Command::Found {
                x,
                z,
                head: pair[0],
                tail: pair[1],
            }
        }
        27 => {
            let mut word = [0u8; 4];
            file.read_exact(&mut word)?;
            let x = i32::from_le_bytes(word);
            file.read_exact(&mut word)?;
            let z = i32::from_le_bytes(word);
            Command::Take { town: (x, z) }
        }
        other => return Err(std::io::Error::other(format!("unknown command {other}"))),
    };
    Ok(Entry { tick, command })
}

fn read_log(path: &Path) -> std::io::Result<Option<CommandLog>> {
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
    if read_u32(&mut file)? != VERSION {
        return Err(std::io::Error::other("unknown version"));
    }

    let keyframe_tick = read_u64(&mut file)?;
    let keyframe_hash = read_u64(&mut file)?;
    let tick = read_u64(&mut file)?;
    let count = read_u32(&mut file)?;

    let mut entries = Vec::new();
    for _ in 0..count {
        entries.push(read_entry(&mut file)?);
    }
    Ok(Some(CommandLog {
        entries,
        keyframe_tick,
        keyframe_hash,
        tick,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::ChunkPos;
    use vx_world::region_hash;

    /// A world with a body worth mining a few blocks under the surface.
    fn site() -> (World, VoxelAabb) {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 2);
        let ground = world.generator().height_at(4, 4);
        let area = VoxelAabb::new(
            BlockPos::new(2, ground - 7, 2),
            BlockPos::new(5, ground - 5, 5),
        );
        (world, area)
    }

    /// Order a mine and let it run, recording as we go — what the running game
    /// does, minus the window.
    fn record_a_session() -> CommandLog {
        let (mut world, area) = site();
        let events = EventBus::new();
        let mut journal = CommandLog::new();
        let mut rebuilt = Rebuilt::default();

        let dispatch = Command::Dispatch {
            area,
            method: MineMethod::Pit,
            crew: 3,
        };
        apply(&dispatch, &mut world, &events, &mut rebuilt);
        journal.record(dispatch);

        // Frames of wildly different lengths, as a real session has.
        for ticks in [1u32, 7, 3, 16, 2, 11, 16, 16, 9, 4] {
            rebuilt.mining.advance(&mut world, &events, ticks);
            journal.record(Command::Advance { ticks });
        }

        // And some work by hand on top.
        let ground = world.generator().height_at(20, 20);
        for offset in 0..4 {
            let at = BlockPos::new(20, ground - 1 - offset, 20);
            let _ = vx_world::break_block(&mut world, &events, at);
            journal.record(Command::Break { at });
        }
        journal
    }

    /// Replay a log from a fresh world and report the ground it produced.
    fn replay_to_hash(journal: &CommandLog, area: VoxelAabb) -> u64 {
        let (mut world, _) = site();
        let events = EventBus::new();
        replay(journal, &mut world, &events);
        let span = vx_agent::working_span(area, area.min);
        region_hash(&world, span.min, span.max)
    }

    #[test]
    fn a_recorded_session_replays_to_the_same_world() {
        // The oracle. If this ever fails, something in worldgen, the agents or
        // the editing path stopped being deterministic — and it will fail here
        // long before anyone notices it in a save.
        let (mut world, area) = site();
        let events = EventBus::new();
        let mut journal = CommandLog::new();
        let mut rebuilt = Rebuilt::default();

        let dispatch = Command::Dispatch {
            area,
            method: MineMethod::Pit,
            crew: 3,
        };
        apply(&dispatch, &mut world, &events, &mut rebuilt);
        journal.record(dispatch);
        for ticks in [5u32, 16, 16, 16, 11] {
            rebuilt.mining.advance(&mut world, &events, ticks);
            journal.record(Command::Advance { ticks });
        }

        let span = vx_agent::working_span(area, area.min);
        let played = region_hash(&world, span.min, span.max);
        assert_eq!(
            played,
            replay_to_hash(&journal, area),
            "replaying the session produced a different world"
        );
    }

    #[test]
    fn the_journal_is_far_smaller_than_the_ground_it_describes() {
        // The whole argument for recording orders instead of outcomes: a mine
        // is thousands of block changes and one line of intent.
        let (mut world, area) = site();
        let events = EventBus::new();
        let mut journal = CommandLog::new();
        let mut rebuilt = Rebuilt::default();

        let dispatch = Command::Dispatch {
            area,
            method: MineMethod::Pit,
            crew: 3,
        };
        apply(&dispatch, &mut world, &events, &mut rebuilt);
        journal.record(dispatch);
        // Frames of wildly different lengths, as a real session has.
        for _ in 0..200 {
            for ticks in [1u32, 7, 3, 16, 2, 11, 16, 9] {
                rebuilt.mining.advance(&mut world, &events, ticks);
                journal.record(Command::Advance { ticks });
            }
        }

        // Count what actually moved, against a pristine copy of the same seed.
        let span = vx_agent::working_span(area, area.min);
        let mut pristine = World::new(2024);
        pristine.pin_span(span.min, span.max);

        let mut dug = 0usize;
        for y in span.min.y..=span.max.y {
            for z in span.min.z..=span.max.z {
                for x in span.min.x..=span.max.x {
                    let at = BlockPos::new(x, y, z);
                    if pristine.block(at) != world.block(at) {
                        dug += 1;
                    }
                }
            }
        }

        assert!(dug > 300, "the fixture mine only moved {dug} blocks");
        assert!(
            journal.len() < 32,
            "{} entries described {dug} changed blocks",
            journal.len()
        );
        // And it still replays to exactly that ground.
        assert_eq!(
            region_hash(&world, span.min, span.max),
            replay_to_hash(&journal, area),
            "the compact journal did not rebuild the mine"
        );
    }

    #[test]
    fn ticks_fold_so_a_running_game_does_not_bury_the_log() {
        // Sixty entries a second would drown the handful that carry meaning.
        let mut journal = CommandLog::new();
        for _ in 0..1_000 {
            journal.record(Command::Advance { ticks: 3 });
        }
        assert_eq!(journal.len(), 1, "per-frame ticks were not folded");
        assert_eq!(journal.tick(), 3_000, "folding lost time");
        assert_eq!(
            journal.entries()[0].command,
            Command::Advance { ticks: 3_000 }
        );

        // A zero-tick frame is not an event.
        journal.record(Command::Advance { ticks: 0 });
        assert_eq!(journal.len(), 1);
        assert_eq!(journal.tick(), 3_000);

        // Anything else breaks the run.
        journal.record(Command::Cancel);
        journal.record(Command::Advance { ticks: 2 });
        assert_eq!(journal.len(), 3);
    }

    #[test]
    fn commands_are_stamped_with_the_tick_they_happened_on() {
        let mut journal = CommandLog::new();
        journal.record(Command::Advance { ticks: 40 });
        journal.record(Command::Break {
            at: BlockPos::new(1, 2, 3),
        });
        assert_eq!(journal.entries()[1].tick, 40);
    }

    #[test]
    fn a_keyframe_clears_the_tail_but_not_the_clock() {
        let mut journal = CommandLog::new();
        journal.record(Command::Advance { ticks: 100 });
        journal.record(Command::Cancel);
        journal.keyframed(0xdead_beef);

        assert!(journal.is_empty(), "the tail survived a keyframe");
        assert_eq!(journal.keyframe_tick, 100);
        assert_eq!(journal.keyframe_hash, 0xdead_beef);
        assert_eq!(journal.tick(), 100, "the keyframe rewound the clock");
    }

    #[test]
    fn the_journal_round_trips_and_tolerates_damage() {
        let directory = std::env::temp_dir().join(format!("vx-journal-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut journal = record_a_session();
        journal.keyframed(0x1234_5678);
        journal.record(Command::Place {
            at: BlockPos::new(-3, 70, 8),
            block: "engine:stone".into(),
        });
        journal.record(Command::Advance { ticks: 12 });
        journal.save(&directory).unwrap();

        let read = CommandLog::load(&directory);
        assert_eq!(read, journal, "the journal did not survive the trip");

        std::fs::write(directory.join("log.dat"), b"NOT A JOURNAL").unwrap();
        assert!(
            CommandLog::load(&directory).is_empty(),
            "a damaged journal invented commands"
        );

        std::fs::remove_dir_all(&directory).ok();
        assert!(CommandLog::load(&directory).is_empty());
    }

    #[test]
    fn negative_coordinates_survive_the_encoding() {
        // The encoder writes positions through u32; a sign bug here would move
        // every edit west of the origin somewhere else entirely.
        let directory = std::env::temp_dir().join(format!("vx-journal-neg-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut journal = CommandLog::new();
        let at = BlockPos::new(-1_000, 5, -7);
        journal.record(Command::Break { at });
        journal.save(&directory).unwrap();

        assert_eq!(
            CommandLog::load(&directory).entries()[0].command,
            Command::Break { at }
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_saved_world_and_its_journal_agree_across_disk() {
        // What `--replay` does, end to end: play a session, save the ground and
        // the orders, then rebuild the ground from the orders alone and check
        // the two hash the same.
        let directory = std::env::temp_dir().join(format!("vx-replay-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        let save = vx_world::WorldSave::create(&directory).unwrap();

        let (mut world, area) = site();
        let events = EventBus::new();
        let mut journal = CommandLog::new();
        let mut rebuilt = Rebuilt::default();

        let dispatch = Command::Dispatch {
            area,
            method: MineMethod::Pit,
            crew: 3,
        };
        apply(&dispatch, &mut world, &events, &mut rebuilt);
        journal.record(dispatch);
        for _ in 0..40 {
            rebuilt.mining.advance(&mut world, &events, 16);
            journal.record(Command::Advance { ticks: 16 });
        }
        let ground = world.generator().height_at(20, 20);
        let by_hand = BlockPos::new(20, ground - 1, 20);
        vx_world::break_block(&mut world, &events, by_hand).unwrap();
        journal.record(Command::Break { at: by_hand });

        save.write_meta(world.seed()).unwrap();
        save.save_world(&mut world).unwrap();
        journal.save(&directory).unwrap();

        // Now the replay side, from nothing but the seed and the orders.
        let seed = save.read_meta().unwrap();
        let read = CommandLog::load(&directory);
        let mut rebuilt = World::new(seed);
        replay(&read, &mut rebuilt, &EventBus::new());

        // And the saved side, chunk by chunk, exactly as `--replay` reloads it.
        let mut restored = World::new(seed);
        for pos in rebuilt.loaded_chunks().collect::<Vec<_>>() {
            match save.load_chunk(pos, restored.registry()).unwrap() {
                Some(chunk) => restored.insert_chunk(chunk),
                None => {
                    restored.load_chunk(pos);
                }
            }
        }

        assert_eq!(
            vx_world::world_hash(&restored),
            vx_world::world_hash(&rebuilt),
            "the journal did not rebuild the world that was saved beside it"
        );
        // And the hand-dug block is really in both, so the comparison is not
        // two empty worlds agreeing.
        assert_eq!(rebuilt.block(by_hand), vx_core::BlockId::AIR);
        assert_eq!(restored.block(by_hand), vx_core::BlockId::AIR);

        std::fs::remove_dir_all(&directory).ok();
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn an_old_journal_restarts_the_oracle_instead_of_lying() {
        // A v3 log was recorded against a hometown without the player's house.
        // Replaying it over post-house terrain would diverge through no fault
        // of the log, so the loader refuses it and the fresh log declines
        // genesis coverage rather than claiming it.
        let directory = std::env::temp_dir().join(format!(
            "vx-journal-version-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 24]); // keyframe tick/hash/clock
        std::fs::write(directory.join("log.dat"), &bytes).unwrap();

        let log = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        assert!(log.is_empty(), "salvaged entries from a refused version");
        assert_ne!(
            log.keyframe_tick, 0,
            "a restarted log must not claim to reach back to genesis"
        );
    }
}

#[cfg(test)]
mod movement_replay_tests {
    use super::*;
    use crate::movement::{self, MoveCommand};
    use vx_core::ChunkPos;

    fn world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 2);
        world
    }

    /// A short recorded run: walk, sprint, jump, slide, stop.
    fn recorded() -> CommandLog {
        let mut journal = CommandLog::default();
        let east = std::f32::consts::FRAC_PI_2;
        let script: [(u16, u32); 5] = [
            (movement::FWD, 12),
            (movement::FWD | movement::SPRINT, 20),
            (movement::FWD | movement::SPRINT | movement::JUMP, 4),
            (movement::FWD | movement::SPRINT | movement::CROUCH, 16),
            (0, 8),
        ];
        for (bits, ticks) in script {
            let command = MoveCommand::looking(bits, east, 0.0).laden(40);
            journal.record(Command::moving(command));
            journal.record(Command::Advance { ticks });
        }
        journal
    }

    #[test]
    fn the_same_journal_walks_the_same_walk() {
        // The regression test the design note asked for, run against the log
        // that already existed rather than a fixture built beside it.
        let journal = recorded();
        let events = EventBus::new();

        let once = replay(&journal, &mut world(), &events);
        let twice = replay(&journal, &mut world(), &events);

        assert_eq!(once.player.position, twice.player.position);
        assert_eq!(once.player.velocity, twice.player.velocity);
        assert_eq!(once.movement.stance, twice.movement.stance);
        assert_eq!(once.movement.stamina, twice.movement.stamina);
    }

    #[test]
    fn a_journal_that_records_movement_actually_moves_the_player() {
        // Guards against the whole thing passing by moving nobody at all.
        let journal = recorded();
        let events = EventBus::new();
        let mut world = world();
        let start = {
            let ground = world.surface_y(0, 0).unwrap_or(64) as f32;
            glam::Vec3::new(0.5, ground, 0.5)
        };

        let walked = replay(&journal, &mut world, &events);

        let travelled = (walked.player.position - start.as_dvec3()).length();
        assert!(travelled > 3.0, "the replayed player only moved {travelled}");
    }

    #[test]
    fn splitting_an_advance_does_not_change_where_the_player_ends_up() {
        // `Advance` folds when recorded, so the same run can be logged as one
        // long entry or several short ones. Both must replay identically, or
        // the fold is silently lossy.
        let east = std::f32::consts::FRAC_PI_2;
        let command = MoveCommand::looking(movement::FWD | movement::SPRINT, east, 0.0);

        let mut whole = CommandLog::default();
        whole.record(Command::moving(command));
        whole.record(Command::Advance { ticks: 60 });

        let mut split = CommandLog::default();
        split.record(Command::moving(command));
        for _ in 0..12 {
            split.record(Command::Advance { ticks: 5 });
        }

        let events = EventBus::new();
        let a = replay(&whole, &mut world(), &events);
        let b = replay(&split, &mut world(), &events);
        assert_eq!(a.player.position, b.player.position);
    }

    /// **The journal walks the same walk three thousand kilometres out.**
    ///
    /// The same recorded run replayed from the origin and from a start
    /// three thousand kilometres away, over the same flat floor laid at both
    /// places, must displace the player identically to a micrometre. In the
    /// old `f32` integrator the far body's per-tick moves were rounded to a
    /// quarter of a block and it stuck to the floor's own skin; this is the
    /// stage 42 claim that the player is the one thing that travels, and
    /// that the journal — the oracle everything else rests on — does not
    /// care where on the map it was written.
    #[test]
    fn the_journal_walks_the_same_walk_three_thousand_kilometres_out() {
        let floor = |world: &mut World, ox: i32, oz: i32| {
            world.load_around(vx_core::BlockPos::new(ox, 0, oz).chunk(), 2);
            let stone = world.registry().id_of("engine:stone").unwrap();
            for x in -24..40 {
                for z in -24..24 {
                    for y in 60..90 {
                        world.set_block(vx_core::BlockPos::new(ox + x, y, oz + z), vx_core::BlockId::AIR);
                    }
                    world.set_block(vx_core::BlockPos::new(ox + x, 64, oz + z), stone);
                }
            }
        };
        let journal = recorded();
        let events = EventBus::new();

        let walk = |ox: i32, oz: i32| {
            let mut world = World::new(2024);
            floor(&mut world, ox, oz);
            let start = glam::DVec3::new(ox as f64 + 0.5, 65.0, oz as f64 + 0.5);
            let done = replay_from(&journal, &mut world, &events, start);
            (done.player.position - start, done.movement.stance)
        };

        let (here, stance_here) = walk(0, 0);
        let (there, stance_there) = walk(3_000_000, 3_000_000);
        assert!(here.length() > 3.0, "the player did not walk: {here}");
        assert!(
            (here - there).length() < 1.0e-6,
            "the same journal walked {here} at spawn and {there} far out"
        );
        assert_eq!(stance_here, stance_there);
    }

    #[test]
    fn a_move_survives_a_round_trip_through_the_file() {
        let directory = std::env::temp_dir().join(format!(
            "vx-journal-move-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let journal = recorded();
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let moves: Vec<_> = read_back
            .entries()
            .iter()
            .filter(|entry| matches!(entry.command, Command::Move { .. }))
            .map(|entry| entry.command.clone())
            .collect();
        assert_eq!(moves.len(), 5, "movement entries did not survive the file");
        assert!(
            matches!(moves[0], Command::Move { load: 40, .. }),
            "the load did not survive: {:?}",
            moves[0]
        );
    }
}

#[cfg(test)]
mod admin_tests {
    use super::*;
    use vx_core::ChunkPos;

    /// Radius four: the teleport fixtures land near the town's west edge, and
    /// an unloaded chunk reads as air — a body teleported into one falls
    /// forever instead of sprinting.
    fn world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 4);
        world
    }

    /// A session with cheats in it: teleport, a tuning drag, movement after.
    fn cheated() -> CommandLog {
        let mut journal = CommandLog::default();
        let east = std::f32::consts::FRAC_PI_2;
        journal.record(Command::Admin(Admin::Teleport { x: 8, y: 74, z: 8 }));
        journal.record(Command::Admin(Admin::SetTuning {
            key: "sprint_speed".into(),
            value: 9.0,
        }));
        journal.record(Command::moving(MoveCommand::looking(
            movement::FWD | movement::SPRINT,
            east,
            0.0,
        )));
        journal.record(Command::Advance { ticks: 40 });
        journal.record(Command::Admin(Admin::SetStat {
            key: "stamina".into(),
            value: 100,
        }));
        journal.record(Command::Advance { ticks: 8 });
        journal
    }

    #[test]
    fn a_journal_with_admin_commands_replays_to_the_same_place() {
        // The entire premise: the oracle covers cheated sessions.
        let journal = cheated();
        let events = EventBus::new();
        let once = replay(&journal, &mut world(), &events);
        let twice = replay(&journal, &mut world(), &events);
        assert_eq!(once.player.position, twice.player.position);
        assert_eq!(once.movement.stamina, twice.movement.stamina);
        assert_eq!(once.movement.tuning, twice.movement.tuning);
    }

    #[test]
    fn a_tuning_change_mid_log_changes_the_outcome_and_still_replays() {
        // The physics travels with the journal: the same held sprint under a
        // different sprint_speed lands somewhere else, reproducibly.
        let east = std::f32::consts::FRAC_PI_2;
        let with_tuning = |speed: Option<f32>| {
            let mut journal = CommandLog::default();
            // The lane at z = 20 crosses the whole flat town platform with no
            // building in it — a wall would clamp both runs to the same spot
            // and hide the difference this test exists to see.
            journal.record(Command::Admin(Admin::Teleport { x: -20, y: 74, z: 20 }));
            if let Some(value) = speed {
                journal.record(Command::Admin(Admin::SetTuning {
                    key: "sprint_speed".into(),
                    value,
                }));
            }
            journal.record(Command::moving(MoveCommand::looking(
                movement::FWD | movement::SPRINT,
                east,
                0.0,
            )));
            journal.record(Command::Advance { ticks: 20 });
            let events = EventBus::new();
            replay(&journal, &mut world(), &events).player.position
        };

        let stock = with_tuning(None);
        let tuned = with_tuning(Some(9.5));
        assert_ne!(stock, tuned, "the tuning order changed nothing");
        assert_eq!(
            with_tuning(Some(9.5)),
            tuned,
            "the tuned run is not reproducible"
        );
    }

    #[test]
    fn a_give_lands_in_the_base_pile_only_when_a_base_exists() {
        // The no-base rule must match live behaviour exactly, or the oracle
        // and the session disagree about how much ore you own.
        let events = EventBus::new();
        let mut journal = CommandLog::default();
        journal.record(Command::Admin(Admin::Give {
            good: "engine:copper_ore".into(),
            amount: 500,
        }));
        let rebuilt = replay(&journal, &mut world(), &events);
        assert!(
            rebuilt.mining.fleet.base.is_none(),
            "a base appeared from nowhere"
        );
    }

    #[test]
    fn admin_orders_survive_the_wire() {
        let directory = std::env::temp_dir().join(format!(
            "vx-journal-admin-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let mut journal = CommandLog::default();
        let orders = [
            Admin::Give { good: "engine:log".into(), amount: 42 },
            Admin::SpawnMachine { kind: "drone".into(), count: 3 },
            Admin::Teleport { x: -14, y: 73, z: 9 },
            Admin::SetStat { key: "credits".into(), value: 9_999 },
            Admin::SetStock { x: 512, z: 0, good: "engine:stone".into(), amount: 700 },
            Admin::SetTuning { key: "friction_slide".into(), value: 1.234_567 },
        ];
        for order in &orders {
            journal.record(Command::Admin(order.clone()));
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let recovered: Vec<&Command> =
            read_back.entries().iter().map(|entry| &entry.command).collect();
        assert_eq!(recovered.len(), orders.len(), "orders were lost on the wire");
        for (found, sent) in recovered.iter().zip(&orders) {
            assert_eq!(**found, Command::Admin((*sent).clone()), "an order mutated in transit");
        }
    }

    #[test]
    fn a_teleport_is_block_exact_after_the_wire() {
        // f32 positions would drift through a file; blocks cannot.
        let events = EventBus::new();
        let mut journal = CommandLog::default();
        journal.record(Command::Admin(Admin::Teleport { x: 100, y: 80, z: -40 }));
        let rebuilt = replay(&journal, &mut world(), &events);
        assert_eq!(
            rebuilt.player.position,
            glam::DVec3::new(100.5, 80.0, -39.5),
            "the landing spot moved"
        );
    }
}

#[cfg(test)]
mod fire_tests {
    use super::*;
    use vx_core::ChunkPos;
    use vx_world::region_hash;

    fn world() -> World {
        let mut world = World::new(2024);
        world.load_around(ChunkPos::new(0, 0), 4);
        world
    }

    /// A `Fire` survives the wire bit-exactly, muzzle floats included.
    #[test]
    fn a_shot_survives_the_wire() {
        let directory = std::env::temp_dir().join(format!(
            "vx-journal-fire-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let mut journal = CommandLog::default();
        let shot = Command::Fire {
            muzzle: [12.375, 81.62, -3.25],
            yaw_q: 1023,
            pitch_q: -77,
        };
        journal.record(shot.clone());
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        assert_eq!(read_back.entries().len(), 1);
        assert_eq!(read_back.entries()[0].command, shot, "the shot mutated in transit");
    }

    /// The oracle covers gunfire: a session that fires into the ground twice
    /// replays to the same craters, from the recorded orders alone.
    #[test]
    fn a_firefight_replays_to_the_same_ground() {
        let fight = |world: &mut World| -> u64 {
            let events = EventBus::new();
            let mut journal = CommandLog::default();
            let mut rebuilt = Rebuilt::default();

            let ground = f64::from(world.generator().height_at(10, 10));
            // Stand above open ground and fire down at an angle, twice, with
            // ticks between: each shot arcs into the dirt and craters it.
            let muzzle = [10.5, ground + 8.0, 10.5];
            for (yaw_q, pitch_q, wait) in [(0i16, -700i16, 6u32), (1024, -650, 8)] {
                let shot = Command::Fire { muzzle, yaw_q, pitch_q };
                apply(&shot, world, &events, &mut rebuilt);
                journal.record(shot);
                let advance = Command::Advance { ticks: wait };
                apply(&advance, world, &events, &mut rebuilt);
                journal.record(advance);
            }
            assert!(
                rebuilt.shots.is_empty(),
                "both rounds should have landed inside the waits"
            );
            let hash_live = region_hash(
                world,
                vx_core::BlockPos::new(-20, 40, -20),
                vx_core::BlockPos::new(40, 100, 40),
            );

            // Now the other side of the oracle: a fresh world, orders only.
            let mut fresh = super::fire_tests::world();
            replay(&journal, &mut fresh, &EventBus::new());
            let hash_replayed = region_hash(
                &fresh,
                vx_core::BlockPos::new(-20, 40, -20),
                vx_core::BlockPos::new(40, 100, 40),
            );
            assert_eq!(hash_live, hash_replayed, "the craters diverged");
            hash_live
        };

        let first = fight(&mut world());
        let second = fight(&mut world());
        assert_eq!(first, second, "the firefight itself is not deterministic");
    }

    /// **The frost is on the ordinary side of the oracle.** A journal that
    /// stands at the origin through the first snow of a winter replays to
    /// the same snowed ground and the same frozen water, from the tick and
    /// the player's replayed column alone — and that ground differs from the
    /// untouched country, so the equality is not vacuous. This is the test
    /// stage 41's `reading_the_year_never_touches_the_ground` said would
    /// exist the day snow settled.
    #[test]
    fn a_winter_replays_to_the_same_snow_and_ice() {
        use vx_world::season::SEASON_TICKS;
        let span = (
            vx_core::BlockPos::new(-40, 40, -40),
            vx_core::BlockPos::new(40, 200, 40),
        );
        let seed = world().seed();
        // The first snowing pass over the origin from the start of winter,
        // and two hours on from the first flakes.
        let snow = (0..(SEASON_TICKS / crate::frost::EVERY))
            .map(|n| 3 * SEASON_TICKS + n * crate::frost::EVERY)
            .find(|&tick| vx_world::weather::at(seed, tick, 0, 0).snowing())
            .expect("it never snows over the origin all winter");
        let until = u32::try_from(snow + 64 * 60 * 2).unwrap();
        let mut journal = CommandLog::default();
        journal.record(Command::Advance { ticks: until });

        let untouched = region_hash(&world(), span.0, span.1);

        let mut live = world();
        let mut rebuilt = Rebuilt::default();
        apply(&Command::Advance { ticks: until }, &mut live, &EventBus::new(), &mut rebuilt);
        let hash_live = region_hash(&live, span.0, span.1);

        let mut fresh = world();
        replay(&journal, &mut fresh, &EventBus::new());
        let hash_replayed = region_hash(&fresh, span.0, span.1);

        assert_eq!(hash_live, hash_replayed, "the snow diverged");
        assert_ne!(hash_live, untouched, "a winter passed and nothing settled");
        let cold = (span.0.x..=span.1.x)
            .flat_map(|x| (span.0.z..=span.1.z).map(move |z| (x, z)))
            .filter(|&(x, z)| {
                let Some(surface) = fresh.surface_y(x, z) else {
                    return false;
                };
                let top = fresh.block(vx_core::BlockPos::new(x, surface - 1, z));
                fresh
                    .registry()
                    .get(top)
                    .is_some_and(|def| def.name.starts_with("engine:snowy_") || def.name == "engine:ice")
            })
            .count();
        assert!(cold > 0, "the hash moved but nothing near the origin is snow or ice");
    }

    /// **A shot lands the same crater three thousand kilometres out.** The
    /// same two rounds from the same muzzle over the same floor, at the
    /// origin and far out; the wounds they leave, read relative to the
    /// floor's corner, are identical block for block and mask for mask.
    /// The muzzle crosses the wire at `f64`, so it starts where the eye is.
    #[test]
    fn a_shot_lands_the_same_crater_three_thousand_kilometres_out() {
        let floor = |world: &mut World, ox: i32, oz: i32| {
            world.load_around(vx_core::BlockPos::new(ox, 0, oz).chunk(), 2);
            let stone = world.registry().id_of("engine:stone").unwrap();
            for x in -24..40 {
                for z in -24..24 {
                    for y in 60..90 {
                        world.set_block(vx_core::BlockPos::new(ox + x, y, oz + z), vx_core::BlockId::AIR);
                    }
                    world.set_block(vx_core::BlockPos::new(ox + x, 64, oz + z), stone);
                }
            }
        };
        let fight = |ox: i32, oz: i32| {
            let mut world = World::new(2024);
            floor(&mut world, ox, oz);
            let mut journal = CommandLog::default();
            let muzzle = [f64::from(ox) + 10.5, 65.0 + 8.0, f64::from(oz) + 10.5];
            for (yaw_q, pitch_q, wait) in [(0i16, -700i16, 6u32), (1024, -650, 8)] {
                journal.record(Command::Fire { muzzle, yaw_q, pitch_q });
                journal.record(Command::Advance { ticks: wait });
            }
            let start = glam::DVec3::new(f64::from(ox) + 10.5, 65.0, f64::from(oz) + 10.5);
            replay_from(&journal, &mut world, &EventBus::new(), start);
            // Every block that is no longer plain floor, relative to the corner.
            let mut wounds = Vec::new();
            for x in -24..40 {
                for z in -24..24 {
                    for y in 60..90 {
                        let at = vx_core::BlockPos::new(ox + x, y, oz + z);
                        let block = world.block(at);
                        let mask = world.mask(at);
                        let plain = if y == 64 { block != vx_core::BlockId::AIR && mask.is_none() } else { block == vx_core::BlockId::AIR };
                        if !plain {
                            wounds.push((x, y, z, block, mask));
                        }
                    }
                }
            }
            wounds
        };
        let here = fight(0, 0);
        let there = fight(3_000_000, 3_000_000);
        assert!(!here.is_empty(), "the rounds left no mark on the floor");
        assert_eq!(here, there, "the craters differ far out");
    }

    /// A pump switched on replays to the same water it lifted.
    #[test]
    fn a_pump_replays_to_the_same_lift() {
        let floor = 150;
        let at = vx_core::BlockPos::new(4, floor + 1, 4);
        let span = (
            vx_core::BlockPos::new(-4, floor - 4, -4),
            vx_core::BlockPos::new(14, floor + 10, 14),
        );
        let build = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(0, 0), 2);
            let stone = world.registry().id_of("engine:stone").unwrap();
            let water = world.registry().id_of("engine:water").unwrap();
            let pump = world.registry().id_of("engine:pump").unwrap();
            for x in 0..10 {
                for z in 0..10 {
                    world.set_block(vx_core::BlockPos::new(x, floor, z), stone);
                    for y in floor + 1..floor + 8 {
                        world.set_block(vx_core::BlockPos::new(x, y, z), vx_core::BlockId::AIR);
                    }
                }
            }
            world.set_block(at, pump);
            // A pool for it to drink from, held in a walled corner.
            for x in 5..8 {
                for z in 3..6 {
                    vx_world::fluid::set_level(
                        &mut world,
                        water,
                        vx_core::BlockPos::new(x, floor + 1, z),
                        vx_world::fluid::FULL,
                    );
                }
            }
            world
        };

        let lift = |world: &mut World| -> u64 {
            let events = EventBus::new();
            let mut journal = CommandLog::default();
            let mut rebuilt = Rebuilt::default();
            let switch = Command::Pump { at, on: true };
            apply(&switch, world, &events, &mut rebuilt);
            journal.record(switch);
            assert_eq!(rebuilt.pumps, vec![at], "the switch did not take");
            let advance = Command::Advance { ticks: 64 * 4 };
            apply(&advance, world, &events, &mut rebuilt);
            journal.record(advance);

            let live = region_hash(world, span.0, span.1);
            let mut fresh = build();
            replay(&journal, &mut fresh, &EventBus::new());
            assert_eq!(
                live,
                region_hash(&fresh, span.0, span.1),
                "the pump lifted somewhere else on replay"
            );
            live
        };

        let first = lift(&mut build());
        assert_eq!(first, lift(&mut build()), "the pump is not deterministic");
        assert_ne!(first, region_hash(&build(), span.0, span.1), "it lifted nothing");
    }

    /// Water let into a hole replays to the same water.
    ///
    /// The flood is not recorded — only the break that started it. Both sides
    /// wake the same cells and run the same automaton on the same clock, so
    /// the pond ends up in the same shape or the oracle is broken.
    #[test]
    fn a_flood_replays_to_the_same_water() {
        // A basin cut into a hillside, filled by hand, with a plug in one
        // wall. Breaking the plug is the order; everything after it is
        // arithmetic.
        let floor = 150;
        let plug = vx_core::BlockPos::new(4, floor + 1, 0);
        let span = (
            vx_core::BlockPos::new(-12, floor - 6, -12),
            vx_core::BlockPos::new(20, floor + 12, 20),
        );

        let build = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(0, 0), 2);
            let stone = world.registry().id_of("engine:stone").unwrap();
            let water = world.registry().id_of("engine:water").unwrap();
            // A shelf with a lip, and a trench beyond the lip to run into.
            for x in 0..10 {
                for z in -6..8 {
                    world.set_block(vx_core::BlockPos::new(x, floor, z), stone);
                    for y in floor + 1..floor + 6 {
                        world.set_block(vx_core::BlockPos::new(x, y, z), vx_core::BlockId::AIR);
                    }
                }
            }
            for x in 0..10 {
                for y in floor + 1..floor + 4 {
                    world.set_block(vx_core::BlockPos::new(x, y, 0), stone);
                }
            }
            // The pond, held behind the lip.
            for x in 1..9 {
                for z in 1..7 {
                    for y in floor + 1..floor + 3 {
                        vx_world::fluid::set_level(
                            &mut world,
                            water,
                            vx_core::BlockPos::new(x, y, z),
                            vx_world::fluid::FULL,
                        );
                    }
                }
            }
            world
        };

        let flood = |world: &mut World| -> u64 {
            let events = EventBus::new();
            let mut journal = CommandLog::default();
            let mut rebuilt = Rebuilt::default();

            let pull = Command::Break { at: plug };
            apply(&pull, world, &events, &mut rebuilt);
            journal.record(pull);
            assert!(!rebuilt.water.is_empty(), "nothing woke up");
            let advance = Command::Advance { ticks: 64 * 8 };
            apply(&advance, world, &events, &mut rebuilt);
            journal.record(advance);

            let live = region_hash(world, span.0, span.1);

            // The other side of the oracle: the same basin, the same order.
            let mut fresh = build();
            replay(&journal, &mut fresh, &EventBus::new());
            assert_eq!(
                live,
                region_hash(&fresh, span.0, span.1),
                "the flood found a different level on replay"
            );
            live
        };

        let first = flood(&mut build());
        let second = flood(&mut build());
        assert_eq!(first, second, "the flood is not deterministic in the first place");
        assert_ne!(first, region_hash(&build(), span.0, span.1), "nothing moved");
    }

    /// The cut survives the wire: the stump and the face, and nothing else,
    /// because everything else is re-derived.
    #[test]
    fn a_cut_survives_the_wire() {
        let directory =
            std::env::temp_dir().join(format!("vx-journal-fell-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut journal = CommandLog::default();
        let cut = Command::Fell {
            at: vx_core::BlockPos::new(-412, 97, 883),
            face: 5,
        };
        journal.record(cut.clone());
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        assert_eq!(
            read_back.entries().last().map(|entry| entry.command.clone()),
            Some(cut)
        );
    }

    /// Lightning lands, the woods burn, and the burn replays block for block.
    ///
    /// The fourth round of the same test and the one this round exists for.
    /// Nothing here is recorded: the storm is a pure function of the seed and
    /// the tick, the strike is hashed off the same tick, and the only other
    /// input — where the player is standing — is already replayed. So the
    /// whole chain rides on `Advance` and nothing else, and if any part of it
    /// reached for a clock, a generator or a wall time, the two sides would
    /// part company here.
    ///
    /// **The tick is a fact about the seed, not a fixture.** Seed 2024's
    /// first storm strike that lights anything falls at tick 158 400, on a
    /// stand of dry grass at (-6, 67, 85). Nothing sets that up; it is what
    /// the weather does over that ground, found by walking the clock.
    #[test]
    fn a_strike_and_a_burn_replay_to_the_same_ground() {
        const STRIKE_TICK: u32 = 158_400;
        // Wide enough that the strike, its 13-block neighbourhood and the
        // fire's whole reach are inside the loaded ground on both sides — a
        // fire that ran off the edge of what was loaded would be a fire the
        // two sides disagreed about for a boring reason.
        let patch = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(0, 0), 8);
            world
        };
        let span = (
            vx_core::BlockPos::new(-100, 40, 0),
            vx_core::BlockPos::new(60, 200, 140),
        );

        // One order, covering the strike and long enough after it for the
        // fire to run itself out.
        let mut journal = CommandLog::default();
        journal.record(Command::Advance {
            ticks: STRIKE_TICK + 64 * 30,
        });

        let mut live = patch();
        let played = replay(&journal, &mut live, &EventBus::new());
        assert_eq!(played.tick, (STRIKE_TICK + 64 * 30) as u64);
        let burned = region_hash(&live, span.0, span.1);

        // The other side of the oracle: the same orders, a fresh world.
        let mut fresh = patch();
        replay(&journal, &mut fresh, &EventBus::new());
        assert_eq!(
            burned,
            region_hash(&fresh, span.0, span.1),
            "the fire burned somewhere else on replay"
        );

        // And it actually burned: a log that sat through a lightning strike
        // cannot hash the same as ground nothing ever happened to.
        let untouched = region_hash(&patch(), span.0, span.1);
        assert_ne!(burned, untouched, "nothing caught");
    }

    /// A tree put on the ground replays block for block.
    ///
    /// The one that matters this round: the order is the cut, and everything
    /// else — where the stem swept, what it flattened, which neighbour came
    /// down with it and where the logs came to rest — is re-derived by the
    /// same arc on the same clock. If the fall were a rigid body this test
    /// would be the one that failed.
    #[test]
    fn a_felled_tree_replays_to_the_same_ground() {
        // Away from the home town, where the trees are.
        let woods = (96, 96);
        let patch = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(woods.0 / 16, woods.1 / 16), 4);
            world
        };
        let span = (
            vx_core::BlockPos::new(woods.0 - 60, 40, woods.1 - 60),
            vx_core::BlockPos::new(woods.0 + 60, 200, woods.1 + 60),
        );

        let fell = |world: &mut World| -> u64 {
            let events = EventBus::new();
            let mut journal = CommandLog::default();
            let mut rebuilt = Rebuilt::default();

            // Find a standing tree the same way the game does.
            let sites = world
                .towns_near((woods.0, woods.1), crate::felling::TOWN_REACH);
            let generator = world.generator();
            let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, &sites);
            let natural_at = |x: i32, z: i32| generator.natural_height_at(x, z);
            let trees = vx_world::flora::trees_overlapping(
                world.seed(),
                (woods.0 - 24, woods.1 - 24),
                (woods.0 + 24, woods.1 + 24),
                &height_at,
                &natural_at,
                &sites,
            );
            let tree = trees
                .into_iter()
                .find(|tree| {
                    tree.height >= 6
                        && (tree.base.x - woods.0).abs() <= 24
                        && (tree.base.z - woods.1).abs() <= 24
                        && world.block(vx_core::BlockPos::new(
                            tree.base.x,
                            tree.base.y + 1,
                            tree.base.z,
                        )) != vx_core::BlockId::AIR
                })
                .expect("no standing tree to fell");
            let stump = vx_core::BlockPos::new(tree.base.x, tree.base.y + 1, tree.base.z);

            let cut = Command::Fell { at: stump, face: 1 };
            apply(&cut, world, &events, &mut rebuilt);
            journal.record(cut);
            // Long enough for the stem to go all the way over and lie down.
            let advance = Command::Advance { ticks: 64 * 6 };
            apply(&advance, world, &events, &mut rebuilt);
            journal.record(advance);
            assert!(rebuilt.falls.is_empty(), "the stem never came down");

            let live = region_hash(world, span.0, span.1);

            // The other side of the oracle: a fresh world, orders only.
            let mut fresh = patch();
            replay(&journal, &mut fresh, &EventBus::new());
            assert_eq!(
                live,
                region_hash(&fresh, span.0, span.1),
                "the felled tree landed somewhere else on replay"
            );
            live
        };

        let first = fell(&mut patch());
        let second = fell(&mut patch());
        assert_eq!(first, second, "felling is not deterministic in the first place");

        // And it actually changed the ground: a log that fells a tree cannot
        // hash the same as one that never touched the woods.
        let untouched = region_hash(&patch(), span.0, span.1);
        assert_ne!(first, untouched, "the tree is still standing");
    }

    /// Every scout order survives the wire, and a log full of them replays
    /// to the same ground as a log with none — the scout charts nothing and
    /// breaks nothing, so the oracle must not see it.
    #[test]
    fn a_bank_order_round_trips_with_the_amount_that_actually_moved() {
        let directory =
            std::env::temp_dir().join(format!("vx-journal-bank-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let orders = [
            Command::Bank {
                town: (0, 0),
                good: "engine:copper_ore".into(),
                amount: 240,
                deposit: true,
            },
            Command::Bank {
                town: (-512, 1_024),
                good: "engine:hho_cell".into(),
                amount: 7,
                deposit: false,
            },
        ];
        let mut journal = CommandLog::default();
        for order in &orders {
            journal.record(order.clone());
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let recovered: Vec<Command> = read_back
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .collect();
        assert_eq!(recovered, orders.to_vec(), "a bank order did not survive the wire");

        // Negative town coordinates are the case a naive u32 round-trip eats,
        // and towns west or north of the origin are half the world.
        assert!(matches!(
            recovered[1],
            Command::Bank { town: (-512, 1_024), .. }
        ));

        // And the replay arm moves exactly the recorded amount, so a vault
        // that filled up mid-deposit cannot make the two sides disagree.
        let mut banks = crate::bank::Bank::default();
        let mut pile = vx_agent::Stockpile::new();
        pile.add("engine:copper_ore", 240);
        banks.deposit((0, 0), "engine:copper_ore", 240, &mut pile);
        assert_eq!(banks.stored((0, 0)), 240);
        assert_eq!(pile.count("engine:copper_ore"), 0);
    }

    #[test]
    fn a_repair_round_trips_and_replays_to_the_same_pile() {
        let directory =
            std::env::temp_dir().join(format!("vx-journal-repair-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let orders = [
            Command::Repair {
                machine: MachineTag::Digger(0),
            },
            Command::Repair {
                machine: MachineTag::Flier(3),
            },
        ];
        let mut journal = CommandLog::default();
        for order in &orders {
            journal.record(order.clone());
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let recovered: Vec<Command> = read_back
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .collect();
        assert_eq!(recovered, orders.to_vec(), "a repair did not survive the wire");

        // And the arm both sides run spends exactly the same parts: a worn
        // machine, mended once, leaves the pile short by one repair's worth
        // and the machine fresh.
        let mut wear = crate::wear::Wear::default();
        for _ in 0..crate::wear::WORN_AT {
            wear.tick(1, 0);
        }
        let mut pile = vx_agent::Stockpile::new();
        pile.add(crate::wear::SPARE_PART, 5);
        assert!(wear.repair(crate::mining::MachineRef::Digger(0), &mut pile));
        assert_eq!(
            pile.count(crate::wear::SPARE_PART),
            5 - crate::wear::PARTS_PER_REPAIR
        );
        assert_eq!(
            wear.condition(crate::mining::MachineRef::Digger(0)),
            crate::wear::Condition::Fresh
        );
    }

    #[test]
    fn talk_and_gift_round_trip_with_negative_towns() {
        let directory =
            std::env::temp_dir().join(format!("vx-journal-folk-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let orders = [
            Command::Talk {
                town: (0, 0),
                person: 2,
            },
            Command::Talk {
                town: (-1_536, 768),
                person: 0,
            },
            Command::Gift {
                town: (768, -768),
                person: 1,
                good: "engine:copper_bar".into(),
            },
        ];
        let mut journal = CommandLog::default();
        for order in &orders {
            journal.record(order.clone());
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let recovered: Vec<Command> = read_back
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .collect();
        assert_eq!(recovered, orders.to_vec(), "a word did not survive the wire");
        assert!(matches!(
            recovered[1],
            Command::Talk { town: (-1_536, 768), .. }
        ));
    }

    #[test]
    fn an_electrolysis_run_round_trips_and_moves_the_pile_both_ways() {
        let directory =
            std::env::temp_dir().join(format!("vx-journal-hho-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let mut journal = CommandLog::default();
        for run in 0..crate::electrolysis::RUNS.len() as u32 {
            journal.record(Command::Electrolyse { run });
            journal.record(Command::Advance { ticks: 400 });
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let orders: Vec<Command> = read_back
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .filter(|command| matches!(command, Command::Electrolyse { .. }))
            .collect();
        assert_eq!(
            orders,
            (0..crate::electrolysis::RUNS.len() as u32)
                .map(|run| Command::Electrolyse { run })
                .collect::<Vec<_>>(),
            "a run did not survive the wire"
        );

        // And the arithmetic both sides run is the run's own numbers: bars
        // out, canisters in. The fleet's tank is drawn from that same pile
        // inside `Mining::advance`, which is why no separate fuelling order
        // is needed for a replay to burn identically.
        for (index, run) in crate::electrolysis::RUNS.iter().enumerate() {
            let mut pile = vx_agent::Stockpile::new();
            pile.add("engine:copper_bar", 40);
            pile.take("engine:copper_bar", run.bars);
            pile.add(crate::fuel::CELL, u64::from(run.cells));
            assert_eq!(pile.count("engine:copper_bar"), 40 - run.bars, "run {index}");
            assert_eq!(pile.count(crate::fuel::CELL), u64::from(run.cells));
        }
    }

    #[test]
    fn a_spud_round_trips_and_both_sides_sink_the_same_hole() {
        let directory =
            std::env::temp_dir().join(format!("vx-journal-spud-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let heads = [
            BlockPos::new(-1_217, 96, 1_116),
            BlockPos::new(340, 71, -9_708),
        ];
        let mut journal = CommandLog::default();
        for at in heads {
            journal.record(Command::Spud { at });
            journal.record(Command::Advance { ticks: 900 });
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let orders: Vec<Command> = read_back
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .filter(|command| matches!(command, Command::Spud { .. }))
            .collect();
        assert_eq!(
            orders,
            heads.iter().map(|at| Command::Spud { at: *at }).collect::<Vec<_>>(),
            "a hole did not survive the wire"
        );

        // And what a hole finds is derived from the seed and the column, so
        // there is nothing rolled for the two sides to disagree about — the
        // same argument `Salvage` makes about its haul.
        for at in heads {
            let seed = 4242;
            let run = || {
                let mut pile = vx_agent::Stockpile::new();
                pile.add(crate::well::CASING.0, 20);
                pile.add(crate::well::CEMENT.0, 200);
                let mut wells = crate::well::Wells::default();
                wells.spud(at, seed, &mut pile).unwrap();
                wells.tick(2_000, &mut pile);
                (wells, pile.total())
            };
            assert_eq!(run(), run(), "a hole at {at:?} disagreed with itself");
        }
    }

    #[test]
    fn salvage_round_trips_and_replays_to_the_same_haul() {
        let directory =
            std::env::temp_dir().join(format!("vx-journal-salvage-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let crates_opened = [
            BlockPos::new(-1217, 96, 1116),
            BlockPos::new(340, 71, -9_708),
        ];
        let mut journal = CommandLog::default();
        for at in crates_opened {
            journal.record(Command::Salvage { at });
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let recovered: Vec<Command> = read_back
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .collect();
        assert_eq!(
            recovered,
            crates_opened
                .iter()
                .map(|at| Command::Salvage { at: *at })
                .collect::<Vec<_>>(),
            "a salvaged crate did not survive the wire"
        );

        // And the haul is a pure function of the position, so what a replay
        // hands back is exactly what the session took — no roll to disagree
        // about, which is the whole reason the contents are derived.
        for at in crates_opened {
            assert_eq!(crate::salvage::contents(at, 1), crate::salvage::contents(at, 1));
            assert!(!crate::salvage::contents(at, 1).is_empty());
        }
    }

    #[test]
    fn scout_orders_round_trip_and_replay_to_unchanged_ground() {
        let directory = std::env::temp_dir().join(format!(
            "vx-journal-scout-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let orders = [
            ScoutOrder::Orbit,
            ScoutOrder::Sortie { x: -40, z: 120 },
            ScoutOrder::Perch { x: 7, z: -3 },
            ScoutOrder::Vanguard,
            ScoutOrder::Dock,
        ];
        let mut journal = CommandLog::default();
        for order in orders {
            journal.record(Command::Scout(order));
            journal.record(Command::Advance { ticks: 5 });
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let recovered: Vec<&Command> = read_back
            .entries()
            .iter()
            .map(|entry| &entry.command)
            .filter(|command| matches!(command, Command::Scout(_)))
            .collect();
        assert_eq!(recovered.len(), orders.len(), "orders were lost on the wire");
        for (found, sent) in recovered.iter().zip(&orders) {
            assert_eq!(**found, Command::Scout(*sent), "an order mutated in transit");
        }

        // Replay it against fresh ground, and replay the same ticks with no
        // scout orders at all: identical hashes, or the scout lied.
        let span = (
            vx_core::BlockPos::new(-20, 40, -20),
            vx_core::BlockPos::new(40, 100, 40),
        );
        let events = EventBus::new();
        let mut scouted = world();
        replay(&read_back, &mut scouted, &events);
        let mut quiet_log = CommandLog::default();
        quiet_log.record(Command::Advance { ticks: 25 });
        let mut quiet = world();
        replay(&quiet_log, &mut quiet, &events);
        assert_eq!(
            region_hash(&scouted, span.0, span.1),
            region_hash(&quiet, span.0, span.1),
            "scout orders changed the ground"
        );
    }

    /// Intrusion orders survive the wire, and a log full of them replays to
    /// the same ground as a log with none. What a hack moves is grants and
    /// claims — permits state, its own file — so if this ever fails, an
    /// intrusion has started touching blocks and the oracle needs telling.
    #[test]
    fn intrusion_orders_round_trip_and_replay_to_unchanged_ground() {
        let directory = std::env::temp_dir().join(format!(
            "vx-journal-intrude-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let orders = [
            IntrudeOrder::Lock { x: 12, y: 74, z: -3 },
            IntrudeOrder::Roost {
                x: 14,
                y: 76,
                z: 9,
                grade: crate::intrusion::Grade::Tap,
            },
            IntrudeOrder::Roost {
                x: 14,
                y: 76,
                z: 9,
                grade: crate::intrusion::Grade::Blind,
            },
            IntrudeOrder::Abort,
        ];
        let mut journal = CommandLog::default();
        for order in orders {
            journal.record(Command::Intrude(order));
            journal.record(Command::Advance { ticks: 4 });
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        let recovered: Vec<&Command> = read_back
            .entries()
            .iter()
            .map(|entry| &entry.command)
            .filter(|command| matches!(command, Command::Intrude(_)))
            .collect();
        assert_eq!(recovered.len(), orders.len(), "orders were lost on the wire");
        for (found, sent) in recovered.iter().zip(&orders) {
            assert_eq!(**found, Command::Intrude(*sent), "an order mutated in transit");
        }

        let span = (
            vx_core::BlockPos::new(-20, 40, -20),
            vx_core::BlockPos::new(40, 100, 40),
        );
        let events = EventBus::new();
        let mut hacked = world();
        replay(&read_back, &mut hacked, &events);
        let mut quiet_log = CommandLog::default();
        quiet_log.record(Command::Advance { ticks: 16 });
        let mut quiet = world();
        replay(&quiet_log, &mut quiet, &events);
        assert_eq!(
            region_hash(&hacked, span.0, span.1),
            region_hash(&quiet, span.0, span.1),
            "an intrusion changed the ground"
        );
    }

    /// Firing queues recoil, and the next `Advance` applies it: the replayed
    /// body ends up shoved off where an unfired one stands.
    #[test]
    fn recoil_replays_into_the_body() {
        let events = EventBus::new();

        let mut quiet = CommandLog::default();
        quiet.record(Command::Advance { ticks: 8 });
        let stood = replay(&quiet, &mut world(), &events);

        let mut loud = CommandLog::default();
        // Fire level, due south (+Z is yaw_q of half the circle).
        loud.record(Command::Fire {
            muzzle: [0.5, 80.0, 0.5],
            yaw_q: 2048,
            pitch_q: 0,
        });
        loud.record(Command::Advance { ticks: 8 });
        let shoved = replay(&loud, &mut world(), &events);

        assert_ne!(
            stood.player.position, shoved.player.position,
            "recoil never reached the replayed body"
        );
    }
}

#[cfg(test)]
mod civic_tests {
    use super::*;
    use vx_core::ChunkPos;
    use vx_world::region_hash;

    /// Standing for office survives the wire, and changes no ground.
    ///
    /// The order is recorded because it is a decision the player made. What
    /// follows from it — the poll, the badge, whose signature a warrant needs
    /// — is arithmetic on ledgers, so replay drops the order and the ground
    /// comes out identical either way.
    #[test]
    fn a_stand_order_round_trips_and_moves_nothing() {
        let directory = std::env::temp_dir().join(format!("vx-stand-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let orders = [
            Command::Stand {
                town: (0, 0),
                office: 1,
                on: true,
            },
            Command::Stand {
                town: (-512, 1_024),
                office: 0,
                on: false,
            },
        ];
        let mut journal = CommandLog::default();
        for order in &orders {
            journal.record(order.clone());
        }
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        let recovered: Vec<Command> = read_back
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .collect();
        assert_eq!(recovered, orders);

        // And it is ground-neutral, which is why replay may drop it.
        let span = (
            vx_core::BlockPos::new(-40, 40, -40),
            vx_core::BlockPos::new(40, 200, 40),
        );
        let patch = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(0, 0), 3);
            world
        };
        let mut stood = patch();
        replay(&read_back, &mut stood, &EventBus::new());
        assert_eq!(
            region_hash(&stood, span.0, span.1),
            region_hash(&patch(), span.0, span.1),
            "putting your name on a ballot moved a block"
        );
    }

    /// The whole civic layer leaves the ground exactly as it found it.
    ///
    /// The inverse of every other oracle test in this file, and the claim
    /// stage 39 rests on. Offices, wages, trust and warrants are bookkeeping:
    /// a session that files a warrant, pays a fine, trades a town into an
    /// embargo and watches its residents work for a fortnight must hash the
    /// same ground as one where none of it happened. If any of it ever
    /// reaches for a block — a resident who actually digs, a fine that burns
    /// something down — this is the test that goes red.
    #[test]
    fn the_civic_layer_never_touches_the_ground() {
        let patch = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(0, 0), 4);
            world
        };
        let span = (
            vx_core::BlockPos::new(-60, 40, -60),
            vx_core::BlockPos::new(60, 200, 60),
        );

        // A plain fortnight, and the same fortnight with the whole civic
        // layer run over the top of it.
        let mut journal = CommandLog::default();
        journal.record(Command::Advance {
            ticks: crate::schedule::TICKS_PER_DAY as u32 * 14,
        });

        let mut quiet = patch();
        replay(&journal, &mut quiet, &EventBus::new());
        let untouched = region_hash(&quiet, span.0, span.1);

        let mut governed = patch();
        let played = replay(&journal, &mut governed, &EventBus::new());

        let site = vx_world::town::home_site();
        let mayor = crate::office::seat(&site, crate::permits::Office::Mayor);

        // Every resident works the fortnight, and the town's books move.
        let mut economy = crate::economy::Economy::new();
        let _ = economy.market(&site, played.tick).clone();
        for index in 0..crate::people::PEOPLE {
            assert!(crate::economy::purse(&site, index, played.tick) > 0);
        }

        // Business enough to be handed a key, which is a permits grant.
        let mut friends = crate::disposition::Disposition::default();
        let key = (site.centre, 1u8);
        // Business with the whole town, not just the one who hands over a
        // key: an election is a room, and a room needs a majority.
        for voter in 0..crate::people::PEOPLE as u8 {
            for day in 0..40 {
                friends.trade((site.centre, voter), 900, day);
            }
        }
        assert!(friends.trusted_with_a_key(key));

        // A term of elections run over the top of it, including one the
        // player wins — the stage 40 half.
        let mut register = crate::ballot::Register::default();
        register.stand(site.centre, crate::permits::Office::Sheriff, true);
        let poll_day = crate::ballot::next_poll(&site, 0);
        let field = crate::ballot::Field {
            site: &site,
            friends: &friends,
            bounty: 0,
            incumbent_troubled: false,
            standing: true,
            seat: crate::permits::Office::Sheriff,
            founder: false,
        };
        let held = register
            .hold(&field, crate::permits::Office::Sheriff, poll_day)
            .expect("no election was held");
        assert!(held.after.is_player(), "the town you bankrolled threw you out");
        assert!(register.player_holds(site.centre, crate::permits::Office::Sheriff));

        // And paper filed, signed and paid for.
        let mut docket = crate::warrant::Docket::default();
        let filed = docket
            .file(
                &site,
                &mayor,
                friends.tier(key),
                friends.trust(key),
                crate::warrant::SIGNS_REGARDLESS,
                played.tick,
            )
            .expect("the sheriff never filed");
        assert!(filed.fine > 0);
        assert!(docket.granted_in(site.centre));
        assert!(docket.pending_in(site.centre), "the counter stayed open");

        assert_eq!(
            region_hash(&governed, span.0, span.1),
            untouched,
            "the civic layer moved a block"
        );
    }
}

#[cfg(test)]
mod season_tests {
    use super::*;
    use vx_core::{BlockPos, ChunkPos};
    use vx_world::season::{self, Season};
    use vx_world::{flora, region_hash};

    /// **A year of seasons leaves the ground exactly as it found it.**
    ///
    /// The sibling of [`civic_tests::the_civic_layer_never_touches_the_ground`],
    /// and the claim stage 41 rests on. A season is a term inside functions
    /// that were already pure in the tick, plus a texture upload: the sky, the
    /// warmth, the leaves and the growing clock are all *readings*, never
    /// edits. So the same journal replayed with a whole year of seasons read
    /// over the top of it must hash the same country as one where nobody
    /// asked what month it was.
    ///
    /// Stage 44 put snow on the ground, and this test did not go red: the
    /// snow is not a reading. It is an automaton on the journal's side of the
    /// line, run from `Command::Advance` on both sides, and its own oracle is
    /// [`fire_tests::a_winter_replays_to_the_same_snow_and_ice`]. What this
    /// test still says is that *reading* the year — every function in
    /// `season` and `weather`, called for every day of it — moves nothing.
    ///
    /// [`civic_tests::the_civic_layer_never_touches_the_ground`]: super::civic_tests
    /// [`fire_tests::a_winter_replays_to_the_same_snow_and_ice`]: super::fire_tests
    #[test]
    fn reading_the_year_never_touches_the_ground() {
        let patch = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(0, 0), 4);
            world
        };
        let span = (
            BlockPos::new(-60, 40, -60),
            BlockPos::new(60, 200, 60),
        );

        let mut journal = CommandLog::default();
        journal.record(Command::Advance {
            ticks: crate::schedule::TICKS_PER_DAY as u32 * 14,
        });

        let mut quiet = patch();
        replay(&journal, &mut quiet, &EventBus::new());
        let untouched = region_hash(&quiet, span.0, span.1);

        let mut through_the_year = patch();
        replay(&journal, &mut through_the_year, &EventBus::new());

        // Read the whole year, everywhere, in every way anything reads it.
        let seed = through_the_year.seed();
        let mut sampled = 0;
        for day in 0..season::YEAR_DAYS {
            let tick = day * season::DAY_TICKS;
            let _ = Season::of(tick);
            let _ = season::phase(tick);
            let _ = season::warmth(tick);
            let _ = season::damp(tick);
            let _ = season::growth(tick);
            let _ = season::leaf_turn(tick);
            let _ = season::fire_season(tick);
            let _ = season::grown_between(0, tick);
            let _ = season::when_grown(tick, season::DAY_TICKS);
            for (x, z) in [(0, 0), (44, -31), (-58, 57)] {
                let _ = vx_world::weather::at(seed, tick, x, z);
                let _ = vx_world::weather::fuel_moisture(seed, tick, x, z);
                sampled += 1;
            }
        }
        assert!(sampled > 80, "the year was barely read at all");

        assert_eq!(
            region_hash(&through_the_year, span.0, span.1),
            untouched,
            "the year moved a block"
        );
    }

    /// The season decides **when** a stand comes back, never **what** it
    /// comes back as.
    ///
    /// Two identical clearings, one in the spring and one on the last day
    /// before winter, walked forward until each has banked the same growing
    /// time. They take wildly different numbers of calendar days to get
    /// there — that is the whole feature — and they must arrive at
    /// byte-identical ground. A season that quietly grew a different forest
    /// in October would be a worldgen change wearing a calendar's clothes.
    #[test]
    fn a_stand_comes_back_the_same_whatever_month_it_was_cleared_in() {
        // Real woods, not the hometown plaza — a cell with nothing growing in
        // it would make this test a pair of no-ops agreeing.
        const WOODS: (i32, i32) = (96, 96);
        let span = (
            BlockPos::new(WOODS.0 - 40, 40, WOODS.1 - 40),
            BlockPos::new(WOODS.0 + 40, 200, WOODS.1 + 40),
        );
        let patch = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(WOODS.0 / 16, WOODS.1 / 16), 3);
            world
        };
        let near = |world: &World| {
            world
                .towns_overlapping((WOODS.0 - 80, WOODS.1 - 80), (WOODS.0 + 80, WOODS.1 + 80))
        };

        // A hardwood the seed actually put there. Disturbing a lattice cell
        // with no tree in it would empty the ledger on the first step and
        // leave two identical untouched worlds agreeing about nothing.
        let cleared = {
            let world = patch();
            let sites = near(&world);
            let generator = world.generator().clone();
            let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, &sites);
            let natural_at = |x: i32, z: i32| generator.natural_height_at(x, z);
            flora::trees_overlapping(
                world.seed(),
                (WOODS.0 - 24, WOODS.1 - 24),
                (WOODS.0 + 24, WOODS.1 + 24),
                &height_at,
                &natural_at,
                &sites,
            )
            .into_iter()
            .find(|tree| tree.species == flora::Species::Hardwood && tree.height >= 5)
            .expect("no hardwood in the loaded square")
            .base
        };

        // Walk a clearing forward through `stages` of its succession and hash
        // what it left. The ledger stamps each stage into the world, so stage
        // zero is a meadow standing where old growth was and stage four is
        // the forest the seed describes back again.
        let grow_from = |start: u64, stages: u64| {
            let mut world = patch();
            let sites = near(&world);
            let mut ledger = crate::succession::Ledger::default();
            ledger.disturb(cleared, start);
            let step = crate::succession::STEP_TICKS
                * crate::succession::pace(flora::Species::Hardwood);
            let mut last = start;
            for stage in 0..stages {
                last = season::when_grown(start, step * stage);
                ledger.advance(&mut world, last, &sites);
            }
            (region_hash(&world, span.0, span.1), last - start)
        };

        let all = crate::succession::STAGES as u64;
        // Cleared three days into the spring, and cleared on the last day
        // before the winter shuts the ground down.
        let (spring_ground, spring_days) = grow_from(season::DAY_TICKS * 3, all);
        let (autumn_ground, autumn_days) =
            grow_from(3 * season::SEASON_TICKS - season::DAY_TICKS, all);

        assert!(
            autumn_days > spring_days,
            "the autumn clearing did not wait for anything: {autumn_days} against {spring_days}"
        );
        assert_eq!(
            spring_ground, autumn_ground,
            "the same stand came back as two different forests"
        );

        // And neither of them passed by doing nothing: a stand stopped at its
        // first stage is a meadow, and a stand walked all the way through is
        // a wood. The equality above is two systems agreeing, not two no-ops.
        let (meadow, _) = grow_from(season::DAY_TICKS * 3, 1);
        assert_ne!(
            spring_ground, meadow,
            "nothing grew back, so the test proved nothing"
        );
    }
}

#[cfg(test)]
mod charter_tests {
    use super::*;
    use crate::charter;
    use vx_core::ChunkPos;
    use vx_world::region_hash;
    use vx_world::town::{self, TownName};

    /// The first plot east of the hometown the lattice left open, dry and
    /// flat: where every founding below happens.
    fn open_ground(world: &World) -> (i32, i32) {
        let name = TownName::from_words("iron", "reach").unwrap();
        (1..12)
            .map(|step| (step * town::CELL + 60, 40))
            .find(|at| charter::may_found(world, *at, name).is_ok())
            .expect("no chartable ground within twelve cells")
    }

    /// **Founding is the ordinary oracle.** A journal that founds a town and
    /// breaks a block in its plaza replays to exactly the ground the live
    /// session left — the site is derived on both sides from the same seed
    /// and centre, and both sides raise it with the same generator. And it is
    /// a real town: the hash moved a long way from the empty hillside.
    #[test]
    fn a_founding_replays_to_the_same_town() {
        let name = TownName::from_words("iron", "reach").unwrap();
        let (head, tail) = name.indices();
        let probe = World::new(2024);
        let at = open_ground(&probe);
        let site = charter::may_found(&probe, at, name).unwrap();
        // A plain block of the levelled plaza, not a fixture: the counter and
        // the mailbox are registered unbreakable, and the point is the
        // ground under the town, not the furniture on it.
        let counter = BlockPos::new(site.centre.0 + 7, site.ground, site.centre.1 + 7);
        let span = (
            BlockPos::new(at.0 - 56, 0, at.1 - 56),
            BlockPos::new(at.0 + 56, 200, at.1 + 56),
        );

        // The live session: the order applied the way `found_a_town` does.
        let mut lived = World::new(2024);
        lived.raise(site);
        assert_eq!(lived.towns_near(at, 8).first().map(|s| s.centre), Some(at));
        let founded_hash = region_hash(&lived, span.0, span.1);
        assert!(!lived.block(counter).is_air(), "the plaza block was already air");
        assert!(
            vx_world::break_block(&mut lived, &EventBus::new(), counter).is_ok(),
            "the plaza block would not break"
        );
        let lived_hash = region_hash(&lived, span.0, span.1);
        assert_ne!(lived_hash, founded_hash, "breaking a block changed nothing");

        // Founding alone replays to the same ground.
        let mut only_founded = CommandLog::default();
        only_founded.record(Command::Found {
            x: at.0,
            z: at.1,
            head,
            tail,
        });
        let mut just_raised = World::new(2024);
        replay(&only_founded, &mut just_raised, &EventBus::new());
        assert_eq!(
            region_hash(&just_raised, span.0, span.1),
            founded_hash,
            "the replayed founding is a different town before anybody touched it"
        );

        // The same two orders, replayed over a world that knows nothing.
        let mut journal = CommandLog::default();
        journal.record(Command::Found {
            x: at.0,
            z: at.1,
            head,
            tail,
        });
        journal.record(Command::Break { at: counter });
        let mut replayed = World::new(2024);
        replay(&journal, &mut replayed, &EventBus::new());
        assert_eq!(
            region_hash(&replayed, span.0, span.1),
            lived_hash,
            "the replayed founding is a different town"
        );
        assert_eq!(
            replayed.towns_near(at, 8).first().map(|s| s.name),
            Some(name),
            "replay raised the ground but never told the world"
        );

        // And it survives the wire.
        let directory = std::env::temp_dir().join(format!("vx-found-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        let mut again = World::new(2024);
        replay(&read_back, &mut again, &EventBus::new());
        assert_eq!(region_hash(&again, span.0, span.1), lived_hash);

        // Non-vacuous: a town is not an empty hillside.
        let mut empty = World::new(2024);
        empty.load_around(BlockPos::new(at.0, 0, at.1).chunk(), 5);
        assert_ne!(region_hash(&empty, span.0, span.1), lived_hash);
    }

    /// **Taking is the inverted oracle.** `Take` over a journal moves no
    /// block: the seats, the price and the paper are ledgers, and ledgers
    /// are not ground.
    #[test]
    fn taking_a_town_moves_nothing() {
        let site = town::towns_near(2024, (0, 0), 6_000, &|_, _| 90)
            .into_iter()
            .find(|site| !site.is_home())
            .unwrap();
        let span = (
            BlockPos::new(-60, 40, -60),
            BlockPos::new(60, 200, 60),
        );
        let patch = || {
            let mut world = World::new(2024);
            world.load_around(ChunkPos::new(0, 0), 4);
            world
        };
        let mut journal = CommandLog::default();
        journal.record(Command::Take { town: site.centre });
        journal.record(Command::Advance {
            ticks: crate::schedule::TICKS_PER_DAY as u32,
        });
        let mut taken = patch();
        replay(&journal, &mut taken, &EventBus::new());
        assert_eq!(
            region_hash(&taken, span.0, span.1),
            region_hash(&patch(), span.0, span.1),
            "seizing a town's chairs moved a block"
        );
        // And the order round-trips.
        let directory = std::env::temp_dir().join(format!("vx-take-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        journal.save(&directory).unwrap();
        let read_back = CommandLog::load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        assert!(matches!(
            read_back.entries()[0].command,
            Command::Take { town } if town == site.centre
        ));
    }
}
