//! The game binary.
//!
//! Wires the world, chunk streaming, renderer and window together and runs the
//! frame loop. Two modes:
//!
//! - default: open a window and play.
//! - `--screenshot <path>`: render one frame offscreen and exit. Needs no
//!   display, so it works over SSH, in CI, and against a software Vulkan
//!   driver — and is how the whole stack gets smoke-tested without a GPU.

mod afoot;
mod arcade;
mod arsenal;
mod audio;
mod awareness;
mod ballot;
mod charter;
mod bank;
mod beacon;
mod belief;
mod board;
mod clinic;
mod clock;
mod controller;
mod debug;
mod device;
mod disposition;
mod drill;
mod dose;
mod economy;
mod felling;
mod fire;
mod frost;
mod garage;
mod garrison;
#[cfg(feature = "gold")]
mod gold;
mod health;
mod homestead;
mod hostile;
mod hud;
mod intro;
mod intrusion;
mod journal;
mod map;
mod mining;
mod people;
mod permits;
mod pile;
mod electrolysis;
mod fuel;
mod movement;
mod office;
mod optics;
mod osk;
mod printer;
mod rain;
mod reputation;
mod rig;
mod salvage;
mod gamepad;
mod schedule;
mod session;
mod roost;
mod scout;
mod shop;
mod skills;
mod stalker;
mod streaming;
mod succession;
mod terminal;
mod tuning;
mod view;
mod villagers;
mod wear;
mod well;
mod wallet;
mod warrant;

use std::sync::Arc;
use std::time::Instant;

use controller::{FlyController, MovementMode, WalkController};
use map::MapState;
use rig::Rig;
use skills::Skills;
use view::ViewMode;
use clock::TimeOfDay;
use journal::{Command, CommandLog};
use villagers::Villagers;
use wallet::Wallet;

/// Overlay slot assignments: the minimap, the HUD, and the modal panels.
const MAP_SLOT: usize = 0;
const HUD_SLOT: usize = 1;
const SHOP_SLOT: usize = 2;
const DEVICE_SLOT: usize = 3;
const FEED_SLOT: usize = 4;
const BOARD_SLOT: usize = 5;
const HOME_SLOT: usize = 6;
const INTRO_SLOT: usize = 7;
const PERMIT_SLOT: usize = 8;
#[cfg(feature = "gold")]
const GOLD_SLOT: usize = 9;
const PRINT_SLOT: usize = 10;
/// The electrolyser's panel.
const FUEL_SLOT: usize = 11;
/// The bank's ledger.
const VAULT_SLOT: usize = 12;
/// The terminal.
const TERM_SLOT: usize = 13;
/// The controller help overlay.
const PAD_SLOT: usize = 14;
/// The F3 diagnostics readout.
const DEBUG_SLOT: usize = 15;
/// The wellhead's panel.
const WELL_SLOT: usize = 16;
/// The clinic's ward panel.
const WARD_SLOT: usize = 17;
/// The on-screen keyboard, over the terminal it types into.
const OSK_SLOT: usize = 18;
use mining::Mining;
use streaming::{chunk_at, ChunkStreamer, StreamingConfig};

use vx_core::{BlockId, EventBus};
use vx_platform::InputState;
use vx_render::headless::{capture_frame, CAPTURE_FORMAT};
use vx_render::{Camera, GpuContext, Renderer, WindowSurface};
use vx_agent::{MineMethod, Operation, VoxelAabb};
use vx_world::{break_block, place_block, raycast_solid, PlayerBody, World, WorldSave};

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

const DEFAULT_SEED: u64 = 2024;

/// How far a town's mast can hear other towns, in blocks. Sets how far away a
/// posting can send you, and costs nothing to widen — the towns inside it are
/// derived, not loaded.
const RADIO_RANGE: i32 = 2_000;

/// An open beacon console: the town, the work it is broadcasting, and the
/// trade runs it will pay somebody to carry.
type Console = (
    vx_world::town::TownSite,
    Vec<beacon::Posting>,
    Vec<board::Run>,
);

/// How near a trade load has to pass before it is drawn as a real machine.
const CARAVAN_SIGHT: f32 = 220.0;

/// How high above the ground a load flies.
const CARAVAN_ALTITUDE: f32 = 26.0;

/// Re-run town discovery once the player has moved this far since the last
/// check. The lattice is cheap, but not free, and nobody discovers a town by
/// standing still.
const DISCOVERY_STEP: i32 = 8;

/// Command line options.
struct Options {
    seed: u64,
    screenshot: Option<String>,
    width: u32,
    height: u32,
    /// Save directory name, under the platform data directory.
    world: String,
    /// World position to place the camera for a screenshot.
    at: (i32, i32),
    /// Find an ore body near `at`, mine it, and screenshot the result.
    dig: Option<Dig>,
    /// Drone ticks to run before capturing, when digging.
    ticks: u64,
    /// Frame the first drone up close instead of the whole excavation.
    close: bool,
    /// Draw the supply shop panel open over the capture, stocked for show.
    shop: bool,
    /// Draw the beacon console open over the capture, with work posted.
    board: bool,
    /// Replay the saved world's command journal and report the ground it
    /// rebuilds, instead of opening a window.
    replay: bool,
    /// Drones to grant for free at startup. A development override: in a real
    /// game you buy them, which is the point of the garage.
    drones: u32,
    /// Frame the capture over the player's shoulder, body in shot.
    third_person: bool,
    /// Draw the handheld's fleet roster over the capture.
    device: bool,
    /// Show the handheld's map page rather than its roster.
    handheld_map: bool,
    /// Hour of the in-game day to light the capture by, 0..1.
    time: f32,
    /// Chunks visible in every direction, when playing windowed.
    view_distance: i32,
    /// Open the operator's console. Only present in builds carrying the
    /// `gold` feature; the shipped build compiles the panel out entirely.
    gold: bool,
    /// Pin the sheriff's badge on the player. A development override, like
    /// `--drones`: the badge is won at the ballot box in stage 13, and this is
    /// how the override is exercised before then.
    sheriff: bool,
    /// Print the derived changelog and exit.
    changelog: bool,
    /// Draw the welcome panel over the capture.
    welcome: bool,
    /// Draw a neighbour's lockbox panel over the capture.
    permit: bool,
    /// Draw the operator's console over the capture (gold builds only).
    gold_capture: bool,
    /// Hold the slug launcher in the capture instead of the drill.
    launcher: bool,
    /// Stand a fabricator in the frame and open its panel mid-print.
    fab: bool,
    /// Frame the capture from inside the largest cave pocket near `--at`.
    cave: bool,
    /// See the capture through the optics kit: lamp, nvg or thermal.
    optic: Option<String>,
    /// Frame the capture on the nearest bunker to `--at`: its hatch by
    /// default, a named room inside it with `--close`.
    bunker: bool,
    /// Stand an electrolyser on the nearest shore and open its panel.
    hho: bool,
    /// Frame the nearest walled town from above its trace.
    fort: bool,
    /// Stand at a town's vault box with the ledger open.
    vault: bool,
    /// Cut the ground away beside a building to show its footing in section.
    footing: bool,
    /// Play a whole loop headlessly — leave the house, walk to the ore, cut it
    /// out, carry it home and sell it — photographing each beat.
    play: bool,
    /// Look through a machine's gimbal, and at where it hangs.
    gimbal: bool,
    /// Scout, collect, save, load, and haul the goods to another town.
    haul: bool,
    /// Draw the terminal over the capture, with a session's worth of log.
    terminal: bool,
    /// Market day in the hometown, with the roster and a word on the
    /// terminal — every line generated by the real people systems.
    people: bool,
    /// Draw the controller scheme overlay, exactly as SELECT shows it.
    pad: bool,
    /// The terminal's `kit` listing: every upgrade line and what is fitted.
    kit: bool,
    /// A worn crew on the terminal: the roster with conditions, and a mend.
    wear: bool,
    /// Chew a wall in front of the camera, to show cells rather than boxes.
    wound: bool,
    /// A warrant posse closing on the player, mid-callout.
    posse: bool,
    /// A held shelter: its garrison on the hatch, mid-search.
    held: bool,
    /// The standing sheet after a raiding career, on the terminal.
    standing: bool,
    /// The F3 diagnostics readout, filled with a busy session.
    debug: bool,
    /// A wellhead panel over a live field, mid-drill.
    well: bool,
    /// A uranium face, cut open, with the dose climbing.
    hot: bool,
    /// The thing in the deep, mid-hunt, in a gallery.
    dark: bool,
    /// The townsfolk close up, with their eyes on the camera.
    faces: bool,
    /// A hamlet inside its mini star, from overhead.
    ministar: bool,
    /// The ward, with its panel open over the cots.
    ward: bool,
    /// Catch the handheld mid-swing rather than fully up.
    raising: bool,
    arcade: bool,
    forest: Option<String>,
    fell: Option<String>,
    flood: Option<String>,
    storm: bool,
    fire: Option<String>,
    /// Which season to paint the country in: spring, summer, autumn, winter.
    season: Option<String>,
    /// The on-screen keyboard, over the terminal it types into.
    osk: bool,
    /// Put real paperwork on the beacon panel's civic block.
    warrant: bool,
    /// Turn the console to its ballot page.
    ballot: bool,
    /// The ballot page of a town that has already elected you.
    elected: bool,
    /// The console of a town whose chairs you seized.
    taken: bool,
    /// Found a town at `--at` and frame it from outside its wall.
    founded: bool,
    /// The first snow of the winter over a lake shore near `--at`.
    frost: bool,
    /// The same shore, part-way through the thaw that follows.
    thaw: bool,
}

/// Which method a `--dig` run should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dig {
    /// Whatever the ranking recommends.
    Proposed,
    Forced(MineMethod),
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        seed: DEFAULT_SEED,
        screenshot: None,
        width: 1280,
        height: 720,
        world: "world".to_string(),
        view_distance: 8,
        gold: false,
        sheriff: false,
        changelog: false,
        welcome: false,
        permit: false,
        gold_capture: false,
        launcher: false,
        fab: false,
        cave: false,
        optic: None,
        bunker: false,
        hho: false,
        fort: false,
        vault: false,
        footing: false,
        play: false,
        gimbal: false,
        haul: false,
        terminal: false,
        people: false,
        pad: false,
        kit: false,
        wear: false,
        wound: false,
        posse: false,
        held: false,
        standing: false,
        debug: false,
        well: false,
        hot: false,
        dark: false,
        faces: false,
        ministar: false,
        ward: false,
        raising: false,
        arcade: false,
        forest: None,
        fell: None,
        flood: None,
        storm: false,
        fire: None,
        season: None,
        osk: false,
        warrant: false,
        ballot: false,
        elected: false,
        taken: false,
        founded: false,
        frost: false,
        thaw: false,
        at: (0, 0),
        dig: None,
        ticks: 20_000,
        close: false,
        shop: false,
        board: false,
        replay: false,
        drones: 0,
        third_person: false,
        device: false,
        handheld_map: false,
        time: TimeOfDay::START.fraction(),
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--seed" => {
                options.seed = value()?
                    .parse()
                    .map_err(|_| "--seed must be a number".to_string())?
            }
            "--screenshot" => options.screenshot = Some(value()?),
            "--world" => options.world = value()?,
            "--at" => {
                let text = value()?;
                let (x, z) = text
                    .split_once(',')
                    .ok_or_else(|| "--at wants X,Z".to_string())?;
                options.at = (
                    x.trim().parse().map_err(|_| "--at X must be a number".to_string())?,
                    z.trim().parse().map_err(|_| "--at Z must be a number".to_string())?,
                );
            }
            "--dig" => {
                options.dig = Some(match value()?.as_str() {
                    // `auto` takes whatever the game would propose, which is
                    // the interesting default: it shows the ranking's answer.
                    "auto" => Dig::Proposed,
                    "adit" => Dig::Forced(MineMethod::Adit),
                    "decline" => Dig::Forced(MineMethod::Decline),
                    "pit" => Dig::Forced(MineMethod::Pit),
                    other => {
                        return Err(format!(
                            "--dig wants auto, adit, decline or pit (got {other})"
                        ))
                    }
                });
            }
            "--close" => options.close = true,
            "--shop" => options.shop = true,
            "--board" => options.board = true,
            "--replay" => options.replay = true,
            "--gold" => options.gold = true,
            "--sheriff" => options.sheriff = true,
            "--changelog" => options.changelog = true,
            "--welcome" => options.welcome = true,
            "--permit" => options.permit = true,
            "--gold-capture" => options.gold_capture = true,
            "--launcher" => options.launcher = true,
            "--fab" => options.fab = true,
            "--cave" => options.cave = true,
            "--optic" => options.optic = Some(value()?),
            "--bunker" => options.bunker = true,
            "--hho" => options.hho = true,
            "--fort" => options.fort = true,
            "--vault" => options.vault = true,
            "--footing" => options.footing = true,
            "--play" => options.play = true,
            "--gimbal" => options.gimbal = true,
            "--haul" => options.haul = true,
            "--terminal" => options.terminal = true,
            "--osk" => {
                options.terminal = true;
                options.osk = true;
            }
            "--people" => options.people = true,
            "--pad" => options.pad = true,
            "--kit" => options.kit = true,
            "--wear" => options.wear = true,
            "--wound" => options.wound = true,
            "--posse" => options.posse = true,
            "--held" => options.held = true,
            "--standing" => options.standing = true,
            "--debug" => options.debug = true,
            "--well" => options.well = true,
            "--hot" => options.hot = true,
            "--dark" => options.dark = true,
            "--faces" => options.faces = true,
            "--ministar" => options.ministar = true,
            "--ward" => options.ward = true,
            "--raising" => options.raising = true,
            "--forest" => options.forest = Some(value()?),
            "--fell" => options.fell = Some(value()?),
            "--flood" => options.flood = Some(value()?),
            "--storm" => options.storm = true,
            "--season" => options.season = Some(value()?),
            // The civic panel is the beacon panel: `--town` is the plain
            // one and `--warrant` is the same console with paper on it.
            "--town" => options.board = true,
            "--ballot" => {
                options.board = true;
                options.ballot = true;
            }
            "--taken" => {
                options.board = true;
                options.taken = true;
            }
            "--founded" => options.founded = true,
            "--frost" => options.frost = true,
            "--thaw" => options.thaw = true,
            "--elected" => {
                options.board = true;
                options.elected = true;
            }
            "--warrant" => {
                options.board = true;
                options.warrant = true;
            }
            "--fire" => options.fire = Some(value()?),
            "--arcade" => {
                options.device = true;
                options.arcade = true;
            }
            "--drones" => {
                options.drones = value()?
                    .parse()
                    .map_err(|_| "--drones must be a number".to_string())?
            }
            "--third-person" => options.third_person = true,
            "--device" => options.device = true,
            "--handheld-map" => {
                options.device = true;
                options.handheld_map = true;
            }
            "--night" => options.time = TimeOfDay::MIDNIGHT.fraction(),
            "--dawn" => options.time = TimeOfDay::DAWN.fraction(),
            "--noon" => options.time = TimeOfDay::NOON.fraction(),
            "--dusk" => options.time = TimeOfDay::DUSK.fraction(),
            "--time" => {
                options.time = value()?
                    .parse()
                    .map_err(|_| "--time wants a fraction of a day, 0..1".to_string())?
            }
            "--ticks" => {
                options.ticks = value()?
                    .parse()
                    .map_err(|_| "--ticks must be a number".to_string())?
            }
            "--view-distance" => {
                let distance: i32 = value()?
                    .parse()
                    .map_err(|_| "--view-distance must be a number".to_string())?;
                // Below four the world ends at your nose; past sixteen the
                // budget maths stops being honest about frame cost.
                options.view_distance = distance.clamp(4, 16);
            }
            "--width" => {
                options.width = value()?
                    .parse()
                    .map_err(|_| "--width must be a number".to_string())?
            }
            "--height" => {
                options.height = value()?
                    .parse()
                    .map_err(|_| "--height must be a number".to_string())?
            }
            "--help" | "-h" => {
                println!(
                    "gamingg\n\n\
                     Options:\n  \
                     --seed <n>          world seed (default {DEFAULT_SEED})\n  \
                     --world <name>      save directory name (default \"world\")\n  \
                     --screenshot <path> render one frame to a PPM file and exit\n  \
                     --at <x,z>          world position to view from (screenshot)\n  \
                     --dig <method>      find ore near --at and mine it before capturing\n  \
                                         (auto, adit, decline or pit)\n  \
                     --close             frame the first drone up close (with --dig)\n  \
                     --shop              draw the supply shop panel over the capture\n  \
                     --board             draw the beacon console over the capture\n  \
                     --drones <n>        grant this many drones for free (default 0;\n  \
                                         normally you buy them at the shop)\n  \
                     --replay            replay the saved world's journal and report the\n  \
                                         ground it rebuilds, then exit\n  \
                     --third-person      frame over the player's shoulder, body in shot\n  \
                     --device            draw the handheld fleet roster over the capture\n  \
                     --handheld-map      draw the handheld's map page instead\n  \
                     --arcade            play the pocket arcade on the raised handheld\n  \
                     --forest <which>    frame a stand of bog, cove or high forest\n  \
                     --fell <when>       a stem mid-arc (swing) or lying down (down)\n  \
                     --flood <when>      a gallery cut into a lake (cut) or settled (level)\n  \
                     --storm             rain over the country with the sky down\n  \
                     --season <name>     spring, summer, autumn or winter\n  \
                     --town              the beacon console, with who runs the place\n  \
                     --warrant           the same console with a warrant standing\n  \
                     --ballot            the console's voting page, with a poll due\n  \
                     --elected           the voting page of a town that elected you\n  \
                     --taken             the console of a town whose chairs you seized\n  \
                     --founded           found a town at --at and frame it from its wall\n  \
                     --fire <when>       a stand alight (burning) or the ash after (after)\n  \
                     --view-distance <n> chunks visible in every direction (4-16, default 8)\n  \
                     --gold              enable the operator console (F10; dev builds only)\n  \
                     --sheriff           wear the badge (dev override; won at the ballot box later)\n  \
                     --changelog         print the welcome panel's changelog and exit\n  \
                     --welcome           draw the welcome panel over the capture\n  \
                     --permit            draw a lockbox panel over the capture\n  \
                     --gold-capture      draw the operator console over the capture\n  \
                     --launcher          hold the slug launcher in the capture\n  \
                     --time <0..1>       hour of the day to light the capture by\n  \
                     --night             shorthand for --time 0\n  \
                     --dawn --noon --dusk\n  \
                                         the other named hours\n  \
                     --ticks <n>         drone ticks to run when digging\n  \
                     --width <n>         window or image width\n  \
                     --height <n>        window or image height"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unrecognised argument: {other}")),
        }
    }
    Ok(options)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("error: {message}\ntry --help");
            std::process::exit(2);
        }
    };

    // Asking for the console in a build that compiled it out deserves a plain
    // answer, not silence.
    #[cfg(not(feature = "gold"))]
    if options.gold {
        eprintln!("this build carries no gold panel; rebuild with --features gold");
    }

    if options.changelog {
        for line in intro::changelog() {
            println!("{line}");
        }
        return;
    }

    let result = if options.replay {
        run_replay(&options)
    } else {
        match &options.screenshot {
            Some(path) => run_screenshot(&options, path),
            None => run_windowed(&options),
        }
    };

    if let Err(message) = result {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

/// Replay a saved world's command journal and report what it rebuilds.
///
/// The determinism oracle, run from the command line. The journal records
/// *orders* — mine that area this way, run this many ticks — so replaying it
/// re-derives every block those orders produced. If the ground it rebuilds
/// disagrees with the ground on disk, something in worldgen, the agents or the
/// editing path has stopped being deterministic, and this says so.
///
/// Needs no GPU and no window, so it is what CI runs.
fn run_replay(options: &Options) -> Result<(), String> {
    let root = vx_platform::paths::saves_dir().join(&options.world);
    let save = WorldSave::new(&root);
    if !save.exists() {
        return Err(format!("no world saved at {}", root.display()));
    }
    let seed = save
        .read_meta()
        .map_err(|error| format!("could not read {}: {error}", root.display()))?;

    let journal = CommandLog::load(&root);
    if journal.is_empty() {
        println!("nothing recorded for {}", root.display());
    }
    println!(
        "replaying {} commands covering ticks {}..{} (seed {seed})",
        journal.len(),
        journal.keyframe_tick,
        journal.tick()
    );

    let mut world = World::new(seed);
    let events = EventBus::new();
    let started = Instant::now();
    let walked = journal::replay(&journal, &mut world, &events);
    let rebuilt = vx_world::world_hash(&world);

    println!(
        "rebuilt {} chunks in {:.2}s, hash {rebuilt:#018x}",
        world.loaded_chunk_count(),
        started.elapsed().as_secs_f32()
    );
    // The player's path is covered now too. The absolute start is not recorded,
    // so this is a path from the world origin rather than from wherever they
    // actually spawned — what it proves is that the same log walks the same
    // walk, which is the property the oracle is for.
    println!(
        "player path ends at {:?}, {:?}, {:.0}% wind",
        walked.player.position,
        walked.movement.stance,
        walked.movement.stamina_fraction() * 100.0
    );
    // The books are not rebuilt by a replay — the network's reach follows the
    // player, and where the player stood is not in the journal. Reported so
    // nobody mistakes an empty economy for a matching one.
    println!(
        "town books are not covered by replay (hash {:#018x} of an unrun network)",
        economy::Economy::new().books_hash()
    );

    // Only a journal that reaches all the way back to an empty world can be
    // checked against the saved ground; a tail after a keyframe describes only
    // part of it, and saying so is better than comparing the wrong things.
    if journal.keyframe_tick != 0 {
        println!("journal starts from a keyframe at tick {}; nothing to compare against", journal.keyframe_tick);
        return Ok(());
    }

    let mut saved = World::new(seed);
    for pos in world.loaded_chunks().collect::<Vec<_>>() {
        match save.load_chunk(pos, saved.registry()) {
            Ok(Some(chunk)) => saved.insert_chunk(chunk),
            Ok(None) => {
                saved.load_chunk(pos);
            }
            Err(error) => return Err(format!("could not read chunk {pos:?}: {error}")),
        }
    }
    let on_disk = vx_world::world_hash(&saved);

    if on_disk == rebuilt {
        println!("match: the journal rebuilds the saved world exactly");
        Ok(())
    } else {
        Err(format!(
            "DIVERGED: saved world hashes {on_disk:#018x}, the journal rebuilds {rebuilt:#018x}"
        ))
    }
}

/// Build a world and mesh everything within range, without a window.
fn build_scene(
    context: &GpuContext,
    renderer: &mut Renderer,
    seed: u64,
    radius: i32,
    at: (i32, i32),
) -> (World, Camera) {
    let mut world = World::new(seed);
    let mut streamer = ChunkStreamer::new(StreamingConfig {
        render_distance: radius,
        // No frame to keep smooth here, so do all the work at once.
        generate_budget: usize::MAX,
        mesh_budget: usize::MAX,
    });

    let centre = vx_core::BlockPos::new(at.0, 0, at.1).chunk();
    world.load_around(centre, radius);
    streamer.update(&mut world, renderer, &context.device, centre, None);

    let surface = world.surface_y(at.0, at.1).unwrap_or(80);
    let camera = Camera {
        position: glam::DVec3::new(at.0 as f64, surface as f64 + 10.0, at.1 as f64 + 20.0),
        pitch: -0.35,
        ..Camera::default()
    };
    (world, camera)
}

/// Mine the nearest body and report what happened.
fn dig_nearby(
    world: &mut World,
    at: (i32, i32),
    choice: Dig,
    ticks: u64,
) -> Result<(Operation, MineMethod, vx_core::BlockPos, VoxelAabb), String> {
    let body = vx_agent::find_body(world, at, 48)
        .ok_or_else(|| format!("no ore outcrop within 48 blocks of {at:?}"))?;

    let plan = match choice {
        Dig::Proposed => vx_agent::propose(world, body, vx_agent::DEFAULT_GRADE),
        Dig::Forced(method) => vx_agent::plan(world, body, vx_agent::DEFAULT_GRADE, method),
    }
    .ok_or_else(|| format!("{choice:?} does not apply to the body at {body:?}"))?;

    let start = vx_agent::settle(world, plan.portal);
    let mut operation = Operation::new(start);
    operation.add_drone(start);
    operation.post_plan(&plan);

    let events = EventBus::new();
    let (outcome, used) = operation.run(world, &events, ticks);
    println!(
        "{}: {outcome:?} after {used} ticks, {} blocks hauled ({} ore)",
        plan.method.name(),
        operation.stockpile.total(),
        operation.stockpile.count("engine:copper_ore"),
    );
    let workings = plan
        .access
        .iter()
        .chain(&plan.extraction)
        .copied()
        .reduce(VoxelAabb::union)
        .unwrap_or(body);
    Ok((operation, plan.method, start, workings))
}

/// One step of a breadth-first walk across an arcade floor: the centre of the
/// next cell on the shortest corridor route from `from` to `to`. Only the
/// `--arcade` capture uses it — the game itself never pathfinds, because the
/// player is the one doing the walking.
fn arcade_waypoint(level: &arcade::Level, from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let side = arcade::SIDE as i32;
    let start = (from.0 as i32, from.1 as i32);
    let goal = (to.0 as i32, to.1 as i32);
    if start == goal {
        return to;
    }
    // Backwards from the goal, so the first hop off the start cell is simply
    // whichever neighbour carries the smaller distance.
    let mut seen = vec![u32::MAX; (side * side) as usize];
    let index = |cell: (i32, i32)| (cell.1 * side + cell.0) as usize;
    let mut queue = std::collections::VecDeque::from([goal]);
    seen[index(goal)] = 0;
    while let Some(cell) = queue.pop_front() {
        if cell == start {
            break;
        }
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (cell.0 + dx, cell.1 + dz);
            if next.0 < 0 || next.1 < 0 || next.0 >= side || next.1 >= side {
                continue;
            }
            if level.solid(next.0, next.1) || seen[index(next)] != u32::MAX {
                continue;
            }
            seen[index(next)] = seen[index(cell)] + 1;
            queue.push_back(next);
        }
    }
    let here = seen[index(start)];
    if here == u32::MAX {
        return to;
    }
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let next = (start.0 + dx, start.1 + dz);
        if next.0 < 0 || next.1 < 0 || next.0 >= side || next.1 >= side {
            continue;
        }
        if !level.solid(next.0, next.1) && seen[index(next)] < here {
            return (next.0 as f32 + 0.5, next.1 as f32 + 0.5);
        }
    }
    to
}

fn run_screenshot(options: &Options, path: &str) -> Result<(), String> {
    let context = GpuContext::headless_blocking()
        .map_err(|error| format!("no graphics device for offscreen rendering: {error}"))?;

    let (width, height) = (options.width, options.height);
    let mut renderer = Renderer::new(&context, CAPTURE_FORMAT, width, height);

    let (mut world, mut camera) = build_scene(&context, &mut renderer, options.seed, 6, options.at);
    camera.aspect = width as f32 / height as f32;

    // `--play` is not a fixture in the usual sense: every other one arranges a
    // scene and photographs it, and this one *plays the game* and photographs
    // what happened. It runs a whole loop — out of the house, out to the ore,
    // cut it, home, sold — through `session`, which drives the same movement,
    // drill and shop the window does, and it photographs four beats along the
    // way rather than one at the end. It returns early because it owns its own
    // output.
    if options.play {
        return play_the_loop(&context, &mut renderer, &mut camera, options, path);
    }

    // The camera came off the hull this round, so the round's picture is what
    // it now sees — and, beside it, where it hangs. Two beats, one process.
    if options.gimbal {
        return photograph_the_gimbal(&context, &mut renderer, &mut camera, options, path);
    }

    // The long loop: scout, collect, save, load, and take it somewhere that
    // pays. The round's whole point is the save in the middle of it.
    if options.haul {
        return haul_it_somewhere(&context, &mut renderer, &mut camera, options, path);
    }

    // A capture's sky is normally the hour alone. The weather fixtures set
    // this instead, and the sun uniform at the bottom of this function reads
    // it — the same tint the running game applies, out of the same struct.
    let mut weather_over: Option<vx_world::weather::Conditions> = None;

    // `--season` picks a tick in the middle of the named season and hands it
    // to the sun and the atlas at the bottom of this function. The middle
    // rather than the first day, because a season's first morning still
    // looks like the one before it — that is the whole point of the year
    // being continuous, and it makes for a poor photograph.
    let mut season_at: Option<u64> = match options.season.as_deref() {
        Some(word) => match vx_world::season::Season::parse(word) {
            Some(season) => {
                let tick = season.index() as u64 * vx_world::season::SEASON_TICKS
                    + vx_world::season::SEASON_TICKS / 2;
                println!(
                    "season capture: {} — day {} of the year, leaves {:.0}% turned, {}",
                    season.label(),
                    vx_world::season::day_of_year(tick),
                    vx_world::season::leaf_turn(tick) * 100.0,
                    if vx_world::season::fire_season(tick) {
                        "fire season"
                    } else {
                        "no fire risk"
                    }
                );
                Some(tick)
            }
            None => return Err(format!("--season {word} is not a season")),
        },
        None => None,
    };

    if options.bunker {
        // Find the nearest bunker to the requested spot and frame it: the
        // hatch from outside, or — with `--close` — standing in the works.
        // The capture reads the generated world, not the layout, so what it
        // shows is what a player walking there would find.
        let site = world
            .generator()
            .bunkers_near(options.at, 4_000)
            .into_iter()
            .next()
            .ok_or("no bunker within four kilometres of --at")?;
        let plan = vx_world::bunker::layout(&site);
        println!(
            "bunker: {:?} {:?} at {:?}, {} levels, {} rooms, bearing {:.0}°",
            site.tier,
            site.system,
            site.centre,
            site.levels,
            plan.rooms.len(),
            site.bearing.to_degrees()
        );

        // Load the ground around it, since `--at` may be kilometres away.
        let centre = vx_core::BlockPos::new(site.centre.0, 0, site.centre.1).chunk();
        world.load_around(centre, 6);
        remesh_all(&context, &mut renderer, &mut world);

        if options.close {
            // Stand in the biggest room on the top floor, looking along it.
            let room = plan
                .rooms
                .iter()
                .filter(|room| room.level == 0)
                .max_by_key(|room| room.w * room.d)
                .ok_or("the bunker has no rooms")?;
            let base = site.level_base(room.level);
            let (cx, cz) = room.centre();
            camera.position = glam::DVec3::new(
                cx as f64 + 0.5 - room.w as f64 * 0.32,
                base as f64 + 2.2,
                cz as f64 + 0.5,
            );
            look_at(
                &mut camera,
                glam::Vec3::new(cx as f32 + room.w as f32 * 0.4, base as f32 + 1.8, cz as f32),
            );
        } else {
            // Stand off along the bearing and above whatever the ground does
            // there — a fixed height over the *hatch* puts the camera inside
            // the hill whenever the approach runs into rising terrain.
            let out = glam::Vec3::new(site.bearing.cos(), 0.0, site.bearing.sin());
            let stand = glam::Vec3::new(site.hatch.0 as f32, 0.0, site.hatch.1 as f32) + out * 14.0;
            let local = world
                .surface_y(stand.x.floor() as i32, stand.z.floor() as i32)
                .unwrap_or(site.hatch_ground);
            camera.position = glam::DVec3::new(
                stand.x as f64,
                local.max(site.hatch_ground) as f64 + 7.0,
                stand.z as f64,
            );
            look_at(
                &mut camera,
                glam::Vec3::new(
                    site.hatch.0 as f32,
                    site.hatch_ground as f32 - 1.0,
                    site.hatch.1 as f32,
                ),
            );
        }
    }

    if options.cave {
        // Hunt the roomiest pocket of underground air near the requested
        // spot and stand the camera in it, facing down the longest gallery.
        // The cave field is pure, but the capture reads the *world* — what
        // this frames is what generation actually built.
        let air = |at: vx_core::BlockPos| {
            world.surface_y(at.x, at.z).is_some_and(|stand| at.y < stand - 1)
                && !world.is_solid(at)
        };
        let mut best: Option<(vx_core::BlockPos, u32)> = None;
        for x in options.at.0 - 48..options.at.0 + 48 {
            for z in options.at.1 - 48..options.at.1 + 48 {
                let Some(stand) = world.surface_y(x, z) else { continue };
                for y in vx_world::caves::CAVE_FLOOR + 2..stand - 6 {
                    let at = vx_core::BlockPos::new(x, y, z);
                    if !air(at) || world.is_solid(at.offset([0, 1, 0])) || !air(at.offset([0, 1, 0])) {
                        continue;
                    }
                    let mut room = 0u32;
                    for dx in -3i32..=3 {
                        for dy in -1i32..=2 {
                            for dz in -3i32..=3 {
                                room += u32::from(air(at.offset([dx, dy, dz])));
                            }
                        }
                    }
                    if best.is_none_or(|(_, held)| room > held) {
                        best = Some((at, room));
                    }
                }
            }
        }
        let (spot, _) = best.ok_or("no cave pocket within 48 blocks of --at")?;
        let mut heading = (0i32, -1i32, 0u32);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let mut clear = 0u32;
            for step in 1..=14 {
                if !air(spot.offset([dx * step, 0, dz * step])) {
                    break;
                }
                clear += 1;
            }
            if clear > heading.2 {
                heading = (dx, dz, clear);
            }
        }
        camera.position = glam::DVec3::new(
            spot.x as f64 + 0.5,
            spot.y as f64 + 1.1,
            spot.z as f64 + 0.5,
        );
        camera.yaw = (heading.0 as f32).atan2(-(heading.1 as f32));
        camera.pitch = -0.08;
        println!(
            "cave capture from {:?}, facing ({}, {}) with {} blocks of gallery",
            spot, heading.0, heading.1, heading.2
        );
    }

    // Late for the same reason as the posse fixture: the villagers' pass
    // would otherwise replace these bodies with the townsfolk.
    if options.standing {
        // A raiding career, run through the real ledger: honest trade and
        // gifts on one side, captures, kills and a cleared shelter on the
        // other. Every number below is what the ledger says afterwards.
        let mut name = reputation::Reputation::default();
        for _ in 0..60 {
            name.with_compact(reputation::TRADE_COMPACT);
        }
        for _ in 0..8 {
            name.with_compact(reputation::GIFT_COMPACT);
        }
        for _ in 0..3 {
            name.with_compact(reputation::CAPTURE_COMPACT);
            name.with_holdouts(reputation::CAPTURE_HOLDOUTS);
        }
        for _ in 0..2 {
            name.with_compact(reputation::KILL_COMPACT);
            name.with_holdouts(reputation::KILL_HOLDOUTS);
        }
        name.with_holdouts(reputation::CLEARED_HOLDOUTS);

        let mut console = terminal::Terminal::default();
        console.toggle();
        console.say(terminal::Kind::Note, "SOLD 40 COPPER ORE FOR 445 CR");
        console.say(terminal::Kind::Note, "TAKEN IN - 120 CREDITS FROM THE BOARD");
        console.say(
            terminal::Kind::Note,
            format!("THE SHELTERS NOW CALL YOU {}", name.holdouts().name()),
        );
        console.say(terminal::Kind::Warn, "THE SHELTER IS JAMMING YOUR SCOUT");
        console.say(terminal::Kind::Echo, "> STANDING");
        console.say(
            terminal::Kind::Note,
            format!(
                "THE TOWNS     {:<8} {:>5}  SHADE {:+}PC AT THE COUNTER",
                name.compact().name(),
                name.compact_points(),
                reputation::price_shade(name.compact())
            ),
        );
        console.say(
            terminal::Kind::Note,
            format!(
                "THE SHELTERS  {:<8} {:>5}  {}",
                name.holdouts().name(),
                name.holdouts_points(),
                if name.holdouts() >= reputation::Standing::Neutral {
                    "THEY CHALLENGE BEFORE THEY SHOOT"
                } else {
                    "SHOT ON SIGHT, AND THEY JAM YOUR SCOUT"
                }
            ),
        );

        let pixels = terminal::render_terminal(&console, false);
        let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            TERM_SLOT,
            &context.device,
            &context.queue,
            (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
        println!(
            "standing capture: towns {} ({}), shelters {} ({})",
            name.compact().name(),
            name.compact_points(),
            name.holdouts().name(),
            name.holdouts_points()
        );
    }

    if options.debug {
        // The F3 readout over a real scene. The engine numbers are the
        // capture's own; the fight and ledger rows are a staged session,
        // since a screenshot has no posse to be chased by.
        let content = debug::DebugContent {
            fps: 60.0,
            position: (options.at.0 as f32 + 0.5, 74.0, options.at.1 as f32 + 0.5),
            chunk: (options.at.0 >> 4, options.at.1 >> 4),
            yaw: 195.0,
            pitch: -9.0,
            aimed: Some("ENGINE:STONE".into()),
            chunks_loaded: world.loaded_chunk_count(),
            chunks_drawn: renderer.visible_chunk_count(),
            triangles: renderer.triangle_count(),
            edits: world.edit_count(),
            composites: world.composite_count(),
            tick: 88_412,
            log_entries: 1_204,
            day: 9,
            hhmm: (13, 42),
            burners: 4,
            fuel_cells: 11,
            worst_machine: "WORN",
            panicking: 0,
            marks: 3,
            shots: 1,
            deputies: 3,
            squads: 1,
            hunting: 2,
            belief: Some((0.82, 0.44)),
            hits: (4, 6),
            bounty: 120,
            compact: "WARM",
            holdouts: "COLD",
            wells: (3, 1),
            rads: (12.5, 88.0),
            dark: (0.6, "HUNTING"),
            medkits: 2,
        };
        let pixels = debug::render_debug(&content);
        let panel_width = debug::DEBUG_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = debug::DEBUG_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            DEBUG_SLOT,
            &context.device,
            &context.queue,
            (debug::DEBUG_WIDTH, debug::DEBUG_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: width as f32 - panel_width - 12.0,
                y: 12.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.wound {
        // Build a wall the camera is looking at and then damage it with the
        // real shapes, through the real `World::carve` — a slug bite here, a
        // blast there, a drilled face, and a hole shot clean through. Every
        // cell drawn below came out of the same code the game runs.
        let stone = world.registry().id_of("engine:stone").unwrap();
        let wall_z = options.at.1 + 6;
        // Take the ground from the wall's own column, not the camera's: on a
        // slope those differ, and the first cut of this fixture buried the
        // whole wall in a hillside.
        let ground = world.surface_y(options.at.0, wall_z).unwrap_or(80);

        // Clear the approach and level the footing, so what the shot shows
        // is the wall rather than whatever the hill was doing.
        for x in options.at.0 - 4..=options.at.0 + 4 {
            for z in wall_z - 7..=wall_z {
                for y in ground + 1..ground + 10 {
                    world.set_block(vx_core::BlockPos::new(x, y, z), vx_core::BlockId::AIR);
                }
                world.set_block(vx_core::BlockPos::new(x, ground, z), stone);
            }
        }
        // A clean face, five wide and four tall, standing on the ground.
        for x in options.at.0 - 2..=options.at.0 + 2 {
            for y in ground + 1..=ground + 4 {
                world.set_block(vx_core::BlockPos::new(x, y, wall_z), stone);
            }
        }

        let at = |x: i32, y: i32| vx_core::BlockPos::new(x, y, wall_z);
        // A slug bite, low left: a small blob out of the face.
        world.carve(
            at(options.at.0 - 2, ground + 2),
            vx_world::micro::Shape::SlugBite.cells(2, 2, 0, 4),
        );
        // Two bites in the same block, which is what sustained fire looks
        // like before a hole opens.
        for cell in [(1, 1), (2, 3)] {
            world.carve(
                at(options.at.0 - 1, ground + 2),
                vx_world::micro::Shape::SlugBite.cells(cell.0, cell.1, 0, 4),
            );
        }
        // A blast, centre. Aimed at a corner rather than the middle: a
        // three-cell radius centred inside a four-cell block takes nearly
        // all of it and the block simply dies, which is correct and shows
        // nothing. Clipped off a corner it leaves a crater to look at.
        world.carve(
            at(options.at.0, ground + 3),
            vx_world::micro::Shape::Blast.cells(0, 3, 0, 4),
        );
        // A drilled face, right: one tick of the bit has taken the near layer.
        // Two ticks of the bit, so the recess reads as depth rather than as
        // a slightly darker square.
        for depth in 0..2 {
            let mut layer = 0;
            for y in 0..vx_world::micro::SIDE {
                for x in 0..vx_world::micro::SIDE {
                    layer |= vx_world::micro::bit(x, y, depth);
                }
            }
            world.carve(at(options.at.0 + 1, ground + 2), layer);
        }
        // And a channel shot clean through, far right — the peephole a ray
        // can pass but a body cannot.
        let mut channel = 0;
        for depth in 0..vx_world::micro::SIDE {
            channel |= vx_world::micro::bit(1, 2, depth);
            channel |= vx_world::micro::bit(2, 2, depth);
        }
        world.carve(at(options.at.0 + 2, ground + 2), channel);

        // The wall and its wounds were written after the scene was built, so
        // the GPU still holds the meshes of an untouched hillside.
        remesh_all(&context, &mut renderer, &mut world);

        let wounds: usize = (options.at.0 - 2..=options.at.0 + 2)
            .map(|x| {
                (ground + 1..=ground + 4)
                    .filter(|y| world.mask(at(x, *y)).is_some())
                    .count()
            })
            .sum();
        let cells: u32 = (options.at.0 - 2..=options.at.0 + 2)
            .flat_map(|x| (ground + 1..=ground + 4).map(move |y| (x, y)))
            .filter_map(|(x, y)| world.mask(at(x, y)))
            .map(|mask| vx_world::micro::CELLS - vx_world::micro::remaining(mask))
            .sum();
        println!("wound capture: {wounds} composite blocks, {cells} cells taken");

        camera.position = glam::DVec3::new(
            options.at.0 as f64 + 0.5,
            ground as f64 + 3.0,
            wall_z as f64 - if options.close { 3.0 } else { 6.5 },
        );
        look_at(
            &mut camera,
            glam::Vec3::new(
                options.at.0 as f32 + 0.5,
                ground as f32 + 2.8,
                wall_z as f32,
            ),
        );
    }

    if options.footing {
        // An excavation beside the bank, so the foundation reads in section:
        // the strip under the strongroom wall, the slab under the floor, and
        // how far both run below the plaza. Cut here rather than drawn,
        // because the point is what generation actually put in the ground.
        let site = world
            .towns_near(options.at, 2_000)
            .into_iter()
            .next()
            .ok_or("no town within two kilometres of --at")?;
        let centre = vx_core::BlockPos::new(site.centre.0, 0, site.centre.1).chunk();
        world.load_around(centre, 4);

        let bank = vx_world::town::plan::buildings(&site)
            .into_iter()
            .find(|building| building.role == vx_world::town::plan::Role::Bank)
            .ok_or("this town has no bank")?;
        // Take the ground away in front of the bank's south wall, down past
        // the bottom of its strip.
        let floor = site.ground - 8;
        for x in bank.min.x - 3..=bank.max.x + 3 {
            for z in bank.max.z + 1..=bank.max.z + 9 {
                for y in floor..=site.ground + 1 {
                    let at = vx_core::BlockPos::new(x, y, z);
                    if world.block(at) != vx_core::BlockId::AIR {
                        world.set_block(at, vx_core::BlockId::AIR);
                    }
                }
            }
        }
        remesh_all(&context, &mut renderer, &mut world);
        println!(
            "footing section at the bank, {:?} to {:?}",
            bank.min, bank.max
        );
        let look = glam::Vec3::new(
            (bank.min.x + bank.max.x) as f32 / 2.0,
            site.ground as f32 - 2.5,
            bank.max.z as f32 + 1.0,
        );
        camera.position = (look + glam::Vec3::new(-1.5, 1.0, 6.5)).as_dvec3();
        look_at(&mut camera, look);
    }

    if options.fort {
        // The nearest town that actually built something, framed from above
        // its own trace: a bastioned wall only reads as bastioned from a
        // height, which is the honest reason forts are drawn on maps.
        let site = world
            .towns_near(options.at, 4_000)
            .into_iter()
            .find(|site| vx_world::fort::fort_for(site).trace != vx_world::fort::Trace::Palisade)
            .ok_or("no walled town within four kilometres of --at")?;
        let fort = vx_world::fort::fort_for(&site);
        println!(
            "fort: {} at {:?}, {} trace, radius {:.0}, ruined {}",
            site.name.head(),
            site.centre,
            fort.trace.name(),
            fort.radius,
            fort.ruined
        );
        let centre = vx_core::BlockPos::new(site.centre.0, 0, site.centre.1).chunk();
        world.load_around(centre, 7);
        remesh_all(&context, &mut renderer, &mut world);

        let look = glam::Vec3::new(site.centre.0 as f32, site.ground as f32, site.centre.1 as f32);
        camera.position =
            (look + glam::Vec3::new(0.0, fort.radius * 1.5, fort.radius * 1.45)).as_dvec3();
        look_at(&mut camera, look);
    }

    if options.vault {
        // The bank's box, found by looking for it rather than by arithmetic:
        // if the blueprint moves, the capture follows it.
        let site = world
            .towns_near(options.at, 2_000)
            .into_iter()
            .next()
            .ok_or("no town within two kilometres of --at")?;
        let centre = vx_core::BlockPos::new(site.centre.0, 0, site.centre.1).chunk();
        world.load_around(centre, 4);
        remesh_all(&context, &mut renderer, &mut world);

        let vault_block = world
            .registry()
            .id_of("engine:vault")
            .ok_or("no vault block registered")?;
        let mut found = None;
        'search: for dx in -40..40 {
            for dz in -40..40 {
                for dy in 0..6 {
                    let at = vx_core::BlockPos::new(
                        site.centre.0 + dx,
                        site.ground + dy,
                        site.centre.1 + dz,
                    );
                    if world.block(at) == vault_block {
                        found = Some(at);
                        break 'search;
                    }
                }
            }
        }
        let at = found.ok_or("this town has no vault")?;
        println!("vault at {at:?} in {}", site.name.head());
        camera.position =
            glam::DVec3::new(at.x as f64 + 0.5, at.y as f64 + 1.3, at.z as f64 + 3.4);
        look_at(
            &mut camera,
            glam::DVec3::new(f64::from(at.x) + 0.5, f64::from(at.y) + 0.6, f64::from(at.z) + 0.5),
        );
    }

    if options.hho {
        // Find water near the capture spot, stand the machine on the shore
        // and frame it: the siting rule is the feature, so the picture has to
        // show a machine that is actually beside a lake.
        let water = world.registry().id_of("engine:water");
        let mut shore: Option<vx_core::BlockPos> = None;
        let Some(water) = water else {
            return Err("this world has no water block registered".into());
        };
        'hunt: for radius in 1..90i32 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if dx.abs() != radius && dz.abs() != radius {
                        continue;
                    }
                    let (x, z) = (options.at.0 + dx, options.at.1 + dz);
                    let Some(stand) = world.surface_y(x, z) else { continue };
                    // Dry footing only: the machine stands at the shore, not
                    // in the lake.
                    if world.block(vx_core::BlockPos::new(x, stand - 1, z)) == water {
                        continue;
                    }
                    let wet = (-2..=2).any(|ox| {
                        (-2..=2).any(|oz| {
                            world.block(vx_core::BlockPos::new(x + ox, stand - 1, z + oz)) == water
                        })
                    });
                    if wet {
                        shore = Some(vx_core::BlockPos::new(x, stand, z));
                        break 'hunt;
                    }
                }
            }
        }
        let stand = shore.ok_or("no shoreline within ninety blocks of --at")?;
        if let Some(id) = world.registry().id_of("engine:electrolyser") {
            world.set_block(stand, id);
            remesh_all(&context, &mut renderer, &mut world);
        }
        println!("electrolyser stood at {stand:?}");
        camera.position = glam::DVec3::new(
            stand.x as f64 + 3.4,
            stand.y as f64 + 2.4,
            stand.z as f64 + 4.2,
        );
        look_at(
            &mut camera,
            glam::DVec3::new(f64::from(stand.x) + 0.5, f64::from(stand.y) + 0.4, f64::from(stand.z) + 0.5),
        );
    }

    if options.fab {
        // A fabricator standing on the ground ahead of the camera, offset
        // left so the panel drawn over the frame's centre does not hide the
        // machine it belongs to.
        if let Some(id) = world.registry().id_of("engine:printer") {
            let (x, z) = (options.at.0 - 5, options.at.1 + 10);
            let ground = world.surface_y(x, z).unwrap_or(80);
            world.set_block(vx_core::BlockPos::new(x, ground, z), id);
            remesh_all(&context, &mut renderer, &mut world);
            if options.close {
                // `--close` walks up to the machine instead of opening its
                // panel: the capture that shows the block, not the screen.
                let subject =
                    glam::Vec3::new(x as f32 + 0.5, ground as f32 + 0.5, z as f32 + 0.5);
                camera.position = (subject + glam::Vec3::new(2.4, 1.6, 3.0)).as_dvec3();
                look_at(&mut camera, subject);
            }
        }
    }

    // Optionally run a whole excavation before capturing, so the frame shows a
    // real mine on real generated terrain rather than a hand-built fixture.
    if let Some(choice) = options.dig {
        let (operation, _, portal, workings) =
            dig_nearby(&mut world, options.at, choice, options.ticks)?;

        // Frame the shot: the whole excavation from a stand-off, or — with
        // `--close` — the first drone filling the frame, drill and all.
        if options.close {
            let drone = operation
                .drones
                .first()
                .ok_or("no drone to frame up close")?;
            let subject = glam::DVec3::new(
                        f64::from(drone.position.x) + 0.5,
                        f64::from(drone.position.y) + 0.4,
                        f64::from(drone.position.z) + 0.5,
                    );
            // Stand toward the portal — the one direction guaranteed open,
            // because the drone drove in through it.
            let doorway = glam::DVec3::new(
                f64::from(portal.x) + 0.5 - subject.x,
                0.0,
                f64::from(portal.z) + 0.5 - subject.z,
            );
            let out = if doorway.length() > 1.0 {
                doorway.normalize()
            } else {
                glam::DVec3::new(0.7, 0.0, 0.7).normalize()
            };
            camera.position = subject + out * 3.2 + glam::DVec3::Y * 1.2;
            look_at(&mut camera, subject);
        } else {
            let centre = glam::Vec3::new(
                workings.centre().x as f32,
                workings.max.y as f32,
                workings.centre().z as f32,
            );
            let span = workings.size();
            let stand_off = (span[0].max(span[2]) as f32 * 1.4).max(24.0);
            camera.position = (centre
                + glam::Vec3::new(stand_off * 0.7, stand_off * 0.8, stand_off * 0.7))
            .as_dvec3();
            look_at(&mut camera, centre);
        }
        let _ = portal;

        // Sweep the sector too, so the capture shows the whole loop: the
        // workings, the drone, the flier and the pings it found.
        let mut fleet = vx_agent::Fleet::new();
        fleet.add_flier(vx_core::BlockPos::new(portal.x, portal.y + 12, portal.z));
        let sector = vx_agent::Sector::containing(options.at.0, options.at.1);
        fleet.dispatch_scan(sector);
        let mut scan_ticks = 0u32;
        while !fleet.is_surveyed(sector) {
            fleet.tick(&world, &mut []);
            scan_ticks += 1;
            if scan_ticks > 20_000 {
                return Err("the survey sweep never finished".into());
            }
        }
        let pings = fleet.pings();
        println!(
            "scan: {} pings over sector ({}, {}) in {scan_ticks} ticks",
            pings.len(),
            sector.x,
            sector.z
        );

        remesh_all(&context, &mut renderer, &mut world);
        renderer.update_camera(&context.queue, &camera);

        // The machines wear their rigs, mid-dig: drills rolled to a visible
        // angle, noses yawed toward the workings so the shot reads as work.
        let digger = Rig::digger();
        let mut objects: Vec<vx_render::Object> = Vec::new();
        for drone in &operation.drones {
            let position = glam::DVec3::new(
                        f64::from(drone.position.x) + 0.5,
                        f64::from(drone.position.y),
                        f64::from(drone.position.z) + 0.5,
                    );
            let toward = workings.centre();
            let yaw = rig::yaw_towards(
                (f64::from(toward.x) + 0.5 - position.x) as f32,
                (f64::from(toward.z) + 0.5 - position.z) as f32,
            )
            .unwrap_or(0.0);
            objects.extend(
                digger
                    .objects(camera.relative(position), yaw, 0.9)
                    .into_iter()
                    .map(vx_render::Object::already_relative),
            );
        }
        let flier_rig = Rig::flier();
        for flier in &fleet.fliers {
            let position = glam::DVec3::new(
                        f64::from(flier.position.x) + 0.5,
                        f64::from(flier.position.y),
                        f64::from(flier.position.z) + 0.5,
                    );
            objects.extend(
                flier_rig
                    .objects(camera.relative(position), 0.0, 2.1)
                    .into_iter()
                    .map(vx_render::Object::already_relative),
            );
        }
        for ping in &pings {
            let centre = glam::DVec3::new(
                        f64::from(ping.position.x) + 0.5,
                        f64::from(ping.position.y) + 1.0,
                        f64::from(ping.position.z) + 0.5,
                    );
            let centre = camera.relative(centre);
            objects.push(
                vx_render::Object::box_between(
                    centre - glam::Vec3::splat(0.25),
                    centre + glam::Vec3::splat(0.25),
                    vx_render::tiles::slot::COPPER_ORE,
                )
                .already_relative(),
            );
        }
        // The player's handheld tool rides the camera, exactly as in play:
        // the drill by default, the launcher under `--launcher`.
        let camera_forward = camera.forward();
        let camera_right = camera.right();
        // In the renderer's own frame, marked so: the tool in your hand is
        // the one object that must never be a quarter of a block off.
        let drill_position = renderer.relative(camera.position)
            + camera_forward * 0.85
            + camera_right * 0.42
            - glam::Vec3::Y * 0.38;
        let drill_yaw = rig::yaw_towards(camera_forward.x, camera_forward.z).unwrap_or(0.0);
        let held = if options.launcher {
            Rig::launcher()
        } else {
            Rig::hand_drill()
        };
        objects.extend(
            held.objects_pitched(drill_position, drill_yaw, camera.pitch, 0.7)
                .into_iter()
                .map(vx_render::Object::already_relative),
        );

        renderer.set_objects(&context.device, &context.queue, &objects);

        // The HUD panel, with a few levels earned so the capture shows the
        // skill sheet doing its job.
        let mut hud_skills = Skills::new();
        hud_skills.add_xp(skills::MINING, 4_000);
        hud_skills.add_xp(skills::PROSPECTING, 1_200);
        hud_skills.add_xp(skills::LOGISTICS, 600);
        hud_skills.add_xp(skills::MINING, 40);
        let hud_pixels = hud::render_hud(&hud::HudContent {
            skills: &hud_skills,
            time: TimeOfDay::new(options.time),
            status: Some(format!("MINING {} JOBS LEFT", operation.board.len())),
            drilling: Some(0.64),
            level_up: None,
            greeting: None,
            reconnecting: false,
            bounty: 40,
            watched: true,
            movement: hud::MovementReadout {
                stance: "SPRINT",
                stamina: 0.72,
                load: 0.4,
            },
            ammo: options.launcher.then_some(8),
            panicking: 0,
            kestrel: None,
            fuel: options.hho.then(|| "HHO 12".to_string()),
            condition: options.posse.then(|| "HITS 4/6".to_string()),
            dose: options.hot.then(|| "DOSE 74%".to_string()),
            dark: options.dark.then(|| "CLOSING - 2/8".to_string()),
            deputies: usize::from(options.posse) * 3,
            optic: options.optic.as_deref().and_then(|choice| match choice {
                "lamp" => Some("LAMP"),
                "nvg" => Some("NIGHT VISION"),
                "thermal" => Some("THERMAL"),
                _ => None,
            }),
        });
        renderer.set_overlay(
            HUD_SLOT,
            &context.device,
            &context.queue,
            (hud::HUD_WIDTH, hud::HUD_HEIGHT),
            &hud_pixels,
            vx_render::OverlayRect {
                x: 10.0,
                y: height as f32 - hud::HUD_HEIGHT as f32 * hud::HUD_SCALE - 10.0,
                width: hud::HUD_WIDTH as f32 * hud::HUD_SCALE,
                height: hud::HUD_HEIGHT as f32 * hud::HUD_SCALE,
            },
        );

        // And the minimap in the corner, exactly as the game draws it: every
        // loaded chunk counts as explored, the surveyed sector shades in, and
        // the dots are the drone, the flier and the pings.
        let mut map_state = MapState::new();
        for chunk in world.loaded_chunks().collect::<Vec<_>>() {
            map_state.explore(chunk);
        }
        let mut markers = Vec::new();
        for ping in &pings {
            markers.push(map::Marker {
                x: ping.position.x,
                z: ping.position.z,
                colour: map::colour::PING,
                radius: 2,
            });
        }
        for drone in &operation.drones {
            markers.push(map::Marker {
                x: drone.position.x,
                z: drone.position.z,
                colour: map::colour::DRONE,
                radius: 1,
            });
        }
        for flier in &fleet.fliers {
            markers.push(map::Marker {
                x: flier.position.x,
                z: flier.position.z,
                colour: map::colour::FLIER,
                radius: 1,
            });
        }
        let map_pixels = map::render_map(&world, &map_state, options.at, &markers);
        let margin = 10.0;
        let panel = map::MAP_SIZE as f32;
        renderer.set_overlay(
            MAP_SLOT,
            &context.device,
            &context.queue,
            (map::MAP_SIZE, map::MAP_SIZE),
            &map_pixels,
            vx_render::OverlayRect {
                x: width as f32 - panel - margin,
                y: margin,
                width: panel,
                height: panel,
            },
        );
    } else {
        renderer.update_camera(&context.queue, &camera);
        // The townsfolk, some way into their stroll, so a village capture
        // shows a town with life in it rather than an empty film set.
        let mut town = Villagers::new();
        if options.people {
            // Market day: the schedule pulls everybody to the square, which
            // is the shot.
            town.set_day(schedule::market_weekday(town.site()));
        }
        // Nobody about to notice in a still capture, so the town simply
        // strolls: `Surroundings::empty()` is "clear air, and alone".
        let alone = awareness::Surroundings::empty();
        for _ in 0..1800 {
            town.update(1.0 / 60.0, TimeOfDay::new(options.time), &alone);
        }
        let rigs = Villagers::rigs();
        renderer.set_objects(
            &context.device,
            &context.queue,
            &town.objects(&rigs, |at| camera.relative(at)),
        );
    }

    if options.third_person {
        // Step the camera back over the shoulder and put a body in the shot,
        // exactly as the live view does it.
        let pivot = camera.position;
        let placed = view::camera_placement(&world, &camera, pivot, view::ViewMode::ThirdPerson);
        let body = (pivot - glam::DVec3::Y * 1.62).as_vec3();
        let forward = camera.forward();
        let yaw = rig::yaw_towards(forward.x, forward.z).unwrap_or(0.0);
        let mut objects = Rig::player().objects(body, yaw, 0.0);
        // Keep the townsfolk that the village capture already drew.
        let mut town = Villagers::new();
        let alone = awareness::Surroundings::empty();
        for _ in 0..900 {
            town.update(1.0 / 60.0, TimeOfDay::new(options.time), &alone);
        }
        camera.position = placed;
        renderer.update_camera(&context.queue, &camera);
        objects.extend(town.objects(&Villagers::rigs(), |at| camera.relative(at)));
        renderer.set_objects(&context.device, &context.queue, &objects);
    }

    // After the villagers' pass on purpose: that branch calls
    // `set_objects` and would otherwise replace the deputies with the
    // townsfolk, which is exactly what it did the first time.
    if options.posse {
        // A callout, stepped by the real `Posse::update` against the real
        // world: the deputies below stand where the code put them, in the
        // modes their own nerve chose.
        let ground = world.surface_y(options.at.0, options.at.1).unwrap_or(80);
        let player = glam::DVec3::new(
            f64::from(options.at.0) + 0.5,
            f64::from(ground) + 1.0,
            f64::from(options.at.1) + 0.5,
        );
        let mut posse = hostile::Posse::default();
        posse.call_out(
            player,
            |x, z| {
                world
                    .surface_y(x.floor() as i32, z.floor() as i32)
                    .map_or(player.y, |top| f64::from(top + 1))
            },
            0xc0ffee,
            None,
        );
        // Run the callout for a few seconds so they close, take cover and
        // settle into modes rather than standing where they spawned.
        let mut lines = Vec::new();
        for _ in 0..240 {
            let report = posse.update(1.0 / 30.0, &world, player, false);
            for bark in report.barks {
                if !lines.contains(&bark) {
                    lines.push(bark);
                }
            }
        }
        // Rattle one of them so the roll call shows a spread of nerve
        // rather than three identical rows.
        if let Some(deputy) = posse.deputies.get_mut(1) {
            deputy.rattle(hostile::WOUNDED + hostile::ROUND_NEAR_COVER);
        }
        if let Some(deputy) = posse.deputies.get_mut(2) {
            deputy.rattle(hostile::ALLY_DOWN * 3.0);
        }

        let mut console = terminal::Terminal::default();
        console.toggle();
        console.say(terminal::Kind::Warn, "A WARRANT IS OUT. DEPUTIES ARE COMING");
        for line in lines.iter().take(4) {
            console.say(terminal::Kind::Warn, line.clone());
        }
        console.say(terminal::Kind::Echo, "> LAW");
        for row in posse.roll_call() {
            console.say(terminal::Kind::Note, row);
        }
        if !options.close {
            let pixels = terminal::render_terminal(&console, false);
            let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
            let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
            renderer.set_overlay(
                TERM_SLOT,
                &context.device,
                &context.queue,
                (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
                &pixels,
                vx_render::OverlayRect {
                    x: (width as f32 - panel_width) / 2.0,
                    y: (height as f32 - panel_height) / 2.0,
                    width: panel_width,
                    height: panel_height,
                },
            );
        }

        // `--close` frames the squad itself: over the player's shoulder,
        // looking at whichever deputy got nearest.
        if options.close {
            // Frame the squad rather than guessing at an angle: stand off
            // from their centre of mass, high enough to see all three.
            let centre = posse
                .deputies
                .iter()
                .fold(glam::DVec3::ZERO, |sum, deputy| sum + deputy.position)
                / posse.deputies.len().max(1) as f64;
            // Straight down over the squad. Every angled framing this
            // fixture tried put a building, a mast or a hillside between the
            // camera and the deputies; overhead cannot be blocked.
            let _ = centre;
            let overhead = posse
                .deputies
                .iter()
                .fold(player, |sum, deputy| sum + deputy.position)
                / (posse.deputies.len() + 1) as f64;
            camera.position = glam::DVec3::new(overhead.x, overhead.y + 18.0, overhead.z + 0.1);
            look_at(&mut camera, overhead);
        } else {
            camera.position = player + glam::DVec3::new(0.0, 6.0, 14.0);
            look_at(&mut camera, player);
        }
        renderer.update_camera(&context.queue, &camera);

        // The bodies after the camera, because they are built in its frame.
        let rigs = Villagers::rigs();
        let mut objects = Vec::new();
        let mut town = Villagers::new();
        let alone = awareness::Surroundings::empty();
        for _ in 0..900 {
            town.update(1.0 / 60.0, TimeOfDay::new(options.time), &alone);
        }
        let placed = |built: Vec<vx_render::Object>| {
            built.into_iter().map(vx_render::Object::already_relative)
        };
        objects.extend(town.objects(&rigs, |at| camera.relative(at)));
        for deputy in &posse.deputies {
            let rig = &rigs[deputy.variant % rigs.len()];
            objects.extend(placed(rig.objects(camera.relative(deputy.position), deputy.yaw, 0.0)));
        }
        objects.extend(placed(Rig::player().objects(
            camera.relative(player - glam::DVec3::Y),
            std::f32::consts::PI,
            0.0,
        )));
        renderer.set_objects(&context.device, &context.queue, &objects);
        println!(
            "posse capture: {} deputies, {} barks",
            posse.deputies.len(),
            lines.len()
        );
    }

    // Late like the posse fixture, and for the same reason: the
    // villagers' pass calls `set_objects` and would replace the holders.
    if options.held {
        let site = world
            .generator()
            .bunkers_near(options.at, 4_000)
            .into_iter()
            .next()
            .ok_or("no bunker within four kilometres of --at")?;
        let hatch = glam::DVec3::new(
            f64::from(site.hatch.0) + 0.5,
            f64::from(site.hatch_ground + 1),
            f64::from(site.hatch.1) + 0.5,
        );
        // Load the ground around it, since `--at` may be kilometres away.
        let centre = vx_core::BlockPos::new(site.hatch.0, 0, site.hatch.1).chunk();
        world.load_around(centre, 6);
        remesh_all(&context, &mut renderer, &mut world);
        // The player stands off the hatch; a drill's worth of noise has
        // reached the squad, so they are searching the zone, not the spot.
        let player = hatch + glam::DVec3::new(14.0, 0.0, 9.0);
        let mut squads = garrison::Garrisons::default();
        squads.squads.push(garrison::Garrison::muster(&site));
        squads.hear(player);
        let mut lines = Vec::new();
        for _ in 0..240 {
            // No truce in the shot: the drill noise inside the leash has
            // already made this personal, which is what the capture shows.
            let report = squads.update(1.0 / 30.0, &world, player, false, false);
            for bark in report.barks {
                if !lines.contains(&bark) {
                    lines.push(bark);
                }
            }
        }

        if !options.close {
            let mut console = terminal::Terminal::default();
            console.toggle();
            console.say(
                terminal::Kind::Warn,
                format!("A {:?} SHELTER, HELD", site.tier).to_uppercase(),
            );
            for line in lines.iter().take(3) {
                console.say(terminal::Kind::Warn, line.clone());
            }
            console.say(terminal::Kind::Echo, "> LAW");
            for squad in &squads.squads {
                console.say(
                    terminal::Kind::Note,
                    format!(
                        "A HELD SHELTER NEARBY - {} STANDING, {} HUNTING",
                        squad.active(),
                        squad
                            .holders
                            .iter()
                            .filter(|holder| holder.active()
                                && holder.mode != hostile::Mode::Patrol)
                            .count()
                    ),
                );
            }
            console.say(terminal::Kind::Note, "TAKEN IN - 120 CREDITS FROM THE BOARD");
            let pixels = terminal::render_terminal(&console, false);
            let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
            let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
            renderer.set_overlay(
                TERM_SLOT,
                &context.device,
                &context.queue,
                (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
                &pixels,
                vx_render::OverlayRect {
                    x: (width as f32 - panel_width) / 2.0,
                    y: (height as f32 - panel_height) / 2.0,
                    width: panel_width,
                    height: panel_height,
                },
            );
        }

        // Overhead of the hatch and the searchers, the framing the posse
        // fixture already earned the hard way.
        let centre = squads
            .squads
            .iter()
            .flat_map(|squad| &squad.holders)
            .fold(hatch, |sum, holder| sum + holder.position)
            / (squads.squads.iter().map(|squad| squad.holders.len()).sum::<usize>() + 1) as f64;
        camera.position = glam::DVec3::new(centre.x, centre.y + 16.0, centre.z + 0.1);
        look_at(&mut camera, centre);
        renderer.update_camera(&context.queue, &camera);

        // The bodies after the camera, because they are built in its frame.
        let rigs = Villagers::rigs();
        let mut objects = Vec::new();
        let placed = |built: Vec<vx_render::Object>| {
            built.into_iter().map(vx_render::Object::already_relative)
        };
        for holder in squads.squads.iter().flat_map(|squad| &squad.holders) {
            let rig = &rigs[holder.variant % rigs.len()];
            objects.extend(placed(rig.objects(camera.relative(holder.position), holder.yaw, 0.0)));
        }
        objects.extend(placed(Rig::player().objects(
            camera.relative(player - glam::DVec3::Y),
            std::f32::consts::PI,
            0.0,
        )));
        renderer.set_objects(&context.device, &context.queue, &objects);
        println!(
            "held capture: {:?} shelter, {} holders, {} barks",
            site.tier,
            squads.squads.first().map_or(0, |squad| squad.holders.len()),
            lines.len()
        );
        for holder in squads.squads.iter().flat_map(|squad| &squad.holders) {
            println!(
                "  holder at {:.1},{:.1},{:.1} mode {:?} (hatch {:.1},{:.1},{:.1})",
                holder.position.x, holder.position.y, holder.position.z,
                holder.mode, hatch.x, hatch.y, hatch.z
            );
        }
    }

    // The wellhead, over a field the seed actually put there. Late like the
    // posse fixture, and for the same reason.
    if let Some(when) = options.flood.as_deref() {
        // Cut a gallery into the side of a lake and let it in. The water is
        // the real automaton on the real clock, so what the picture shows is
        // what the game does, not a still life of blue blocks.
        let settled = match when {
            "cut" | "in" | "open" => false,
            "level" | "settled" | "after" => true,
            other => return Err(format!("--flood wants cut or level (got {other})")),
        };
        let generator = world.generator().clone();

        // Find a shoreline: a column at the water's edge with dry land beside
        // it, walking out from `--at`.
        let mut shore = None;
        'coast: for ring in 1..90 {
            for step in -ring..=ring {
                for (x, z) in [
                    (options.at.0 + step * 12, options.at.1 + ring * 12),
                    (options.at.0 + step * 12, options.at.1 - ring * 12),
                    (options.at.0 + ring * 12, options.at.1 + step * 12),
                    (options.at.0 - ring * 12, options.at.1 + step * 12),
                ] {
                    let here = generator.natural_height_at(x, z);
                    // Land, with water within a few blocks of it and deep
                    // enough to be worth letting in.
                    if here <= vx_world::gen::SEA_LEVEL + 2 || here > vx_world::gen::SEA_LEVEL + 8 {
                        continue;
                    }
                    let wet = [(6, 0), (-6, 0), (0, 6), (0, -6)]
                        .iter()
                        .filter(|(dx, dz)| {
                            generator.natural_height_at(x + dx, z + dz)
                                < vx_world::gen::SEA_LEVEL - 1
                        })
                        .count();
                    if wet > 0 {
                        shore = Some((x, z));
                        break 'coast;
                    }
                }
            }
        }
        let (x, z) = shore.ok_or("no shoreline within a kilometre of --at")?;
        let centre = vx_core::BlockPos::new(x, 0, z).chunk();
        world.load_around(centre, 7);

        // Which way the water is. Cut the gallery in from the dry side, so
        // the last block to go is the one holding the lake back.
        let towards = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .min_by_key(|(dx, dz)| generator.natural_height_at(x + dx * 6, z + dz * 6))
            .unwrap_or((1, 0));
        // The gallery is driven at the water's own level, not the hill's:
        // a tunnel five blocks above the lake lets nothing in, which is the
        // first thing this fixture got wrong.
        let floor_y = vx_world::gen::SEA_LEVEL - 2;
        let ground = floor_y;
        let air = vx_core::BlockId::AIR;
        for step in 0..15 {
            for across in -1..=1 {
                let (gx, gz) = (
                    x - towards.0 * step - towards.1 * across,
                    z - towards.1 * step - towards.0 * across,
                );
                // Four tall, not three: the sea settles to its own level
                // two blocks up, and a gallery with no headroom over that is
                // a gallery you can only photograph from inside the water.
                for dy in 0..4 {
                    world.set_block(vx_core::BlockPos::new(gx, floor_y + dy, gz), air);
                }
            }
        }
        // And the mouth: keep driving toward the lake until the next block
        // *is* the lake. The shore slopes, so the water is rarely exactly one
        // block out — the first cut of this fixture assumed it was and let
        // nothing in at all.
        let id = world.registry().id_of("engine:water").unwrap_or(air);
        let mut plug = vx_core::BlockPos::new(x, floor_y + 1, z);
        for step in 1..12 {
            let ahead = vx_core::BlockPos::new(
                x + towards.0 * step,
                floor_y + 1,
                z + towards.1 * step,
            );
            if world.block(ahead) == id {
                plug = ahead;
                break;
            }
            for across in -1..=1 {
                for dy in 0..4 {
                    world.set_block(
                        vx_core::BlockPos::new(
                            ahead.x - towards.1 * across,
                            floor_y + dy,
                            ahead.z - towards.0 * across,
                        ),
                        air,
                    );
                }
            }
            plug = ahead;
        }
        let mut water = Vec::new();
        journal::wake_water(&mut water, &mut world, plug);

        let mut steps = 0;
        let want = if settled { 64 * 20 } else { 30 };
        while steps < want {
            journal::settle_water(&mut water, &mut world);
            steps += 1;
            if water.is_empty() {
                break;
            }
        }
        remesh_all(&context, &mut renderer, &mut world);

        // Look down the gallery from the dry end, so the water is coming at
        // the lens.
        let eye = glam::Vec3::new(
            (x - towards.0 * 13) as f32 + 0.5,
            ground as f32 + 3.1,
            (z - towards.1 * 13) as f32 + 0.5,
        );
        camera.position = eye.as_dvec3();
        look_at(
            &mut camera,
            glam::Vec3::new(x as f32 + 0.5, ground as f32 + 1.4, z as f32 + 0.5),
        );
        renderer.update_camera(&context.queue, &camera);

        let wet: u32 = {
            let mut sum = 0;
            for step in 0..16 {
                for dy in 0..4 {
                    let at = vx_core::BlockPos::new(
                        x - towards.0 * step,
                        floor_y + dy,
                        z - towards.1 * step,
                    );
                    sum += vx_world::fluid::level_at(&world, id, at);
                }
            }
            sum
        };
        println!(
            "flood capture: {when} at {x},{z} (ground {ground}) — {steps} steps, {wet} cells of \
             water down the gallery, {} bodies still moving",
            water.len()
        );
    }

    if options.founded {
        // Found a town where `--at` points — or the first open plot east of
        // it the lattice's own rules allow — exactly as the FOUND order does,
        // and frame it from outside its wall with the tower in the middle.
        // The picture is the whole claim of the round: a place that was a
        // hillside a moment ago, indistinguishable from a town the seed drew.
        let name = vx_world::town::TownName::from_words("iron", "reach").unwrap();
        let mut plot = None;
        for step in 0..12 {
            let at = (options.at.0 + step * vx_world::town::CELL, options.at.1);
            match charter::may_found(&world, at, name) {
                Ok(site) => {
                    plot = Some(site);
                    break;
                }
                Err(refusal) => println!("founding at {at:?}: {}", refusal.line()),
            }
        }
        let site = plot.ok_or("no chartable ground within twelve cells of --at")?;
        let raised = world.raise(site);
        remesh_all(&context, &mut renderer, &mut world);
        let fort = vx_world::fort::fort_for(&site);
        let centre = glam::DVec3::new(
            site.centre.0 as f64 + 0.5,
            site.ground as f64,
            site.centre.1 as f64 + 0.5,
        );
        // A high oblique from just outside the wall: well above anything the
        // lattice's relief rule lets stand within the town's reach, so no
        // hillside and no canopy can get between the eye and the plot.
        let back = fort.reach() as f64 + 6.0;
        camera.position = centre + glam::DVec3::new(back * 0.25, 58.0, back);
        look_at(&mut camera, centre + glam::DVec3::Y * 3.0);
        renderer.update_camera(&context.queue, &camera);
        println!(
            "founded capture: {} at {},{} — ground {}, {raised} chunks raised, {} buildings, wall reach {}",
            site.name,
            site.centre.0,
            site.centre.1,
            site.ground,
            vx_world::town::plan::buildings(&site).len(),
            fort.reach()
        );
    }

    if options.storm {
        // Weather over real country. The conditions are not invented: the
        // fixture walks the clock forward until `weather::at` says this
        // region is under rain or a storm, so the sky in the picture is one
        // the seed actually produces on a tick the game would have run.
        let seed = world.seed();
        let mut found = None;
        for step in 0..40_000u64 {
            let tick = step * 64;
            let sky = vx_world::weather::at(seed, tick, options.at.0, options.at.1);
            if sky.rain > 0.75 {
                found = Some((tick, sky));
                break;
            }
        }
        let (tick, sky) = found.ok_or("no downpour over --at inside ten hours of the clock")?;

        // Stand on a rise looking out, so the sheet is between the lens and
        // the country rather than all around one hollow.
        let generator = world.generator().clone();
        let mut best = (options.at, generator.natural_height_at(options.at.0, options.at.1));
        for ring in 1..14 {
            for step in -ring..=ring {
                for (x, z) in [
                    (options.at.0 + step * 10, options.at.1 + ring * 10),
                    (options.at.0 + step * 10, options.at.1 - ring * 10),
                    (options.at.0 + ring * 10, options.at.1 + step * 10),
                    (options.at.0 - ring * 10, options.at.1 + step * 10),
                ] {
                    let here = generator.natural_height_at(x, z);
                    if here > best.1 {
                        best = ((x, z), here);
                    }
                }
            }
        }
        let ((x, z), top) = best;
        world.load_around(vx_core::BlockPos::new(x, 0, z).chunk(), 7);

        // Rain wets the ground it falls on, which is stage 37's automaton
        // taking a source term rather than anything new: a scatter of
        // exposed columns take a few cells and then settle.
        let mut water = Vec::new();
        let mut wetted = 0;
        let wet_id = world.registry().id_of("engine:water").unwrap_or(vx_core::BlockId::AIR);
        for dx in -20..=20i32 {
            for dz in -20..=20i32 {
                if (dx.rem_euclid(7), dz.rem_euclid(7)) != (0, 0) {
                    continue;
                }
                let (cx, cz) = (x + dx, z + dz);
                let Some(surface) = world.surface_y(cx, cz) else {
                    continue;
                };
                if surface <= vx_world::gen::SEA_LEVEL {
                    continue;
                }
                let at = vx_core::BlockPos::new(cx, surface + 1, cz);
                vx_world::fluid::set_level(&mut world, wet_id, at, 12);
                if world.block(at) == wet_id {
                    journal::wake_water(&mut water, &mut world, at);
                    wetted += 1;
                }
            }
        }
        for _ in 0..64 * 3 {
            if water.is_empty() {
                break;
            }
            journal::settle_water(&mut water, &mut world);
        }
        remesh_all(&context, &mut renderer, &mut world);

        // Eye well above the rise — over the canopy rather than inside it —
        // looking out along the wind, so the streaks lean across the frame
        // and the country is under them rather than behind one trunk.
        let eye = glam::Vec3::new(x as f32 + 0.5, top as f32 + 13.0, z as f32 + 0.5);
        camera.position = eye.as_dvec3();
        let along = glam::Vec3::new(sky.wind.0, 0.0, sky.wind.1).normalize_or_zero();
        let out = if along == glam::Vec3::ZERO { glam::Vec3::Z } else { along };
        look_at(&mut camera, eye + out * 60.0 - glam::Vec3::Y * 22.0);
        renderer.update_camera(&context.queue, &camera);
        renderer.set_objects(
            &context.device,
            &context.queue,
            &rain::streaks(seed, tick as f32 / 64.0, eye.as_dvec3(), &sky),
        );
        weather_over = Some(sky);

        println!(
            "storm capture: {} at tick {tick} over {x},{z} (top {top}) — rain {:.2}, wind {:.1} m/s, \
             {} streaks, {wetted} columns wetted",
            sky.state.label(),
            sky.rain,
            sky.wind_speed(),
            rain::drops(&sky)
        );
    }

    if options.frost || options.thaw {
        // Winter on real country. Nothing is painted: the fixture finds a
        // shore near `--at` with a lake within the frost's reach, walks the
        // clock to the first snowing sky of the winter over it, and runs
        // `frost::settle` — the automaton both sides of the journal run —
        // through a day's worth of freezing passes, until the slope is
        // white and the lake is solid. `--thaw` runs it on into the warm
        // spell after and stops three quarters of the way through, so the
        // picture is the bare slope with the last of the snow on it and
        // open water again.
        let seed = world.seed();
        let generator = world.generator().clone();
        let sea = vx_world::gen::SEA_LEVEL;
        let lake_at = |x: i32, z: i32| generator.natural_height_at(x, z) < sea;
        let lake_within = |x: i32, z: i32| {
            let mut count = 0;
            let mut sum = (0i64, 0i64);
            for sz in (-frost::REACH..=frost::REACH).step_by(4) {
                for sx in (-frost::REACH..=frost::REACH).step_by(4) {
                    if lake_at(x + sx, z + sz) {
                        count += 1;
                        sum.0 += i64::from(x + sx);
                        sum.1 += i64::from(z + sz);
                    }
                }
            }
            (count, sum)
        };
        // The shore: dry ground with the most lake within reach of it —
        // where, how many lake columns, and their summed position.
        type Shore = ((i32, i32), i32, (i64, i64));
        let mut shore: Option<Shore> = None;
        for dz in (-160..=160).step_by(8) {
            for dx in (-160..=160).step_by(8) {
                let (x, z) = (options.at.0 + dx, options.at.1 + dz);
                if generator.natural_height_at(x, z) < sea + 2 {
                    continue;
                }
                let (count, sum) = lake_within(x, z);
                if count > 0 && shore.is_none_or(|(_, best, _)| count > best) {
                    shore = Some(((x, z), count, sum));
                }
            }
        }
        let ((x, z), lake, sum) =
            shore.ok_or("no lake shore within a hundred and sixty blocks of --at")?;
        let lake_centre = (
            (sum.0 / i64::from(lake)) as i32,
            (sum.1 / i64::from(lake)) as i32,
        );
        world.load_around(vx_core::BlockPos::new(x, 0, z).chunk(), 5);
        let top = world.surface_y(x, z).ok_or("the shore did not load")?;
        let standing = vx_core::BlockPos::new(x, top, z);

        let winter = 3 * vx_world::season::SEASON_TICKS;
        let first = (0..(vx_world::season::SEASON_TICKS / frost::EVERY))
            .map(|n| winter + n * frost::EVERY)
            .find(|&tick| vx_world::weather::at(seed, tick, x, z).snowing())
            .ok_or("it never snows over that shore all winter")?;
        let (mut snowed, mut froze) = (0, 0);
        let mut tick = first;
        let mut cold_passes = 0;
        let mut pictured = first;
        while cold_passes < vx_world::season::DAY_TICKS / frost::EVERY
            && tick < winter + 2 * vx_world::season::SEASON_TICKS
        {
            let sky = vx_world::weather::at(seed, tick, x, z);
            if sky.freezing() {
                let pass = frost::settle(&mut world, seed, tick, standing);
                snowed += pass.snowed;
                froze += pass.froze;
                cold_passes += 1;
                if sky.snowing() {
                    pictured = tick;
                }
            }
            tick += frost::EVERY;
        }

        let (mut thawed, mut melted) = (0, 0);
        if options.thaw {
            let mut water = Vec::new();
            while thawed + melted < (snowed + froze) * 3 / 4
                && tick < winter + 3 * vx_world::season::SEASON_TICKS
            {
                let pass = frost::settle(&mut world, seed, tick, standing);
                thawed += pass.thawed;
                melted += pass.melted;
                for at in pass.melted_at {
                    journal::wake_water(&mut water, &mut world, at);
                }
                tick += frost::EVERY;
            }
            for _ in 0..64 * 3 {
                if water.is_empty() {
                    break;
                }
                journal::settle_water(&mut water, &mut world);
            }
            pictured = tick;
        }
        remesh_all(&context, &mut renderer, &mut world);

        // From above the shore, looking down across the lake: the slope in
        // the foreground and the ice — or the water — filling the frame.
        let eye = glam::DVec3::new(x as f64 + 0.5, f64::from(top) + 12.0, z as f64 + 0.5);
        camera.position = eye;
        look_at(
            &mut camera,
            glam::DVec3::new(
                lake_centre.0 as f64 + 0.5,
                f64::from(sea) - 1.0,
                lake_centre.1 as f64 + 0.5,
            ),
        );
        renderer.update_camera(&context.queue, &camera);
        let sky = vx_world::weather::at(seed, pictured, x, z);
        weather_over = Some(sky);
        season_at = Some(pictured);

        println!(
            "{} capture: shore at {x},{z} (top {top}), lake centre {},{} — day {} of the year, \
             sky {} at {:.2}, {snowed} columns snowed, {froze} frozen, {thawed} thawed, {melted} melted",
            if options.thaw { "thaw" } else { "frost" },
            lake_centre.0,
            lake_centre.1,
            vx_world::season::day_of_year(pictured) + 1,
            sky.sky_word(),
            sky.temperature,
        );
    }

    if let Some(when) = options.fire.as_deref() {
        // A stand actually alight, run on the fire's own clock. Nothing here
        // is painted: the fixture lights one stem and lets `advance_fire` eat
        // outward under the real weather at a real tick, then photographs it
        // either mid-run or once it has gone out and started coming back.
        let after = match when {
            "burning" | "alight" | "running" => false,
            "after" | "ash" | "out" => true,
            other => return Err(format!("--fire wants burning or after (got {other})")),
        };
        let seed = world.seed();
        let generator = world.generator().clone();
        let natural = |x: i32, z: i32| generator.natural_height_at(x, z);

        // Woods on a slope, clear of any town: fire runs uphill, so a flat
        // site would photograph the least interesting half of the model. The
        // score is slope times how much of the stand is actually fuel, so a
        // bare scree face does not win it.
        let mut best: Option<((i32, i32), f32, f32)> = None;
        for ring in 1..30 {
            for step in -ring..=ring {
                for (x, z) in [
                    (options.at.0 + step * 14, options.at.1 + ring * 14),
                    (options.at.0 + step * 14, options.at.1 - ring * 14),
                    (options.at.0 + ring * 14, options.at.1 + step * 14),
                    (options.at.0 - ring * 14, options.at.1 + step * 14),
                ] {
                    if !generator.towns_near((x, z), 96).is_empty() {
                        continue;
                    }
                    let ground = vx_world::forest::survey(seed, x, z, &natural);
                    if ground.height <= vx_world::gen::SEA_LEVEL + 6 {
                        continue;
                    }
                    // How much of a small window grows something that burns.
                    // Off the flora field rather than the world, so this
                    // costs nothing before the chunks are loaded.
                    let sites = generator.towns_near((x, z), 96);
                    let height_at = |hx: i32, hz: i32| generator.height_with_sites(hx, hz, &sites);
                    let stand = vx_world::flora::trees_overlapping(
                        seed,
                        (x - 16, z - 16),
                        (x + 16, z + 16),
                        &height_at,
                        &natural,
                        &sites,
                    );
                    let fuel: f32 = stand
                        .iter()
                        .map(|tree| match tree.species {
                            vx_world::flora::Species::BogSpruce => 1.4,
                            vx_world::flora::Species::Spruce => 1.2,
                            vx_world::flora::Species::Ancient => 0.0,
                            _ => 0.7,
                        })
                        .sum();
                    // Slope is a gate, not the score. Maximising it lands on
                    // a cliff with three trees on it; what a fire wants is a
                    // stand thick enough for the crowns to touch, on ground
                    // that tilts enough for the uphill term to show.
                    // Slope is a gate, not the score. Maximising it lands on
                    // a cliff with three trees on it; what carries a fire is
                    // a stand thick enough for the crowns to touch, and the
                    // tilt only has to be enough for the uphill term to show.
                    if ground.slope < 0.45 {
                        continue;
                    }
                    let score = fuel;
                    if best.is_none_or(|(_, top, _)| score > top) {
                        best = Some(((x, z), score, ground.slope));
                    }
                }
            }
        }
        let ((x, z), score, slope) =
            best.ok_or("no forested slope within half a kilometre of --at")?;
        world.load_around(vx_core::BlockPos::new(x, 0, z).chunk(), 7);

        // A tick the country is actually dry and windy on. Hunted for rather
        // than written down, so the numbers the spread model reads are ones
        // the seed produces — the same discipline the storm capture uses.
        let mut weather_at = None;
        for step in 0..80_000u64 {
            let tick = step * 64;
            let sky = vx_world::weather::at(seed, tick, x, z);
            if sky.state.wet() || sky.wind_speed() < 7.0 {
                continue;
            }
            if vx_world::weather::fuel_moisture(seed, tick, x, z) > 0.9 {
                weather_at = Some((tick, sky));
                break;
            }
        }
        let (tick, sky) = weather_at.ok_or("no dry windy hour over --at in a day of the clock")?;

        // Light a crown, not a stem. A lone trunk's neighbours are air and
        // dirt, so a fire started at the base of one has nothing to catch:
        // what carries a fire is the fine fuel — needles and leaves — packed
        // against more of itself. The first capture of this fixture lit a
        // tuft of grass in a clearing and photographed a puff of smoke.
        let fine = |name: &str| name.ends_with("needles") || name.ends_with("leaves");
        let mut lit = None;
        'search: for radius in 0..20i32 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if dx.abs() != radius && dz.abs() != radius {
                        continue;
                    }
                    let (cx, cz) = (x + dx, z + dz);
                    let Some(surface) = world.surface_y(cx, cz) else {
                        continue;
                    };
                    for dy in 0..10 {
                        let at = vx_core::BlockPos::new(cx, surface - dy, cz);
                        let name = world.registry().get_or_air(world.block(at)).name.clone();
                        if fine(&name) && fire::fuel(&name).is_some() {
                            lit = Some(at);
                            break 'search;
                        }
                    }
                }
            }
        }
        let start = lit.ok_or("no crown to light within twenty blocks")?;
        let mut blaze = fire::Fire::new(start);
        if !blaze.light(&mut world, start) {
            return Err("the strike did not take".into());
        }
        let mut fires = vec![blaze];
        let want = if after { 60_000 } else { 2_400 };
        let mut steps = 0u64;
        // Counted off the reports rather than off the fires: a fire that has
        // finished is removed from the vector, so reading the totals at the
        // end of an `after` run would always say nothing ever burned.
        let mut eaten = 0;
        let mut caught = 0;
        // The ledger is written from the burn's own reports, exactly as
        // `journal::burn_and_grow` writes it — so the stands that come back
        // are the stands that actually went, not a square drawn round the
        // site by the fixture.
        let mut stands = succession::Ledger::default();
        while steps < want && !fires.is_empty() {
            for report in fire::advance_fire(&mut fires, &mut world, seed, tick + steps, &sky) {
                eaten += report.spent;
                caught += report.caught;
                if let Some(gone) = report.at.filter(|_| report.spent > 0) {
                    stands.disturb(gone, tick + steps);
                }
            }
            steps += 1;
            // `burning` wants the fire caught in the act rather than the
            // ashes: stop once a front is properly running, which is a
            // condition rather than a step count because how fast a stand
            // goes up is the model's business, not the fixture's.
            if !after && caught > 50 {
                break;
            }
        }
        let alight: usize = fires.iter().map(|blaze| blaze.burning().len()).sum();

        // And, for `after`, the first green coming back: the ledger takes the
        // burn and the clock stamps the stand it grew back to.
        let mut regrown = 0;
        if after {
            let sites = world.towns_near((x, z), 160);
            // Far enough on that the slowest thing here is back to a mixed
            // stand rather than bare ground: the clock runs per species, and
            // conifer is the slowest of the three.
            let seasons = succession::STEP_TICKS * 7;
            regrown = stands.advance(&mut world, tick + steps + seasons, &sites).len();
        }
        remesh_all(&context, &mut renderer, &mut world);

        // Stand downhill and look up the run, which is the direction the
        // model says the fire went.
        let uphill = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .max_by_key(|(dx, dz)| natural(x + dx * 10, z + dz * 10))
            .unwrap_or((1, 0));
        let back = glam::DVec3::new(f64::from(uphill.0), 0.0, f64::from(uphill.1));
        // Aimed at what actually burned, not at the site the search picked:
        // on a steep hillside those are tens of blocks apart in height, and
        // the first version of this photographed a hill with the fire off
        // the bottom of the frame.
        let heart = glam::DVec3::new(f64::from(start.x) + 0.5, f64::from(start.y), f64::from(start.z) + 0.5);
        let eye = heart + glam::DVec3::Y * 11.0 - back * 24.0;
        camera.position = eye;
        look_at(&mut camera, heart + back * 6.0);
        renderer.update_camera(&context.queue, &camera);
        renderer.set_objects(&context.device, &context.queue, &[]);

        println!(
            "fire capture: {when} at {x},{z} (slope {slope:.2}, stand {score:.1}) at tick {tick} \
             — wind {:.1} m/s, lit {start:?}, {steps} steps, {caught} caught, {eaten} eaten, \
             {alight} still alight in {} fronts, {regrown} stands stamped",
            sky.wind_speed(),
            fires.len()
        );
    }

    if let Some(when) = options.fell.as_deref() {
        // Fell a real tree and photograph it. The stem is found off worldgen,
        // cut on the face the camera stands on, and stepped on the game's own
        // clock — so the picture is a moment in a fall the game would have
        // had, not a pose.
        let swinging = match when {
            "swing" | "mid" | "arc" => true,
            "down" | "after" | "logs" => false,
            other => return Err(format!("--fell wants swing or down (got {other})")),
        };
        let seed = world.seed();
        let generator = world.generator().clone();
        let natural = |x: i32, z: i32| generator.natural_height_at(x, z);

        // A tall stem in hardwood country, well clear of any town.
        let mut found = None;
        'hunt: for ring in 0..70 {
            for step in -ring..=ring {
                for (cx, cz) in [
                    (options.at.0 + step * 16, options.at.1 + ring * 16),
                    (options.at.0 + step * 16, options.at.1 - ring * 16),
                    (options.at.0 + ring * 16, options.at.1 + step * 16),
                    (options.at.0 - ring * 16, options.at.1 + step * 16),
                ] {
                    if vx_world::forest::biome_at(seed, cx, cz, &natural)
                        != vx_world::forest::Biome::Hardwood
                    {
                        continue;
                    }
                    if !generator.towns_near((cx, cz), 110).is_empty() {
                        continue;
                    }
                    let sites = generator.towns_near((cx, cz), felling::TOWN_REACH);
                    let height_at = |x: i32, z: i32| generator.height_with_sites(x, z, &sites);
                    let trees = vx_world::flora::trees_overlapping(
                        seed,
                        (cx - 8, cz - 8),
                        (cx + 8, cz + 8),
                        &height_at,
                        &natural,
                        &sites,
                    );
                    if let Some(tree) = trees.into_iter().find(|tree| tree.height >= 6) {
                        found = Some(tree);
                        break 'hunt;
                    }
                }
            }
        }
        let tree = found.ok_or("no tree worth felling within a kilometre of --at")?;

        // Bring its ground with it, and cut it.
        let centre = vx_core::BlockPos::new(tree.base.x, 0, tree.base.z).chunk();
        world.load_around(centre, 8);
        let face = 1; // cut from the +X side, so it comes toward the camera
        let lean = felling::lean_at(&world, tree.base.x, tree.base.z);
        let (direction, chair) = felling::aim(face, lean);
        let mut falls = vec![felling::start(&mut world, &tree, direction, chair)];

        // Step it: half way over for the swing, all the way down for the
        // aftermath.
        let mut logs = 0;
        for _ in 0..64 * 8 {
            if swinging && falls.first().is_some_and(|fall| fall.angle > 0.9) {
                break;
            }
            for sweep in felling::advance_falls(&mut falls, &mut world) {
                if let Some(landing) = sweep.landing {
                    logs += landing.logs;
                }
            }
            if falls.is_empty() {
                break;
            }
        }
        remesh_all(&context, &mut renderer, &mut world);

        // Stand off to the side of the fall line, so the arc reads across the
        // frame rather than coming at the lens.
        let hinge = glam::DVec3::new(
            f64::from(tree.base.x) + 0.5,
            f64::from(tree.base.y) + 1.0,
            f64::from(tree.base.z) + 0.5,
        );
        let across = glam::DVec3::new(-direction.z, 0.0, direction.x);
        let eye = hinge + direction * 9.0 + across * 15.0 + glam::DVec3::Y * 7.0;
        camera.position = eye;
        look_at(&mut camera, hinge + direction * 4.0 + glam::DVec3::Y * 2.5);
        // Camera before objects: `set_objects` culls against the last
        // uploaded frustum, the trap stage 31 found the hard way.
        renderer.update_camera(&context.queue, &camera);

        let mut objects = Vec::new();
        for fall in &falls {
            let rig = trunk_rig(fall);
            if let Some(yaw) = rig::yaw_towards(fall.direction.x as f32, fall.direction.z as f32) {
                objects.extend(
                    rig.objects_pitched(
                        camera.relative(fall.hinge_point()),
                        yaw,
                        std::f32::consts::FRAC_PI_2 - fall.angle,
                        0.0,
                    )
                    .into_iter()
                    .map(vx_render::Object::already_relative),
                );
            }
        }
        renderer.set_objects(&context.device, &context.queue, &objects);

        println!(
            "fell capture: {:?} {} high at {},{} — angle {:.2} rad, {logs} logs down{}",
            tree.species,
            tree.height,
            tree.base.x,
            tree.base.z,
            falls.first().map_or(std::f32::consts::FRAC_PI_2, |fall| fall.angle),
            if chair { ", barber chair" } else { "" }
        );
    }

    if let Some(which) = options.forest.as_deref() {
        // Frame a stand of one of the three forests. The site is *found*
        // rather than written down: the fixture walks out from `--at` looking
        // for a column of the right forest with more of the same around it,
        // so the picture is a real stand rather than one lucky tree.
        let wanted = match which {
            "bog" | "swamp" | "low" => vx_world::forest::Biome::Bog,
            "cove" | "hardwood" | "mid" => vx_world::forest::Biome::Hardwood,
            "high" | "subalpine" | "conifer" | "treeline" => vx_world::forest::Biome::Subalpine,
            other => {
                return Err(format!(
                    "--forest wants bog, cove, high or treeline (got {other})"
                ))
            }
        };
        // `treeline` wants the wind-flattened stuff, which only grows on the
        // very tops: score by height instead of by how solid the stand is.
        let summit = which == "treeline";
        let seed = world.seed();
        // A generator of its own, so the search and the census can go on
        // reading the height field while the world is being loaded and
        // remeshed around the site they picked.
        let generator = world.generator().clone();
        let natural = |x: i32, z: i32| generator.natural_height_at(x, z);

        // How much of a 40-block square around a column grows the same
        // forest. A stand, not a stray column on an ecotone.
        let solidity = |x: i32, z: i32| {
            let mut same = 0;
            for dx in (-20..=20).step_by(10) {
                for dz in (-20..=20).step_by(10) {
                    if vx_world::forest::biome_at(seed, x + dx, z + dz, &natural) == wanted {
                        same += 1;
                    }
                }
            }
            same
        };

        let mut best: Option<(i32, (i32, i32))> = None;
        'search: for ring in 0..90 {
            for step in -ring..=ring {
                for (x, z) in [
                    (options.at.0 + step * 24, options.at.1 + ring * 24),
                    (options.at.0 + step * 24, options.at.1 - ring * 24),
                    (options.at.0 + ring * 24, options.at.1 + step * 24),
                    (options.at.0 - ring * 24, options.at.1 + step * 24),
                ] {
                    if vx_world::forest::biome_at(seed, x, z, &natural) != wanted {
                        continue;
                    }
                    // Not on somebody's main street: a town's lawns and
                    // plateau are not what a forest capture is about.
                    if !generator.towns_near((x, z), 110).is_empty() {
                        continue;
                    }
                    let score = if summit {
                        natural(x, z)
                    } else {
                        solidity(x, z)
                    };
                    if best.is_none_or(|(had, _)| score > had) {
                        best = Some((score, (x, z)));
                    }
                    if !summit && score == 25 {
                        break 'search;
                    }
                    if summit && score > vx_world::forest::TREELINE_Y + 6 {
                        break 'search;
                    }
                }
            }
        }
        let (score, (x, z)) = best.ok_or_else(|| {
            format!("no {which} forest within two kilometres of --at; try another seed")
        })?;

        // The stand may be a long way from `--at`: bring its ground with it.
        let centre = vx_core::BlockPos::new(x, 0, z).chunk();
        // Wide enough that the far edge of the loaded ground is out of shot
        // — the chunks past it have no lit neighbours and read as a black
        // bite out of the hillside — and no wider, because past about nine
        // the frame starts losing meshes it has no room for.
        world.load_around(centre, 8);
        remesh_all(&context, &mut renderer, &mut world);

        let ground = world.surface_y(x, z).unwrap_or(80) as f32;
        // Stand off and a little above: a forest is a silhouette, and a
        // silhouette needs sky behind it. The stand-off column has its own
        // ground, which may be higher than the stand's — put the camera
        // above whichever is higher or the shot is taken from inside a hill,
        // looking up at the underside of the country.
        // Stand downhill of the stand, so the trees have sky behind them
        // rather than more hillside. On flat ground the gradient says
        // nothing, and any direction will do.
        let slope_x = (natural(x + 8, z) - natural(x - 8, z)) as f32;
        let slope_z = (natural(x, z + 8) - natural(x, z - 8)) as f32;
        let fall = (slope_x * slope_x + slope_z * slope_z).sqrt();
        let (back_x, back_z) = if fall > 3.0 {
            (
                x - (slope_x / fall * 34.0) as i32,
                z - (slope_z / fall * 34.0) as i32,
            )
        } else {
            (x - 26, z - 34)
        };
        let behind = world.surface_y(back_x, back_z).unwrap_or(80) as f32;
        let eye = glam::Vec3::new(
            back_x as f32,
            ground.max(behind) + 13.0,
            back_z as f32,
        );
        camera.position = eye.as_dvec3();
        look_at(&mut camera, glam::Vec3::new(x as f32, ground + 6.0, z as f32));
        renderer.update_camera(&context.queue, &camera);

        // What actually grows here, counted rather than claimed.
        let sites = generator.towns_overlapping((x - 120, z - 120), (x + 120, z + 120));
        let height_at = |cx: i32, cz: i32| generator.height_with_sites(cx, cz, &sites);
        let trees = vx_world::flora::trees_overlapping(
            seed,
            (x - 120, z - 120),
            (x + 120, z + 120),
            &height_at,
            &natural,
            &sites,
        );
        let count = |species| {
            trees
                .iter()
                .filter(|tree| tree.species == species)
                .count()
        };
        println!(
            "forest capture: {which} at {x},{z} (ground {ground}, score {score}) — \
             {} hardwood, {} giant, {} spruce, {} bog spruce, {} krummholz in 240 blocks",
            count(vx_world::flora::Species::Hardwood),
            count(vx_world::flora::Species::Giant),
            count(vx_world::flora::Species::Spruce),
            count(vx_world::flora::Species::BogSpruce),
            count(vx_world::flora::Species::Krummholz),
        );
    }

    if options.well {
        // The nearest field to `--at`, off the lattice itself rather than by
        // sampling columns: bodies are hundreds of blocks apart, and a
        // sampler coarse enough to be quick is coarse enough to miss one.
        let bodies = vx_world::reservoir::reservoirs_overlapping(
            world.seed(),
            vx_core::BlockPos::new(options.at.0 - 2_400, 0, options.at.1 - 2_400),
            vx_core::BlockPos::new(options.at.0 + 2_400, 128, options.at.1 + 2_400),
        );
        let nearest = bodies.into_iter().min_by(|left, right| {
            let span = |body: &vx_world::reservoir::Reservoir| {
                let dx = body.centre[0] - options.at.0 as f32;
                let dz = body.centre[2] - options.at.1 as f32;
                dx * dx + dz * dz
            };
            span(left).total_cmp(&span(right))
        });
        let body = nearest.ok_or("no oil or gas within two kilometres of --at")?;
        let (x, z) = (
            body.centre[0].round() as i32,
            body.centre[2].round() as i32,
        );


        // The head may be kilometres from `--at`: bring the ground with it.
        let centre = vx_core::BlockPos::new(x, 0, z).chunk();
        world.load_around(centre, 6);
        let ground = world.surface_y(x, z).unwrap_or(80);
        let head = vx_core::BlockPos::new(x, ground + 1, z);
        // A wellhead stands on a pad. Levelling one is what a crew would do
        // anyway, and it is also the difference between a picture of a
        // machine and a picture of a hillside.
        let stone = world.registry().id_of("engine:stone").unwrap();
        for dx in -6..=6 {
            for dz in -6..=6 {
                let column = vx_core::BlockPos::new(x + dx, ground, z + dz);
                world.set_block(column, stone);
                for dy in 1..14 {
                    world.set_block(
                        vx_core::BlockPos::new(x + dx, ground + dy, z + dz),
                        vx_core::BlockId::AIR,
                    );
                }
            }
        }
        if let Some(id) = world.registry().id_of("engine:wellhead") {
            world.set_block(head, id);
        }
        remesh_all(&context, &mut renderer, &mut world);

        // A real hole, sunk through the real machine and drilled most of the
        // way down, so the panel shows a percentage the code chose.
        let mut pile = vx_agent::Stockpile::default();
        pile.add(well::CASING.0, 20);
        pile.add(well::CEMENT.0, 200);
        let mut holes = well::Wells::default();
        holes.spud(head, world.seed(), &mut pile)?;
        let total = holes.at(head).map_or(0, |hole| hole.total_drill);
        holes.tick(total * 3 / 5, &mut pile);

        let content = well::WellContent {
            at: head,
            trace: true,
            hole: holes.at(head).copied(),
            refusal: None,
            feedback: Some("SPUDDED IN - THE STRING IS GOING DOWN".into()),
        };
        let pixels = well::render_well(&content);
        let panel_width = well::WELL_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = well::WELL_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            WELL_SLOT,
            &context.device,
            &context.queue,
            (well::WELL_WIDTH, well::WELL_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );

        let at = glam::DVec3::new(f64::from(head.x) + 0.5, f64::from(head.y), f64::from(head.z) + 0.5);
        // Camera first: `set_objects` culls against the last uploaded
        // frustum, so a scene built before the camera moves is a scene the
        // culler throws away.
        // Framed off-centre on purpose: the panel is drawn in the middle of
        // the screen, so the machine it belongs to has to live beside it.
        camera.position = at + glam::DVec3::new(7.0, 4.0, 9.0);
        look_at(&mut camera, at + glam::DVec3::new(4.0, 1.0, 5.0));
        renderer.update_camera(&context.queue, &camera);

        let mut objects = Vec::new();
        objects.extend(
            Rig::player()
                .objects(
                    camera.relative(at + glam::DVec3::new(3.5, -1.0, 3.0)),
                    std::f32::consts::PI * 1.15,
                    0.0,
                )
                .into_iter()
                .map(vx_render::Object::already_relative),
        );
        renderer.set_objects(&context.device, &context.queue, &objects);
        println!(
            "well capture: {} at {x},{z}, {} in the ground, drilled {}%",
            body.fluid.name(),
            body.volume(),
            holes.at(head).map_or(0.0, |hole| hole.drilled() * 100.0)
        );
    }

    // A uranium face. Staged, and worth saying so: the world never puts the
    // deep ore this close to daylight, so the fixture cuts a bench into the
    // hillside and lines its back wall — what the same face looks like at
    // the bottom of a decline, where a camera cannot see anything at all.
    if options.hot {
        let hot = world
            .registry()
            .id_of("engine:uranium_ore")
            .ok_or("no uranium ore registered")?;
        let stone = world.registry().id_of("engine:stone").unwrap();
        let wall_z = options.at.1 + 6;
        let ground = world.surface_y(options.at.0, wall_z).unwrap_or(80);

        for x in options.at.0 - 5..=options.at.0 + 5 {
            for z in wall_z - 8..=wall_z {
                for y in ground + 1..ground + 12 {
                    world.set_block(vx_core::BlockPos::new(x, y, z), vx_core::BlockId::AIR);
                }
                world.set_block(vx_core::BlockPos::new(x, ground, z), stone);
            }
        }
        // The face itself: ore in the middle, host rock around it, so the
        // tile reads against something rather than filling the frame.
        for x in options.at.0 - 4..=options.at.0 + 4 {
            for y in ground + 1..=ground + 6 {
                let ore = (options.at.0 - 3..=options.at.0 + 3).contains(&x)
                    && (ground + 2..=ground + 5).contains(&y);
                world.set_block(
                    vx_core::BlockPos::new(x, y, wall_z),
                    if ore { hot } else { stone },
                );
            }
        }
        remesh_all(&context, &mut renderer, &mut world);

        let face = glam::Vec3::new(
            options.at.0 as f32 + 0.5,
            (ground + 3) as f32,
            wall_z as f32 + 0.5,
        );
        // Camera first, then the scene: the culler works off the last
        // uploaded frustum.
        camera.position = (face + glam::Vec3::new(2.5, 2.0, -7.0)).as_dvec3();
        look_at(&mut camera, face);
        renderer.update_camera(&context.queue, &camera);

        let mut objects = Vec::new();
        objects.extend(Rig::player().objects(
            face + glam::Vec3::new(1.5, -3.0, -4.0),
            0.0,
            0.0,
        ));
        renderer.set_objects(&context.device, &context.queue, &objects);

        // What standing here actually costs, run through the real dose so
        // the readout is a measurement rather than a caption: a minute at
        // the face, shielded by nothing.
        let standing = face + glam::Vec3::new(1.5, -2.0, -2.5);
        let rads = dose::exposure(&world, standing.as_dvec3());
        let mut carried = dose::Dose::default();
        for _ in 0..600 {
            carried.tick(rads, 0.1, 0);
        }
        let hud_skills = Skills::new();
        let hud_pixels = hud::render_hud(&hud::HudContent {
            skills: &hud_skills,
            time: TimeOfDay::new(options.time),
            status: Some("CUTTING THE FACE".to_string()),
            drilling: Some(0.42),
            level_up: None,
            greeting: None,
            reconnecting: false,
            bounty: 0,
            watched: false,
            movement: hud::MovementReadout {
                stance: "STAND",
                stamina: 0.9,
                load: 0.55,
            },
            ammo: None,
            panicking: 0,
            kestrel: None,
            fuel: Some("HHO 9".to_string()),
            condition: None,
            dose: carried.readout(),
            dark: None,
            deputies: 0,
            optic: Some("LAMP"),
        });
        renderer.set_overlay(
            HUD_SLOT,
            &context.device,
            &context.queue,
            (hud::HUD_WIDTH, hud::HUD_HEIGHT),
            &hud_pixels,
            vx_render::OverlayRect {
                x: 10.0,
                y: height as f32 - hud::HUD_HEIGHT as f32 * hud::HUD_SCALE - 10.0,
                width: hud::HUD_WIDTH as f32 * hud::HUD_SCALE,
                height: hud::HUD_HEIGHT as f32 * hud::HUD_SCALE,
            },
        );
        println!(
            "hot capture: {rads:.1} rads a second at the face, {:.0} carried after a minute",
            carried.rads
        );
    }

    // The thing in the deep, hunting. Everything below is the real director
    // and the real creature: the fixture cuts a gallery and then gets out of
    // the way.
    if options.dark {
        let stone = world.registry().id_of("engine:stone").unwrap();
        let floor = 24;
        // A straight gallery, a hundred blocks of it, buried deep enough
        // that `is_deep` is satisfied along its whole length.
        for x in options.at.0 - 110..=options.at.0 + 110 {
            for z in options.at.1 - 5..=options.at.1 + 5 {
                for y in floor..floor + 6 {
                    world.set_block(vx_core::BlockPos::new(x, y, z), vx_core::BlockId::AIR);
                }
                // Floor, ceiling and walls all poured explicitly. A gallery
                // with a hole in its roof is a gallery with daylight in it,
                // and one lit block at the end of a dark corridor reads as a
                // rendering bug rather than as rock.
                world.set_block(vx_core::BlockPos::new(x, floor - 1, z), stone);
                world.set_block(vx_core::BlockPos::new(x, floor + 6, z), stone);
                world.set_block(vx_core::BlockPos::new(x, floor + 7, z), stone);
            }
            for z in [options.at.1 - 5, options.at.1 + 5] {
                for y in floor..floor + 6 {
                    world.set_block(vx_core::BlockPos::new(x, y, z), stone);
                }
            }
        }
        remesh_all(&context, &mut renderer, &mut world);

        let player = glam::DVec3::new(
            f64::from(options.at.0) + 0.5,
            f64::from(floor),
            f64::from(options.at.1) + 0.5,
        );
        let mut dark = stalker::TheDark::default();
        let mut tells: Vec<String> = Vec::new();
        // Cut rock until something comes, then keep cutting while it closes.
        for step in 0..6_000 {
            if step % 4 == 0 {
                dark.hear(player + glam::DVec3::new(2.0, 0.0, 0.0), 0.6);
            }
            let report = dark.update(1.0 / 30.0, &world, player, 0xdeadbeef);
            for line in report.tells {
                if !tells.contains(&line) {
                    tells.push(line);
                }
            }
            let close = dark
                .present()
                .is_some_and(|it| (it.position - player).length() < 4.5);
            if close {
                break;
            }
        }

        // The camera goes up *first*. `set_objects` culls against whatever
        // frustum was last uploaded, so building the scene before pointing
        // the camera at it quietly throws the scene away — the third
        // variation on this fixture trap, after "run after the villagers'
        // pass" and "remember to call `update_camera` at all".
        let at = dark
            .present()
            .map_or(player + glam::DVec3::new(20.0, 0.0, 0.0), |it| it.position);
        // Close enough that it fills the lamp cone: this is the last thing a
        // player sees before it is on them, so the picture is that distance
        // and not a safer one.
        let towards = (at - player).normalize_or_zero();
        camera.position = at - towards * 3.2 + glam::DVec3::Y * 1.45;
        look_at(&mut camera, at + glam::DVec3::Y * 0.55);
        renderer.update_camera(&context.queue, &camera);

        // No player body in this one: the camera is standing where they
        // are, and the picture is about what is coming up the gallery.
        let mut objects = Vec::new();
        if let Some(stalker) = dark.present() {
            objects.extend(
                Rig::stalker()
                    .objects(camera.relative(stalker.position), stalker.yaw, 0.0)
                    .into_iter()
                    .map(vx_render::Object::already_relative),
            );
        }
        renderer.set_objects(&context.device, &context.queue, &objects);

        let mut console = terminal::Terminal::default();
        console.toggle();
        for line in tells.iter().take(4) {
            console.say(terminal::Kind::Warn, line.clone());
        }
        if let Some(stalker) = dark.present() {
            let (taken, of) = stalker.wounds();
            console.say(
                terminal::Kind::Note,
                format!(
                    "{} - {taken}/{of} - {:.0} BLOCKS OFF",
                    stalker.mood.name(),
                    (stalker.position - player).length()
                ),
            );
        }
        if !options.close {
            let pixels = terminal::render_terminal(&console, false);
            let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
            let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
            renderer.set_overlay(
                TERM_SLOT,
                &context.device,
                &context.queue,
                (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
                &pixels,
                vx_render::OverlayRect {
                    x: (width as f32 - panel_width) / 2.0,
                    y: (height as f32 - panel_height) / 2.0,
                    width: panel_width,
                    height: panel_height,
                },
            );
        }

        // Down the gallery from behind the player, so the thing is coming at
        // the camera the way it comes at you.
        println!(
            "dark camera {:.1},{:.1},{:.1} looking at {:.1},{:.1},{:.1}; player {:.1},{:.1},{:.1}",
            camera.position.x, camera.position.y, camera.position.z,
            at.x, at.y, at.z,
            player.x, player.y, player.z
        );
        println!(
            "dark capture: {} at {:.0} blocks, {} tells",
            dark.present().map_or("NOTHING", |it| it.mood.name()),
            dark.present().map_or(0.0, |it| (it.position - player).length()),
            tells.len()
        );
    }

    // The townsfolk, close enough to see whether anybody is home behind the
    // eyes. Late like the posse fixture: the villagers' own pass calls
    // `set_objects` and would replace this one.
    if options.faces {
        let mut town = Villagers::new();
        let ground = world.surface_y(options.at.0, options.at.1).unwrap_or(80);
        let player = glam::DVec3::new(
            f64::from(options.at.0) + 0.5,
            f64::from(ground) + 1.0,
            f64::from(options.at.1) + 0.5,
        );

        // Walk the town a while with the player standing there, so the ones
        // who notice have noticed and the gaze is theirs rather than staged.
        let watching = awareness::Surroundings {
            player: Some(player),
            ..awareness::Surroundings::empty()
        };
        for _ in 0..600 {
            town.update(1.0 / 60.0, TimeOfDay::new(options.time), &watching);
        }

        // The three nearest, stood in a row facing the camera at a
        // conversational distance — the variants differ, and a row is how
        // you see that.
        let mut folk = town.positions();
        folk.sort_by(|left, right| {
            (*left - player)
                .length()
                .total_cmp(&(*right - player).length())
        });
        let rigs = Villagers::rigs();

        // The camera first: `set_objects` culls against the last uploaded
        // frustum, which is the trap stage 31 found the hard way.
        let front = player + glam::DVec3::new(0.0, 0.0, 3.2);
        camera.position = front + glam::DVec3::new(0.0, 1.55, 0.0);
        look_at(&mut camera, player + glam::DVec3::new(0.0, 1.45, 0.0));
        renderer.update_camera(&context.queue, &camera);

        let mut objects = Vec::new();
        for (index, offset) in [-1.15f32, 0.0, 1.15].into_iter().enumerate() {
            let at = player + glam::DVec3::new(f64::from(offset), 0.0, 0.0);
            let rig = &rigs[index % rigs.len()];
            // Facing the camera, and looking at it: the gaze is computed by
            // the same call the game makes every frame.
            let yaw = rig::yaw_towards(0.0, (camera.position.z - at.z) as f32).unwrap_or(0.0);
            let eye = at + glam::DVec3::new(0.0, 1.4, 0.0);
            let gaze = rig::Gaze::towards(eye, yaw, camera.position);
            objects.extend(
                rig.objects_looking(camera.relative(at), yaw, 0.0, gaze)
                    .into_iter()
                    .map(vx_render::Object::already_relative),
            );
        }
        renderer.set_objects(&context.device, &context.queue, &objects);
        println!(
            "faces capture: {} in town, three drawn looking at the camera",
            folk.len()
        );
    }

    // A hamlet inside the wall it finally has.
    if options.ministar {
        // A whole one for the picture: a third of them have been let go, and
        // a ruin shows the rubble rather than the shape.
        let near = world.towns_near(options.at, 6_000);
        let mini = |site: &&vx_world::town::TownSite| {
            vx_world::fort::fort_for(site).trace == vx_world::fort::Trace::MiniStar
        };
        let site = near
            .iter()
            .filter(mini)
            .find(|site| !vx_world::fort::fort_for(site).ruined)
            .or_else(|| near.iter().find(mini))
            .copied()
            .ok_or("no mini star within six kilometres of --at")?;
        let fort = vx_world::fort::fort_for(&site);

        // Bring the ground with us: the hamlet may be kilometres from --at.
        let centre = vx_core::BlockPos::new(site.centre.0, 0, site.centre.1).chunk();
        world.load_around(centre, 8);
        remesh_all(&context, &mut renderer, &mut world);

        let middle = glam::Vec3::new(
            site.centre.0 as f32,
            site.ground as f32,
            site.centre.1 as f32,
        );
        // Overhead, because a bastioned trace only reads as bastioned from a
        // height — the same lesson the full fort fixture already learned.
        // High enough that the whole ring fits: at a sixty-degree field of
        // view the ground covered is about the height, so anything less than
        // twice the radius crops the wall the picture is about.
        camera.position = (middle + glam::Vec3::new(0.0, fort.radius * 2.7, 0.1)).as_dvec3();
        look_at(&mut camera, middle);
        renderer.update_camera(&context.queue, &camera);
        renderer.set_objects(&context.device, &context.queue, &[]);
        println!(
            "ministar capture: {} at {:?}, radius {:.0}, ruined {}",
            site.name.head(),
            site.centre,
            fort.radius,
            fort.ruined
        );
    }

    // The ward, and the panel that is the whole of what it does.
    if options.ward {
        let content = clinic::ClinicContent {
            town: vx_world::town::home_site().name.to_string(),
            cursor: 0,
            condition: (2, health::MAX_HITS),
            rads: 143.0,
            medkits: 1,
            credits: 260,
            feedback: Some("A DEPUTY GOT THREE OF THEM INTO YOU".into()),
        };
        let pixels = clinic::render_clinic(&content);
        let panel_width = clinic::WARD_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = clinic::WARD_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            WARD_SLOT,
            &context.device,
            &context.queue,
            (clinic::WARD_WIDTH, clinic::WARD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );

        // And frame the building behind it, so the panel is attached to a
        // place rather than floating over a field.
        //
        // From outside, deliberately. A ward is a nine-by-seven shed with
        // one block of headroom over the cots: a camera in there photographs
        // a sheet of corrugated metal and whatever the panel does not cover,
        // which is a worse picture than the door somebody walks through.
        let site = vx_world::town::home_site();
        let door = glam::Vec3::new(
            site.centre.0 as f32 + 14.5,
            site.ground as f32 + 1.0,
            site.centre.1 as f32 + 7.0,
        );
        camera.position = (door + glam::Vec3::new(-8.0, 9.0, -15.0)).as_dvec3();
        look_at(&mut camera, door + glam::Vec3::new(1.0, 0.0, 3.0));
        renderer.update_camera(&context.queue, &camera);
        let cot = vx_core::BlockPos::new(site.centre.0 + 14, site.ground + 1, site.centre.1 + 12);
        println!(
            "ward capture: the clinic door at {:?}, cots at {:?} ({})",
            (site.centre.0 + 14, site.centre.1 + 7),
            (cot.x, cot.z),
            world.registry().get_or_air(world.block(cot)).name
        );
    }

    if options.device {
        // The handheld's roster, over whatever the frame already shows.
        let mut mining = Mining::default();
        let ground = world.surface_y(options.at.0, options.at.1).unwrap_or(80);
        mining.ensure_flier(glam::DVec3::new(
            f64::from(options.at.0),
            f64::from(ground),
            f64::from(options.at.1),
        ));
        let mut handheld = device::Device::new();
        handheld.open_list();
        handheld.feedback = Some("CONTROL TAKEN".into());
        if options.handheld_map {
            handheld.turn_page();
        }
        let roster = mining.roster(camera.position);
        // The map page needs the country under it; the roster page does not.
        let explored = {
            let mut seen = map::MapState::new();
            seen.explore_around(chunk_at(camera.position), 6);
            seen
        };
        let markers = [map::Marker {
            x: options.at.0,
            z: options.at.1,
            colour: map::colour::PLAYER,
            radius: 2,
        }];
        let country = device::Country {
            world: &world,
            explored: &explored,
            centre: options.at,
            markers: &markers,
        };
        // The unit itself, held. `--raising` catches it mid-swing, which is
        // the frame that shows it is an object rather than a panel.
        handheld.raise = if options.raising { 0.45 } else { 1.0 };

        // `--arcade` puts the toy on the glass instead of the roster: a
        // cartridge printed, a run started, and the game played until the
        // frame is worth looking at. The bot below only ever presses the
        // buttons a player presses, so the picture is a real position in a
        // real run rather than a pose set by hand.
        let mut cabinet = arcade::Arcade::default();
        if options.arcade {
            handheld.page = device::Page::Arcade;
            handheld.feedback = None;
            cabinet.print();
            cabinet.start();
            let tick = 1.0 / 60.0;
            for _ in 0..3_600 {
                let pose = cabinet.pose();
                let seen = cabinet.sighted();
                let Some((tx, tz)) = seen.or_else(|| cabinet.nearest_standing()) else {
                    break;
                };
                // Head for what you can see; otherwise for the next corner on
                // the way to the nearest one, so the bot walks the corridors
                // instead of leaning on a wall with a target behind it.
                let (ax, az) = if seen.is_some() {
                    (tx, tz)
                } else {
                    arcade_waypoint(cabinet.level(), (pose.x, pose.z), (tx, tz))
                };
                let (dx, dz) = (ax - pose.x, az - pose.z);
                // Two distances: to the thing being aimed at, and to the one
                // being hunted. The first steers, the second decides when to
                // stop closing.
                let reach = (dx * dx + dz * dz).sqrt();
                let range = ((tx - pose.x).powi(2) + (tz - pose.z).powi(2)).sqrt();
                // Shortest way round to the bearing of the thing being headed
                // for.
                let mut error = dz.atan2(dx) - pose.facing;
                while error > std::f32::consts::PI {
                    error -= std::f32::consts::TAU;
                }
                while error < -std::f32::consts::PI {
                    error += std::f32::consts::TAU;
                }
                // Standing near the middle of a corridor cell rather than
                // scraping a wall, one already down, and the next one square
                // in the sights a few paces off: that is the frame.
                let centred = (pose.x.fract() - 0.5).abs() < 0.25
                    && (pose.z.fract() - 0.5).abs() < 0.25;
                if cabinet.score > 0
                    && seen.is_some()
                    && centred
                    && (3.0..7.0).contains(&range)
                    && error.abs() < 0.06
                {
                    break;
                }
                cabinet.step(
                    tick,
                    arcade::Buttons {
                        turn_left: error < -0.04,
                        turn_right: error > 0.04,
                        forward: error.abs() < 0.5
                            && reach > 0.2
                            && (seen.is_none() || range > 3.2),
                        // Shoot the first one on sight to get a score on the
                        // strip, then hold fire so the capture catches the
                        // next one alive and square in the sights.
                        fire: seen.is_some()
                            && error.abs() < 0.10
                            && (cabinet.score == 0 || range < 2.8),
                        ..arcade::Buttons::default()
                    },
                );
            }
            println!(
                "arcade capture: floor {}, health {}, ammo {}, {} standing, score {}",
                cabinet.floor,
                cabinet.health,
                cabinet.ammo,
                cabinet.standing(),
                cabinet.score
            );
        }

        let mut pixels = if options.arcade {
            arcade::render(&cabinet)
        } else {
            device::render_device(&handheld, &roster, Some(&country), None)
        };
        device::dim(&mut pixels, handheld.raise);

        // Camera before objects: `set_objects` culls against the last
        // uploaded frustum, the trap stage 31 found the hard way.
        renderer.update_camera(&context.queue, &camera);
        let forward = camera.forward();
        let right = camera.right();
        let held = device::carried_at(renderer.relative(camera.position), forward, right, handheld.raise);
        let (_, _, _, tilt) = device::carry(handheld.raise);
        let mut objects: Vec<vx_render::Object> = Rig::handheld()
            .objects_pitched(
                held,
                rig::yaw_towards(forward.x, forward.z).unwrap_or(0.0),
                camera.pitch - tilt,
                0.0,
            )
            .into_iter()
            .map(vx_render::Object::already_relative)
            .collect();
        // Whoever was already on screen stays on screen.
        let rigs = Villagers::rigs();
        let mut town = Villagers::new();
        let alone = awareness::Surroundings::empty();
        for _ in 0..600 {
            town.update(1.0 / 60.0, TimeOfDay::new(options.time), &alone);
        }
        objects.extend(town.objects(&rigs, |at| camera.relative(at)));
        renderer.set_objects(&context.device, &context.queue, &objects);

        // And the readout on its glass, exactly as the game places it.
        let corners = device::screen_corners(camera.local_position(), forward, right, handheld.raise);
        if let Some(rect) =
            device::screen_rect(camera.view_projection(), corners, (width as f32, height as f32))
        {
            renderer.set_overlay(
                DEVICE_SLOT,
                &context.device,
                &context.queue,
                (device::DEVICE_WIDTH, device::DEVICE_HEIGHT),
                &pixels,
                rect,
            );
            println!(
                "handheld capture: raise {:.2}, screen {:.0}x{:.0} at {:.0},{:.0}",
                handheld.raise, rect.width, rect.height, rect.x, rect.y
            );
        }

        // And the banner that rides a live feed.
        if let Some(listing) = roster.first() {
            let banner = device::render_feed_banner(listing, true, 0.72);
            let banner_width = device::BANNER_WIDTH as f32 * device::BANNER_SCALE;
            let banner_height = device::BANNER_HEIGHT as f32 * device::BANNER_SCALE;
            renderer.set_overlay(
                FEED_SLOT,
                &context.device,
                &context.queue,
                (device::BANNER_WIDTH, device::BANNER_HEIGHT),
                &banner,
                vx_render::OverlayRect {
                    x: (width as f32 - banner_width) / 2.0,
                    y: 12.0,
                    width: banner_width,
                    height: banner_height,
                },
            );
        }
    }

    #[cfg(feature = "gold")]
    if options.gold_capture {
        // Fixture telemetry: the capture shows the console's shape, and the
        // gold border marks the frame as an operator's session.
        let tuning = tuning::Tuning::default();
        let telemetry = gold::Telemetry {
            tick: 4_200,
            position: glam::DVec3::new(-13.5, 73.0, 9.5),
            stance: "STAND",
            stamina: 87.0,
            credits: 250,
            bounty: 0,
            drones: 1,
            fliers: 1,
            base_total: 340,
            town_name: Some("STONEHAVEN".into()),
            town_centre: Some((0, 0)),
            stocks: [400.0, 200.0, 100.0, 40.0, 15.0, 2.0, 24.0, 31.0],
            tuning: &tuning,
            world_hash: None,
        };
        let panel = gold::Gold {
            open: true,
            tab_index: 4,
            ..gold::Gold::default()
        };
        let pixels = gold::render_gold(&panel, &telemetry);
        let panel_width = gold::GOLD_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = gold::GOLD_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            GOLD_SLOT,
            &context.device,
            &context.queue,
            (gold::GOLD_WIDTH, gold::GOLD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }
    #[cfg(not(feature = "gold"))]
    if options.gold_capture {
        eprintln!("this build carries no gold panel; --gold-capture ignored");
    }

    if options.permit {
        // A neighbour's lock, seen by somebody with no business there — the
        // panel's whole job this round is telling you that plainly.
        let here = vx_world::town::home_site();
        let mut book = permits::Permits::new();
        book.set_sites(vec![here]);
        book.caught(permits::BOUNTY_BREACH, 1);
        let claim = permits::claims_for(&here)
            .into_iter()
            .find(|claim| matches!(claim.owner, permits::Claimant::Resident(_)))
            .expect("the hometown has residents");
        let mut panel = permits::PermitPanel::default();
        let lock = claim.lock.unwrap_or(vx_core::BlockPos::new(0, 0, 0));
        panel.open_at(lock, claim);
        let pixels = permits::render_permit(&panel, &book);
        let panel_width = permits::PERMIT_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = permits::PERMIT_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            PERMIT_SLOT,
            &context.device,
            &context.queue,
            (permits::PERMIT_WIDTH, permits::PERMIT_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.welcome {
        let panel = intro::Intro::new();
        let pixels = intro::render_intro(&panel);
        let panel_width = intro::INTRO_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = intro::INTRO_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            INTRO_SLOT,
            &context.device,
            &context.queue,
            (intro::INTRO_WIDTH, intro::INTRO_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.terminal {
        // A session's worth of console: a few answered questions, an order,
        // a refusal and the log lines the game itself pushed in.
        let mut console = terminal::Terminal::default();
        console.toggle();
        for (kind, line) in [
            (terminal::Kind::Echo, "> WHERE"),
            (terminal::Kind::Note, "STANDING AT 13 73 9"),
            (terminal::Kind::Note, "NEAREST STONEHAVEN - N 11M"),
            (terminal::Kind::Echo, "> FLEET"),
            (terminal::Kind::Note, "DRONE 1         14M  HAULING"),
            (terminal::Kind::Note, "DRONE 2         21M  CUTTING"),
            (terminal::Kind::Note, "FLIER 1         48M  FERRYING"),
            (terminal::Kind::Echo, "> SCOUT SORTIE -40 120"),
            (terminal::Kind::Note, "KESTREL AWAY TO -40 120"),
            (terminal::Kind::Note, "SALVAGED 18 COPPER BAR, 34 STONE"),
            (terminal::Kind::Note, "FLEET DRY - NO FUEL"),
            (terminal::Kind::Echo, "> FROBNICATE"),
            (terminal::Kind::Warn, "NO SUCH COMMAND: FROBNICATE"),
        ] {
            console.say(kind, line);
        }
        // With `--osk` the line is being typed from the pad's own board
        // rather than a keyboard, which is the whole of stage 46b.
        let typed = if options.osk { "found iron reach" } else { "status" };
        for character in typed.chars() {
            console.type_char(character);
        }
        let pixels = terminal::render_terminal(&console, true);
        let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            TERM_SLOT,
            &context.device,
            &context.queue,
            (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );

        if options.osk {
            // The board, under the line it types into, with the cursor over
            // a key — exactly as `refresh_osk` places it in play.
            let mut board = osk::Osk::default();
            board.open();
            board.move_cursor(5, 1);
            let keys = osk::render_osk(&board);
            let board_width = osk::OSK_WIDTH as f32 * osk::OSK_SCALE;
            let board_height = osk::OSK_HEIGHT as f32 * osk::OSK_SCALE;
            renderer.set_overlay(
                OSK_SLOT,
                &context.device,
                &context.queue,
                (osk::OSK_WIDTH, osk::OSK_HEIGHT),
                &keys,
                vx_render::OverlayRect {
                    x: (width as f32 - board_width) / 2.0,
                    y: (height as f32 + panel_height) / 2.0 + 8.0,
                    width: board_width,
                    height: board_height,
                },
            );
        }
    }

    if options.people && !options.close {
        // With `--close` the shot is the square itself; otherwise the panel:
        // the roster and a word with a neighbour, drawn by the real systems:
        // people, schedule, disposition and speech, not typed-in props. A
        // short gift history first, so the tiers on show actually differ.
        let site = vx_world::town::home_site();
        let day = schedule::market_weekday(&site);
        let when = TimeOfDay::new(options.time);
        let folk = people::roster(&site);

        let mut friends = disposition::Disposition::default();
        for (index, gift_days) in [(0usize, 8u32), (1, 2)] {
            let key = (site.centre, index as u8);
            for week in 0..gift_days {
                let gift_day = (week / 2) * 7 + week % 2;
                friends.gift(key, &folk[index], folk[index].loved[0], gift_day);
                friends.talk(key, gift_day);
            }
        }

        let mut console = terminal::Terminal::default();
        console.toggle();
        console.say(terminal::Kind::Echo, "> WHO");
        console.say(
            terminal::Kind::Note,
            format!("THE PEOPLE OF {}{}", site.name.head(), site.name.tail()),
        );
        for (index, person) in folk.iter().enumerate() {
            let key = (site.centre, index as u8);
            let place = schedule::where_is(&site, index, day, when, false);
            console.say(
                terminal::Kind::Note,
                format!(
                    "{:<22} {:<8} {:<12} {}",
                    person.name,
                    person.temperament.archetype.name(),
                    friends.tier(key).name(),
                    place.name()
                ),
            );
        }
        console.say(terminal::Kind::Echo, "> TALK");
        let facts = people::Facts {
            town: format!("{}{}", site.name.head(), site.name.tail()),
            ore_price: 11,
            bounty: 0,
            fleet_dry: false,
            bunker: None,
        };
        let speaker = &folk[0];
        console.say(
            terminal::Kind::Note,
            format!(
                "{}: {}",
                speaker.name,
                people::line_for(speaker, friends.tier((site.centre, 0)), &facts)
            ),
        );
        console.say(terminal::Kind::Echo, "> GIFT COPPER BAR");
        console.say(
            terminal::Kind::Note,
            format!("{}: NOW THAT IS A FINE THING (+62)", speaker.name),
        );

        let pixels = terminal::render_terminal(&console, false);
        let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            TERM_SLOT,
            &context.device,
            &context.queue,
            (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.wear {
        // A crew part-way through its life, drawn from the real ledger:
        // every condition below is what `Wear` says about ticks worked, and
        // the mend spends what `Wear::repair` spends.
        let mut ledger = wear::Wear::default();
        // Three diggers and a flier, worked until the second digger is in
        // trouble. Ticking the real ledger is what makes the row honest.
        while ledger.condition(mining::MachineRef::Digger(0)) != wear::Condition::Failing {
            ledger.tick(3, 1);
        }
        // Then mend one of them, so the roster shows a spread rather than a
        // row of identical numbers.
        let mut pile = vx_agent::Stockpile::new();
        pile.add(wear::SPARE_PART, 8);
        ledger.repair(mining::MachineRef::Digger(2), &mut pile);

        let mut console = terminal::Terminal::default();
        console.toggle();
        console.say(terminal::Kind::Echo, "> FLEET");
        for (name, state, machine) in [
            ("DIGGER 1", "DIGGING", mining::MachineRef::Digger(0)),
            ("DIGGER 2", "HAULING", mining::MachineRef::Digger(1)),
            ("DIGGER 3", "DIGGING", mining::MachineRef::Digger(2)),
            ("FLIER 1", "FERRYING", mining::MachineRef::Flier(0)),
        ] {
            console.say(
                terminal::Kind::Note,
                format!(
                    "{:<12} {:>5}M  {:<9} {}",
                    name,
                    18,
                    state,
                    ledger.condition(machine).name()
                ),
            );
        }
        console.say(terminal::Kind::Note, "A MACHINE IS FAILING - REPAIR IT");
        console.say(terminal::Kind::Echo, "> REPAIR");
        console.say(
            terminal::Kind::Note,
            format!("DIGGER 1 MENDED - {} PARTS SPENT", wear::PARTS_PER_REPAIR),
        );
        console.say(terminal::Kind::Echo, "> PILE");
        console.say(
            terminal::Kind::Note,
            format!("{:<20} {}", "SPARE PART", pile.count(wear::SPARE_PART) - wear::PARTS_PER_REPAIR),
        );

        let pixels = terminal::render_terminal(&console, false);
        let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            TERM_SLOT,
            &context.device,
            &context.queue,
            (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.kit {
        // The kit sheet, drawn by the same code the verb uses: real lines,
        // real costs, real descriptions — a wallet part-way up three of
        // them so the sheet shows a workshop in progress rather than zeros.
        let mut fitted = wallet::Wallet::new();
        fitted.earn(2_400);
        for _ in 0..3 {
            fitted.raise(wallet::DRILL);
        }
        fitted.raise(wallet::CARGO);
        fitted.raise(wallet::CARGO);
        for _ in 0..wallet::MAX_UPGRADE {
            fitted.raise(wallet::PACK);
        }
        fitted.raise(wallet::PRESS);

        let mut console = terminal::Terminal::default();
        console.toggle();
        console.say(terminal::Kind::Echo, "> KIT");
        console.say(
            terminal::Kind::Note,
            format!("CREDITS {}", fitted.credits()),
        );
        for line in wallet::LINES {
            let level = fitted.upgrade(line);
            let next = if level >= wallet::MAX_UPGRADE {
                "FULL".to_string()
            } else {
                format!("{}C", shop::upgrade_cost(level + 1))
            };
            console.say(
                terminal::Kind::Note,
                format!(
                    "{:<7} {}/{}  {:<6} {}",
                    line.to_uppercase(),
                    level,
                    wallet::MAX_UPGRADE,
                    next,
                    wallet::describes(line)
                ),
            );
        }
        console.say(terminal::Kind::Echo, "> PRINT PRESS ROLLERS");
        console.say(terminal::Kind::Note, "FITTED - PRESS NOW 2 OF 5");

        let pixels = terminal::render_terminal(&console, false);
        let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            TERM_SLOT,
            &context.device,
            &context.queue,
            (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.pad {
        // The scheme panel, exactly as SELECT raises it in play — same
        // renderer, same table the tests hold drawable.
        let pixels = gamepad::render_pad_help();
        let panel_width = gamepad::PAD_WIDTH as f32 * gamepad::PAD_SCALE;
        let panel_height = gamepad::PAD_HEIGHT as f32 * gamepad::PAD_SCALE;
        renderer.set_overlay(
            PAD_SLOT,
            &context.device,
            &context.queue,
            (gamepad::PAD_WIDTH, gamepad::PAD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.vault {
        let mut pile = vx_agent::Stockpile::new();
        pile.add("engine:copper_ore", 143);
        pile.add("engine:hho_cell", 9);
        pile.add("engine:copper_bar", 4);
        let mut vaults = bank::Bank::default();
        let mut deposited = vx_agent::Stockpile::new();
        deposited.add("engine:copper_bar", 260);
        deposited.add("engine:stone", 1_400);
        vaults.deposit((0, 0), "engine:copper_bar", 260, &mut deposited);
        vaults.deposit((0, 0), "engine:stone", 1_400, &mut deposited);
        vaults.open_at(vx_core::BlockPos::new(0, 0, 0), (0, 0));
        vaults.cursor = 1;
        vaults.feedback = Some("BANKED 260 COPPER BAR".into());
        let pixels = bank::render_vault(&vaults, (0, 0), "STONEHAVEN VAULT", Some(&pile));
        let panel_width = bank::VAULT_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = bank::VAULT_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            VAULT_SLOT,
            &context.device,
            &context.queue,
            (bank::VAULT_WIDTH, bank::VAULT_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.hho && !options.close {
        let mut pile = vx_agent::Stockpile::new();
        pile.add("engine:copper_bar", 7);
        pile.add(fuel::CELL, 12);
        let machine = electrolysis::Electrolyser {
            open: true,
            cursor: 1,
            job: Some(electrolysis::Job {
                run: 1,
                done: 21.0,
                total: electrolysis::duration(&electrolysis::RUNS[1], 6),
            }),
            ..electrolysis::Electrolyser::default()
        };
        let pixels = electrolysis::render_electrolyser(&machine, Some(&pile), 6, false);
        let panel_width = electrolysis::FUEL_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = electrolysis::FUEL_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            FUEL_SLOT,
            &context.device,
            &context.queue,
            (electrolysis::FUEL_WIDTH, electrolysis::FUEL_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.fab && !options.close {
        // The fabricator's screen over the frame, stocked mid-print: a slug
        // batch running, the cursor parked on a row the pile cannot cover so
        // the SHORT line shows too.
        let mut pile = vx_agent::Stockpile::new();
        pile.add("engine:copper_bar", 5);
        pile.add("engine:stone", 64);
        pile.add("engine:copper_ore", 22);
        pile.add("engine:log", 9);
        let mut fab_skills = Skills::new();
        while fab_skills.level(skills::FABRICATION) < 8 {
            fab_skills.add_xp(skills::FABRICATION, 500);
        }
        let fabrication = fab_skills.level(skills::FABRICATION);
        // A workshop with some marks already fitted: the panel's mark
        // column is half the point of the shot.
        let mut fitted = wallet::Wallet::new();
        for _ in 0..3 {
            fitted.raise(wallet::DRILL);
        }
        fitted.raise(wallet::CARGO);
        fitted.raise(wallet::PACK);
        fitted.raise(wallet::PACK);
        let panel = printer::Printer {
            at: Some(vx_core::BlockPos::new(options.at.0 - 5, 0, options.at.1 + 10)),
            open: true,
            cursor: 4,
            job: Some(printer::Job {
                recipe: 0,
                done: 9.1,
                total: printer::duration(
                    &printer::CATALOGUE[0],
                    fabrication,
                    fitted.upgrade(wallet::PRESS),
                ),
            }),
            feedback: None,
        };
        let pixels = printer::render_printer(&panel, Some(&pile), fabrication, &fitted);
        let panel_width = printer::PRINT_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = printer::PRINT_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            PRINT_SLOT,
            &context.device,
            &context.queue,
            (printer::PRINT_WIDTH, printer::PRINT_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.shop {
        // The shop panel over the frame, stocked the way a good session
        // leaves it — this is the capture that eyeballs the trade UI.
        let mut pile = vx_agent::Stockpile::new();
        pile.add("engine:copper_ore", 240);
        pile.add("engine:log", 17);
        let mut walletbook = wallet::Wallet::new();
        walletbook.earn(350);
        let mut panel = shop::Shop::new();
        panel.open_at_counter();
        panel.feedback = Some("SOLD 60 COPPER ORE FOR 480 CR".into());
        let here = vx_world::town::home_site();
        let market = economy::Economy::new()
            .market(&here, 0)
            .clone();
        let shed = garage::Garage::new();
        let pixels = shop::render_shop(
            &panel,
            Some(&pile),
            &walletbook,
            &here,
            &market,
            &shed,
            &arsenal::Arsenal::default(),
            &intrusion::Intrusions::default(),
            1,
            &[],
            true,
        );
        let panel_width = shop::SHOP_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = shop::SHOP_HEIGHT as f32 * shop::SHOP_SCALE;
        renderer.set_overlay(
            SHOP_SLOT,
            &context.device,
            &context.queue,
            (shop::SHOP_WIDTH, shop::SHOP_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    if options.board {
        // A console at the hometown mast, one contract already taken so both
        // states show: work on offer, and work in hand pointing somewhere the
        // player has never been.
        let here = vx_world::town::home_site();
        let neighbours = world.towns_near(here.centre, RADIO_RANGE);
        let postings = beacon::postings_for(&here, &neighbours);
        let mut ledger = beacon::Ledger::new();
        ledger.visit(here.centre);
        if let Some(first) = postings.first() {
            ledger.accept(first);
        }
        let mut walletbook = Wallet::new();
        walletbook.earn(1_240);
        let mut panel = board::Board::new();
        panel.open_at_beacon();
        let market = economy::Economy::new().market(&here, 0).clone();
        let pixels =
            {
                // A run to somewhere the player has never walked, so the
                // capture shows the thing worth showing: a pin sitting in the
                // dark with a bearing under it.
                let runs: Vec<board::Run> = neighbours
                    .iter()
                    .find(|other| other.centre != here.centre)
                    .map(|target| {
                        vec![board::Run {
                            good: economy::ORE,
                            to: target.centre,
                            name: target.name.to_string(),
                        }]
                    })
                    .unwrap_or_default();
                let explored = map::MapState::new();
                let view = board::TradeView {
                    world: &world,
                    explored: &explored,
                    traffic: &[],
                };
                // Cursor onto the run, which is the last row.
                let rows = board::Board::rows_with_runs(here.centre, &postings, &ledger, &runs);
                panel.move_cursor(rows.len() as i32, rows.len());

                // Real paperwork rather than a drawn-on line: the sheriff
                // files against the real threshold and the mayor of the
                // hometown — authored `Proud` — makes the real decision.
                let mut docket = warrant::Docket::default();
                if options.warrant {
                    let mayor = office::seat(&here, permits::Office::Mayor);
                    let bounty = permits::WARRANT_THRESHOLD * 2;
                    let filed = docket.file(
                        &here,
                        &mayor,
                        disposition::Tier::Stranger,
                        0,
                        bounty,
                        0,
                    );
                    println!(
                        "warrant capture: {} on {} CR — {:?}, fine {} CR",
                        mayor.name,
                        bounty,
                        docket.get(here.centre).map(|paper| paper.stage.name()),
                        filed.map_or(0, |filed| filed.fine)
                    );
                }
                // The fixture's own register, so `--ballot` can photograph a
                // seat held without a session behind it — and `--taken` a
                // pair of chairs seized over a broken posse.
                let mut register = ballot::Register::default();
                let charters = charter::Charters::default();
                if options.taken {
                    register.seize(here.centre, 3);
                    println!("taken capture: both chairs of {} seized on day 4", here.name);
                }
                if options.elected {
                    register.stand(here.centre, permits::Office::Sheriff, true);
                    let mut friends = disposition::Disposition::default();
                    for voter in 0..people::PEOPLE {
                        for day in 0..60 {
                            friends.trade((here.centre, voter as u8), 1_000, day);
                        }
                    }
                    let field = ballot::Field {
                        site: &here,
                        friends: &friends,
                        bounty: 0,
                        incumbent_troubled: false,
                        standing: true,
                        seat: permits::Office::Sheriff,
                        founder: false,
                    };
                    let day = ballot::next_poll(&here, 0);
                    register.hold(&field, permits::Office::Sheriff, day);
                }
                let civic = civic_snapshot(&here, &docket, &charters, &register);
                let mut voters = disposition::Disposition::default();
                for voter in 0..people::PEOPLE {
                    for day in 0..(6 + voter * 14) {
                        voters.trade((here.centre, voter as u8), 900, day as u32);
                    }
                }
                let ballot = ballot_snapshot(&here, &register, &voters, 0, 2);
                if options.ballot || options.elected {
                    panel.turn_page();
                    panel.move_cursor(0, board::ballot_rows(&ballot).len());
                    println!(
                        "ballot capture: {} — polls in {} days, {} seats, {} held by you",
                        here.name,
                        ballot.days_to_poll,
                        ballot.seats.len(),
                        ballot.seats.iter().filter(|seat| seat.yours).count()
                    );
                }
                board::render_board(
                    &panel,
                    &board::Counter {
                        here: &here,
                        postings: &postings,
                        runs: &runs,
                        market: &market,
                        civic: &civic,
                        ballot: &ballot,
                    },
                    &ledger,
                    &walletbook,
                    Some(&view),
                )
            };
        let panel_width = board::BOARD_WIDTH as f32 * board::BOARD_SCALE;
        let panel_height = board::BOARD_HEIGHT as f32 * board::BOARD_SCALE;
        renderer.set_overlay(
            BOARD_SLOT,
            &context.device,
            &context.queue,
            (board::BOARD_WIDTH, board::BOARD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    // The capture is lit by an explicit hour, never by a wall clock — which
    // is what keeps two runs of the same command byte-identical.
    let mut sun = clock::sun_uniform(clock::sky_at(TimeOfDay::new(options.time)));
    // `--optic` sees the capture through the kit: the lamp shines from the
    // capture camera, the visors change how every fragment reads.
    match options.optic.as_deref() {
        Some("lamp") => {
            let aim = camera.forward();
            let eye = renderer.relative(camera.position);
            sun.lamp_position = [eye.x, eye.y, eye.z, 1.35];
            sun.lamp_direction = [aim.x, aim.y, aim.z, 30.0];
        }
        Some("nvg") => {
            sun.light[2] = 1.0;
            sun.sky = [0.01, 0.09, 0.02, 1.0];
        }
        Some("thermal") => {
            sun.light[2] = 2.0;
            sun.sky = [0.01, 0.01, 0.04, 1.0];
        }
        Some(other) => eprintln!("--optic {other} is not lamp, nvg or thermal"),
        None => {}
    }
    // The season fixtures paint the atlas and tint the sky the same way
    // `Game::frame` does, in the same order, so a captured autumn is the
    // autumn a player would walk into.
    if let Some(tick) = season_at {
        clock::tint_for_season(&mut sun, tick);
        renderer.repaint_foliage(&context.queue, vx_world::season::leaf_turn(tick));
    }
    // The weather fixtures tint the sky the same way `Game::frame` does, so
    // an overcast capture reads as overcast rather than as a strange dusk.
    if let Some(sky) = weather_over {
        let overcast = match sky.state {
            vx_world::weather::State::Clear => 0.0,
            vx_world::weather::State::Cloud => 0.45,
            vx_world::weather::State::Rain => 0.75,
            vx_world::weather::State::Storm => 1.0,
        };
        if overcast > 0.0 {
            let grey = [0.36, 0.38, 0.41];
            for (channel, tone) in grey.iter().enumerate() {
                sun.sky[channel] = sun.sky[channel] * (1.0 - overcast) + tone * overcast;
            }
            sun.light[0] *= 1.0 - 0.55 * overcast;
            sun.light[1] = (sun.light[1] + 0.10 * overcast).min(0.6);
        }
    }
    renderer.set_sun(&context.queue, sun);

    let capture = capture_frame(&context, &renderer, width, height);
    capture
        .write_ppm(path)
        .map_err(|error| format!("could not write {path}: {error}"))?;

    println!(
        "wrote {path} ({width}x{height}, {} of {} chunks drawn, {} triangles)",
        renderer.visible_chunk_count(),
        renderer.loaded_chunk_count(),
        renderer.triangle_count()
    );
    Ok(())
}

/// Point `camera` at `target` from where it stands.
///
/// Derived from the same yaw/pitch convention `Camera::forward` uses, so a
/// framing that looks right here looks the same in the window.
/// The rig for a stem on its way down, in its own species' wood.
/// What the ballot page should print for this town.
///
/// A snapshot, like the civic block: names, days and wordings, so the panel
/// stays a pure function of what it is handed.
fn ballot_snapshot(
    site: &vx_world::town::TownSite,
    register: &ballot::Register,
    friends: &disposition::Disposition,
    bounty: u64,
    day: u32,
) -> board::Ballot {
    let seats = office::OFFICES
        .into_iter()
        .map(|office| {
            let holder = match register.seated(site, office) {
                ballot::Candidate::Player => "YOU".to_string(),
                ballot::Candidate::Resident(index) => people::person(site, index).name,
            };
            board::BallotSeat {
                title: office::title(office).to_string(),
                holder,
                yours: register.player_holds(site.centre, office),
                standing: register.is_standing(site.centre, office),
            }
        })
        .collect();

    // How each resident is leaning, said in words rather than in points: the
    // panel's job is to make the arithmetic legible, not to print it.
    let leanings = (0..people::PEOPLE)
        .map(|index| {
            let person = people::person(site, index);
            let worth = ballot::standing_with(friends, site.centre, index, bounty);
            let mood = if worth >= 50 {
                "WOULD VOTE FOR YOU"
            } else if worth >= 20 {
                "IS WARMING TO YOU"
            } else if worth >= 0 {
                "IS NOT CONVINCED"
            } else {
                "THINKS YOU ARE TROUBLE"
            };
            format!("{:<22} {mood}", person.name)
        })
        .collect();

    board::Ballot {
        seats,
        days_to_poll: ballot::next_poll(site, day).saturating_sub(day),
        leanings,
    }
}

/// What the beacon panel should print about who runs this place.
///
/// A snapshot, not a borrow of the ledgers: the panel is a pure function over
/// what it is given, which is why it can be tested without a town.
fn civic_snapshot(
    site: &vx_world::town::TownSite,
    docket: &warrant::Docket,
    charters: &charter::Charters,
    elections: &ballot::Register,
) -> board::Civic {
    let charter = charter_line(site, charters, elections);
    let seats = office::OFFICES
        .into_iter()
        .map(|office| {
            (
                office::title(office).to_string(),
                office::seat(site, office).name,
            )
        })
        .collect();
    let warrant = docket.get(site.centre).map(|paper| {
        format!(
            "WARRANT {} - FILED ON {} CR",
            paper.stage.name(),
            paper.at_bounty
        )
    });
    board::Civic {
        seats,
        charter,
        warrant,
        closed: docket.pending_in(site.centre),
    }
}

/// How the player came to hold a town, when it was not the ballot box.
fn charter_line(
    site: &vx_world::town::TownSite,
    charters: &charter::Charters,
    elections: &ballot::Register,
) -> Option<String> {
    if let Some(paper) = charters.get(site.centre) {
        let day = paper.founded / schedule::TICKS_PER_DAY;
        return Some(format!("FOUNDED BY YOU - DAY {}", day + 1));
    }
    elections
        .taken(site.centre)
        .map(|day| format!("TAKEN - DAY {}", day + 1))
}

fn trunk_rig(fall: &felling::Falling) -> rig::Rig {
    use vx_render::tiles::slot;
    let (bark, crown) = match fall.species {
        vx_world::flora::Species::Ancient => (slot::ANCIENT_BARK, slot::LEAVES),
        vx_world::flora::Species::BogSpruce => (slot::BOG_BARK, slot::BOG_NEEDLES),
        species if species.conifer() => (slot::SPRUCE_SIDE, slot::NEEDLES),
        _ => (slot::LOG_SIDE, slot::LEAVES),
    };
    // A standing trunk is a whole block wide however slender the timber
    // is, so the falling one has to be too, or the tree visibly thins the
    // moment it lets go. The physical radius still does the arithmetic.
    let thickness = (felling::radius(fall.species) * 2.0).max(0.9);
    rig::Rig::trunk(fall.height, thickness, bark, crown)
}

fn look_at(camera: &mut Camera, target: impl Into<glam::DVec3>) {
    // The difference in `f64`, then narrowed: a fixture three thousand
    // kilometres out aims as precisely as one at spawn.
    let to = (target.into() - camera.position).as_vec3();
    camera.yaw = to.x.atan2(-to.z);
    camera.pitch = to.y.atan2((to.x * to.x + to.z * to.z).sqrt());
    camera.clamp_pitch();
}

/// Play a whole loop and photograph it: leave, collect, trade.
///
/// The only fixture that is a *playthrough* rather than a tableau. Everything
/// here goes through [`session::Session`], which drives the game's own
/// movement, drill and counter — so if this run stops working, the game has
/// stopped working, not the picture.
///
/// Writes one image per beat beside `path`: a capture asked for as `play.ppm`
/// lands as `play-01-door.ppm` and so on, because four moments are what a loop
/// looks like and one is what a screenshot looks like.
fn play_the_loop(
    context: &GpuContext,
    renderer: &mut Renderer,
    camera: &mut Camera,
    options: &Options,
    path: &str,
) -> Result<(), String> {
    let stem = path.strip_suffix(".ppm").unwrap_or(path);
    let mut shot = 0;

    // Frame the player from behind and a little above — near enough that the
    // house, the seam of ore or the counter fills the frame with them.
    let mut photograph = |context: &GpuContext,
                          renderer: &mut Renderer,
                          camera: &mut Camera,
                          session: &mut session::Session,
                          beat: &str,
                          subject: glam::DVec3|
     -> Result<(), String> {
        shot += 1;
        let here = vx_core::BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32,
            session.player.position.z.floor() as i32,
        )
        .chunk();
        // Only what is in shot: after a hundred and seventy blocks of walking
        // the session is holding far more world than a frame needs, and
        // meshing all of it for every beat is minutes of nothing.
        session.world.load_around(here, 4);
        let dropped: Vec<vx_core::ChunkPos> = session
            .world
            .loaded_chunks()
            .filter(|pos| (pos.x - here.x).abs() > 5 || (pos.z - here.z).abs() > 5)
            .collect();
        for pos in dropped {
            renderer.remove_chunk(pos);
        }
        session.world.unload_beyond(here, 5);
        remesh_all(context, renderer, &mut session.world);

        // Over the player's shoulder and well back, aimed at the middle of
        // the two — so the frame holds the player *and* what they came for
        // rather than a close-up of the paving.
        let eye = session.player.eye_position();
        let away = {
            let to = subject - eye;
            let level = glam::DVec3::new(to.x, 0.0, to.z);
            if level.length() > 0.5 {
                -level.normalize()
            } else {
                glam::DVec3::new(-1.0, 0.0, -1.0).normalize()
            }
        };
        // Framed by the game's own follow camera rather than by hand: aim it
        // where the player is looking, then let `view::camera_placement` stand
        // it off. That is the camera stage 47 taught to refuse a wall instead
        // of clamping through one, which is exactly what a shot taken inside
        // the shop needs — the first framing of this beat put the lens six
        // blocks behind the player and therefore outside the building,
        // photographing the wall they were stood behind.
        let _ = away;
        camera.position = eye;
        look_at(camera, subject);
        camera.position = view::camera_placement(
            &session.world,
            camera,
            eye,
            view::ViewMode::ThirdPerson,
        );
        look_at(camera, subject);
        renderer.update_camera(&context.queue, camera);

        // Draw the player, so the pictures are of somebody playing rather
        // than of scenery they happen to be stood in. Built in the camera's
        // own frame, like every other body since stage 45.
        //
        // Except when the follow camera refused to stand off — inside the
        // shop it collapses to the eye, and `ViewMode::draws_body` says a
        // first-person frame draws no body, for the good reason that all it
        // would show is the inside of your own head.
        let third_person = camera.position != eye;
        let drawn: Vec<vx_render::Object> = if third_person {
            let origin = renderer.render_origin().as_dvec3();
            let feet = (session.player.position - origin).as_vec3();
            rig::Rig::player()
                .objects(feet, session.yaw, 0.0)
                .into_iter()
                .map(|object| object.already_relative())
                .collect()
        } else {
            Vec::new()
        };
        renderer.set_objects(&context.device, &context.queue, &drawn);

        let out = format!("{stem}-{shot:02}-{beat}.ppm");
        let capture = capture_frame(context, renderer, options.width, options.height);
        capture
            .write_ppm(&out)
            .map_err(|error| format!("could not write {out}: {error}"))?;
        println!("  beat {shot}: {beat} -> {out}");
        Ok(())
    };

    let mut session = session::Session::open(options.seed);
    let home = session.home();
    let spawn = vx_world::town::spawn_position(&home);
    let counter = vx_world::town::counter_position(&home);
    println!("playing a loop from {spawn:?} on seed {}", options.seed);

    // The gesture the game never told anyone to make: without a container
    // there is no pile, and everything cut by hand is thrown away.
    let chest = vx_world::town::chest_position(&home);
    session.place_base(vx_core::BlockPos::new(chest.x, chest.y, chest.z));

    // --- 1. Out of the door -------------------------------------------
    let doorstep = session.doorstep();
    let went = session.walk_to(doorstep, 8 * 30);
    println!("  out of the house: {went:?}");
    photograph(context, renderer, camera, &mut session, "door", doorstep)?;

    // --- 2. Clear of the town, on the way out --------------------------
    let outcrop = (options.at.0.max(120), options.at.1.max(30));
    session
        .world
        .load_around(vx_core::BlockPos::new(outcrop.0, 0, outcrop.1).chunk(), 2);
    let body = vx_agent::find_body(&session.world, outcrop, 48)
        .ok_or_else(|| format!("no ore body near {outcrop:?} to walk to"))?;
    let seam = body
        .blocks()
        .filter(|pos| vx_agent::is_ore(&session.world, *pos))
        .filter(|pos| {
            session
                .world
                .block(vx_core::BlockPos::new(pos.x, pos.y + 1, pos.z))
                .is_air()
        })
        .max_by_key(|pos| pos.y)
        .ok_or("the body never reaches daylight")?;

    let halfway = glam::DVec3::new(
        (f64::from(spawn.x) + f64::from(seam.x)) * 0.5,
        f64::from(spawn.y),
        (f64::from(spawn.z) + f64::from(seam.z)) * 0.5,
    );
    let leg = session.walk_to(halfway, 8 * 240);
    println!("  clear of the town: {leg:?}");
    let ahead = session.player.position + (session.forward() * 8.0).as_dvec3();
    photograph(context, renderer, camera, &mut session, "out", ahead)?;

    // --- 3. At the seam, mid-bite --------------------------------------
    let stand = glam::DVec3::new(
        f64::from(seam.x) + 0.5,
        f64::from(seam.y) + 1.0,
        f64::from(seam.z) + 0.5,
    );
    let there = session.walk_to(stand, 8 * 300);
    println!("  at the ore: {there:?}");
    // What the bit is about to cost, from the drill's own law.
    let power = drill::power_of(1, 0);
    println!(
        "  copper is {:.1}s a block on a drill nobody has spent anything on",
        drill::seconds_for(2.5, power)
    );
    let mut cut = 0;
    for _ in 0..16 {
        let Some(next) = body
            .blocks()
            .filter(|pos| vx_agent::is_ore(&session.world, *pos))
            .filter(|pos| {
                (glam::DVec3::new(
                    f64::from(pos.x) + 0.5,
                    f64::from(pos.y) + 0.5,
                    f64::from(pos.z) + 0.5,
                ) - session.eye())
                .length()
                    < f64::from(session::REACH) - 0.5
            })
            .min_by_key(|pos| -pos.y)
        else {
            break;
        };
        session.look_at(next);
        if matches!(session.drill_through(600), Some(drill::Deposited::Piled(_))) {
            cut += 1;
        } else {
            break;
        }
    }
    let pack = session.pile().map_or(0, |pile| pile.total());
    println!("  cut {cut} blocks; the pack holds {pack}");
    // Leave the last one half-drilled so the face in shot is a worked face.
    if let Some(next) = body
        .blocks()
        .filter(|pos| vx_agent::is_ore(&session.world, *pos))
        .min_by_key(|pos| -pos.y)
    {
        session.look_at(next);
        for _ in 0..40 {
            if session.drill_through(1).is_some() {
                break;
            }
        }
    }
    let face = session.player.position + (session.forward() * 3.0).as_dvec3();
    photograph(context, renderer, camera, &mut session, "ore", face)?;

    // --- 4. Home, and paid ---------------------------------------------
    let back = session.walk_route(&session.counter_route(&home), 8 * 400);
    println!(
        "  home again: {back:?} ({} ticks laden, {} out)",
        back.ticks(),
        went.ticks() + leg.ticks() + there.ticks()
    );
    if !back.reached() || !session.at_the_counter(&home) {
        return Err(format!(
            "could not get back to the counter to trade: {back:?}, standing at {:?}",
            session.player.position
        ));
    }
    let earned = session.sell_everything();
    println!(
        "  sold the haul for {earned} CR; the wallet holds {}",
        session.wallet.credits()
    );

    let till = glam::DVec3::new(
        f64::from(counter.x) + 0.5,
        f64::from(counter.y) + 0.5,
        f64::from(counter.z) + 0.5,
    );
    photograph(context, renderer, camera, &mut session, "counter", till)?;

    println!(
        "played {} ticks, {} journal entries, {} blocks lost to no pile",
        session.tick,
        session.journal.entries().len(),
        session.lost
    );
    Ok(())
}

/// Scout, collect, save, load, haul, sell — the long loop, photographed.
///
/// The round's reason for existing is the save in the middle: the fleet's base
/// pile used to be the one thing a player owned that a save forgot, so the
/// beats either side of the reload are the picture worth having.
fn haul_it_somewhere(
    context: &GpuContext,
    renderer: &mut Renderer,
    camera: &mut Camera,
    options: &Options,
    path: &str,
) -> Result<(), String> {
    let stem = path.strip_suffix(".ppm").unwrap_or(path);
    let mut shot = 0;

    let mut photograph = |context: &GpuContext,
                          renderer: &mut Renderer,
                          camera: &mut Camera,
                          session: &mut session::Session,
                          beat: &str,
                          subject: glam::DVec3|
     -> Result<(), String> {
        shot += 1;
        let here = vx_core::BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32,
            session.player.position.z.floor() as i32,
        )
        .chunk();
        session.world.load_around(here, 4);
        let dropped: Vec<vx_core::ChunkPos> = session
            .world
            .loaded_chunks()
            .filter(|pos| (pos.x - here.x).abs() > 5 || (pos.z - here.z).abs() > 5)
            .collect();
        for pos in dropped {
            renderer.remove_chunk(pos);
        }
        session.world.unload_beyond(here, 5);
        remesh_all(context, renderer, &mut session.world);

        // Frame it the way a person stood there would: level with the eye and
        // far enough off to see something. A subject at your own feet — the
        // seam you walked to, the till you are leaning on — pitches the
        // follow camera straight down and fills the picture with the back of
        // your own head, which is what the first four beats came out as.
        let eye = session.player.eye_position();
        let mut bearing = subject - eye;
        bearing.y = 0.0;
        let subject = if bearing.length() < 0.5 {
            eye + session.forward().as_dvec3() * 8.0
        } else {
            eye + bearing.normalize() * bearing.length().max(8.0)
        };
        camera.position = eye;
        look_at(camera, subject);
        camera.position = view::camera_placement(
            &session.world,
            camera,
            eye,
            view::ViewMode::ThirdPerson,
        );
        look_at(camera, subject);
        renderer.update_camera(&context.queue, camera);

        let third_person = camera.position != eye;
        let drawn: Vec<vx_render::Object> = if third_person {
            let origin = renderer.render_origin().as_dvec3();
            let feet = (session.player.position - origin).as_vec3();
            rig::Rig::player()
                .objects(feet, session.yaw, 0.0)
                .into_iter()
                .map(|object| object.already_relative())
                .collect()
        } else {
            Vec::new()
        };
        renderer.set_objects(&context.device, &context.queue, &drawn);

        let out = format!("{stem}-{shot:02}-{beat}.ppm");
        capture_frame(context, renderer, options.width, options.height)
            .write_ppm(&out)
            .map_err(|error| format!("could not write {out}: {error}"))?;
        println!("  beat {shot}: {beat} -> {out}");
        Ok(())
    };

    let root = std::env::temp_dir().join(format!("vx-haul-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();

    let mut session = session::Session::open(options.seed);
    let home = session.home();
    let chest = vx_world::town::chest_position(&home);
    session.place_base(vx_core::BlockPos::new(chest.x, chest.y, chest.z));
    // Fuel first, or declaring that base is what grounds the flier.
    session.fuel_the_fleet(6);

    // --- Scouting ------------------------------------------------------
    let target = (options.at.0.max(120), options.at.1.max(30));
    let ticks = session
        .scan_sector(target, 8 * 900)
        .ok_or("the flier never finished its sweep")?;
    let pings = session.pings();
    println!(
        "swept the sector around {target:?} in {ticks} ticks: {} pings",
        pings.len()
    );
    let ping = pings
        .iter()
        .max_by_key(|ping| ping.ore_columns)
        .ok_or("the sweep found nothing to dig")?;
    println!(
        "  richest: {:?}, {} columns, {} blocks of overburden",
        ping.position, ping.ore_columns, ping.depth
    );

    let seam = glam::DVec3::new(
        f64::from(ping.position.x) + 0.5,
        f64::from(ping.position.y),
        f64::from(ping.position.z) + 0.5,
    );
    let went = session.walk_route(&[session.doorstep(), seam], 8 * 400);
    println!("  walked to the ping: {went:?}");
    photograph(context, renderer, camera, &mut session, "ping", seam)?;

    // --- Collecting ----------------------------------------------------
    let body = vx_agent::find_body(&session.world, (ping.position.x, ping.position.z), 48);
    let mut cut = 0;
    if let Some(body) = body {
        for _ in 0..20 {
            let Some(next) = body
                .blocks()
                .filter(|pos| vx_agent::is_ore(&session.world, *pos))
                .filter(|pos| {
                    (glam::DVec3::new(
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ) - session.eye())
                    .length()
                        < f64::from(session::REACH) - 0.5
                })
                .min_by_key(|pos| -pos.y)
            else {
                break;
            };
            session.look_at(next);
            if matches!(session.drill_through(600), Some(drill::Deposited::Piled(_))) {
                cut += 1;
            } else {
                break;
            }
        }
    }
    let before = session.pile().map_or(0, |pile| pile.total());
    println!("  cut {cut} blocks; the pile holds {before}");
    let ahead = session.player.position + (session.forward() * 4.0).as_dvec3();
    photograph(context, renderer, camera, &mut session, "cut", ahead)?;

    // --- Save, and load it straight back -------------------------------
    session
        .save_to(&root)
        .map_err(|error| format!("could not save: {error}"))?;
    let credits_before = session.wallet.credits();
    drop(session);

    let mut session = session::Session::load_from(&root)
        .map_err(|error| format!("could not load the save back: {error}"))?;
    std::fs::remove_dir_all(&root).ok();
    let after = session.pile().map_or(0, |pile| pile.total());
    println!("  saved and reloaded: the pile holds {after} (was {before})");
    if after != before {
        return Err(format!("the pile lost {} goods across a save", before - after));
    }
    let doorstep = session.doorstep();
    photograph(context, renderer, camera, &mut session, "reloaded", doorstep)?;

    // --- Somewhere that pays -------------------------------------------
    let towns = session.world.towns_near((0, 0), 4_000);
    let elsewhere = towns
        .iter()
        .find(|site| !site.is_home())
        .ok_or("no other town on this frontier")
        .copied()?;
    let away = ((elsewhere.centre.0 as f64).powi(2) + (elsewhere.centre.1 as f64).powi(2)).sqrt();
    println!(
        "  nearest other town: {:?} {:?}, {away:.0} blocks off",
        elsewhere.speciality, elsewhere.centre
    );

    // Walk it in legs so each gets its own detour allowance.
    let counter = vx_world::town::counter_position(&elsewhere);
    // Out of the door first, and then *clear of it*. Reloading a save stands
    // you back in your own house — the live game does the same — so a route
    // that sets off on the bearing of a town two hundred blocks away walks
    // into the kitchen wall. The doorstep alone is not enough: the town the
    // goods are going to is west of here and the door faces east, so a walker
    // that turns the moment it is over the threshold turns straight back
    // through it. The lane outside is the first place a bearing is safe.
    let doorstep = session.doorstep();
    let clear = doorstep + glam::DVec3::new(6.0, 0.0, 0.0);
    let to = glam::DVec3::new(
        f64::from(counter.x) + 0.5,
        doorstep.y,
        f64::from(counter.z) + 0.5,
    );
    // Out of the house, out of the *town*, across, in through the far gate,
    // and only then at the counter. Both walls are the reason for this shape:
    // a bearing taken on the plaza points at the inside of your own rampart.
    let home_gate = session.gateway_toward(&home, to);
    let far_gate = session.gateway_toward(&elsewhere, clear);
    let mut legs: Vec<glam::DVec3> = vec![doorstep, clear, home_gate];
    let from = home_gate;
    let steps = ((far_gate - from).length() / 90.0).ceil().max(1.0) as i32;
    for step in 1..=steps {
        let along = f64::from(step) / f64::from(steps);
        legs.push(from + (far_gate - from) * along);
    }
    legs.extend_from_slice(&session.counter_route(&elsewhere));
    let hauled = session.walk_route(&legs, 8 * 6_000);
    println!("  hauled it there: {hauled:?}");
    let earned = if hauled.reached() && session.at_the_counter(&elsewhere) {
        session.sell_everything_at(&elsewhere)
    } else {
        println!(
            "  stopped {:.0} blocks short at {:?}",
            (to - session.player.position).length(),
            session.player.position.round()
        );
        0
    };
    println!(
        "  sold for {earned} CR; the wallet holds {} (was {credits_before})",
        session.wallet.credits()
    );
    let till = glam::DVec3::new(
        f64::from(counter.x) + 0.5,
        f64::from(counter.y) + 0.5,
        f64::from(counter.z) + 0.5,
    );
    photograph(context, renderer, camera, &mut session, "sold", till)?;

    println!("played {} ticks over the whole loop", session.tick);
    Ok(())
}

/// Photograph a machine's gimbal: what it sees, and where it is bolted.
///
/// The first beat is the feed itself — the frame a pilot actually flies on,
/// which before this round was taken from inside the machine's own hull. The
/// second is the same instant from outside, so the mount is visible under the
/// nose rather than merely asserted in a test.
fn photograph_the_gimbal(
    context: &GpuContext,
    renderer: &mut Renderer,
    camera: &mut Camera,
    options: &Options,
    path: &str,
) -> Result<(), String> {
    let stem = path.strip_suffix(".ppm").unwrap_or(path);

    let mut world = World::new(options.seed);
    let centre = vx_core::BlockPos::new(options.at.0, 0, options.at.1).chunk();
    world.load_around(centre, 4);

    // A flier over the ground the screenshots use, holding station.
    let ground = world.surface_y(options.at.0, options.at.1).unwrap_or(80);
    let mut mining = mining::Mining::default();
    mining.ensure_flier(glam::DVec3::new(
        f64::from(options.at.0),
        f64::from(ground),
        f64::from(options.at.1),
    ));
    let machine = mining::MachineRef::Flier(0);
    if !mining.take_control(machine) {
        return Err("the flier would not take the wheel".into());
    }
    // Fly it a few ticks so it has a heading to glide along rather than a
    // cold zero, and so the camera is riding a real interpolation.
    let events = EventBus::new();
    mining.set_pilot_look(std::f32::consts::FRAC_PI_2);
    mining.set_pilot_command(vx_agent::PilotCommand {
        heading: Some(vx_agent::Heading::PosX),
        cut: false,
        climb: 0,
    });
    mining.advance(&mut world, &events, 6);

    remesh_all(context, renderer, &mut world);
    let eye = mining
        .machine_eye(machine)
        .ok_or("the flier has no camera to look through")?;

    // Beat one: down the gimbal, with the machine's own rig culled exactly as
    // the live feed culls it.
    camera.position = eye;
    camera.yaw = std::f32::consts::FRAC_PI_2;
    camera.pitch = -0.45;
    renderer.update_camera(&context.queue, camera);
    let through = mining.objects(|at| renderer.relative(at), Some(machine));
    renderer.set_objects(&context.device, &context.queue, &through);
    let out = format!("{stem}-01-through.ppm");
    capture_frame(context, renderer, options.width, options.height)
        .write_ppm(&out)
        .map_err(|error| format!("could not write {out}: {error}"))?;
    println!("  beat 1: down the gimbal -> {out}");

    // Beat two: the same instant from outside, machine drawn, so the mount
    // under the nose is a thing you can see.
    let mount = mining
        .machine_position(machine)
        .ok_or("the flier is nowhere")?;
    let hull = glam::DVec3::new(
        f64::from(mount.x) + 0.5,
        f64::from(mount.y),
        f64::from(mount.z) + 0.5,
    );
    camera.position = hull + glam::DVec3::new(-2.6, 1.1, 2.6);
    look_at(camera, eye);
    renderer.update_camera(&context.queue, camera);
    let watched = mining.objects(|at| renderer.relative(at), None);
    renderer.set_objects(&context.device, &context.queue, &watched);
    let out = format!("{stem}-02-mount.ppm");
    capture_frame(context, renderer, options.width, options.height)
        .write_ppm(&out)
        .map_err(|error| format!("could not write {out}: {error}"))?;
    println!("  beat 2: where it hangs -> {out}");

    println!(
        "gimbal at {:?} in the rig's own frame; eye {eye:?}",
        mining::Mining::gimbal(machine)
    );
    Ok(())
}

/// Rebuild every loaded chunk's mesh. Used after a headless excavation, where
/// there is no streamer running to pick up the dirty chunks.
fn remesh_all(context: &GpuContext, renderer: &mut Renderer, world: &mut World) {
    let loaded: Vec<vx_core::ChunkPos> = world.loaded_chunks().collect();
    for pos in loaded {
        let origin = pos.origin();
        let mesh = vx_mesh::build_mesh(world, world.registry(), [origin.x, 0, origin.z]);
        renderer.set_chunk_mesh(&context.device, pos, &mesh);
        world.clear_dirty(pos);
    }
}

fn run_windowed(options: &Options) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| {
        format!(
            "could not open a window system connection: {error}\n\
             If this machine has no display, render offscreen instead:\n  \
             gamingg --screenshot frame.ppm"
        )
    })?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new(
        options.seed,
        options.width,
        options.height,
        options.world.clone(),
        DevOverrides {
            crew: options.drones,
            view_distance: options.view_distance,
            sheriff: options.sheriff,
            gold_enabled: options.gold,
        },
    );
    event_loop
        .run_app(&mut app)
        .map_err(|error| format!("event loop failed: {error}"))
}

/// One row of the handheld's kestrel page: what pressing Enter on it does.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ScoutRow {
    /// A standing order for the scout's own flying.
    Stand(journal::ScoutOrder),
    /// A job for the spoofer coil.
    Hack(journal::IntrudeOrder),
    /// Call the running job off.
    Abort,
}

/// The command-line switches that exist for development rather than play,
/// bundled so `App::new` does not grow a parameter per round.
struct DevOverrides {
    crew: u32,
    view_distance: i32,
    sheriff: bool,
    gold_enabled: bool,
}

/// Is the operator's console open? False by construction when the panel is
/// compiled out, which is what keeps the frame loop free of feature soup.
#[cfg(feature = "gold")]
fn gold_open(active: &Active) -> bool {
    active.gold.open
}
#[cfg(not(feature = "gold"))]
fn gold_open(_active: &Active) -> bool {
    false
}

/// Everything that only exists once there is a window.
struct Active {
    window: Arc<Window>,
    context: GpuContext,
    surface: WindowSurface,
    renderer: Renderer,
    world: World,
    streamer: ChunkStreamer,
    camera: Camera,
    /// The physical body. Only drives the camera in walk mode; in fly mode the
    /// camera moves freely and the body is carried along behind it.
    player: PlayerBody,
    /// Stance, stamina and the ledge verbs. See `movement`.
    movement: movement::Movement,
    /// Turns elapsed wall-clock into whole movement ticks, so the player runs
    /// on a fixed clock like everything else in the world.
    move_ticks: movement::Ticker,
    /// The last command written to the journal, so only changes are recorded.
    last_move: Option<movement::MoveCommand>,
    /// The same, for the machine you are driving. Cleared when the wheel is
    /// handed back, so taking it again records the first command afresh
    /// rather than assuming the log still remembers it.
    last_pilot: Option<vx_agent::PilotCommand>,
    mode: MovementMode,
    /// Carries block edits to any listeners. Mods hook in here in M3.
    events: EventBus,
    save: Option<WorldSave>,
    /// Blocks the player can place, and which one is selected.
    palette: Vec<BlockId>,
    selected: usize,
    /// Marking a body and running a drone on it.
    mining: Mining,
    /// The fog-of-war minimap.
    map: MapState,
    /// The player's skill sheet.
    skills: Skills,
    /// The block being drilled and how far through it the bit has got.
    digging: Option<(vx_core::BlockPos, f32)>,
    /// A recent level-up, shown on the HUD for a moment.
    level_up: Option<(String, u32, Instant)>,
    /// The hour of the in-game day.
    clock: TimeOfDay,
    /// Whose eyes the player is looking through.
    view: ViewMode,
    /// The handheld, and whatever feed it has open.
    device: device::Device,
    /// The player's own look, parked while they stare at the handheld.
    body_yaw: f32,
    body_pitch: f32,
    /// True between hanging up and the player's own ground arriving back.
    resuming: bool,
    /// The player's own body, drawn in third person.
    player_rig: Rig,
    /// The shape of the thing in the deep. Built once, like every other rig.
    stalker_rig: Rig,
    /// The handheld PC, as an object you hold rather than a panel that
    /// appears.
    handheld_rig: Rig,
    /// The handheld drill's shape, built once.
    hand_rig: Rig,
    /// The body a trade load borrows when one passes close enough to see.
    trade_rig: Rig,
    /// The viewmodel drill's accumulated rotation.
    drill_spin: f32,
    /// The town's inhabitants.
    villagers: Villagers,
    /// Their bodies, built once per variant.
    villager_rigs: Vec<Rig>,
    /// What you can take before you go down.
    health: health::Health,
    /// The squad the warrant sent, once it has been sent.
    posse: hostile::Posse,
    /// Every held shelter the session has met.
    garrisons: garrison::Garrisons,
    /// Your name across the county, and among the shelters.
    reputation: reputation::Reputation,
    /// The jam toast has been shown for the current overflight.
    jam_warned: bool,
    /// The F3 readout is up.
    debug_open: bool,
    /// Seconds until the warrant is checked again — a callout starts on a
    /// beat rather than the instant a bounty ticks over.
    warrant_check: f32,
    /// A villager's freshly spoken line, shown on the HUD for a moment.
    greeting: Option<(String, Instant)>,
    /// Credits and bought upgrades.
    wallet: Wallet,
    /// The supply shop's panel state.
    shop: shop::Shop,
    /// The beacon console's panel state.
    board: board::Board,
    /// Contracts taken, settled, and towns actually stood in.
    ledger: beacon::Ledger,
    /// Every order given since the last keyframe, and the clock it is kept in.
    journal: CommandLog,
    /// Town markets: what each place holds, makes and charges.
    economy: economy::Economy,
    /// The machines the player actually owns.
    garage: garage::Garage,
    /// The house: the chest, the mailbox, and what they hold.
    homestead: homestead::Homestead,
    /// Mail offers priced when the counter opened. Re-checked at confirm.
    offers: Vec<shop::Offer>,
    /// The welcome panel, open on the first boot of this machine.
    intro: intro::Intro,
    /// Who may edit what, and what the town has caught you doing. Shared with
    /// the edit gate on the event bus, which is why it is behind an `Rc`.
    permits: permits::Shared,
    /// The lockbox panel.
    permit_panel: permits::PermitPanel,
    /// Whether anybody can see the player right now, for the HUD's eye.
    watched: bool,
    /// Enter is held at a lockbox: keep working at it.
    picking: bool,
    /// The operator's console.
    #[cfg(feature = "gold")]
    gold: gold::Gold,
    /// The hash the operator asked for, cleared when the panel closes.
    #[cfg(feature = "gold")]
    gold_hash: Option<u64>,
    /// The chest panel's cursor and feedback.
    home_panel: homestead::HomePanel,
    /// The town whose counter is open. `TownSite` is `Copy`, so this can be
    /// taken out and handed to the shop beside a mutable borrow of the books.
    counter: Option<vx_world::town::TownSite>,
    /// The town whose beacon is open, and the work it is broadcasting.
    /// Derived when the console opens, not stored in the world.
    console: Option<Console>,
    /// The column discovery last ran at.
    last_scan: (i32, i32),
    /// The dispatch window the trade network was last run for.
    last_network: u64,
    /// The launcher, the satchel, the warnings spent and the wreckage.
    arsenal: arsenal::Arsenal,
    /// Slugs in the air right now — the live twin of `Rebuilt::shots`.
    shots: Vec<arsenal::Shot>,
    /// And stems on their way down: the live twin of `Rebuilt::falls`,
    /// stepped on the same clock so both sides edit the same ground.
    falls: Vec<felling::Falling>,
    /// And water that is still moving: the live twin of `Rebuilt::water`.
    water: Vec<vx_world::fluid::Water>,
    /// Pumps switched on, and the step they lift on. Live-only: a pump is a
    /// machine you stand and watch, and one left running in a world you
    /// closed is off when you come back.
    pumps: Vec<vx_core::BlockPos>,
    pump_step: u32,
    /// What is burning — the live twin of `Rebuilt::fires`.
    fires: Vec<fire::Fire>,
    /// And what is growing back, which is the one part of this round that
    /// outlives a session.
    stands: succession::Ledger,
    /// What the towns have open on you: who has asked the mayor for paper,
    /// who got it, and who was told to wait.
    warrants: warrant::Docket,
    /// Every seat anywhere that is not what the seed said, and the ones you
    /// are standing for.
    elections: ballot::Register,
    /// The towns you founded, and the charters you have not filed yet.
    charters: charter::Charters,
    /// The speaker, when the machine has one.
    audio: audio::Audio,
    /// The launcher's viewmodel shape, built once.
    launcher_rig: Rig,
    /// Screen shake energy, 0..1. Decays; firing tops it up. Visual only —
    /// it offsets the camera pivot and never touches the simulation.
    shake: f32,
    /// Accumulated *frame* time driving the shake wobble. Never wall clock.
    shake_phase: f32,
    /// The scout's collected intelligence.
    marks: scout::Marks,
    /// The hometown's watcher, once its box is in loaded ground.
    roost: Option<roost::Roost>,
    /// The spoofer kit: what it is working on, and what the pound is owed.
    intrusion: intrusion::Intrusions,
    /// The fabricator: where it stands, and what is on its bed.
    printer: printer::Printer,
    /// The lamp and the visors: how the player sees the dark.
    optics: optics::Optics,
    /// The machine that turns a lake into fuel.
    electrolyser: electrolysis::Electrolyser,
    /// The wellhead panel: which head is open, and what it last said. The
    /// holes themselves live on `Mining`, where the oracle carries them.
    well_panel: well::Panel,
    /// The ward, when its door is open.
    clinic: clinic::Clinic,
    /// The pocket arcade: the cartridge, the run on it and the record.
    arcade: arcade::Arcade,
    /// The deep, and whatever it has sent. Live-only like the posse: it
    /// reads the world, spends health and says things, and never writes a
    /// block or touches the pile.
    dark: stalker::TheDark,
    /// What the deep ore has done to you. Live-only, like the health it
    /// spends: no journal ever hears about a dose.
    dose: dose::Dose,
    /// Seconds until the next exposure sample. The sum is over a box of a
    /// thousand blocks, which is cheap but not free, and a body does not
    /// move far in a quarter second.
    dose_check: f32,
    /// The last exposure sample, held between beats.
    last_rads: f32,
    /// Every town's strongroom.
    banks: bank::Bank,
    /// Who in every town knows you, and how well: the friendship ledger.
    friends: disposition::Disposition,
    /// The typed console, and the log the toasts also land in.
    terminal: terminal::Terminal,
    /// The pad's own keyboard, raised over the terminal.
    osk: osk::Osk,
    /// The last toast written into that log, so one line is not written
    /// every frame it happens to still be on screen.
    logged: Option<Instant>,
}

struct App {
    seed: u64,
    width: u32,
    height: u32,
    world_name: String,
    /// Drones a dispatch puts on the job.
    crew: u32,
    /// Chunks visible in every direction.
    view_distance: i32,
    /// Start wearing the sheriff's badge.
    sheriff: bool,
    /// The operator's console is armed (still needs F10 to open). Unread in
    /// a build that compiled the console out.
    #[allow(dead_code)]
    gold_enabled: bool,
    active: Option<Active>,
    fly: FlyController,
    walk: WalkController,
    input: InputState,
    /// The pad, when the machine has one. Optional everywhere: input must
    /// never be the reason the game cannot start.
    pad: gamepad::Pad,
    /// The left button is held: the drill is running.
    drill_held: bool,
    last_frame: Instant,
    // Frame timing for the title bar readout.
    frames: u32,
    last_report: Instant,
    /// The last measured frames-per-second, for the F3 readout — measured
    /// in one place so the title bar and the panel can never disagree.
    fps: f32,
}

impl App {
    fn new(seed: u64, width: u32, height: u32, world_name: String, dev: DevOverrides) -> Self {
        let DevOverrides {
            crew,
            view_distance,
            sheriff,
            gold_enabled,
        } = dev;
        App {
            seed,
            width,
            height,
            world_name,
            crew,
            view_distance,
            sheriff,
            gold_enabled,
            active: None,
            fly: FlyController::default(),
            drill_held: false,
            walk: WalkController::default(),
            input: InputState::new(),
            pad: gamepad::Pad::new(),
            last_frame: Instant::now(),
            frames: 0,
            last_report: Instant::now(),
            fps: 0.0,
        }
    }

    /// Grab or release the pointer for mouse-look.
    fn set_capture(&mut self, captured: bool) {
        let Some(active) = &self.active else { return };

        if captured {
            // Locked is unavailable on some X11 setups; Confined is the
            // fallback that still lets the pointer drive the view.
            let grabbed = active
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| active.window.set_cursor_grab(CursorGrabMode::Confined));
            if let Err(error) = grabbed {
                log::warn!("could not capture the cursor: {error}");
                return;
            }
        } else if let Err(error) = active.window.set_cursor_grab(CursorGrabMode::None) {
            log::warn!("could not release the cursor: {error}");
        }

        active.window.set_cursor_visible(!captured);
        self.input.mouse_captured = captured;
        // Drop any motion accumulated across the transition.
        self.input.take_mouse_delta();
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        self.poll_pad(dt);

        // Disjoint field borrows: the controllers and input are separate
        // fields from `active`, so this is fine and lets the camera be mutated
        // in place rather than through a copy.
        let Some(active) = &mut self.active else { return };

        // Trading suspends walking and drilling: the shop owns the input
        // while it is open, and drifting away mid-deal would be silly.
        // The day rolls on. The dt above is already clamped, so a stalled
        // frame cannot lurch the clock forward by an hour.
        active.clock = active.clock.advance(dt);

        // The handheld swings up or down. Kept out here rather than in the
        // key handler because closing it starts a *movement*, and the slot
        // is only cleared once the unit is genuinely off the frame.
        if !active.device.raise_by(dt) {
            active.renderer.clear_overlay(DEVICE_SLOT);
        }

        // The toy. Its controls are held keys read straight off the input
        // state — plus the pad's sticks, because walk and strafe are `KeyW`,
        // `KeyS` and `KeyQ`, and no button on a pad produces those. Stage 24
        // claimed the panel mapping already reached the arcade; it did not,
        // and the toy was unplayable on a Deck.
        if active.device.open && active.device.page == device::Page::Arcade {
            let stick = self.input.pad_axes();
            const TILT: f32 = 0.35;
            let held = arcade::Buttons {
                forward: self.input.is_down(KeyCode::KeyW) || stick.z > TILT,
                back: self.input.is_down(KeyCode::KeyS) || stick.z < -TILT,
                turn_left: self.input.is_down(KeyCode::KeyA)
                    || self.input.is_down(KeyCode::ArrowLeft)
                    || self.pad.right_stick().0 < -TILT,
                turn_right: self.input.is_down(KeyCode::KeyD)
                    || self.input.is_down(KeyCode::ArrowRight)
                    || self.pad.right_stick().0 > TILT,
                strafe_left: self.input.is_down(KeyCode::KeyQ) || stick.x < -TILT,
                strafe_right: self.input.is_down(KeyCode::KeyE) || stick.x > TILT,
                fire: self.input.is_down(KeyCode::Space) || self.drill_held,
                start: self.input.is_down(KeyCode::Enter)
                    || self.input.is_down(KeyCode::NumpadEnter),
            };
            active.arcade.step(dt, held);
        }

        // The body stands still whenever the player's attention is elsewhere:
        // haggling at a counter, or staring at a handheld. For the feed that
        // is also the fiction — you are standing there looking at a screen.
        let feed = active.device.feed();
        let busy = active.shop.open
            || active.board.open
            || active.device.open
            || active.home_panel.open
            || active.permit_panel.open
            || active.printer.open
            || gold_open(active)
            || active.intro.open
            || feed.is_some();
        if busy || active.resuming {
            active.player.velocity = glam::DVec3::ZERO;
        }
        let trading = busy;

        match active.mode {
            MovementMode::Fly if trading => {}
            MovementMode::Walk if trading => {}
            MovementMode::Fly => {
                self.fly.apply(&mut active.camera, &mut self.input, dt);
                // Keep the body under the camera so switching to walk mode
                // drops the player where they are looking from, not wherever
                // they last stood.
                active.player.position =
                    active.camera.position - glam::DVec3::Y * active.player.eye_height;
                active.player.velocity = glam::DVec3::ZERO;
                active.player.on_ground = false;
            }
            // Just back from a feed: the ground under the body may still be
            // streaming in. Stepping physics now drops it through the world.
            MovementMode::Walk if active.resuming => {}
            MovementMode::Walk => {
                // Sample once, then run whole ticks of it. This is the seam the
                // movement round exists for: nothing below this line sees `dt`.
                let carried = active
                    .mining
                    .fleet
                    .base
                    .as_ref()
                    .map(|base| base.stockpile.total())
                    .unwrap_or(0)
                    // The launcher weighs like cargo: folded into the load
                    // *before* the command is journalled, so the weight
                    // replays without the oracle learning what a weapon is.
                    + if active.arsenal.owned {
                        arsenal::LAUNCHER_HEFT
                    } else {
                        0
                    };
                // What you learned, then what you fitted. Safe against the
                // oracle because the load reaches the journal as the byte
                // computed below, not as something replay re-derives.
                let capacity = wallet::pack_capacity(
                    skills::capacity(
                        vx_agent::DEFAULT_CAPACITY,
                        active.skills.level(skills::LOGISTICS),
                    ),
                    active.wallet.upgrade(wallet::PACK),
                );
                let load = movement::load_byte(carried, capacity);
                let command = self
                    .walk
                    .sample(&mut active.camera, &mut self.input)
                    .laden(load);

                if active.last_move != Some(command) {
                    active.journal.record(journal::Command::moving(command));
                    active.last_move = Some(command);
                }

                let ticks = active.move_ticks.take(dt);
                for _ in 0..ticks {
                    active.movement.advance(
                        &mut active.player,
                        &active.world,
                        command,
                        command.mass(),
                        movement::MOVE_TICK,
                    );
                }
            }
        }

        // One owner for where the camera physically sits. The controllers set
        // orientation (and, walking, the body); this decides first person
        // against over-the-shoulder, and pulls in past anything solid.
        let pivot = match active.mode {
            MovementMode::Fly => active.camera.position,
            MovementMode::Walk => active.player.eye_position(),
        };
        // The launcher's kick, worn by the camera. Offset the *pivot*, not
        // the body: the shake is theatre, the simulation never feels it, and
        // in third person the orbit's own wall raycast still runs from the
        // shaken anchor so the camera cannot be kicked into rock. Driven by
        // accumulated frame time, never the wall clock.
        active.shake = (active.shake - dt * 1.6).max(0.0);
        active.shake_phase += dt * 35.0;
        let pivot = if active.shake > 0.0 {
            let wobble =
                active.shake * active.shake * active.movement.tuning.shake_power * 0.14;
            let phase = active.shake_phase;
            pivot
                + (glam::Vec3::new(
                    phase.sin(),
                    (phase * 1.31).sin(),
                    (phase * 0.73).cos(),
                ) * wobble)
                    .as_dvec3()
        } else {
            pivot
        };
        active.camera.position =
            view::camera_placement(&active.world, &active.camera, pivot, active.view);

        // A live feed steers the machine and rides its camera. Mouse-look
        // belongs entirely to the drone while it is up; the body's own view is
        // parked and handed back on hang-up.
        if let Some(machine) = feed {
            controller::apply_mouse_look(&mut active.camera, &mut self.input, self.walk.sensitivity);
            // The simulation is the authority on who has the wheel: the
            // handheld can ask, but only `Mining` grants it.
            let command = if active.mining.piloted() == Some(machine) {
                Self::pilot_command(&active.camera, &self.input, self.drill_held)
            } else {
                vx_agent::PilotCommand::default()
            };
            // On change only, like `Move`: it is a held input and `Advance`
            // counts the ticks it covers. Without this the log knew how many
            // ticks a hand-driven drone worked and nothing about what it was
            // told to do, so replay ran them with nobody at the controls and
            // every hand-dug block failed to appear.
            if active.last_pilot != Some(command) {
                active.journal.record(Command::piloting(command));
                active.last_pilot = Some(command);
            }
            active.mining.set_pilot_command(command);
            // Deliberately not journalled: the look only points the drawn
            // nose and never reaches a world edit.
            active.mining.set_pilot_look(active.camera.yaw);
            if let Some(eye) = active.mining.machine_eye(machine) {
                active.camera.position = eye;
            }
        }


        // Once the body's own ground is back, gravity may have it again.
        if active.resuming && self.body_chunk_ready() {
            if let Some(active) = &mut self.active {
                active.resuming = false;
            }
        }

        // The player's drill runs before streaming too, for the same reason
        // the drones dig first: edits land in this frame's remesh.
        if !trading {
            self.update_drilling(dt);
        }
        let Some(active) = &mut self.active else { return };

        // The town puts its broken locks back up. Recorded as an ordinary
        // `Place` so a replay puts them back at the same tick — the repair is
        // a world edit, and a world edit the journal cannot see is a world
        // edit that makes the oracle lie.
        {
            let now = active.journal.tick();
            let due = active.permits.borrow_mut().due_rebuilds(now);
            for at in due {
                let tier = active.permits.borrow().lock_tier_at(at);
                let Some(tier) = tier else { continue };
                let name = tier.block_name();
                let Some(block) = active.world.registry().id_of(name) else {
                    continue;
                };
                if active.world.set_block(at, block).is_some() {
                    active.journal.record(Command::Place {
                        at,
                        block: name.to_string(),
                    });
                }
            }
        }


        // The townsfolk take their stroll, notice what is about, and one of
        // them may say hello. Machines count as things worth watching, so a
        // villager will turn to follow a drone trundling past.
        let machines: Vec<(awareness::TargetKind, glam::DVec3)> = active
            .mining
            .drone_positions()
            .into_iter()
            .map(|at| {
                (
                    awareness::TargetKind::Digger,
                    glam::DVec3::new(f64::from(at.x) + 0.5, f64::from(at.y), f64::from(at.z) + 0.5),
                )
            })
            .chain(active.mining.fleet.fliers.iter().map(|flier| {
                (
                    awareness::TargetKind::Flier,
                    glam::DVec3::new(
                        f64::from(flier.position.x) + 0.5,
                        f64::from(flier.position.y),
                        f64::from(flier.position.z) + 0.5,
                    ),
                )
            }))
            .collect();
        // The player's stance decides how tall they are to look at, which is
        // the whole of the stealth system: prone is 0.35 m, and a one-block
        // wall is 1.0 m.
        let around = awareness::Surroundings {
            player_eye: active.player.eye_height as f32,
            world: Some(&active.world),
            player: Some(active.player.position),
            machines: &machines,
        };
        // Ground witnesses, plus the eye in the sky. The roost counting as
        // a witness is what the "observed" warning is warning you about.
        let aerial = active.roost.as_ref().is_some_and(|roost| {
            roost.sees(&active.world, active.player.eye_position())
        });
        active.watched = aerial || active.villagers.witnesses(&around) > 0;
        if aerial {
            if let Some(roost) = &mut active.roost {
                roost.hold_interest();
                if !roost.observed {
                    roost.observed = true;
                    active.greeting = Some((
                        "THE TOWN'S EYE IS ON YOU".into(),
                        Instant::now(),
                    ));
                }
            }
        }

        // Picking a lock: you stand still and exposed while it runs, which is
        // the whole cost of the quiet route. Being seen finishing one is what
        // puts it on your sheet.
        if active.permit_panel.open && active.picking {
            let level = active.skills.level(skills::SECURITY);
            let outcome = {
                let mut permits = active.permits.borrow_mut();
                active.permit_panel.work_bypass(&mut permits, level, dt)
            };
            match outcome {
                permits::Bypass::Working(_) => {}
                permits::Bypass::Opened { xp } => {
                    active.picking = false;
                    let seen = usize::from(active.watched);
                    let caught = active
                        .permits
                        .borrow_mut()
                        .caught(permits::BOUNTY_HACK, seen);
                    if let Some(level) = active.skills.add_xp(skills::SECURITY, xp) {
                        active.level_up =
                            Some((skills::SECURITY.to_string(), level, Instant::now()));
                    }
                    active.permit_panel.feedback = Some(
                        if caught { "OPEN - AND YOU WERE SEEN" } else { "OPEN. NOBODY SAW A THING" }
                            .into(),
                    );
                }
                permits::Bypass::Refused(why) => {
                    active.picking = false;
                    active.permit_panel.feedback = Some(why);
                }
            }
        }
        active
            .villagers
            .set_day((active.journal.tick() / schedule::TICKS_PER_DAY) as u32);
        active.villagers.update(dt, active.clock, &around);

        // The law in force is the law of the towns you are standing near. A
        // fixed set taken at startup would enforce spawn's rules on the far
        // side of the map and none at all on the near side.
        {
            let column = (
                active.player.position.x as i32,
                active.player.position.z as i32,
            );
            let nearby = active.world.towns_near(column, RADIO_RANGE);
            active.permits.borrow_mut().set_sites(nearby);
        }
        if let Some(line) = active.villagers.greeting_for() {
            active.greeting = Some((line.to_string(), Instant::now()));
        }
        // And standing on somebody's feet earns a noise rather than a line.
        // Both the sound and the toast, because a machine with no speaker is
        // a supported machine here and the tell has to survive one.
        if let Some(variant) = active.villagers.grunt_for(active.player.position) {
            active.audio.play(audio::Cue::Grunt(variant), 0.7);
            active.greeting = Some((
                "SOMEBODY GRUNTS AND SHIFTS OUT OF YOUR WAY".into(),
                Instant::now(),
            ));
        }

        // Carrying the launcher raised is menacing all by itself: anyone under
        // the muzzle who can see you panics, and each fresh panic is on your
        // sheet — the victim is their own witness.
        if active.arsenal.equipped
            && active.mode == MovementMode::Walk
            && self.input.mouse_captured
            && !trading
        {
            let muzzle = arsenal::muzzle_of(&active.player);
            let frightened =
                active
                    .villagers
                    .menaced(muzzle, active.camera.forward(), &around);
            if frightened > 0 {
                active
                    .permits
                    .borrow_mut()
                    .billed(arsenal::BOUNTY_MENACE * frightened as u64);
                active.greeting = Some((
                    "THEY SAW THE GUN. THAT COSTS YOU".into(),
                    Instant::now(),
                ));
            }
        }
        // The law's own frame, once the borrow of the world that
        // `Surroundings` holds has been let go.
        Self::advance_law(active, dt);

        // An alarm that reaches the security office is a signed statement.
        let reports = active.villagers.take_reports();
        if reports > 0 {
            active
                .permits
                .borrow_mut()
                .billed(arsenal::BOUNTY_REPORTED * u64::from(reports));
            active.greeting = Some((
                "YOU HAVE BEEN REPORTED AT THE SECURITY OFFICE".into(),
                Instant::now(),
            ));
        }

        // Drones dig before streaming, so the chunks their edits dirty get
        // re-meshed in the same frame rather than the next one.
        let report = active.mining.update(
            &mut active.world,
            &active.events,
            std::time::Duration::from_secs_f32(dt),
        );
        // Ticks, not seconds. How many ticks a frame is worth depends on frame
        // rate and stalls; how the world evolves across them does not.
        active
            .journal
            .record(Command::Advance {
                ticks: active.mining.last_ticks(),
            });
        // Slugs in the air step on the same clock, through the same function
        // replay uses — the sweeps are where the live game reads the bills.
        Self::advance_gunfire(active, active.mining.last_ticks());
        // And so do stems on their way down, for the same reason: what a
        // trunk sweeps through is ground, and ground is the hash.
        Self::advance_felling(active, active.mining.last_ticks());
        Self::advance_water(active, active.mining.last_ticks());
        Self::advance_weather(active, active.mining.last_ticks());
        Self::collect_crashes(active);
        Self::advance_scouts(active, active.mining.last_ticks());
        Self::advance_printing(active, active.mining.last_ticks());
        Self::advance_electrolysis(active, active.mining.last_ticks());
        Self::report_the_wells(active);

        // The fleet's work becomes the player's experience.
        if report.sectors_completed > 0 || report.pings_found > 0 {
            let xp = u64::from(report.sectors_completed) * skills::SWEEP_XP
                + u64::from(report.pings_found) * skills::PING_XP;
            if let Some(level) = active.skills.add_xp(skills::PROSPECTING, xp) {
                active.level_up = Some((skills::PROSPECTING.to_string(), level, Instant::now()));
            }
        }
        if report.delivered > 0 {
            let xp = report.delivered * skills::DELIVERY_XP;
            if let Some(level) = active.skills.add_xp(skills::LOGISTICS, xp) {
                active.level_up = Some((skills::LOGISTICS.to_string(), level, Instant::now()));
            }
        }
        // Levels feed straight back into the machines' stats, and bought
        // cargo upgrades multiply on top of what Logistics earned.
        let cargo_level = active.wallet.upgrade(wallet::CARGO);
        active.mining.apply_skills(
            skills::scan_depth(active.skills.level(skills::PROSPECTING)),
            wallet::boosted_capacity(
                skills::capacity(
                    vx_agent::DEFAULT_CAPACITY,
                    active.skills.level(skills::LOGISTICS),
                ),
                cargo_level,
            ),
            wallet::boosted_capacity(
                skills::capacity(
                    vx_agent::DEFAULT_FLIER_CAPACITY,
                    active.skills.level(skills::LOGISTICS),
                ),
                cargo_level,
            ),
        );

        // Keep chunks in step with where the camera is — which, on a feed, is
        // the machine. That is what lets you drive a drone across the map and
        // actually see where it goes.
        let centre = chunk_at(active.camera.position);
        active.streamer.update(
            &mut active.world,
            &mut active.renderer,
            &active.context.device,
            centre,
            active.save.as_ref(),
        );

        // Ground near the player is explored by being there; swept sectors
        // are explored from the air — half the reason to send the flier.
        // Exploration is earned by being somewhere yourself. A drone's camera
        // is not the player, and painting the map by remote would take the
        // scouting job away from the flier, whose swept sectors still count.
        if feed.is_none() {
            active.map.explore_around(centre, 8);
        }
        for sector in active.mining.fleet.surveyed_sectors().collect::<Vec<_>>() {
            for chunk in map::sector_chunks(sector) {
                active.map.explore(chunk);
            }
        }

        // Walking near a town finds it, and finding it is permanent: the
        // ledger is the only thing that says a town exists as far as the map
        // is concerned. Like exploration, it is earned in person — a drone
        // wandering into a settlement does not put it on your map.
        if feed.is_none() {
            let column = (
                active.player.position.x.floor() as i32,
                active.player.position.z.floor() as i32,
            );
            let moved = (column.0 - active.last_scan.0).saturating_abs()
                + (column.1 - active.last_scan.1).saturating_abs();
            if moved >= DISCOVERY_STEP {
                active.last_scan = column;
                let near = active
                    .world
                    .towns_near(column, beacon::DISCOVERY_RANGE);
                for site in &near {
                    if active.ledger.visit(site.centre) {
                        log::info!(
                            "found {} ({}) at {} {}",
                            site.name,
                            site.speciality.name(),
                            site.centre.0,
                            site.centre.1
                        );
                    }
                }
                // Only the town you are standing in has people in it. Distant
                // towns are architecture until you arrive, which is what keeps
                // the frame budget flat however many of them exist.
                if let Some(site) = near.first() {
                    if site.centre != active.villagers.site().centre {
                        active.villagers = Villagers::for_site(site);
                    }
                }
            }
        }

        // The towns do business. Once a dispatch window, not once a frame:
        // the network only looks for work every four in-game minutes, and the
        // one real cost here is gathering who is within radio range.
        let now = active.journal.tick();
        let window = now / economy::DISPATCH_EVERY;
        if window != active.last_network {
            active.last_network = window;
            let column = (
                active.player.position.x.floor() as i32,
                active.player.position.z.floor() as i32,
            );
            let reachable = active.world.towns_near(column, RADIO_RANGE);
            for landed in active.economy.run(&reachable, now) {
                // Mail lands whether or not the player is anywhere near home:
                // nothing about it needs the destination market, so it must
                // run before the radio-range guard below.
                if landed.owner == economy::Owner::Mail {
                    active
                        .homestead
                        .mailbox
                        .add(economy::GOODS[landed.good], landed.amount.round() as u64);
                    active.greeting = Some((
                        "MAIL FOR YOU. CHECK THE MAILBOX AT HOME".into(),
                        Instant::now(),
                    ));
                    continue;
                }
                // A load of the player's that has arrived: paid at the far
                // town's price, which is the whole reason to have sent it.
                let Some(site) = reachable.iter().find(|site| site.centre == landed.to) else {
                    continue;
                };
                let paid = landed.amount.round() as u64
                    * active.economy.market(site, now).price(landed.good);
                active.wallet.earn(paid);
                active.greeting = Some((
                    format!(
                        "{} DELIVERED AT {}. +{paid} CR",
                        shop::display_name(economy::GOODS[landed.good]),
                        site.name
                    ),
                    Instant::now(),
                ));
                log::info!("a run landed at {}: {paid} credits", site.name);
            }
        }

        self.refresh_minimap();
        let Some(active) = &mut self.active else { return };

        active
            .renderer
            .update_camera(&active.context.queue, &active.camera);
        let mut sun = clock::sun_uniform(clock::sky_at(active.clock));
        // Weather rides the sun uniform the way the optics do: a storm greys
        // the sky, takes the sun off the hills and lifts the ambient, so an
        // overcast noon reads as overcast rather than as dusk. No shader
        // change — the values were always there to be written.
        let sky = vx_world::weather::at(
            active.world.seed(),
            active.journal.tick(),
            active.player.position.x as i32,
            active.player.position.z as i32,
        );
        // The month, under the cloud: a January overcast and a July one are
        // not the same afternoon, and doing these the other way round would
        // make them one.
        clock::tint_for_season(&mut sun, active.journal.tick());
        let overcast = match sky.state {
            vx_world::weather::State::Clear => 0.0,
            vx_world::weather::State::Cloud => 0.45,
            vx_world::weather::State::Rain => 0.75,
            vx_world::weather::State::Storm => 1.0,
        };
        if overcast > 0.0 {
            let grey = [0.36, 0.38, 0.41];
            for (channel, tone) in grey.iter().enumerate() {
                sun.sky[channel] = sun.sky[channel] * (1.0 - overcast) + tone * overcast;
            }
            sun.light[0] *= 1.0 - 0.55 * overcast;
            sun.light[1] = (sun.light[1] + 0.10 * overcast).min(0.6);
        }
        // And the leaves. Free unless the year has actually moved on, which
        // is why this can sit in the frame path at all.
        active.renderer.repaint_foliage(
            &active.context.queue,
            vx_world::season::leaf_turn(active.journal.tick()),
        );
        // The optics ride the sun uniform: the lamp is a light from the
        // active eye, the visors are how the eye reads what arrives.
        if active.optics.mode == optics::Mode::Lamp {
            let beam = active.optics.beam();
            // The reflector fits whatever lamp you are carrying, suit or
            // printed. A shader uniform and nothing else, which is why an
            // upgrade is allowed to touch it at all.
            let (strength, reach) = wallet::boosted_beam(
                beam.strength,
                beam.reach,
                active.wallet.upgrade(wallet::LAMP),
            );
            // In the renderer's frame: the one world position that reaches
            // a uniform by hand goes through the one seam for it.
            let eye = active.renderer.relative(active.camera.position);
            let aim = active.camera.forward();
            sun.lamp_position = [eye.x, eye.y, eye.z, strength];
            sun.lamp_direction = [aim.x, aim.y, aim.z, reach];
        }
        sun.light[2] = active.optics.shader_mode();
        match active.optics.mode {
            // The sky is drawn by clear colour, not fragments, so the visors
            // have to tint it here or a green cave would open onto a blue day.
            optics::Mode::NightVision => sun.sky = [0.01, 0.09, 0.02, 1.0],
            optics::Mode::Thermal => sun.sky = [0.01, 0.01, 0.04, 1.0],
            _ => {}
        }
        active.renderer.set_sun(&active.context.queue, sun);
        // After the camera, because objects are culled against the frustum it
        // just refreshed.
        // Every body is built in the camera's frame: `relative` measures a
        // place from the render origin the camera just set, and the objects
        // are marked so, which is what keeps a villager's feet exact three
        // thousand kilometres out.
        let relative = |at: glam::DVec3| active.renderer.relative(at);
        let placed = |built: Vec<vx_render::Object>| {
            built.into_iter().map(vx_render::Object::already_relative)
        };
        let mut objects = active.mining.objects(relative, feed);
        objects.extend(active.villagers.objects(&active.villager_rigs, relative));
        for deputy in &active.posse.deputies {
            let rig = &active.villager_rigs[deputy.variant % active.villager_rigs.len()];
            objects.extend(placed(rig.objects(relative(deputy.position), deputy.yaw, 0.0)));
        }
        if let Some(stalker) = active.dark.present() {
            objects.extend(placed(active.stalker_rig.objects(
                relative(stalker.position),
                stalker.yaw,
                0.0,
            )));
        }
        // Stems on their way down. The rig is built along +X from the hinge,
        // so a pitch of the arc's own angle swings it about its base — the
        // same transform the handheld rides on, and no renderer change.
        for fall in &active.falls {
            let rig = trunk_rig(fall);
            let Some(yaw) = rig::yaw_towards(fall.direction.x as f32, fall.direction.z as f32)
            else {
                continue;
            };
            objects.extend(placed(rig.objects_pitched(
                relative(fall.hinge_point()),
                yaw,
                std::f32::consts::FRAC_PI_2 - fall.angle,
                0.0,
            )));
        }
        for holder in active.garrisons.squads.iter().flat_map(|squad| &squad.holders) {
            if holder.mode == hostile::Mode::Down {
                continue;
            }
            let rig = &active.villager_rigs[holder.variant % active.villager_rigs.len()];
            objects.extend(placed(rig.objects(relative(holder.position), holder.yaw, 0.0)));
        }
        // And the rain, on the same instanced path as everything above it —
        // a sheet of streaks around the eye, derived from the clock rather
        // than spawned, so it costs no state and no allocation per drop.
        if sky.rain > 0.0 {
            objects.extend(rain::streaks(
                active.world.seed(),
                active.journal.tick() as f32 / 64.0,
                active.camera.position,
                &sky,
            ));
        }

        // Trade traffic. A load in the air is not simulated — where it is now
        // is a sum — so drawing one costs a lerp and a height lookup, and only
        // for the handful close enough to see. Everything else on the network
        // stays pure bookkeeping.
        let eye = active.camera.position;
        let now = active.journal.tick();
        for load in active.economy.shipments() {
            let (x, z) = load.position_at(now);
            let (dx, dz) = (x - eye.x, z - eye.z);
            if dx * dx + dz * dz > f64::from(CARAVAN_SIGHT * CARAVAN_SIGHT) {
                continue;
            }
            let ground = active
                .world
                .generator()
                .height_at(x.floor() as i32, z.floor() as i32);
            let at = relative(glam::DVec3::new(
                x,
                f64::from(ground) + f64::from(CARAVAN_ALTITUDE),
                z,
            ));
            let heading = (load.to.1 - load.from.1) as f32;
            let yaw = ((load.to.0 - load.from.0) as f32).atan2(-heading);
            objects.extend(placed(active.trade_rig.objects(at, yaw, active.mining.spin())));
        }

        // Slugs in the air: a small steel cube each, drawn where the last
        // journal tick left them. At eight steps a second a round visibly
        // *travels*, which is half of what makes leading a caravan a skill.
        for shot in &active.shots {
            let model = glam::Mat4::from_translation(relative(shot.position))
                * glam::Mat4::from_scale(glam::Vec3::splat(0.14))
                * glam::Mat4::from_translation(glam::Vec3::splat(-0.5));
            objects.push(
                vx_render::Object::new(model, vx_render::tiles::slot::STEEL).already_relative(),
            );
        }
        // The town's watcher, whenever it is off its box — drawn with the
        // same silhouette as the player's own scout, which is the point.
        if let Some(roost) = active.roost.as_ref().filter(|roost| roost.aloft()) {
            objects.extend(active.mining.kestrel_objects(roost.position(), relative));
        }
        // Live marks: a small hovering cube over wherever a contact was
        // last seen. It hangs over the *report*, not the contact — stale
        // intelligence pointing at empty ground is the honest picture.
        let mark_now = active.journal.tick();
        for mark in active.marks.live(mark_now) {
            let over = relative(mark.position + glam::DVec3::Y * 2.4);
            objects.push(
                vx_render::Object::box_between(
                    over - glam::Vec3::splat(0.16),
                    over + glam::Vec3::splat(0.16),
                    vx_render::tiles::slot::COPPER_ORE,
                )
                .already_relative(),
            );
        }

        // Downed cargo, waiting where it fell.
        for crash in &active.arsenal.crashes {
            let ground = active
                .world
                .generator()
                .height_at(crash.x.floor() as i32, crash.z.floor() as i32);
            let at = relative(glam::DVec3::new(crash.x, f64::from(ground) + 1.4, crash.z));
            let model = glam::Mat4::from_translation(at)
                * glam::Mat4::from_scale(glam::Vec3::new(0.9, 0.8, 0.9))
                * glam::Mat4::from_translation(glam::Vec3::splat(-0.5));
            objects.push(
                vx_render::Object::new(model, vx_render::tiles::slot::HULL).already_relative(),
            );
        }

        // The held tool, drawn in camera space so it rides the view. The
        // drill's bit spins up while it is cutting and idles otherwise; a
        // slight bob sells the vibration. The launcher hangs heavier and
        // kicks back with the shake.
        active.drill_spin += dt * if active.digging.is_some() { 22.0 } else { 1.6 };
        let camera_forward = active.camera.forward();
        let camera_right = active.camera.right();
        let bob = if active.digging.is_some() {
            (active.drill_spin * 0.9).sin() * 0.02
        } else {
            0.0
        };
        let recoil_back = active.shake * 0.16;
        // The viewmodel is built in the renderer's own frame and marked so.
        // It is the one thing that sits right in front of the eye, where a
        // quarter-block quantisation far from the origin would show most.
        let eye_relative = active.renderer.relative(active.camera.position);
        let drill_position = eye_relative
            + camera_forward * (0.85 - recoil_back)
            + camera_right * 0.42
            + glam::Vec3::Y * (-0.38 + bob);
        let drill_yaw = rig::yaw_towards(camera_forward.x, camera_forward.z).unwrap_or(0.0);
        if active.view.draws_viewmodel() {
            // Both hands are busy while the handheld is up, so the drill and
            // the launcher go away. That is the whole reason raising it
            // reads as a gesture rather than as a menu: checking on your
            // fleet with something chasing you is a decision now.
            let carried: Vec<vx_render::Object> = if active.device.showing() {
                let held = device::carried_at(
                    eye_relative,
                    camera_forward,
                    camera_right,
                    active.device.raise,
                );
                let (_, _, _, tilt) = device::carry(active.device.raise);
                active.handheld_rig.objects_pitched(
                    held,
                    drill_yaw,
                    active.camera.pitch - tilt,
                    0.0,
                )
            } else if active.arsenal.equipped {
                active.launcher_rig.objects_pitched(
                    drill_position,
                    drill_yaw,
                    active.camera.pitch + active.shake * 0.25,
                    0.0,
                )
            } else {
                active.hand_rig.objects_pitched(
                    drill_position,
                    drill_yaw,
                    active.camera.pitch,
                    active.drill_spin,
                )
            };
            objects.extend(carried.into_iter().map(vx_render::Object::already_relative));
        }

        // Your own body, once you can actually see it.
        if active.view.draws_body() {
            // Faced by the quantised heading rather than the raw camera angle:
            // the drawn body should point where the simulation thinks it is
            // pointing, not a hair off it.
            let facing = active
                .last_move
                .filter(|_| active.mode == MovementMode::Walk)
                .map_or(drill_yaw, |command| command.yaw());
            let stance = active.movement.stance;
            let posed = active
                .player_rig
                .compressed(stance.body_height() / movement::STAND_HEIGHT);
            // Your own body rides the same frame as the tool in your hand.
            let feet = active.renderer.relative(active.player.position);
            objects.extend(
                posed
                    .objects(feet, facing, 0.0)
                    .into_iter()
                    .map(vx_render::Object::already_relative),
            );
        }

        // Machines and people darken with the ground they stand on, by the
        // same column-depth rule the mesher bakes into terrain. A model in
        // the camera's frame gets the render origin added back first, so
        // the column read is the one the body is actually standing in.
        let origin = active.renderer.render_origin().as_dvec3();
        for object in &mut objects {
            let local = object.model.transform_point3(glam::Vec3::splat(0.5)).as_dvec3();
            let at = if object.relative { local + origin } else { local };
            if let Some(stand) = active.world.surface_y(at.x.floor() as i32, at.z.floor() as i32)
            {
                let depth = (stand - 1) - at.y.floor() as i32;
                object.light = vx_mesh::sky_light(depth) as f32 / vx_mesh::FULL_LIGHT as f32;
            }
        }
        active
            .renderer
            .set_objects(&active.context.device, &active.context.queue, &objects);
        self.refresh_hud();
        self.refresh_shop();
        self.refresh_home();
        self.refresh_permit();
        self.refresh_printer();
        self.refresh_electrolyser();
        self.refresh_well();
        self.refresh_clinic();
        self.refresh_vault();
        self.refresh_terminal();
        self.refresh_debug();
        #[cfg(feature = "gold")]
        self.refresh_gold();
        self.refresh_intro();
        self.refresh_board();
        self.refresh_device();
        self.refresh_feed();
        let Some(active) = &mut self.active else { return };

        use wgpu::CurrentSurfaceTexture as Acquired;

        let acquired = active.surface.surface.get_current_texture();
        // Suboptimal still hands over a usable texture, so draw this frame and
        // reconfigure afterwards rather than dropping it.
        let reconfigure = matches!(acquired, Acquired::Suboptimal(_));

        match acquired {
            Acquired::Success(frame) | Acquired::Suboptimal(frame) => {
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = active.context.device.create_command_encoder(
                    &wgpu::CommandEncoderDescriptor {
                        label: Some("frame"),
                    },
                );
                active.renderer.render(&mut encoder, &view);
                active.context.queue.submit([encoder.finish()]);
                frame.present();
            }

            // Nothing to draw into this frame; try again next tick.
            Acquired::Timeout | Acquired::Occluded => {}

            // The surface went stale — on resize, or when the compositor
            // reconfigures. Rebuilding it is the documented recovery.
            Acquired::Outdated | Acquired::Lost => {
                let (width, height) = active.surface.size();
                active
                    .surface
                    .surface
                    .configure(&active.context.device, &active.surface.config);
                active.renderer.resize(&active.context, width, height);
            }

            Acquired::Validation => {
                log::error!("surface acquisition raised a validation error");
            }
        }

        if reconfigure {
            active
                .surface
                .surface
                .configure(&active.context.device, &active.surface.config);
        }

        self.report_framerate();
    }

    /// How far the player can reach to break or place, in blocks.
    const REACH: f32 = 6.0;

    /// The trigger, held or tapped. Fires at most once per cooldown; the
    /// order is journalled with its muzzle and quantised aim, then applied
    /// through the same `arsenal::launch` replay uses.
    fn update_firing(&mut self) {
        let firing = self.drill_held && self.input.mouse_captured;
        let Some(active) = &mut self.active else { return };
        active.digging = None;
        if !firing || active.mode != MovementMode::Walk || active.resuming {
            return;
        }
        if active.arsenal.cooldown > 0.0 {
            return;
        }
        if !active.arsenal.ready() {
            // The dry click is paced like a shot, or holding the button
            // rattles every frame.
            active.audio.play(audio::Cue::Click, 0.8);
            active.arsenal.cooldown = active.movement.tuning.slug_rate;
            active.greeting = Some(("OUT OF SLUGS".into(), Instant::now()));
            return;
        }

        // Quantised exactly as movement quantises a look, so live fire and
        // replay dequantise to the same line.
        let quantised =
            movement::MoveCommand::looking(0, active.camera.yaw, active.camera.pitch);
        let muzzle = arsenal::muzzle_of(&active.player);
        active.journal.record(Command::Fire {
            muzzle: muzzle.to_array(),
            yaw_q: quantised.yaw_q,
            pitch_q: quantised.pitch_q,
        });
        arsenal::launch(
            &mut active.shots,
            &mut active.movement,
            muzzle,
            quantised.yaw_q,
            quantised.pitch_q,
        );
        active.arsenal.spend(&active.movement.tuning);

        // The felt half: the aim climbs (real input state, so it rides the
        // next Move command), the view shakes, the street hears it.
        active.camera.pitch += 0.035;
        active.camera.clamp_pitch();
        active.shake = (active.shake + 0.6).min(1.0);
        active.audio.play(audio::Cue::Boom, 1.0);
        active.villagers.startled(muzzle);
        // And the box on the office roof has ears for gunfire.
        let muzzle_block = vx_core::BlockPos::new(
            muzzle.x.floor() as i32,
            muzzle.y.floor() as i32,
            muzzle.z.floor() as i32,
        );
        if let Some(roost) = &mut active.roost {
            roost.report(muzzle_block, roost::Report::Gunshot);
        }

        // Town rules: the first shot inside a town's line gets a warning.
        // Harm is billed separately, warning or none — the warning only ever
        // covers the noise.
        let column = vx_core::BlockPos::new(
            muzzle.x.floor() as i32,
            muzzle.y.floor() as i32,
            muzzle.z.floor() as i32,
        );
        let town = active
            .permits
            .borrow()
            .town_here(column)
            .map(|site| site.name.to_string());
        if let Some(name) = town {
            if active.arsenal.warn_once(&name) {
                active.greeting = Some((
                    format!("{name} SAYS: THAT IS YOUR ONE WARNING. NO GUNPLAY IN TOWN"),
                    Instant::now(),
                ));
            }
        }
    }

    /// Step every slug `ticks` journal ticks and settle what they did:
    /// caravans downed, property broken in view of witnesses, bystanders
    /// startled. The world edits happen inside `arsenal::advance_shots`,
    /// which is the half replay re-runs; everything else here is live-only
    /// consequence, the same side of the line the economy lives on.
    /// A band crossing is worth saying out loud; a point is not.
    fn note_band(active: &mut Active, faction: &str, band: Option<reputation::Standing>) {
        if let Some(band) = band {
            let line = format!("{faction} NOW CALL YOU {}", band.name());
            active.terminal.say(terminal::Kind::Note, line.clone());
            active.greeting = Some((line, Instant::now()));
        }
    }

    /// Holders put down or shelters broken: what everybody thinks of that.
    fn note_the_deeds(active: &mut Active, downed: u8, cleared: u8) {
        for _ in 0..downed {
            let compact = active.reputation.with_compact(reputation::KILL_COMPACT);
            Self::note_band(active, "THE TOWNS", compact);
            let holdouts = active.reputation.with_holdouts(reputation::KILL_HOLDOUTS);
            Self::note_band(active, "THE SHELTERS", holdouts);
        }
        for _ in 0..cleared {
            let holdouts = active.reputation.with_holdouts(reputation::CLEARED_HOLDOUTS);
            Self::note_band(active, "THE SHELTERS", holdouts);
        }
    }

    /// The law, one frame of it: mend, muster, and let the squad work.
    ///
    /// Live-only from top to bottom — nothing here touches a block, so the
    /// replay oracle never learns a callout happened. That is the same line
    /// villagers, the roost and contact marks draw, and it is why combat
    /// needed no journal version of its own.
    fn advance_law(active: &mut Active, dt: f32) {
        Self::advance_dose(active, dt);
        Self::advance_the_dark(active, dt);
        if active.health.tick(dt) && active.health.readout().is_some() {
            active.greeting = Some((
                active.health.readout().unwrap_or_default(),
                Instant::now(),
            ));
        }

        // A warrant musters a squad, on a beat rather than the instant the
        // counter ticks over. Held shelters muster on the same beat.
        active.warrant_check = (active.warrant_check - dt).max(0.0);
        if active.warrant_check == 0.0 {
            active.warrant_check = 5.0;
            active
                .garrisons
                .muster_near(&active.world, active.player.position);
            // The chain, which is what stands between a bounty and a posse
            // now. The sheriff asks; the mayor decides; the town starts
            // leaning on you the moment the asking happens.
            let town = *active.villagers.site();
            let tick = active.journal.tick();
            active.warrants.lapse(tick);
            let bounty = active.permits.borrow().bounty;
            if bounty >= permits::WARRANT_THRESHOLD {
                let (mayor, key) = Self::signing_mayor(active, &town);
                let tier = active.friends.tier(key);
                let trust = active.friends.trust(key);
                if let Some(filed) =
                    active.warrants.file(&town, &mayor, tier, trust, bounty, tick)
                {
                    Self::pay_the_fine(active, &town, &mayor, filed.fine);
                }
            } else {
                // Settle the bill and the paperwork goes in the drawer.
                active.warrants.clear(town.centre);
            }
            // And the ballot box, on the same beat. An election is a fact
            // about the day and the town rather than an order, so neither
            // side of the oracle needs telling it happened.
            Self::hold_the_polls(active, &town, tick);
            let wanted = active.warrants.granted_in(town.centre);
            if wanted && !active.posse.called_out() {
                let seed = active.journal.tick() ^ 0x51ed_5eed;
                let at = active.player.position;
                let ground = |x: f64, z: f64| {
                    active
                        .world
                        .surface_y(x.floor() as i32, z.floor() as i32)
                        .map_or(at.y, |top| f64::from(top + 1))
                };
                active.posse.call_out(at, ground, seed, Some(town.centre));
                let line = format!(
                    "{} SIGNED THE WARRANT. DEPUTIES ARE COMING",
                    office::seat(&town, permits::Office::Mayor).name
                );
                active.terminal.say(terminal::Kind::Warn, line.clone());
                active.greeting = Some((line, Instant::now()));
            }
            if !wanted && active.posse.called_out() {
                // Settle the bill and they lose interest.
                active.posse.stand_down();
                active.greeting = Some(("THE DEPUTIES HAVE STOOD DOWN".into(), Instant::now()));
            }
        }

        // The shelters' holders run whether or not a warrant is out: they
        // answer to nobody's bounty board but their own door.
        let held = active.garrisons.update(
            dt,
            &active.world,
            active.player.position,
            !active.health.standing(),
            // The truce is what Neutral buys: a challenge before a volley.
            active.reputation.holdouts() >= reputation::Standing::Neutral,
        );
        Self::note_the_deeds(active, held.downed, held.cleared);
        for line in &held.barks {
            active.terminal.say(terminal::Kind::Warn, line.clone());
            active.greeting = Some((line.clone(), Instant::now()));
        }
        if held.hits > 0 && active.health.standing() {
            let downed = active.health.take(held.hits * health::ROUND_HITS);
            active.shake = (active.shake + 0.35).min(1.0);
            if downed {
                active.greeting = Some(("YOU ARE DOWN".into(), Instant::now()));
            } else if let Some(line) = active.health.readout() {
                active.greeting = Some((line, Instant::now()));
            }
        }

        if !active.posse.called_out() {
            return;
        }
        let report = active.posse.update(
            dt,
            &active.world,
            active.player.position,
            !active.health.standing(),
        );

        for line in report.barks {
            active.terminal.say(terminal::Kind::Warn, line.clone());
            active.greeting = Some((line, Instant::now()));
        }
        if report.hits > 0 && active.health.standing() {
            let downed = active.health.take(report.hits * health::ROUND_HITS);
            active.shake = (active.shake + 0.35).min(1.0);
            if downed {
                active.greeting = Some(("YOU ARE DOWN".into(), Instant::now()));
            } else if active.health.hits() == 1 {
                active.greeting = Some((
                    "ONE MORE AND YOU ARE DONE - BREAK CONTACT".into(),
                    Instant::now(),
                ));
            } else if let Some(line) = active.health.readout() {
                active.greeting = Some((line, Instant::now()));
            }
        }
        if report.arrested {
            Self::make_the_arrest(active);
        }
    }

    /// Downed in front of the law: the bounty is settled out of credits,
    /// whatever is left is written off, and you wake up whole.
    ///
    /// This is the end the crime systems have been building toward since
    /// permits shipped — crime raises bounty, bounty crosses the threshold,
    /// the warrant sends deputies, and the deputies close the loop.
    fn make_the_arrest(active: &mut Active) {
        let owed = active.permits.borrow().bounty;
        let paid = owed.min(active.wallet.credits());
        // Whatever they can pay, they pay; the refusal case cannot happen
        // because `paid` is capped at the balance a line above.
        let settled = active.wallet.spend(paid);
        debug_assert!(settled, "the arrest charged more than the wallet held");
        active.permits.borrow_mut().bounty = 0;
        // The sheet is clear, so every town's paperwork goes with it — a
        // warrant that survived the arrest it was served for would be a
        // sentence nobody ever finishes.
        active.warrants.clear_all();
        active.posse.stand_down();
        // Whatever was in the dark loses interest too: you are a long way
        // up and a long way off, and the deep does not follow anybody home.
        active.dark.stand_down();
        active.health.revive();

        // Put down at the homestead, which is where you wake up.
        let home = vx_world::town::home_site();
        let ground = active
            .world
            .surface_y(home.centre.0, home.centre.1)
            .map_or(active.player.position.y, |top| (top + 1) as f64);
        active.player.position =
            glam::DVec3::new(home.centre.0 as f64 + 0.5, ground, home.centre.1 as f64 + 0.5);
        active.player.velocity = glam::DVec3::ZERO;
        active.resuming = true;

        let line = if paid < owed {
            format!("ARRESTED. {paid} PAID, THE REST WORKED OFF")
        } else {
            format!("ARRESTED. {paid} CREDITS TAKEN")
        };
        active.terminal.say(terminal::Kind::Warn, line.clone());
        active.greeting = Some((line, Instant::now()));
    }

    /// The tree under the drill bit, and the face being cut, if this block is
    /// a stump rather than a block of somebody's trunk.
    ///
    /// Cut low — inside the stump band — and you are felling. Cut higher up
    /// and the drill does what it has always done, which is the rule a feller
    /// learns first and the one this teaches by doing.
    fn stump_under_the_bit(
        active: &Active,
        hit: &vx_world::raycast::RayHit,
    ) -> Option<(vx_world::flora::Tree, usize)> {
        let name = &active.world.registry().get_or_air(hit.id).name;
        if !name.ends_with("log") {
            return None;
        }
        let sites = active.world.towns_near(
            (hit.block.x, hit.block.z),
            felling::TOWN_REACH,
        );
        let tree = felling::standing_tree(&active.world, hit.block, &sites)?;
        if !felling::is_stump(&tree, hit.block) {
            return None;
        }
        let face = vx_core::Face::ALL
            .iter()
            .position(|other| *other == hit.face)?;
        // Cutting the top or the bottom of a stump is not a notch. A feller
        // stands beside a tree, not on it.
        (!(2..4).contains(&face)).then_some((tree, face))
    }

    /// Put the tree over: record the cut, and start the stem falling.
    fn fell_the_tree(
        active: &mut Active,
        tree: &vx_world::flora::Tree,
        stump: vx_core::BlockPos,
        face: usize,
    ) {
        // Felling somebody's tree is felling somebody's property.
        if let Some(line) = Self::charge_for_refusal(active, stump, permits::BOUNTY_PRYING) {
            active.greeting = Some((line, Instant::now()));
        }
        let lean = felling::lean_at(&active.world, tree.base.x, tree.base.z);
        let (direction, chair) = felling::aim(face, lean);
        active.journal.record(Command::Fell {
            at: stump,
            face: face as u8,
        });
        active
            .falls
            .push(felling::start(&mut active.world, tree, direction, chair));
        // The stump goes in the ledger, same as a burn does: what took the
        // trees off a cell does not change how the cell comes back.
        let tick = active.journal.tick();
        active.stands.disturb(tree.base, tick);
        let line = if chair {
            "IT SPLIT - SHE'S GOING WHERE SHE'S HEAVY".to_string()
        } else {
            "TIMBER".to_string()
        };
        active.terminal.say(terminal::Kind::Warn, line.clone());
        active.greeting = Some((line, Instant::now()));
        active.audio.play(audio::Cue::Thud, 0.7);
    }

    /// Step every stem on its way down, and spend what the sweeps say.
    ///
    /// Shaped exactly like [`Self::advance_gunfire`], and for the same
    /// reason: the world edits already happened inside `advance_falls`, so
    /// what is left here is the live-only half — who was standing under it,
    /// and how loud it was.
    fn advance_felling(active: &mut Active, ticks: u32) {
        if active.falls.is_empty() {
            return;
        }
        for _ in 0..ticks {
            if active.falls.is_empty() {
                break;
            }
            let sweeps = felling::advance_falls(&mut active.falls, &mut active.world);
            for sweep in &sweeps {
                // A stem sweeping through somebody is a line passing through
                // them, which is the test the posse and the garrisons already
                // answer — they simply do not care what drew the line.
                let _ = active.posse.under_fire(sweep.from, sweep.to);
                let _ = active.garrisons.under_fire(sweep.from, sweep.to);
                let _ = active.dark.under_fire(sweep.from, sweep.to);

                // And you — tested as the box you are, with the same
                // segment-against-box the arsenal already uses for rounds
                // going past. The energy span from a sapling to an
                // old-growth giant is a thousandfold; the hits are the
                // compressed version of it, and the ordering is the design.
                let body = active.player.position + glam::DVec3::Y * 0.9;
                if arsenal::segment_hits_box(
                    sweep.from,
                    sweep.to,
                    body,
                    glam::DVec3::new(0.5, 1.0, 0.5),
                ) {
                    let hits = (sweep.energy / 120_000.0).clamp(1.0, 6.0) as u8;
                    if active.health.take(hits) {
                        let line = "A TREE PUT YOU DOWN".to_string();
                        active.terminal.say(terminal::Kind::Warn, line.clone());
                        active.greeting = Some((line, Instant::now()));
                    } else {
                        let line = format!("THE TRUNK CAUGHT YOU - {hits} HIT");
                        active.terminal.say(terminal::Kind::Warn, line.clone());
                        active.greeting = Some((line, Instant::now()));
                    }
                }

                let Some(landing) = sweep.landing else { continue };
                // The loudest thing in the woods. Everything that listens,
                // hears it — including whatever is out in the dark.
                active.villagers.startled(landing.at);
                active.garrisons.hear(landing.at);
                active.dark.hear(landing.at, 6.0);
                let heard = (landing.at - active.player.eye_position()).length() as f32;
                let volume = (1.0 - heard / 90.0).clamp(0.1, 1.0);
                active.audio.play(audio::Cue::Thud, volume);
                let line = if landing.hung_up {
                    "IT'S HUNG UP".to_string()
                } else if landing.chained > 0 {
                    format!("DOWN - AND IT TOOK {} WITH IT", landing.chained)
                } else {
                    format!("DOWN - {} LOGS", landing.logs)
                };
                active.terminal.say(terminal::Kind::Note, line.clone());
                active.greeting = Some((line, Instant::now()));
            }
        }
    }

    /// Let the water move, on the same clock the rest of the ground moves on.
    ///
    /// The whole of the simulation is in `journal::settle_water`, which the
    /// replay calls too. What is left here is the live-only half: the noise a
    /// flood makes, and telling you it has stopped.
    fn advance_water(active: &mut Active, ticks: u32) {
        // The pumps run first: what they lift is what the water then has to
        // carry away, and both sides run them in that order.
        for _ in 0..ticks {
            active.pump_step = active.pump_step.wrapping_add(1);
            if active.pumps.is_empty() {
                break;
            }
            journal::run_pumps(
                &active.pumps.clone(),
                &mut active.water,
                &mut active.world,
                active.pump_step,
            );
        }
        if active.water.is_empty() {
            return;
        }
        let before = active.water.len();
        let mut moved = 0;
        for _ in 0..ticks {
            if active.water.is_empty() {
                break;
            }
            moved += journal::settle_water(&mut active.water, &mut active.world);
        }
        if moved > 0 {
            // Running water is a noise, and a noise is a thing that carries.
            let heard = match active.water.first().map(|body| body.origin) {
                Some(at) => glam::Vec3::new(at.x as f32, at.y as f32, at.z as f32)
                    .distance(active.player.eye_position().as_vec3()),
                None => 0.0,
            };
            let volume = (1.0 - heard / 40.0).clamp(0.0, 0.5);
            if volume > 0.05 && ticks > 0 {
                active.audio.play(audio::Cue::Thud, volume * 0.4);
            }
        }
        if before > 0 && active.water.is_empty() {
            let line = "THE WATER'S FOUND ITS LEVEL".to_string();
            active.terminal.say(terminal::Kind::Note, line.clone());
            active.greeting = Some((line, Instant::now()));
        }
    }

    /// The sky, the lightning, the fire and the forest coming back.
    ///
    /// The simulation is `journal::burn_and_grow`, which the replay calls
    /// too; what is left here is the live-only half — the thunder, the rain
    /// wetting the ground, and the lines that tell you the woods are alight.
    fn advance_weather(active: &mut Active, ticks: u32) {
        let seed = active.world.seed();
        let standing = active.player.position;
        let at = vx_core::BlockPos::new(
            standing.x.floor() as i32,
            standing.y.floor() as i32,
            standing.z.floor() as i32,
        );
        for step in 0..ticks {
            let tick = active.journal.tick().saturating_sub(u64::from(ticks - 1 - step));
            let before = active.fires.len();
            let frost = journal::burn_and_grow(
                &mut active.fires,
                &mut active.stands,
                &mut active.world,
                tick,
                standing,
            );
            // The frost's live half: water that thawed is woken so the
            // automaton looks at it again, and the news is said once on the
            // same beat the fire line uses.
            for melted in &frost.melted_at {
                journal::wake_water(&mut active.water, &mut active.world, *melted);
            }
            if tick.is_multiple_of(64 * 3) {
                if let Some(line) = frost.line() {
                    active.terminal.say(terminal::Kind::Note, line);
                    active.greeting = Some((line.to_string(), Instant::now()));
                }
            }
            // A strike that lights something is the loudest thing in the
            // country, and everything that listens hears it.
            if active.fires.len() > before {
                let line = "LIGHTNING - SOMETHING'S ALIGHT".to_string();
                active.terminal.say(terminal::Kind::Warn, line.clone());
                active.greeting = Some((line, Instant::now()));
                active.audio.play(audio::Cue::Boom, 0.9);
                if let Some(fire) = active.fires.last() {
                    let struck = fire.origin;
                    let spot = glam::DVec3::new(
                        f64::from(struck.x),
                        f64::from(struck.y),
                        f64::from(struck.z),
                    );
                    active.villagers.startled(spot);
                    active.garrisons.hear(spot);
                    active.dark.hear(spot, 10.0);
                }
            }
        }

        // Rain wets the ground: a few cells on an exposed column near you,
        // handed to the automaton stage 37 already built. It fills the
        // hollows, runs off downhill, and drains away after.
        let sky = vx_world::weather::at(seed, active.journal.tick(), at.x, at.z);
        if sky.state.wet() && active.journal.tick().is_multiple_of(64) {
            let spread = 10;
            let hash = vx_world::seed::finalise(
                seed ^ active.journal.tick().wrapping_mul(0x9e37_79b9_7f4a_7c15),
            );
            let pick = vx_world::seed::unit(hash);
            // Two finalised draws: `unit` reads a hash's top bits, and a
            // hash merely xored with a small constant kept every shower on
            // the diagonal through the player.
            let (dx, dz) = (
                ((pick * 2.0 - 1.0) * spread as f32) as i32,
                ((vx_world::seed::unit(vx_world::seed::finalise(hash ^ 0x51)) * 2.0 - 1.0)
                    * spread as f32) as i32,
            );
            let (x, z) = (at.x + dx, at.z + dz);
            if let (Some(ground), Some(water)) = (
                active.world.surface_y(x, z),
                active.world.registry().id_of("engine:water"),
            ) {
                let onto = vx_core::BlockPos::new(x, ground + 1, z);
                let held = vx_world::fluid::level_at(&active.world, water, onto);
                let fall = (sky.rain * 12.0) as u32;
                if held + fall <= vx_world::fluid::FULL {
                    vx_world::fluid::set_level(&mut active.world, water, onto, held + fall);
                    journal::wake_water(&mut active.water, &mut active.world, onto);
                }
            }
        }

        // What the fires are doing, said once rather than every tick.
        if !active.fires.is_empty() && active.journal.tick().is_multiple_of(64 * 3) {
            let eaten: u32 = active.fires.iter().map(|fire| fire.eaten).sum();
            let alight: usize = active.fires.iter().map(|fire| fire.burning().len()).sum();
            let line = format!("FIRE: {alight} ALIGHT, {eaten} GONE");
            active.terminal.say(terminal::Kind::Warn, line.clone());
            active.greeting = Some((line, Instant::now()));
        }
    }

    fn advance_gunfire(active: &mut Active, ticks: u32) {
        active.arsenal.cool(ticks);
        if active.shots.is_empty() {
            return;
        }
        let now = active.journal.tick();
        for step in 0..ticks {
            if active.shots.is_empty() {
                break;
            }
            let sweeps = arsenal::advance_shots(
                &mut active.shots,
                &mut active.world,
                &active.movement.tuning,
            );
            let tick_at = now.saturating_sub(u64::from(ticks - 1 - step));
            for sweep in &sweeps {
                Self::settle_caravan_hits(active, sweep, tick_at);
                // The other half of the fight: a round of yours going past
                // hits, suppresses or merely rattles whoever it went past.
                let answer = active.posse.under_fire(sweep.from, sweep.to);
                for line in answer.barks {
                    active.terminal.say(terminal::Kind::Warn, line.clone());
                    active.greeting = Some((line, Instant::now()));
                }
                // And whatever is out in the dark, which does not bark
                // back — it either wears the round or it does not.
                if active.dark.under_fire(sweep.from, sweep.to) {
                    let line = match active.dark.present().map(|it| it.wounds()) {
                        Some((taken, of)) => format!("YOU HIT IT - {taken}/{of}"),
                        None => "YOU HIT IT".to_string(),
                    };
                    active.terminal.say(terminal::Kind::Warn, line.clone());
                    active.greeting = Some((line, Instant::now()));
                }
                let held = active.garrisons.under_fire(sweep.from, sweep.to);
                Self::note_the_deeds(active, held.downed, held.cleared);
                for line in held.barks {
                    active.terminal.say(terminal::Kind::Warn, line.clone());
                    active.greeting = Some((line, Instant::now()));
                }
                let Some(impact) = &sweep.hit else { continue };
                active.villagers.startled(impact.at);
                // A shot landing is the loudest thing in held country, and
                // the shelters hear it as a zone, never a spot.
                active.garrisons.hear(impact.at);
                active.dark.hear(impact.at, 4.0);
                let heard = (impact.at - active.player.eye_position()).length() as f32;
                let volume = (1.0 - heard / 80.0).clamp(0.1, 1.0);
                active.audio.play(audio::Cue::Thud, volume);
                if impact.broke {
                    Self::bill_property_damage(active, impact);
                }
            }
        }
    }

    /// A broken block on somebody's claim, priced by the agreed curve: the
    /// damage, plus half again per witness beyond the first. Nobody saw,
    /// nobody pays — seen is the rule for gunfire like everything else.
    fn bill_property_damage(active: &mut Active, impact: &arsenal::Impact) {
        let claim = active.permits.borrow().claim_here(impact.block);
        let Some(claim) = claim else { return };
        if active.permits.borrow().may_edit(&claim) {
            return;
        }
        let around = awareness::Surroundings {
            player_eye: active.player.eye_height as f32,
            world: Some(&active.world),
            player: Some(active.player.position),
            machines: &[],
        };
        let mut seen = active.villagers.witnesses(&around);
        // The eye overhead counts, once it has made itself known: the first
        // sighting is the warning, everything after it is a statement.
        let overhead = active.roost.as_ref().is_some_and(|roost| {
            roost.observed
                && roost.sees(&active.world, active.player.eye_position())
        });
        seen += usize::from(overhead);
        let bill = arsenal::witnessed_bounty(arsenal::BOUNTY_PROPERTY, seen);
        if bill > 0 {
            active.permits.borrow_mut().caught(bill, seen);
            active.greeting = Some((
                format!("THAT WAS {}. {} SAW IT", claim.label, seen),
                Instant::now(),
            ));
        }
    }

    /// Did this sweep pass through a caravan? Down it: the load falls where
    /// it was hit, and the network logs the loss against you — the manifest
    /// is its own witness, no eyes required.
    fn settle_caravan_hits(active: &mut Active, sweep: &arsenal::Sweep, tick_at: u64) {
        let hull = glam::DVec3::new(2.0, 1.4, 2.0);
        let downed = active
            .economy
            .shipments()
            .iter()
            .position(|load| {
                let (x, z) = load.position_at(tick_at);
                let ground = active
                    .world
                    .generator()
                    .height_at(x.floor() as i32, z.floor() as i32);
                let centre = glam::DVec3::new(x, f64::from(ground) + f64::from(CARAVAN_ALTITUDE), z);
                arsenal::segment_hits_box(sweep.from, sweep.to, centre, hull)
            });
        let Some(index) = downed else { return };
        let Some(load) = active.economy.intercept(index) else {
            return;
        };
        let (x, z) = load.position_at(tick_at);
        let amount = load.amount.round() as u64;
        active.arsenal.crashes.push(arsenal::Crash {
            x,
            z,
            good: load.good,
            amount,
        });
        active.audio.play(audio::Cue::DistantBoom, 1.0);
        match load.owner {
            economy::Owner::Player | economy::Owner::Mail => {
                // Your own delivery. The goods are on the ground and the
                // payment you were owed went down with the airframe.
                active.greeting = Some((
                    "THAT WAS YOUR OWN DELIVERY. IT IS ON THE GROUND NOW".into(),
                    Instant::now(),
                ));
            }
            economy::Owner::Town => {
                active.permits.borrow_mut().billed(amount / 2);
                active.greeting = Some((
                    format!("CARAVAN DOWN, {amount} CARGO. THE NETWORK LOGS THE LOSS"),
                    Instant::now(),
                ));
            }
        }
    }

    /// Enter on the fabricator: put the highlighted pattern on the bed.
    ///
    /// The materials leave the pile here, and the order goes in the log —
    /// which is what lets a replay spend the same materials at the same
    /// tick and finish holding the same pile.
    fn start_print(&mut self) {
        let Some(active) = &mut self.active else { return };
        let level = active.skills.level(skills::FABRICATION);
        let index = active.printer.cursor;
        // Optics are one-per-person, like the machines the counter refuses to
        // double-sell: refuse before the journal ever hears about it.
        if let Some(printer::Output::Optic(name)) =
            printer::recipe(index).map(|recipe| recipe.output)
        {
            if active.optics.owned.contains(name) {
                active.printer.feedback = Some("YOU ALREADY OWN ONE".into());
                return;
            }
        }
        // And one cartridge is one cartridge, for the same reason.
        if matches!(
            printer::recipe(index).map(|recipe| recipe.output),
            Some(printer::Output::Cartridge)
        ) && active.arcade.owned
        {
            active.printer.feedback = Some("YOU ALREADY OWN ONE".into());
            return;
        }
        // A charter is filed before another is printed: one town at a time
        // is a decision, a drawer full of them is a land grab.
        if matches!(
            printer::recipe(index).map(|recipe| recipe.output),
            Some(printer::Output::Charter)
        ) && active.charters.unfiled > 0
        {
            active.printer.feedback = Some("FILE THE CHARTER YOU HOLD FIRST".into());
            return;
        }
        let Some(base) = active.mining.fleet.base.as_mut() else {
            active.printer.feedback = Some("NO BASE PILE TO DRAW ON".into());
            return;
        };
        match active.printer.begin(index, &mut base.stockpile, level, &active.wallet) {
            Ok(()) => {
                active.journal.record(Command::Print {
                    recipe: index as u32,
                });
                let label = printer::recipe(index).map_or("", |recipe| recipe.label);
                active.printer.feedback = Some(format!("PRINTING {label}"));
            }
            Err(reason) => active.printer.feedback = Some(reason),
        }
    }

    /// Run whatever was typed at the terminal.
    ///
    /// Questions are answered here, where the live state is; orders are
    /// handed to the very same calls the keys use, so the journal only ever
    /// sees one kind of dispatch. A terminal that recorded its own orders
    /// would be a second implementation of every rule in this game.
    fn run_typed_command(&mut self) {
        let parsed = match &mut self.active {
            Some(active) => active.terminal.submit(),
            None => return,
        };
        match parsed {
            terminal::Parsed::Empty | terminal::Parsed::Say(_) | terminal::Parsed::Refuse(_) => {}
            terminal::Parsed::Ask(verb, args) => {
                let answer = self.answer(&verb, &args);
                if let Some(active) = &mut self.active {
                    for line in answer {
                        active.terminal.say(terminal::Kind::Note, line);
                    }
                }
            }
            terminal::Parsed::Run(order) => self.carry_out(order),
        }
    }

    /// Answer a question about the running world.
    fn answer(&mut self, verb: &str, args: &[String]) -> Vec<String> {
        let Some(active) = &mut self.active else {
            return Vec::new();
        };
        match verb {
            "status" => {
                let permits = active.permits.borrow();
                let mut lines = vec![
                    format!(
                        "MINING {}  PROSPECTING {}  FABRICATION {}",
                        active.skills.level(skills::MINING),
                        active.skills.level(skills::PROSPECTING),
                        active.skills.level(skills::FABRICATION)
                    ),
                    format!(
                        "CREDITS {}   BOUNTY {}   {}",
                        active.wallet.credits(),
                        permits.bounty,
                        {
                            let (hour, minute) = active.clock.hhmm();
                            format!("{hour:02}:{minute:02}")
                        }
                    ),
                ];
                let spare = active
                    .mining
                    .fleet
                    .base
                    .as_ref()
                    .map_or(0, |base| base.stockpile.count(fuel::CELL));
                let burners = active.mining.burners();
                if let Some(line) = active.mining.tank.readout(burners, spare) {
                    lines.push(format!("{line}   MACHINES BURNING {burners}"));
                }
                lines
            }
            "fleet" => {
                let rows = active.mining.roster(active.player.position);
                if rows.is_empty() {
                    return vec!["NO MACHINES. BUY ONE AT A COUNTER.".into()];
                }
                rows.into_iter()
                    .map(|row| {
                        format!(
                            "{:<12} {:>5}M  {:<9} {}",
                            row.name,
                            row.distance as i32,
                            row.state,
                            row.condition.name()
                        )
                    })
                    .collect()
            }
            "where" => {
                let at = active.player.position;
                let mut lines = vec![format!(
                    "STANDING AT {} {} {}",
                    at.x.floor() as i32,
                    at.y.floor() as i32,
                    at.z.floor() as i32
                )];
                match active
                    .world
                    .towns_near((at.x.floor() as i32, at.z.floor() as i32), 900)
                    .into_iter()
                    .next()
                {
                    Some(site) => {
                        let here = (at.x.floor() as i32, at.z.floor() as i32);
                        lines.push(format!(
                            "NEAREST {}{} - {}",
                            site.name.head(),
                            site.name.tail(),
                            map::bearing(here, site.centre)
                        ));
                    }
                    None => lines.push("NO TOWN WITHIN NINE HUNDRED METRES".into()),
                }
                lines
            }
            "pile" => match active.mining.fleet.base.as_ref() {
                Some(base) if !base.stockpile.is_empty() => base
                    .stockpile
                    .entries()
                    .map(|(name, count)| {
                        format!("{:<20} {count}", shop::display_name(name))
                    })
                    .collect(),
                Some(_) => vec!["THE PILE IS EMPTY".into()],
                None => vec!["NO BASE PILE. PLACE A CONTAINER.".into()],
            },
            "bank" => {
                let at = active.player.position;
                let Some(site) = active
                    .world
                    .towns_near((at.x.floor() as i32, at.z.floor() as i32), 900)
                    .into_iter()
                    .next()
                else {
                    return vec!["NO TOWN WITHIN NINE HUNDRED METRES".into()];
                };
                let held = active.banks.stored(site.centre);
                let mut lines = vec![format!(
                    "{}{} HOLDS {held} OF {}",
                    site.name.head(),
                    site.name.tail(),
                    bank::CAPACITY
                )];
                if let Some(vault) = active.banks.vault(site.centre) {
                    for (name, count) in vault.entries() {
                        lines.push(format!("{:<20} {count}", shop::display_name(name)));
                    }
                }
                lines
            }
            "standing" => {
                let name = &active.reputation;
                vec![
                    format!(
                        "THE TOWNS     {:<8} {:>5}  SHADE {:+}PC AT THE COUNTER",
                        name.compact().name(),
                        name.compact_points(),
                        reputation::price_shade(name.compact())
                    ),
                    format!(
                        "THE SHELTERS  {:<8} {:>5}  {}",
                        name.holdouts().name(),
                        name.holdouts_points(),
                        if name.holdouts() >= reputation::Standing::Neutral {
                            "THEY CHALLENGE BEFORE THEY SHOOT"
                        } else {
                            "SHOT ON SIGHT, AND THEY JAM YOUR SCOUT"
                        }
                    ),
                ]
            }
            "law" => {
                let mut lines = if active.posse.called_out() {
                    active.posse.roll_call()
                } else {
                    let bounty = active.permits.borrow().bounty;
                    vec![format!(
                        "NO WARRANT OUT. BOUNTY {bounty} OF {}",
                        permits::WARRANT_THRESHOLD
                    )]
                };
                for squad in &active.garrisons.squads {
                    lines.push(format!(
                        "A HELD SHELTER NEARBY - {} STANDING, {} HUNTING",
                        squad.active(),
                        squad
                            .holders
                            .iter()
                            .filter(|holder| holder.active()
                                && holder.mode != hostile::Mode::Patrol)
                            .count()
                    ));
                }
                lines
            }
            "repair" => self.mend(args),
            "kit" => {
                // The character sheet the game never had: what is fitted,
                // what it does, and what the next mark costs at a counter.
                let mut lines = vec![format!("CREDITS {}", active.wallet.credits())];
                for line in wallet::LINES {
                    let level = active.wallet.upgrade(line);
                    let next = if level >= wallet::MAX_UPGRADE {
                        "FULL".to_string()
                    } else {
                        format!("{}C", shop::upgrade_cost(level + 1))
                    };
                    lines.push(format!(
                        "{:<7} {}/{}  {:<6} {}",
                        line.to_uppercase(),
                        level,
                        wallet::MAX_UPGRADE,
                        next,
                        wallet::describes(line)
                    ));
                }
                lines
            }
            "patch" => {
                // The field half of the ward: a medkit is what gets you out
                // of a gallery, and the cot in town is what makes you whole.
                match active.health.patch() {
                    Ok(back) => vec![
                        format!("PATCHED - {back} BACK"),
                        format!(
                            "HITS {}/{}   {} MEDKITS LEFT",
                            active.health.hits(),
                            health::MAX_HITS,
                            active.health.medkits()
                        ),
                    ],
                    Err(reason) => vec![reason, "THE CLINIC IN ANY TOWN SELLS THEM".into()],
                }
            }
            "wells" => {
                // The roster the wellhead panel cannot give you: every hole
                // at once, from wherever you happen to be standing.
                let holes = active.mining.wells.all();
                if active.mining.wells.is_empty() {
                    vec!["NO HOLES SUNK".to_string()]
                } else {
                    let mut lines = vec![format!("{} HOLES", holes.len())];
                    for hole in holes {
                        let what = match (hole.stage, hole.fluid) {
                            (well::Stage::Drilling { .. }, _) => format!(
                                "DRILLING {}%",
                                (hole.drilled() * 100.0).round() as u32
                            ),
                            (well::Stage::Pumping, Some(fluid)) => {
                                format!("{} {} LEFT", fluid.name(), hole.remaining)
                            }
                            _ => "DRY".to_string(),
                        };
                        lines.push(format!(
                            "{:<14} {:<18} LIFTED {}",
                            format!("{} {}", hole.at.x, hole.at.z),
                            what,
                            hole.lifted
                        ));
                    }
                    lines
                }
            }
            "weather" => {
                // The whole stage-38 chain, said in five lines: what the sky
                // is doing, which way it is blowing, how dry the fuel is,
                // what is alight, and whether the ground you stand on is
                // still coming back from the last time something took it.
                let at = active.player.position;
                let column = vx_core::BlockPos::new(
                    at.x.floor() as i32,
                    at.y.floor() as i32,
                    at.z.floor() as i32,
                );
                let seed = active.world.seed();
                let tick = active.journal.tick();
                let sky = vx_world::weather::at(seed, tick, column.x, column.z);
                // Which way it is blowing, in the same eight points the
                // beacon map names a town in.
                let bearing = if sky.wind_speed() < 0.5 {
                    "CALM".to_string()
                } else {
                    let reach = 1_000.0 / sky.wind_speed().max(0.1);
                    map::bearing(
                        (0, 0),
                        (
                            (sky.wind.0 * reach) as i32,
                            (sky.wind.1 * reach) as i32,
                        ),
                    )
                    .split_whitespace()
                    .next()
                    .unwrap_or("CALM")
                    .to_string()
                };
                let dryness = vx_world::weather::fuel_moisture(seed, tick, column.x, column.z);
                let season = vx_world::season::Season::of(tick);
                let mut lines = vec![
                    // The month first, because everything under it is a
                    // reading of the month.
                    format!(
                        "SEASON    {} - DAY {} OF {}{}",
                        season.label(),
                        vx_world::season::day_of_year(tick) + 1,
                        vx_world::season::YEAR_DAYS,
                        if vx_world::season::fire_season(tick) {
                            "  - FIRE SEASON"
                        } else {
                            ""
                        }
                    ),
                    format!("SKY       {}", sky.sky_word()),
                    format!(
                        "AIR       {}",
                        if sky.freezing() { "FREEZING" } else { "ABOVE FREEZING" }
                    ),
                    format!(
                        "WIND      {} AT {:.0} M/S",
                        bearing,
                        sky.wind_speed()
                    ),
                    format!(
                        "FUEL      {:.0}% DRY{}",
                        dryness * 100.0,
                        if dryness > 0.75 { "  - IT WILL LIGHT" } else { "" }
                    ),
                ];
                if active.fires.is_empty() {
                    lines.push("FIRE      NOTHING BURNING".to_string());
                } else {
                    let alight: usize =
                        active.fires.iter().map(|fire| fire.burning().len()).sum();
                    let eaten: u32 = active.fires.iter().map(|fire| fire.eaten).sum();
                    lines.push(format!(
                        "FIRE      {} FRONT{} - {alight} ALIGHT, {eaten} GONE",
                        active.fires.len(),
                        if active.fires.len() == 1 { "" } else { "S" }
                    ));
                }
                let generator = active.world.generator().clone();
                let natural = |x: i32, z: i32| generator.natural_height_at(x, z);
                let biome = vx_world::forest::biome_at(seed, column.x, column.z, &natural);
                let species = vx_world::flora::Species::of(biome);
                let cell = succession::Ledger::cell_of(column);
                match active.stands.stand(cell) {
                    Some(_) => {
                        let stage = active.stands.due(cell, tick, species) as usize;
                        lines.push(format!(
                            "STAND     COMING BACK - {}",
                            succession::LABELS[stage.min(succession::LABELS.len() - 1)]
                        ));
                    }
                    None => lines.push("STAND     UNTOUCHED".to_string()),
                }
                if !active.stands.is_empty() {
                    lines.push(format!(
                        "LEDGER    {} DISTURBED STAND{}",
                        active.stands.len(),
                        if active.stands.len() == 1 { "" } else { "S" }
                    ));
                }
                lines
            }
            "who" => {
                let site = *active.villagers.site();
                let day = (active.journal.tick() / schedule::TICKS_PER_DAY) as u32;
                let mut lines = vec![format!(
                    "THE PEOPLE OF {}{}",
                    site.name.head(),
                    site.name.tail()
                )];
                for (index, person) in people::roster(&site).iter().enumerate() {
                    let key = (site.centre, index as u8);
                    let place = schedule::where_is(&site, index, day, active.clock, false);
                    // The office and the trust are the civic round's columns:
                    // who is somebody here, and how far they will go with you
                    // on business rather than on kindness.
                    let seat = office::office_of(&site, index)
                        .map_or("", |office| office::title(office));
                    lines.push(format!(
                        "{:<22} {:<8} {:<8} {:<11} TRUST {:<5} {}",
                        person.name,
                        seat,
                        person.temperament.archetype.name(),
                        active.friends.tier(key).name(),
                        active.friends.trust(key),
                        place.name()
                    ));
                }
                lines
            }
            "found" => Self::found_a_town(active, args),
            "take" => Self::take_the_town(active),
            "town" => {
                // The whole civic layer in one screen: who runs it, what the
                // people who live here are worth, and where the paperwork on
                // you has got to.
                let site = *active.villagers.site();
                let tick = active.journal.tick();
                let day = (tick / schedule::TICKS_PER_DAY) as u32;
                let mut lines = vec![
                    format!(
                        "{}{} - {} AT {} {}",
                        site.name.head(),
                        site.name.tail(),
                        site.speciality.name(),
                        site.centre.0,
                        site.centre.1
                    ),
                ];
                if let Some(line) = charter_line(&site, &active.charters, &active.elections) {
                    lines.push(line);
                }
                for office in office::OFFICES {
                    match active.elections.seated(&site, office) {
                        ballot::Candidate::Player => lines.push(format!(
                            "{:<8} {:<22} SINCE TERM {}",
                            office::title(office),
                            "YOU",
                            active.elections.since(site.centre, office).unwrap_or(0)
                        )),
                        ballot::Candidate::Resident(index) => {
                            let person = people::person(&site, index);
                            let key = (site.centre, index as u8);
                            lines.push(format!(
                                "{:<8} {:<22} {:<11} TRUST {}",
                                office::title(office),
                                person.name,
                                active.friends.tier(key).name(),
                                active.friends.trust(key)
                            ));
                        }
                    }
                }
                let poll_in = ballot::next_poll(&site, day).saturating_sub(day);
                lines.push(match poll_in {
                    0 => "BALLOT   POLLING TODAY".to_string(),
                    1 => "BALLOT   POLLS TOMORROW".to_string(),
                    days => format!("BALLOT   POLLS IN {days} DAYS"),
                });
                let standing: Vec<&str> = office::OFFICES
                    .into_iter()
                    .filter(|office| active.elections.is_standing(site.centre, *office))
                    .map(office::title)
                    .collect();
                if !standing.is_empty() {
                    lines.push(format!("STANDING {}", standing.join(" AND ")));
                }
                // What doors that actually opens, read off the permits set
                // rather than the register: the badge is the thing the locks
                // answer to, and it belongs to the town that issued it.
                let badges: Vec<String> = active
                    .permits
                    .borrow()
                    .badges()
                    .map(|(town, office)| {
                        if town == site.centre {
                            office::title(office).to_string()
                        } else {
                            format!("{} AT {} {}", office::title(office), town.0, town.1)
                        }
                    })
                    .collect();
                if !badges.is_empty() {
                    lines.push(format!("BADGES   {}", badges.join(", ")));
                }
                lines.push(format!(
                    "MARKET DAY IS {}",
                    ["ONE", "TWO", "THREE", "FOUR", "FIVE", "SIX", "SEVEN"]
                        [schedule::market_weekday(&site) as usize % 7]
                ));
                for (index, person) in people::roster(&site).iter().enumerate() {
                    let place = schedule::where_is(&site, index, day, active.clock, false);
                    lines.push(format!(
                        "{:<22} {:<10} {:<11} {} CR",
                        person.name,
                        person.trade,
                        place.name(),
                        economy::purse(&site, index, tick)
                    ));
                }
                // And what the rest of the frontier has out on you, because
                // walking to the next town is the obvious move and finding
                // out there that its counter is shut too should not be a
                // surprise.
                let elsewhere: Vec<String> = active
                    .warrants
                    .iter()
                    .filter(|(town, _)| *town != site.centre)
                    .map(|(town, paper)| {
                        format!("ALSO {} AT {} {}", paper.stage.name(), town.0, town.1)
                    })
                    .collect();
                let bounty = active.permits.borrow().bounty;
                match active.warrants.get(site.centre) {
                    Some(paper) => {
                        lines.push(format!(
                            "WARRANT  {} - FILED ON {} CR",
                            paper.stage.name(),
                            paper.at_bounty
                        ));
                        if active.warrants.pending_in(site.centre) {
                            lines.push("THE COUNTER IS CLOSED TO YOU".into());
                        }
                    }
                    None if bounty > 0 => {
                        lines.push(format!(
                            "WARRANT  NONE - BOUNTY {bounty} OF {} NEEDED",
                            permits::WARRANT_THRESHOLD
                        ));
                    }
                    None => lines.push("WARRANT  NOTHING ON YOU HERE".into()),
                }
                if !active.warrants.is_empty() {
                    lines.push(format!(
                        "{} TOWN{} HOLD PAPER ON YOU",
                        active.warrants.len(),
                        if active.warrants.len() == 1 { "" } else { "S" }
                    ));
                }
                lines.extend(elsewhere);
                lines
            }
            "talk" => {
                let lines = self.have_a_word();
                if lines.is_empty() {
                    return vec!["NOBODY CLOSE ENOUGH FOR A WORD".into()];
                }
                lines
            }
            "gift" => self.hand_a_gift(args),
            "scout-perch" => {
                let at = active.player.position;
                let (x, z) = match (args.get(1), args.get(2)) {
                    (Some(x), Some(z)) => match (x.parse::<i32>(), z.parse::<i32>()) {
                        (Ok(x), Ok(z)) => (x, z),
                        _ => return vec!["PERCH WANTS TWO WHOLE NUMBERS".into()],
                    },
                    // Bare `perch` means here, which is what a person means
                    // when they point at the ground they are standing on.
                    _ => (at.x.floor() as i32, at.z.floor() as i32),
                };
                self.order_scout(journal::ScoutOrder::Perch { x, z }, format!("PERCH {x} {z}"));
                vec![format!("PERCHING AT {x} {z}")]
            }
            _ => vec![],
        }
    }

    /// Mend a machine: the named one, or whichever is worst.
    ///
    /// Goes through the journal like every other order that moves the pile,
    /// and through `Wear::repair` like the replay arm does — the live game
    /// and the oracle run one function between them.
    fn mend(&mut self, args: &[String]) -> Vec<String> {
        let Some(active) = &mut self.active else {
            return Vec::new();
        };
        let rows = active.mining.roster(active.player.position);
        let wanted: Option<mining::MachineRef> = if args.is_empty() {
            // No name: the machine holding everyone else up, which is the
            // one the player means nine times in ten.
            rows.iter()
                .filter(|row| row.condition != wear::Condition::Fresh)
                .max_by_key(|row| row.condition)
                .map(|row| row.machine)
        } else {
            let wanted = args.join(" ").to_uppercase();
            rows.iter()
                .find(|row| row.name == wanted)
                .map(|row| row.machine)
        };

        let Some(machine) = wanted else {
            return if args.is_empty() {
                vec!["EVERY MACHINE IS FRESH".into()]
            } else {
                vec![format!("NO SUCH MACHINE: {}", args.join(" ").to_uppercase())]
            };
        };
        let Some(tag) = journal::MachineTag::of(machine) else {
            return vec!["THE KESTREL TAKES NO WEAR".into()];
        };
        let name = rows
            .iter()
            .find(|row| row.machine == machine)
            .map_or_else(String::new, |row| row.name.clone());

        let Some(base) = active.mining.fleet.base.as_mut() else {
            return vec!["NO BASE PILE TO DRAW ON".into()];
        };
        let held = base.stockpile.count(wear::SPARE_PART);
        if held < wear::PARTS_PER_REPAIR {
            return vec![format!(
                "NEEDS {} SPARE PARTS, THE PILE HAS {held}",
                wear::PARTS_PER_REPAIR
            )];
        }
        if !active.mining.wear.repair(machine, &mut base.stockpile) {
            return vec![format!("{name} IS FRESH ALREADY")];
        }
        active.journal.record(Command::Repair { machine: tag });
        vec![format!("{name} MENDED - {} PARTS SPENT", wear::PARTS_PER_REPAIR)]
    }

    /// Metres within which a townsperson can hear you.
    const TALK_RANGE: f32 = 4.0;

    /// The nearest townsperson within speaking range, by roster index.
    fn nearest_person(active: &Active) -> Option<usize> {
        let player = active.player.position;
        active
            .villagers
            .positions()
            .into_iter()
            .enumerate()
            .map(|(index, at)| (index, (at - player).length() as f32))
            .filter(|(_, distance)| *distance <= Self::TALK_RANGE)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// What is true right now, for a townsperson to talk about. Gossip is
    /// telemetry wearing a coat: every field reads a live system.
    fn gather_facts(active: &mut Active, tier: disposition::Tier) -> people::Facts {
        let site = *active.villagers.site();
        let now = active.journal.tick();
        let bunker = (tier >= disposition::Tier::Trusted)
            .then(|| {
                active
                    .world
                    .generator()
                    .bunkers_near(site.centre, 1_200)
                    .into_iter()
                    .next()
                    .map(|shelter| map::bearing(site.centre, shelter.centre))
            })
            .flatten();
        people::Facts {
            town: format!("{}{}", site.name.head(), site.name.tail()),
            ore_price: active.economy.market(&site, now).price(economy::ORE) as u32,
            bounty: active.permits.borrow().bounty,
            fleet_dry: active.mining.tank.dry,
            bunker,
        }
    }

    /// At Close, a key: an authenticate-rung grant to this person's own
    /// door, through the permit system, not around it. Fires once — a
    /// granted key stays granted.
    fn door_opens(
        active: &mut Active,
        site: &vx_world::town::TownSite,
        index: usize,
        person: &people::Person,
    ) -> Option<String> {
        // Two ways in, and they are different relationships. Close is a
        // friend handing you a key. Trust is a supplier deciding you are
        // somebody they do business with — the note's "trust through trade",
        // and the reason a trader who never gave anybody a gift can still
        // end up with the run of a town.
        let key = (site.centre, index as u8);
        let friendly = active.friends.tier(key) >= disposition::Tier::Close;
        let trusted = active.friends.trusted_with_a_key(key);
        if !friendly && !trusted {
            return None;
        }
        let claim = permits::claims_for(site)
            .into_iter()
            .find(|claim| claim.owner == permits::Claimant::Resident(index))?;
        let mut permits = active.permits.borrow_mut();
        if permits.granted(claim.key) {
            return None;
        }
        permits.grant(claim.key);
        let why = if friendly { "TRUSTS YOU WITH" } else { "DOES ENOUGH BUSINESS TO HAND YOU" };
        Some(format!("{} {} A KEY TO {}", person.name, why, claim.label))
    }

    /// Put your name on this town's ballot, or take it off.
    ///
    /// The order goes in the journal because it is a decision the player
    /// made; the election that follows is not recorded, because polling day
    /// and the count are both pure functions of things both sides already
    /// hold.
    /// `FOUND <HEAD> <TAIL>`: file a charter on the ground you stand on.
    ///
    /// The order is the ground — the journal carries it and both sides raise
    /// the same town from it. Everything else here is the bookkeeping that
    /// lives beside the ground: the charter consumed, the chairs, the badge,
    /// the beacon ledger, and you moved onto the plaza you just levelled.
    fn found_a_town(active: &mut Active, words: &[String]) -> Vec<String> {
        if active.charters.unfiled == 0 {
            return vec!["YOU HAVE NO CHARTER TO FILE - PRINT ONE".into()];
        }
        let (head, tail) = match (words.first(), words.get(1)) {
            (Some(head), Some(tail)) => (head.as_str(), tail.as_str()),
            _ => return vec!["FOUND WANTS TWO WORDS OUT OF THE BOOK, LIKE IRON REACH".into()],
        };
        let Some(name) = vx_world::town::TownName::from_words(head, tail) else {
            let (heads, tails) = vx_world::town::name_book();
            return vec![
                format!("NO SUCH NAME. HEADS: {}", heads.join(" ")),
                format!("TAILS: {}", tails.join(" ")),
            ];
        };
        let at = (
            active.player.position.x.floor() as i32,
            active.player.position.z.floor() as i32,
        );
        let site = match charter::may_found(&active.world, at, name) {
            Ok(site) => site,
            Err(refusal) => return vec![refusal.line()],
        };
        let tick = active.journal.tick();
        let day = (tick / schedule::TICKS_PER_DAY) as u32;
        let (head, tail) = name.indices();
        active.journal.record(Command::Found {
            x: at.0,
            z: at.1,
            head,
            tail,
        });
        // The ground, exactly as replay will do it.
        let raised = active.world.raise(site);
        // The bookkeeping beside it.
        active.charters.file(site, tick);
        for office in office::OFFICES {
            active.elections.seat_player(site.centre, office, day);
            active.permits.borrow_mut().take_office(site.centre, office);
        }
        active.ledger.visit(site.centre);
        // Onto the plaza: the plot was levelled under your feet, and standing
        // inside the new ground is nobody's idea of a first morning.
        let plaza = vx_world::town::beacon_position(&site);
        active.player.position =
            glam::DVec3::new(plaza.x as f64 + 2.5, plaza.y as f64, plaza.z as f64 + 2.5);
        active.player.velocity = glam::DVec3::ZERO;
        active.player.on_ground = false;
        let line = format!(
            "FOUNDED {}{}. THREE SETTLERS ARRIVED. YOU ARE MAYOR AND SHERIFF",
            site.name.head(),
            site.name.tail()
        );
        active.greeting = Some((line.clone(), Instant::now()));
        vec![line, format!("{raised} CHUNKS RAISED - THE PLOT IS LEVELLED")]
    }

    /// `TAKE`: seize this town's chairs over its broken deputies.
    ///
    /// Only at the town's own console, and only while its posse is out and
    /// none of it is standing. Recorded because you did it; replayed as
    /// nothing, because a seat is a ledger entry. The price is every ledger
    /// that already exists reading the same fact.
    fn take_the_town(active: &mut Active) -> Vec<String> {
        let town = *active.villagers.site();
        if !active.board.open {
            return vec!["TAKE IT AT THE TOWN'S OWN CONSOLE".into()];
        }
        if office::OFFICES
            .into_iter()
            .all(|office| active.elections.player_holds(town.centre, office))
        {
            return vec!["YOU ALREADY HOLD IT".into()];
        }
        if active.posse.town() != Some(town.centre) {
            return vec!["THIS IS NOT YOUR FIGHT - NO DEPUTIES OF THIS TOWN ARE OUT".into()];
        }
        if !active.posse.broken() {
            return vec![format!(
                "THE DEPUTIES ARE STILL STANDING - {} OF THEM",
                active.posse.active()
            )];
        }
        let tick = active.journal.tick();
        let day = (tick / schedule::TICKS_PER_DAY) as u32;
        active.journal.record(Command::Take { town: town.centre });
        active.elections.seize(town.centre, day);
        for office in office::OFFICES {
            active.permits.borrow_mut().take_office(town.centre, office);
        }
        active.posse.stand_down();
        // The price, read off what exists: the factions, the sheet, the
        // neighbours' paper on the next civic tick, and the people who live
        // here — who trust you exactly nothing from this morning on.
        active.reputation.with_compact(reputation::TAKEN_COMPACT);
        active.reputation.with_holdouts(reputation::TAKEN_HOLDOUTS);
        active.permits.borrow_mut().bounty += permits::TAKEN_BOUNTY;
        active.friends.forfeit(town.centre);
        active.warrants.clear(town.centre);
        let line = format!(
            "{}{} IS YOURS. THE COMPACT WILL REMEMBER IT",
            town.name.head(),
            town.name.tail()
        );
        active.greeting = Some((line.clone(), Instant::now()));
        vec![
            line,
            "BOTH CHAIRS ARE YOURS. THE TOWN VOTES ON YOU AT ITS NEXT POLL".into(),
            "EVERY NEIGHBOUR WILL HAVE PAPER ON YOU BY MORNING".into(),
        ]
    }

    fn put_your_name_in(active: &mut Active, town: &vx_world::town::TownSite) {
        let Some(office) = office::OFFICES.get(active.board.cursor()).copied() else {
            return;
        };
        // Holding a seat and standing for it are the same switch: resigning
        // is taking your name off, and the next poll gives the chair back to
        // whoever the seed always said should have it.
        let holds = active.elections.player_holds(town.centre, office);
        let on = !(active.elections.is_standing(town.centre, office) || holds);
        active.elections.stand(town.centre, office, on);
        active.journal.record(Command::Stand {
            town: town.centre,
            office: match office {
                permits::Office::Mayor => 0,
                permits::Office::Sheriff => 1,
            },
            on,
        });
        let title = office::title(office);
        let line = if on {
            format!("YOUR NAME IS ON THE BALLOT FOR {title}")
        } else if holds {
            format!("YOU WILL STAND DOWN AS {title} AT THE NEXT POLL")
        } else {
            format!("YOUR NAME IS OFF THE BALLOT FOR {title}")
        };
        active.board.feedback = Some(line.clone());
        active.terminal.say(terminal::Kind::Note, line);
    }

    /// Run this town's poll if one is due, and hand out what it decided.
    ///
    /// Every effect is an existing system reading a new answer: the badge is
    /// a permits entry, the standing is a reputation entry, and the seat
    /// itself is the register. Nothing here edits a block, which is why the
    /// stage 39 oracle test still holds with elections running through it.
    fn hold_the_polls(active: &mut Active, town: &vx_world::town::TownSite, tick: u64) {
        let day = (tick / schedule::TICKS_PER_DAY) as u32;
        // Each seat checks its own term inside `hold`, so a market day polls
        // both chairs rather than the first one shutting the second out.
        if !ballot::is_polling_day(town, day) {
            return;
        }
        let bounty = active.permits.borrow().bounty;
        let troubled = active.warrants.pending_in(town.centre);
        let mut results = Vec::new();
        for office in office::OFFICES {
            let field = ballot::Field {
                site: town,
                friends: &active.friends,
                bounty,
                incumbent_troubled: troubled,
                standing: active.elections.is_standing(town.centre, office),
                seat: office,
                founder: active.charters.founded(town.centre),
            };
            if let Some(held) = active.elections.hold(&field, office, day) {
                results.push(held);
            }
        }
        if results.is_empty() {
            return;
        }

        // The badges are re-issued from the register wholesale rather than
        // patched, so a seat lost is a badge handed back without anybody
        // having to remember to take it off you.
        let seats: Vec<((i32, i32), permits::Office)> =
            active.elections.player_seats().collect();
        active.permits.borrow_mut().seat_all(seats.into_iter());

        for held in results {
            let line = match (held.changed, held.after.is_player()) {
                (true, true) => format!(
                    "{} ELECTED YOU {}",
                    town.name.head(),
                    office::title(held.office)
                ),
                (true, false) => format!(
                    "{} LOST THE {} SEAT",
                    if held.before.is_player() { "YOU" } else { "SOMEBODY" },
                    office::title(held.office)
                ),
                (false, true) => format!(
                    "{} RETURNED YOU AS {}",
                    town.name.head(),
                    office::title(held.office)
                ),
                (false, false) => continue,
            };
            active.terminal.say(terminal::Kind::Note, line.clone());
            active.greeting = Some((line, Instant::now()));
            if held.changed && held.after.is_player() {
                // A town that has just elected you has said what it thinks of
                // you out loud, which is the note's tie between offices and
                // the factions.
                let band = active.reputation.with_compact(reputation::ELECTED_COMPACT);
                Self::note_band(active, "THE TOWNS", band);
                let band = active
                    .reputation
                    .with_holdouts(reputation::ELECTED_HOLDOUTS);
                Self::note_band(active, "THE HOLDOUTS", band);
            }
        }
    }

    /// Whose signature a warrant here needs, and whose ledger to read it off.
    ///
    /// Normally the town's own mayor. But **you cannot sign your own
    /// warrant**, so when the seat is yours the sheriff takes it up the road
    /// to the nearest other town's mayor — a different man with his own
    /// archetype and his own opinion of you. A town you run is a haven; it is
    /// not a sanctuary, and `warrant::decides` is reused unchanged with a
    /// different person in the chair.
    fn signing_mayor(
        active: &Active,
        town: &vx_world::town::TownSite,
    ) -> (people::Person, disposition::PersonKey) {
        let mine = active
            .elections
            .seated(town, permits::Office::Mayor)
            .is_player();
        if !mine {
            let index = office::holder(town, permits::Office::Mayor);
            return (people::person(town, index), (town.centre, index as u8));
        }
        // The nearest neighbour whose mayor is not also you. Failing that —
        // a frontier you have run the table on — the town's own sheriff signs
        // it, because somebody has to and he is the one holding the paper.
        let neighbours = active
            .world
            .towns_near(town.centre, beacon::DISCOVERY_RANGE * 40);
        for other in neighbours {
            if other.centre == town.centre {
                continue;
            }
            if active
                .elections
                .seated(&other, permits::Office::Mayor)
                .is_player()
            {
                continue;
            }
            let index = office::holder(&other, permits::Office::Mayor);
            return (people::person(&other, index), (other.centre, index as u8));
        }
        let index = office::holder(town, permits::Office::Sheriff);
        (people::person(town, index), (town.centre, index as u8))
    }

    /// The town's bill for the paperwork.
    ///
    /// Credits first. What the wallet cannot cover is not forgiven — it goes
    /// back onto the bounty, so a fine you cannot pay makes you more wanted,
    /// which is the honest consequence of being broke and in trouble.
    fn pay_the_fine(
        active: &mut Active,
        town: &vx_world::town::TownSite,
        mayor: &people::Person,
        fine: u64,
    ) {
        if fine == 0 {
            return;
        }
        let paid = active.wallet.credits().min(fine);
        let _ = active.wallet.spend(paid);
        let owed = fine - paid;
        if owed > 0 {
            active.permits.borrow_mut().billed(owed);
        }
        let granted = active.warrants.granted_in(town.centre);
        let line = if granted {
            format!("{} SIGNED. FINED {fine} CR", mayor.name)
        } else {
            format!("{} IS THINKING ABOUT IT. FINED {fine} CR", mayor.name)
        };
        active.terminal.say(terminal::Kind::Warn, line.clone());
        active.greeting = Some((line, Instant::now()));
        if owed > 0 {
            let short = format!("YOU COULD NOT PAY. {owed} CR ADDED TO THE BOUNTY");
            active.terminal.say(terminal::Kind::Warn, short.clone());
        }
        // And the market closes while the paperwork stands. Said once, here,
        // rather than checked at the counter and explained nowhere.
        active
            .terminal
            .say(terminal::Kind::Warn, format!("{} HAS CLOSED ITS COUNTER TO YOU", town.name.head()));
    }

    /// Put a sale on somebody's slate.
    ///
    /// Whoever is standing close enough is who you dealt with; failing that,
    /// whoever the schedule has minding the counter. Nothing is recorded in
    /// the journal, because nothing here touches the ground — the pile and
    /// the market moved through paths that are already replayed, and what a
    /// resident thinks of you was never in the hash.
    fn book_the_trade(active: &mut Active, site: &vx_world::town::TownSite, earned: u64) {
        let day = (active.journal.tick() / schedule::TICKS_PER_DAY) as u32;
        let index = Self::nearest_person(active)
            .unwrap_or_else(|| office::at_the_counter(site, day, active.clock));
        let key = (site.centre, index as u8);
        active.friends.trade(key, earned, day);
        let person = people::person(site, index);
        if let Some(line) = Self::door_opens(active, site, index, &person) {
            active.terminal.say(terminal::Kind::Note, line.clone());
            active.greeting = Some((line, Instant::now()));
        }
    }

    /// A word with the nearest townsperson. The ledger takes the
    /// conversation, the journal takes the order, and the person says
    /// something true. Empty when nobody is in earshot.
    fn have_a_word(&mut self) -> Vec<String> {
        let Some(active) = &mut self.active else {
            return Vec::new();
        };
        let Some(index) = Self::nearest_person(active) else {
            return Vec::new();
        };
        let site = *active.villagers.site();
        let day = (active.journal.tick() / schedule::TICKS_PER_DAY) as u32;
        let key = (site.centre, index as u8);
        let person = people::person(&site, index);

        active.friends.talk(key, day);
        active.journal.record(Command::Talk {
            town: site.centre,
            person: index as u8,
        });

        let tier = active.friends.tier(key);
        let facts = Self::gather_facts(active, tier);
        let mut lines = vec![format!(
            "{}: {}",
            person.name,
            people::line_for(&person, tier, &facts)
        )];
        if let Some(granted) = Self::door_opens(active, &site, index, &person) {
            lines.push(granted);
        }
        lines
    }

    /// Hand the nearest townsperson one good off the base pile.
    ///
    /// The good leaves the pile before anyone decides what it was worth —
    /// the journal's `Gift` replays the same unconditional take, so the two
    /// sides cannot disagree about the pile. What it *earned* lives in the
    /// disposition ledger, outside the hash, scored at entry time.
    fn hand_a_gift(&mut self, args: &[String]) -> Vec<String> {
        let Some(active) = &mut self.active else {
            return Vec::new();
        };
        if args.is_empty() {
            return vec!["GIFT WHAT? NAME A GOOD OFF THE PILE".into()];
        }
        let Some(index) = Self::nearest_person(active) else {
            return vec!["NOBODY CLOSE ENOUGH FOR A GIFT".into()];
        };
        let wanted = args.join(" ");
        let Some(base) = active.mining.fleet.base.as_mut() else {
            return vec!["NO BASE PILE. PLACE A CONTAINER.".into()];
        };
        let Some(good) = base
            .stockpile
            .entries()
            .map(|(name, _)| name.to_string())
            .find(|name| {
                let tail = name.rsplit(':').next().unwrap_or(name).replace('_', " ");
                shop::display_name(name).to_ascii_lowercase() == wanted || tail == wanted
            })
        else {
            return vec![format!("NO {} ON THE PILE", wanted.to_ascii_uppercase())];
        };
        if base.stockpile.take(&good, 1) == 0 {
            return vec![format!("NO {} ON THE PILE", wanted.to_ascii_uppercase())];
        }

        let site = *active.villagers.site();
        let day = (active.journal.tick() / schedule::TICKS_PER_DAY) as u32;
        let key = (site.centre, index as u8);
        let person = people::person(&site, index);
        let given = active.friends.gift(key, &person, &good, day);
        let band = active.reputation.with_compact(reputation::GIFT_COMPACT);
        Self::note_band(active, "THE TOWNS", band);
        active.journal.record(Command::Gift {
            town: site.centre,
            person: index as u8,
            good: good.clone(),
        });

        let mut lines = vec![match given {
            disposition::Given::Scored {
                points,
                birthday: true,
            } => format!(
                "{}: MY BIRTHDAY - AND YOU REMEMBERED ({points:+})",
                person.name
            ),
            disposition::Given::Scored { points, .. } if points >= disposition::LOVED => {
                format!("{}: NOW THAT IS A FINE THING ({points:+})", person.name)
            }
            disposition::Given::Scored { points, .. } if points < 0 => {
                format!("{}: ...KEEP IT NEXT TIME ({points:+})", person.name)
            }
            disposition::Given::Scored { points, .. } => {
                format!("{}: MUCH OBLIGED ({points:+})", person.name)
            }
            disposition::Given::Enough => format!(
                "{}: YOU HAVE DONE ENOUGH THIS WEEK. TRULY.",
                person.name
            ),
        }];
        if let Some(granted) = Self::door_opens(active, &site, index, &person) {
            lines.push(granted);
        }
        lines
    }

    /// E with nothing solid in reach: a word with whoever is standing there.
    /// The line lands as a toast; the terminal keeps the transcript.
    fn chat_up_the_street(&mut self) {
        let lines = self.have_a_word();
        let Some(active) = &mut self.active else { return };
        for line in &lines {
            active.terminal.say(terminal::Kind::Note, line.clone());
        }
        if let Some(line) = lines.first() {
            let at = Instant::now();
            active.greeting = Some((line.clone(), at));
            // The transcript lines above already are the log; without this
            // the toast mirror in refresh_hud would enter the first one
            // twice.
            active.logged = Some(at);
        }
    }

    /// Carry out a terminal order through the path the keys already use.
    fn carry_out(&mut self, order: terminal::Order) {
        match order {
            terminal::Order::Close => {
                if let Some(active) = &mut self.active {
                    active.terminal.close();
                    active.renderer.clear_overlay(TERM_SLOT);
                }
            }
            terminal::Order::Dig => self.start_mining(),
            terminal::Order::Cancel => {
                if let Some(active) = &mut self.active {
                    active.mining.cancel(&mut active.world);
                    active.journal.record(Command::Cancel);
                    active.terminal.say(terminal::Kind::Note, "PLAN DROPPED");
                }
            }
            terminal::Order::Survey => {
                if let Some(active) = &mut self.active {
                    let at = active.camera.position;
                    let (x, z) = (at.x.floor() as i32, at.z.floor() as i32);
                    let line = if active.mining.dispatch_scan(x, z) {
                        "FLIER SWEEPING THIS SECTOR"
                    } else {
                        "THE FLIER IS BUSY"
                    };
                    active.terminal.say(terminal::Kind::Note, line);
                }
            }
            terminal::Order::Lights => {
                if let Some(active) = &mut self.active {
                    active.optics.cycle();
                    let line = active.optics.label().unwrap_or("LIGHTS OFF").to_string();
                    active.terminal.say(terminal::Kind::Note, line);
                }
            }
            terminal::Order::Save => {
                self.save_world();
                if let Some(active) = &mut self.active {
                    active.terminal.say(terminal::Kind::Note, "WORLD WRITTEN OUT");
                }
            }
            terminal::Order::Scout(scout) => {
                let label = format!("{scout:?}").to_uppercase();
                self.order_scout(scout, label);
            }
        }
    }

    /// Move the selected good across the bank's counter.
    ///
    /// The journal records what actually moved rather than what was asked
    /// for: a vault's capacity can bite mid-deposit, and a log saying "all of
    /// it" while the world took two thirds is a divergence waiting for the
    /// next replay.
    fn move_at_the_bank(&mut self, deposit: bool) {
        let Some(active) = &mut self.active else { return };
        let Some(town) = active.banks.town else { return };
        let Some(base) = active.mining.fleet.base.as_mut() else {
            active.banks.feedback = Some("NO BASE PILE TO MOVE".into());
            return;
        };
        let rows = active.banks.rows(town, Some(&base.stockpile));
        let Some(good) = rows.get(active.banks.cursor).cloned() else {
            active.banks.feedback = Some("NOTHING TO BANK".into());
            return;
        };
        let wanted = if deposit {
            base.stockpile.count(&good)
        } else {
            active.banks.vault(town).map_or(0, |vault| vault.count(&good))
        };
        let moved = if deposit {
            active.banks.deposit(town, &good, wanted, &mut base.stockpile)
        } else {
            active.banks.withdraw(town, &good, wanted, &mut base.stockpile)
        };
        if moved == 0 {
            active.banks.feedback = Some(
                if deposit && active.banks.room(town) == 0 {
                    "THE VAULT IS FULL".into()
                } else {
                    "NOTHING TO MOVE".into()
                },
            );
            return;
        }
        active.journal.record(Command::Bank {
            town,
            good: good.clone(),
            amount: moved,
            deposit,
        });
        active.banks.feedback = Some(format!(
            "{} {moved} {}",
            if deposit { "BANKED" } else { "DREW" },
            shop::display_name(&good)
        ));
    }

    /// Start a run on the electrolyser.
    fn start_run(&mut self) {
        let Some(active) = &mut self.active else { return };
        let level = active.skills.level(skills::FABRICATION);
        let index = active.electrolyser.cursor;
        let dry_shore = active
            .electrolyser
            .at
            .is_none_or(|at| !electrolysis::water_near(&active.world, at));
        let Some(base) = active.mining.fleet.base.as_mut() else {
            active.electrolyser.feedback = Some("NO BASE PILE TO DRAW ON".into());
            return;
        };
        match active
            .electrolyser
            .begin(index, &mut base.stockpile, level, dry_shore)
        {
            Ok(()) => {
                active.journal.record(Command::Electrolyse {
                    run: index as u32,
                });
                active.electrolyser.feedback = Some("RUNNING".into());
            }
            Err(reason) => active.electrolyser.feedback = Some(reason),
        }
    }

    /// One frame of whatever is down there.
    ///
    /// Everything it says goes through the toast-and-terminal channel the
    /// townsfolk round built, because a search nobody can perceive may as
    /// well be a random walk — the note's own argument, and the reason the
    /// tells are not optional.
    fn advance_the_dark(active: &mut Active, dt: f32) {
        let seed = active.world.seed() ^ active.journal.tick();
        let report = active
            .dark
            .update(dt, &active.world, active.player.position, seed);
        for line in report.tells {
            active.terminal.say(terminal::Kind::Warn, line.clone());
            active.greeting = Some((line, Instant::now()));
        }
        if report.hits > 0 {
            active.health.take(report.hits);
            active.greeting = Some((
                active
                    .health
                    .readout()
                    .unwrap_or_else(|| "IT GOT A HOLD OF YOU".into()),
                Instant::now(),
            ));
        }
    }

    /// Charge the player for whatever they are standing next to.
    ///
    /// Sampled on its own small beat rather than every frame: the sum walks
    /// a box of a thousand blocks, and nobody moves far enough in a quarter
    /// of a second for the difference to be visible.
    fn advance_dose(active: &mut Active, dt: f32) {
        active.dose_check -= dt;
        let rads = if active.dose_check <= 0.0 {
            active.dose_check = 0.25;
            active.last_rads = dose::exposure(&active.world, active.player.position);
            active.last_rads
        } else {
            active.last_rads
        };

        let marks = active.wallet.upgrade(wallet::SHIELD);
        match active.dose.tick(rads, dt, marks) {
            Some(dose::Told::Warned) => {
                active.greeting = Some((
                    "YOU ARE COOKING - THAT FACE IS HOT".into(),
                    Instant::now(),
                ));
            }
            Some(dose::Told::Burned) => {
                active.health.take(1);
                active.greeting = Some((
                    active
                        .health
                        .readout()
                        .unwrap_or_else(|| "THE DOSE IS TELLING ON YOU".into()),
                    Instant::now(),
                ));
            }
            None => {}
        }
    }

    /// Take whichever row the ward's cursor is on.
    ///
    /// Nothing here is journalled: the bed spends nothing, the medkit spends
    /// credits, and both of them only ever touch the player. The oracle has
    /// no business in a hospital.
    fn take_the_ward(&mut self) {
        let Some(active) = &mut self.active else { return };
        let row = active.clinic.row();
        let credits = active.wallet.credits();
        if let Some(reason) = clinic::refuse(row, &active.health, credits, active.dose.rads) {
            active.clinic.feedback = Some(reason);
            return;
        }
        let line = match row {
            clinic::Row::Rest => {
                active.health.revive();
                // The half that matters after the deep resources: a ward is
                // the only thing that takes a dose off you in a hurry.
                let scrubbed = active.dose.rads;
                active.dose.flush();
                active.last_rads = 0.0;
                if scrubbed > 1.0 {
                    format!("PATCHED UP AND SCRUBBED - {scrubbed:.0} RADS OFF YOU")
                } else {
                    "PATCHED UP - YOU ARE WHOLE".to_string()
                }
            }
            clinic::Row::Medkit => {
                if !active.wallet.spend(clinic::MEDKIT_PRICE) {
                    active.clinic.feedback = Some("SHORT CREDITS".into());
                    return;
                }
                active.health.stock_medkit();
                format!("ONE MEDKIT - {} IN THE BAG", active.health.medkits())
            }
        };
        active.clinic.feedback = Some(line.clone());
        active.greeting = Some((line, Instant::now()));
    }

    /// Sink a hole under the open wellhead.
    ///
    /// The order goes in the log *before* anything moves, like every other
    /// order that touches the pile: the log is the record of what was asked
    /// for, and the same call runs on the replay side.
    fn spud_in(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some(at) = active.well_panel.at else { return };
        let seed = active.world.seed();
        let Some(base) = active.mining.fleet.base.as_mut() else {
            active.well_panel.feedback = Some("NO BASE PILE TO DRAW ON".into());
            return;
        };
        match active.mining.wells.spud(at, seed, &mut base.stockpile) {
            Ok(()) => {
                active.journal.record(Command::Spud { at });
                let line = "SPUDDED IN - THE STRING IS GOING DOWN".to_string();
                active.well_panel.feedback = Some(line.clone());
                active.greeting = Some((line, Instant::now()));
            }
            Err(reason) => active.well_panel.feedback = Some(reason),
        }
    }

    /// Say out loud what the holes did, once per thing rather than per tick.
    ///
    /// The report comes off `Mining` because the wells tick inside the call
    /// replay re-runs — and replay has nobody to tell, which is exactly why
    /// the telling happens here and not in there.
    fn report_the_wells(active: &mut Active) {
        let report = std::mem::take(&mut active.mining.well_report);
        if report.quiet() {
            return;
        }
        for (at, fluid) in &report.struck {
            let line = format!("STRUCK {} AT {} {}", fluid.name(), at.x, at.z);
            active.greeting = Some((line, Instant::now()));
        }
        for at in &report.dusters {
            active.greeting = Some((
                format!("DRY HOLE AT {} {} - NOTHING DOWN THERE", at.x, at.z),
                Instant::now(),
            ));
        }
        for at in &report.spent {
            active.greeting = Some((
                format!("THE WELL AT {} {} IS PUMPED OUT", at.x, at.z),
                Instant::now(),
            ));
        }
    }

    /// Run the bath on the journal's clock, like the fabricator's bed.
    fn advance_electrolysis(active: &mut Active, ticks: u32) {
        if ticks == 0 || active.electrolyser.job.is_none() {
            return;
        }
        let dt = ticks as f32 / crate::mining::TICK_RATE as f32;
        let Some(electrolysis::Progress::Done { cells, xp }) = active.electrolyser.work(dt) else {
            return;
        };
        if let Some(base) = active.mining.fleet.base.as_mut() {
            base.stockpile.add(fuel::CELL, u64::from(cells));
        }
        if let Some(level) = active.skills.add_xp(skills::FABRICATION, xp) {
            active.level_up = Some((skills::FABRICATION.to_string(), level, Instant::now()));
        }
        let line = format!("BANKED {cells} HHO");
        active.electrolyser.feedback = Some(line.clone());
        active.greeting = Some((line, Instant::now()));
    }

    /// Run whatever is on the fabricator's bed, on the journal's clock.
    ///
    /// Distance is deliberately not a condition: once a pattern is started
    /// the materials are spent and the machine is working, so walking away
    /// is fine. It is a factory, not a lockpick.
    fn advance_printing(active: &mut Active, ticks: u32) {
        if ticks == 0 || active.printer.job.is_none() {
            return;
        }
        let dt = ticks as f32 / crate::mining::TICK_RATE as f32;
        let Some(printer::Progress::Done { output, xp }) = active.printer.work(dt) else {
            return;
        };
        if let Some(level) = active.skills.add_xp(skills::FABRICATION, xp) {
            active.level_up = Some((skills::FABRICATION.to_string(), level, Instant::now()));
        }
        let line = match output {
            printer::Output::Good { name, count } => {
                if let Some(base) = active.mining.fleet.base.as_mut() {
                    base.stockpile.add(name, count);
                }
                format!("PRINTED {count} {}", shop::display_name(name))
            }
            printer::Output::Slugs(count) => {
                active.arsenal.ammo += count;
                format!("PRINTED {count} SLUGS")
            }
            printer::Output::Cell => {
                // A charged cell is the point of printing one: the scout
                // flies now rather than after its recharge.
                match &mut active.mining.kestrel {
                    Some(kestrel) => {
                        kestrel.endurance = vx_agent::kestrel::ENDURANCE;
                        kestrel.cooldown = 0;
                        "CELL SWAPPED. THE KESTREL IS FRESH".to_string()
                    }
                    None => "CELL PRINTED. NO KESTREL TO PUT IT IN".to_string(),
                }
            }
            printer::Output::Machine(kind) => {
                active.garage.grant(kind, 1);
                format!("PRINTED A {}", garage::display_name(kind))
            }
            printer::Output::Module(module) => {
                active.garage.grant(module, 1);
                format!("PRINTED A {}", module.to_uppercase())
            }
            printer::Output::Optic(name) => {
                active.optics.owned.insert(name.to_string());
                format!("PRINTED A {}", name.to_uppercase())
            }
            printer::Output::Cartridge => {
                // Live-only like the optics and the wallet: a toy on a screen
                // is not state a replay carries.
                active.arcade.print();
                "PRINTED A CARTRIDGE - IT IS ON THE HANDHELD".to_string()
            }
            printer::Output::Charter => {
                // Live-only too: the charter is paper until it is filed, and
                // filing is the order the journal carries.
                active.charters.print();
                "PRINTED A TOWN CHARTER - FOUND <HEAD> <TAIL> ON OPEN GROUND".to_string()
            }
            printer::Output::Upgrade(line) => {
                // The same line the counter sells, raised the same way —
                // this is a second door onto one upgrade, not a second
                // upgrade system. Live-only, like every other print that is
                // not a good: the wallet is not state a replay carries.
                let level = active.wallet.raise(line);
                format!("FITTED - {} NOW {level} OF {}", line.to_uppercase(), wallet::MAX_UPGRADE)
            }
        };
        active.printer.feedback = Some(line.clone());
        active.greeting = Some((line, Instant::now()));
    }

    /// Step the kestrel and the roost `ticks` journal ticks, and let both
    /// scan. Flight is deterministic in ticks; the marks they leave are
    /// live-side intelligence, outside the oracle like the town books.
    fn advance_scouts(active: &mut Active, ticks: u32) {
        // The bought scout materialises docked, and its cell upgrade level
        // applies retroactively like every other upgrade.
        if active.garage.owned(garage::KESTREL) > 0
            && active.mining.kestrel.is_none()
            && active.intrusion.impounded.is_none()
        {
            let anchor = vx_core::BlockPos::new(
                active.player.position.x.floor() as i32,
                active.player.position.y.floor() as i32,
                active.player.position.z.floor() as i32,
            );
            active.mining.kestrel = Some(vx_agent::Kestrel::new(anchor));
            active.greeting = Some((
                "KESTREL DELIVERED. IT RIDES YOUR PACK - SEE THE HANDHELD".into(),
                Instant::now(),
            ));
        }
        if let Some(kestrel) = &mut active.mining.kestrel {
            kestrel.recharge_cost = wallet::recharge_ticks(active.wallet.upgrade(wallet::CELL));
        }
        // A bought watch box mounts itself on your own roof. Recorded as an
        // ordinary `Place`, exactly as the town's lock rebuilds are: a world
        // edit the journal cannot see is a world edit that makes the oracle
        // lie. If the house is not loaded yet this simply waits a frame.
        if active.garage.owned(garage::WATCHBOX) > 0 && active.intrusion.roost_at.is_none() {
            let site = vx_world::town::home_site();
            let house = vx_world::town::plan::buildings(&site)
                .into_iter()
                .find(|building| building.role == vx_world::town::plan::Role::PlayerHouse);
            if let Some(house) = house {
                let at = vx_core::BlockPos::new(house.max.x - 2, house.max.y - 1, house.max.z - 2);
                if let Some(id) = active.world.registry().id_of("engine:roost") {
                    if active.world.set_block(at, id).is_some() {
                        active.intrusion.roost_at = Some(at);
                        active.journal.record(Command::Place {
                            at,
                            block: "engine:roost".to_string(),
                        });
                        active.greeting = Some((
                            "WATCH BOX MOUNTED. IT KEEPS AN EYE ON YOUR YARD".into(),
                            Instant::now(),
                        ));
                    }
                }
            }
        }
        if ticks == 0 {
            return;
        }

        let anchor = vx_core::BlockPos::new(
            active.player.position.x.floor() as i32,
            active.player.position.y.floor() as i32,
            active.player.position.z.floor() as i32,
        );
        // The vanguard flies along the body's facing, snapped to the axis
        // with the most of it — the flier's own movement grid.
        let facing = active
            .last_move
            .map(|command| command.yaw())
            .unwrap_or(active.camera.yaw);
        let (dx, dz) = (facing.sin(), -facing.cos());
        let heading = if dx.abs() >= dz.abs() {
            (dx.signum() as i32, 0)
        } else {
            (0, dz.signum() as i32)
        };

        let started = active.journal.tick().saturating_sub(u64::from(ticks));
        for step in 0..ticks {
            if let Some(kestrel) = &mut active.mining.kestrel {
                // A piloted kestrel is moved by pilot_sub_tick; ticking it
                // here too would double its meter, so Manual only drains.
                kestrel.tick(&active.world, anchor, heading);
            }
            let clock = started + u64::from(step) + 1;
            if let Some(roost) = &mut active.roost {
                roost.tick(&active.world, clock);
            }
        }

        // One scan per frame is plenty: contacts move at walking pace.
        let now = active.journal.tick();
        let mut contacts: Vec<(scout::MarkKind, glam::DVec3)> = active
            .villagers
            .positions()
            .into_iter()
            .map(|at| (scout::MarkKind::Person, at))
            .collect();
        for at in active.mining.drone_positions() {
            contacts.push((
                scout::MarkKind::Machine,
                glam::DVec3::new(f64::from(at.x) + 0.5, f64::from(at.y), f64::from(at.z) + 0.5),
            ));
        }
        for flier in &active.mining.fleet.fliers {
            contacts.push((
                scout::MarkKind::Machine,
                glam::DVec3::new(
                        f64::from(flier.position.x) + 0.5,
                        f64::from(flier.position.y),
                        f64::from(flier.position.z) + 0.5,
                    ),
            ));
        }
        if let Some(kestrel) = active
            .mining
            .kestrel
            .as_ref()
            .filter(|kestrel| kestrel.aloft())
        {
            let at = kestrel.craft.position;
            let eye = glam::DVec3::new(f64::from(at.x) + 0.5, f64::from(at.y) + 0.3, f64::from(at.z) + 0.5);
            // The spoofers stage 15 taught the player have arrived in the
            // other side's hands: over a shelter with a grudge, the scout's
            // link is jammed and its marks simply do not take.
            if active.garrisons.jamming_at(eye) {
                if !active.jam_warned {
                    active.jam_warned = true;
                    let line = "THE SHELTER IS JAMMING YOUR SCOUT".to_string();
                    active.terminal.say(terminal::Kind::Warn, line.clone());
                    active.greeting = Some((line, Instant::now()));
                }
            } else {
                active.jam_warned = false;
                active.marks.scan(
                    &active.world,
                    eye,
                    vx_agent::kestrel::SCAN_RADIUS,
                    &contacts,
                    now,
                );
            }
        }
        // A tapped watch box files its sightings to you as well as to the
        // sheriff: the same eye, the same radius, the same occlusion. The tap
        // grants their eyes, never better ones — which is the whole reason it
        // is worth having and the reason it is not a wallhack.
        let tap_eye = active
            .roost
            .as_ref()
            .filter(|roost| roost.tapped() && roost.aloft())
            .map(|roost| {
                let at = roost.position();
                glam::DVec3::new(f64::from(at.x) + 0.5, f64::from(at.y) + 0.3, f64::from(at.z) + 0.5)
            });
        if let Some(eye) = tap_eye {
            active
                .marks
                .scan(&active.world, eye, roost::WATCH_RADIUS, &contacts, now);
        }
        // Your own box on your own roof: it does not fly, because it has
        // nothing to respond to — it only has to watch the yard.
        if let Some(box_at) = active.intrusion.roost_at {
            let eye = glam::DVec3::new(f64::from(box_at.x) + 0.5, f64::from(box_at.y) + 1.3, f64::from(box_at.z) + 0.5);
            active
                .marks
                .scan(&active.world, eye, roost::WATCH_RADIUS, &contacts, now);
        }
        active.marks.cull(now);
        Self::advance_intrusion(active, ticks);
    }

    /// Which machine is doing the intruding, and what frame it has.
    ///
    /// The machine you are looking through if there is one — piloted or just
    /// watched — else the scout, if it is off the pack. Position is the only
    /// input, which is what makes "flown there by hand" and "sent there"
    /// produce the same world.
    fn intruder(active: &Active) -> Option<(intrusion::Frame, glam::DVec3, u64)> {
        let machine = active
            .device
            .feed()
            .or_else(|| {
                active
                    .mining
                    .kestrel
                    .as_ref()
                    .filter(|kestrel| kestrel.aloft())
                    .map(|_| mining::MachineRef::Kestrel)
            })?;
        let at = active.mining.machine_eye(machine)?;
        let (frame, value) = match machine {
            mining::MachineRef::Kestrel => (
                intrusion::Frame::Light,
                garage::cost(garage::KESTREL, 0),
            ),
            mining::MachineRef::Flier(_) => {
                (intrusion::Frame::Heavy, garage::cost(garage::FLIER, 0))
            }
            mining::MachineRef::Digger(_) => {
                (intrusion::Frame::Heavy, garage::cost(garage::DRONE, 0))
            }
        };
        Some((frame, at, value))
    }

    /// Work whatever intrusion is running, on the journal's clock.
    ///
    /// Every condition is re-judged each tick: fly the machine out of reach,
    /// walk out of link, and the job stops where it stands. And every tick
    /// the machine is *visible* is a tick it can be caught — the trade the
    /// coil makes is distance from the scene, never from the consequence.
    fn advance_intrusion(active: &mut Active, ticks: u32) {
        if ticks == 0 || active.intrusion.job.is_none() {
            return;
        }
        let Some(job) = active.intrusion.job else { return };
        let Some((frame, machine_at, value)) = Self::intruder(active) else {
            active.intrusion.abort();
            active.greeting = Some(("THE MACHINE IS GONE".into(), Instant::now()));
            return;
        };
        let target_at = job.target.at();
        let target_centre = glam::DVec3::new(f64::from(target_at.x) + 0.5, f64::from(target_at.y) + 0.5, f64::from(target_at.z) + 0.5);
        let attempt = intrusion::Attempt {
            frame,
            fitted: active.garage.fitted(frame.coil()),
            security: active.skills.level(skills::SECURITY),
            reach: (machine_at - target_centre).length() as f32,
            link: (machine_at - active.player.eye_position()).length() as f32,
            target: job.target,
        };

        // Caught at it: the bill lands on the name in the garage papers, and
        // an unattended machine is grabbed where a held one can be flown off.
        let seen_by_town = active.villagers.watchers_of(&active.world, machine_at);
        let seen_by_roost = active
            .roost
            .as_ref()
            .is_some_and(|roost| roost.witnesses(&active.world, machine_at));
        let witnesses = seen_by_town + usize::from(seen_by_roost);
        if witnesses > 0 {
            let bill = arsenal::witnessed_bounty(intrusion::BOUNTY_MACHINE, witnesses);
            active.permits.borrow_mut().caught(bill, witnesses);
            let unattended = !active.device.is_piloting();
            if unattended {
                active.intrusion.impound(value);
                active.mining.kestrel = None;
                active.greeting = Some((
                    "YOUR MACHINE WAS SEIZED AT THE LOCK".into(),
                    Instant::now(),
                ));
            } else {
                active.intrusion.abort();
                active.greeting = Some((
                    "SPOTTED. GET IT OUT OF THERE".into(),
                    Instant::now(),
                ));
            }
            return;
        }

        let dt = ticks as f32 / crate::mining::TICK_RATE as f32;
        match active.intrusion.work(&attempt, dt) {
            intrusion::Progress::Working(_) => {}
            intrusion::Progress::Refused(why) => {
                active.greeting = Some((why, Instant::now()));
            }
            intrusion::Progress::Opened { xp } => {
                let claim = active.permits.borrow().claim_here(target_at);
                let label = match claim {
                    Some(claim) => {
                        let label = claim.label.clone();
                        active.permits.borrow_mut().grant(claim.key);
                        label
                    }
                    None => "IT".to_string(),
                };
                if let Some(level) = active.skills.add_xp(skills::SECURITY, xp) {
                    active.level_up =
                        Some((skills::SECURITY.to_string(), level, Instant::now()));
                }
                active.greeting = Some((
                    format!("{label} IS OPEN. NOBODY SAW A THING"),
                    Instant::now(),
                ));
            }
            intrusion::Progress::Graded { grade, hold, xp } => {
                let now = active.journal.tick();
                if let Some(roost) = &mut active.roost {
                    roost.hack(grade, now + hold);
                }
                if let Some(level) = active.skills.add_xp(skills::SECURITY, xp) {
                    active.level_up =
                        Some((skills::SECURITY.to_string(), level, Instant::now()));
                }
                active.greeting = Some((
                    format!("THE TOWNS EYE IS {}", grade.label()),
                    Instant::now(),
                ));
            }
        }
    }

    /// Walk-up salvage: stand next to a downed load and it goes onto the
    /// base pile — the same pile everything else the fleet hauls lands on.
    /// No base means the wreck simply waits.
    fn collect_crashes(active: &mut Active) {
        let feet = active.player.position;
        let mut collected: Vec<arsenal::Crash> = Vec::new();
        active.arsenal.crashes.retain(|crash| {
            let ground = active
                .world
                .generator()
                .height_at(crash.x.floor() as i32, crash.z.floor() as i32);
            let at = glam::DVec3::new(crash.x, f64::from(ground) + 1.0, crash.z);
            let near = (at - feet).length() < 3.0;
            if near && active.mining.fleet.base.is_some() {
                collected.push(*crash);
                false
            } else {
                true
            }
        });
        for crash in collected {
            if let Some(base) = active.mining.fleet.base.as_mut() {
                let good = economy::GOODS[crash.good].to_string();
                base.stockpile.add(good, crash.amount);
                active.greeting = Some((
                    format!(
                        "SALVAGED {} {}",
                        crash.amount,
                        shop::display_name(economy::GOODS[crash.good])
                    ),
                    Instant::now(),
                ));
            }
        }
    }

    /// Hand a crate's contents to the pile, and say what came out.
    ///
    /// Shared by the two ways in — pressing `E` on a crate and prising it
    /// open with the drill — so one of them can never start paying out
    /// something the other does not.
    fn pay_out_cache(active: &mut Active, at: vx_core::BlockPos) {
        let level = active.skills.level(salvage::SKILL);
        let haul = salvage::contents(at, level);
        let Some(base) = active.mining.fleet.base.as_mut() else {
            return;
        };
        for (name, count) in &haul {
            base.stockpile.add(*name, *count);
        }
        let listed: Vec<String> = haul
            .iter()
            .map(|(name, count)| format!("{count} {}", shop::display_name(name)))
            .collect();
        let xp = salvage::experience(at);
        if let Some(level) = active.skills.add_xp(salvage::SKILL, xp) {
            active.level_up = Some((salvage::SKILL.to_string(), level, Instant::now()));
        }
        active.greeting = Some((format!("SALVAGED {}", listed.join(", ")), Instant::now()));
    }

    /// Run the drill for one frame, if it is held on something drillable.
    ///
    /// Hold-to-dig: progress accumulates while the bit stays on one block and
    /// resets when the aim moves — like lifting a real tool. The break itself
    /// still goes through `break_block`, so events fire and a mod's veto works
    /// exactly as it does for the drones.
    fn update_drilling(&mut self, dt: f32) {
        // With the launcher in hand the trigger is the trigger; the drill
        // stays on your hip.
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.arsenal.equipped)
        {
            self.update_firing();
            return;
        }
        let drilling = self.drill_held && self.input.mouse_captured;
        let Some(active) = &mut self.active else { return };
        if !drilling {
            active.digging = None;
            return;
        }

        // A running drill is the dinner bell: every held shelter in earshot
        // gets the zone — never the spot — and has to come looking. The
        // tension attaches to the mining loop, exactly as the hunt note
        // wanted: run it loud and rich, or slow and quiet, or dig decoy
        // noise a valley over and work in the shadow of your own diversion.
        if active.journal.tick().is_multiple_of(16) {
            active.garrisons.hear(active.player.position);
            // And the deep hears it better than the shelters do. This is the
            // whole attachment between the hunt and the mining loop: a hole
            // being cut is a dinner bell, and the only dial the player has
            // is how long they keep cutting.
            active.dark.hear(active.player.position, 0.55);
        }

        let Some(hit) = raycast_solid(
            &active.world,
            active.world.registry(),
            active.camera.position,
            active.camera.forward(),
            Self::REACH,
        ) else {
            active.digging = None;
            return;
        };

        // Hardness is the block's resistance; no hardness (bedrock) means the
        // bit just skates.
        let Some(hardness) = active
            .world
            .registry()
            .get(hit.id)
            .and_then(|def| def.hardness)
        else {
            active.digging = None;
            return;
        };

        let power = drill::power_of(
            active.skills.level(skills::MINING),
            active.wallet.upgrade(wallet::DRILL),
        );
        let step = drill::bite(hardness, power, dt);

        // A lockbox is not an ordinary block. It has a power gate below which
        // the bit simply skates, and its progress *persists* — a breach is a
        // project you come back to, where ordinary drilling resets the moment
        // your aim wobbles. At twenty-odd seconds of continuous hold for the
        // softest grade, the ordinary rule would make grade I miserable and
        // the higher grades impossible in practice.
        let lock_tier = permits::tier_of(active.world.registry(), hit.id);
        if let Some(tier) = lock_tier {
            let needed = permits::min_power(tier);
            if power < needed {
                active.digging = None;
                active.greeting = Some((
                    "YOUR DRILL IS TOO WEAK FOR THIS LOCK".into(),
                    Instant::now(),
                ));
                return;
            }
            let carried = active.permits.borrow().breach_progress(hit.block) + step;
            if carried < 1.0 {
                active.permits.borrow_mut().set_breach(hit.block, carried);
                active.digging = Some((hit.block, carried));
                return;
            }
        } else if let Some((tree, face)) = Self::stump_under_the_bit(active, &hit) {
            // A trunk cut low is not a block being broken, it is a tree being
            // felled. The hold goes into a notch on the trunk's own
            // cross-section instead of into the block's four layers, because
            // the rule that drops the tree is written in fractions of the
            // section and needs a finer step than a quarter to say a third.
            let before = match &active.digging {
                Some((target, progress)) if *target == hit.block => *progress,
                _ => 0.0,
            };
            let after = (before + step).min(1.0);
            active.digging = Some((hit.block, after));
            // Every step of the hold takes another few cells out of the
            // section, and the mask is where the state lives — so a cut you
            // walked away from is still there when you come back.
            let cells = (after * vx_world::micro::CELLS as f32) as u32;
            active
                .world
                .carve(hit.block, vx_world::micro::Shape::Notch { cells }.cells(0, 0, 0, face));
            let mask = active
                .world
                .mask(hit.block)
                .unwrap_or(vx_world::micro::FULL);
            if !felling::ready(mask, face) {
                return;
            }
            active.digging = None;
            Self::fell_the_tree(active, &tree, hit.block, face);
            return;
        } else {
            // Drilling is the same amount of work it always was; it is
            // simply visible now. Each quarter of the way through takes the
            // layer of cells nearest the bit, so a face being worked looks
            // worked — and the block still finishes on the same tick, by the
            // same `break_block` below.
            //
            // The arithmetic itself lives in `drill`, so the headless session
            // that plays the loop drills with this very rule rather than with
            // a second copy of it.
            let carried = match &active.digging {
                Some((target, progress)) if *target == hit.block => Some(*progress),
                _ => None,
            };
            let bite = drill::advance_bite(carried, step);
            active.digging = Some((hit.block, bite.progress));
            if bite.carve {
                let face = drill::face_index(hit.face);
                active
                    .world
                    .carve(hit.block, vx_world::micro::Shape::DrillFace.cells(0, 0, 0, face));
            }
            if !bite.through {
                return;
            }
        }

        // Through: break it, learn from it.
        active.digging = None;
        match break_block(&mut active.world, &active.events, hit.block) {
            Err(error) => {
                // `Cancelled` covers both a permission refusal and plain
                // bedrock, and bedrock never emits an event — so the refusal
                // is only believed when it names this very block.
                if let Some(line) = Self::charge_for_refusal(active, hit.block, permits::BOUNTY_PRYING) {
                    active.greeting = Some((line, Instant::now()));
                }
                log::debug!("could not break {:?}: {error}", hit.block);
            }
            Ok(_) => {
                // A crate prised open with a drill is still a crate opened:
                // it pays out its haul rather than yielding one crate-shaped
                // block, and it records the order that says so, so a replay
                // that only saw `Break` cannot end up holding different goods
                // than the session did.
                let crate_here = active.world.registry().get_or_air(hit.id).name
                    == "engine:supply_cache";
                if crate_here {
                    active.journal.record(Command::Salvage { at: hit.block });
                    Self::pay_out_cache(active, hit.block);
                } else {
                    active.journal.record(Command::Break { at: hit.block });
                }
                // Cut into a lake and the lake notices. The break is already
                // the order, so the replay wakes the same water on the same
                // tick and the flood is re-derived rather than recorded.
                journal::wake_water(&mut active.water, &mut active.world, hit.block);
                // Everything you break is stock. Every block yields itself by
                // name onto the same pile the drones haul into and the shop
                // sells out of — one pile, no transfer minigame, and the
                // fabricator's whole catalogue is fed by it. A hand-cut block
                // used to simply vanish, which made the drill the one tool
                // in the game that produced nothing.
                let landed = drill::deposit(
                    active
                        .mining
                        .fleet
                        .base
                        .as_mut()
                        .map(|base| &mut base.stockpile),
                    active.world.registry(),
                    hit.id,
                    crate_here,
                );
                if landed == drill::Deposited::NoBase {
                    // There is no base at boot, and this was the one system in
                    // the game that produced goods and never said where they
                    // went: the block broke, the ore evaporated, and nothing
                    // was printed anywhere. Every sibling says this line.
                    active.greeting = Some((drill::NO_BASE.to_string(), Instant::now()));
                }
                let xp = (hardness * skills::MINING_XP_PER_HARDNESS) as u64;
                if let Some(level) = active.skills.add_xp(skills::MINING, xp) {
                    active.level_up = Some((skills::MINING.to_string(), level, Instant::now()));
                }

                // Breaking the base container un-declares the base. The pile
                // it held is set aside for now; stage 7's inventory gives it
                // somewhere to go.
                let was_base = active
                    .mining
                    .fleet
                    .base
                    .as_ref()
                    .is_some_and(|base| base.position == hit.block);
                if was_base {
                    if let Some(pile) = active.mining.fleet.clear_base() {
                        log::info!(
                            "base container broken; {} blocks in it are set aside",
                            pile.total()
                        );
                    }
                }

                // A lock went down. The claim it held sleeps until the town
                // puts a new box up, and if anybody saw, that is the loudest
                // thing on the sheet.
                if let Some(tier) = lock_tier {
                    let now = active.journal.tick();
                    let claim = active.permits.borrow().claim_here(hit.block);
                    let label = claim
                        .as_ref()
                        .map_or_else(|| "SOMETHING".to_string(), |claim| claim.label.clone());
                    // What a breach costs depends on what was breached, and
                    // the claim already carries the grade. A bank is the one
                    // building whose strongroom holds what a whole town left
                    // with it, so it is the one crime that bills the maximum.
                    let bill = permits::breach_bounty(
                        claim.as_ref().and_then(|claim| claim.tier).or(Some(tier)),
                    );
                    let seen = usize::from(active.watched);
                    let mut permits = active.permits.borrow_mut();
                    permits.broke(hit.block, now);
                    let caught = permits.caught(bill, seen);
                    drop(permits);
                    // A witnessed crime sours the whole town, scaled by the
                    // bounty board — disposition and law linked but
                    // distinct, exactly as the ledger's own tests put it.
                    if seen > 0 {
                        if let Some(claim) = &claim {
                            let day = (now / schedule::TICKS_PER_DAY) as u32;
                            active.friends.crime(claim.key.town, bill, day);
                        }
                        // And the county remembers, long after the fine is
                        // paid: the bill scales the standing cost too.
                        let band = active.reputation.crime(bill);
                        Self::note_band(active, "THE TOWNS", band);
                    }
                    active.greeting = Some((
                        match (caught, bill >= permits::BOUNTY_VAULT) {
                            (true, true) => {
                                format!("{label} IS OPEN - AND EVERY BADGE IN THE COUNTY SAW IT")
                            }
                            (true, false) => format!("{label} IS OPEN - AND YOU WERE SEEN"),
                            (false, true) => format!("{label} IS OPEN. THEY WILL COUNT IT"),
                            (false, false) => format!("{label} IS OPEN"),
                        },
                        Instant::now(),
                    ));
                    // Breaching is the loudest thing in a quiet town, and
                    // the box on the office roof has ears.
                    if let Some(roost) = &mut active.roost {
                        roost.report(hit.block, roost::Report::Breach);
                    }
                }

                // The watch box drilled out: the town is blind until it is
                // re-boxed, which is the loud way to buy what a hack buys
                // quietly — and it costs the loud price.
                if active.world.registry().id_of("engine:roost") == Some(hit.id) {
                    let now = active.journal.tick();
                    let mine = active.intrusion.roost_at == Some(hit.block);
                    if mine {
                        active.intrusion.roost_at = None;
                        active.greeting =
                            Some(("YOUR WATCH BOX IS DOWN".into(), Instant::now()));
                    } else {
                        if let Some(roost) = &mut active.roost {
                            roost.knock_out(now + permits::REBUILD_TICKS);
                        }
                        let seen = usize::from(active.watched);
                        let caught = active
                            .permits
                            .borrow_mut()
                            .caught(permits::BOUNTY_BREACH, seen);
                        active.greeting = Some((
                            if caught {
                                "THE TOWNS EYE IS DOWN - AND YOU WERE SEEN".into()
                            } else {
                                "THE TOWNS EYE IS DOWN".to_string()
                            },
                            Instant::now(),
                        ));
                    }
                }

                // Breaking the chest packs it up: the block is forgotten, the
                // contents stay in the homestead — they were never in the
                // world to begin with.
                if active.printer.at == Some(hit.block) {
                    active.printer.at = None;
                    active.printer.close();
                    active.greeting = Some((
                        "FABRICATOR PACKED UP. PLACE IT WHERE YOU LIKE".into(),
                        Instant::now(),
                    ));
                }
                // A head broken off is a hole abandoned. Everything still
                // in the ground stays in the ground: the reservoir is
                // worldgen, so spudding here again finds exactly what was
                // left — minus the casing, which is the price of moving.
                if active.mining.wells.remove(hit.block) {
                    if active.well_panel.at == Some(hit.block) {
                        active.well_panel.close();
                        active.renderer.clear_overlay(WELL_SLOT);
                    }
                    active.greeting = Some(("WELLHEAD PULLED".into(), Instant::now()));
                }
                if active.electrolyser.at == Some(hit.block) {
                    active.electrolyser.at = None;
                    active.electrolyser.job = None;
                    active.electrolyser.close();
                    active.greeting = Some((
                        "ELECTROLYSER PACKED UP. FIND ANOTHER SHORE".into(),
                        Instant::now(),
                    ));
                }
                if active.homestead.chest_at == Some(hit.block) {
                    active.homestead.chest_at = None;
                    active.greeting = Some((
                        "CHEST PACKED UP. PLACE IT AGAIN, HERE OR AFIELD".into(),
                        Instant::now(),
                    ));
                }
            }
        }
    }

    /// Place the selected block against whatever the player is looking at.
    fn place_at_target(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some(&block) = active.palette.get(active.selected) else {
            return;
        };
        let Some(hit) = raycast_solid(
            &active.world,
            active.world.registry(),
            active.camera.position,
            active.camera.forward(),
            Self::REACH,
        ) else {
            return;
        };

        // One chest is the rule, enforced at the door: the homestead is a
        // single side table, and a second block would be a chest-shaped lie.
        let chest = active.world.registry().id_of("engine:chest");
        if chest == Some(block) && active.homestead.chest_at.is_some() {
            active.greeting = Some((
                "YOU HAVE A CHEST ALREADY. BREAK IT TO MOVE IT".into(),
                Instant::now(),
            ));
            return;
        }

        // A fabricator is a machine, not a building block: you may place the
        // one you bought, and only the one you bought. Otherwise the palette
        // would quietly hand out free drone factories.
        let press = active.world.registry().id_of("engine:printer");
        if press == Some(block) {
            if active.garage.owned(garage::PRINTER) == 0 {
                active.greeting = Some((
                    "BUY A FABRICATOR AT THE SHOP COUNTER FIRST".into(),
                    Instant::now(),
                ));
                return;
            }
            if active.printer.at.is_some() {
                active.greeting = Some((
                    "YOU HAVE A FABRICATOR ALREADY. BREAK IT TO MOVE IT".into(),
                    Instant::now(),
                ));
                return;
            }
        }

        // The same rule for the electrolyser, plus the one that makes it
        // interesting: it has to stand where there is water. The refusal is
        // at placement rather than at the panel, because a machine that
        // *looks* built and quietly does nothing is the worse lie.
        let bath = active.world.registry().id_of("engine:electrolyser");
        if bath == Some(block) {
            if active.garage.owned(garage::ELECTROLYSER) == 0 {
                active.greeting = Some((
                    "BUY AN ELECTROLYSER AT THE SHOP COUNTER FIRST".into(),
                    Instant::now(),
                ));
                return;
            }
            if active.electrolyser.at.is_some() {
                active.greeting = Some((
                    "YOU HAVE AN ELECTROLYSER ALREADY. BREAK IT TO MOVE IT".into(),
                    Instant::now(),
                ));
                return;
            }
            if !electrolysis::water_near(&active.world, hit.block.offset(hit.face.offset())) {
                active.greeting =
                    Some(("IT NEEDS WATER WITHIN TWO BLOCKS".into(), Instant::now()));
                return;
            }
        }

        // A pump is stock, not a block you always have: you print one, and
        // placing it spends it. The refusal is at placement, for the reason
        // the electrolyser's is — a machine that looks built and does
        // nothing is the worse lie.
        let pump = active.world.registry().id_of("engine:pump");
        if pump == Some(block) {
            let held = active
                .mining
                .fleet
                .base
                .as_ref()
                .map_or(0, |base| base.stockpile.count("engine:pump"));
            if held == 0 {
                active.greeting = Some((
                    "NO PUMP ON THE PILE. PRINT ONE AT A FABRICATOR".into(),
                    Instant::now(),
                ));
                return;
            }
        }

        // The player's own box must not be built into, or you seal yourself in.
        let body = active.player.aabb();
        let result = place_block(
            &mut active.world,
            &active.events,
            &hit,
            block,
            |position| body.contains_block(position),
        );
        // Placing a container declares it the fleet's base.
        if let Ok(position) = result {
            let name = active.world.registry().get_or_air(block).name.clone();
            active.journal.record(Command::Place {
                at: position,
                block: name,
            });
            if chest == Some(block) {
                active.homestead.chest_at = Some(position);
            }
            if press == Some(block) {
                active.printer.at = Some(position);
            }
            if bath == Some(block) {
                active.electrolyser.at = Some(position);
            }
            if pump == Some(block) {
                if let Some(base) = active.mining.fleet.base.as_mut() {
                    base.stockpile.take("engine:pump", 1);
                }
                active.greeting = Some((
                    "PUMP DOWN. PRESS E TO RUN IT".into(),
                    Instant::now(),
                ));
            }
            let container = active.world.registry().id_of("engine:container");
            if container == Some(block) {
                active.mining.fleet.set_base(position);
                log::info!("base container set at {position:?}");
            }
        }
        if let Err(error) = result {
            if let Some(line) =
                Self::charge_for_refusal(active, hit.placement(), permits::BOUNTY_PRYING)
            {
                active.greeting = Some((line, Instant::now()));
            }
            log::debug!("could not place at {:?}: {error}", hit.placement());
        }
    }

    /// React to a key going down, ignoring auto-repeat.
    /// Which layer the pad is on this frame.
    ///
    /// Stage 24 asked one question — is a panel open — which was not enough
    /// to play from. A live machine feed is neither a panel nor the world,
    /// and while the pad thought it was the world there was no button that
    /// hung up; and the two held modifiers are layers of their own, because
    /// thirteen buttons cannot name twenty actions without one.
    fn pad_context(&self) -> gamepad::Context {
        // A held modifier outranks everything: it is the player's thumb
        // saying which layer they mean, and it is checked first so that
        // opening a panel mid-hold cannot swap the mapping under them.
        if self.pad.held.contains(&gilrs::Button::Select) {
            return gamepad::Context::Second;
        }
        if self.pad.held.contains(&gilrs::Button::LeftTrigger) {
            return gamepad::Context::Palette;
        }
        // The board is typed on, so it outranks the panel it sits over.
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.osk.open && active.terminal.open)
        {
            return gamepad::Context::Typing;
        }
        if self.a_panel_is_open() {
            return gamepad::Context::Panel;
        }
        // A feed with no panel over it: looking through a machine.
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.device.feed().is_some())
        {
            return gamepad::Context::Feed;
        }
        gamepad::Context::World
    }

    /// Is any panel holding the screen? One input to [`App::pad_context`],
    /// and the same set the keyboard's own per-panel dispatch routes by.
    fn a_panel_is_open(&self) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.shop.open
                || active.board.open
                || active.device.open
                || active.home_panel.open
                || active.permit_panel.open
                || active.printer.open
                || active.electrolyser.open
                || active.well_panel.open
                || active.clinic.open
                || active.banks.open
                || active.intro.open
                || active.terminal.open
                || self.gold_is_open()
        })
    }

    #[cfg(feature = "gold")]
    fn gold_is_open(&self) -> bool {
        self.active.as_ref().is_some_and(|active| active.gold.open)
    }

    #[cfg(not(feature = "gold"))]
    fn gold_is_open(&self) -> bool {
        false
    }

    /// One frame of pad input, synthesized into the keyboard and mouse
    /// seams. Buttons go through `handle_press` and `InputState` exactly as
    /// keys do; the sticks feed the movement axes and the mouse-look
    /// accumulator; the triggers mirror the mouse buttons. Nothing
    /// downstream knows a pad exists.
    fn poll_pad(&mut self, dt: f32) {
        let changes = self.pad.poll();
        for change in changes {
            match change {
                gamepad::Change::Connected => {
                    if let Some(active) = &mut self.active {
                        active.greeting =
                            Some(("CONTROLLER CONNECTED - SELECT FOR CONTROLS".into(), Instant::now()));
                    }
                }
                gamepad::Change::Disconnected => {
                    // Nothing a vanished pad was holding may stay held —
                    // the modifiers included, or the next pad plugged in
                    // arrives already on the second layer.
                    for (_, code) in self.pad.down.drain() {
                        self.input.release(code);
                    }
                    self.pad.held.clear();
                    self.pad.used.clear();
                    self.drill_held = false;
                    if let Some(active) = &mut self.active {
                        active.greeting = Some(("CONTROLLER GONE".into(), Instant::now()));
                    }
                }
                gamepad::Change::Press(button) => {
                    // The first press captures the pointer, the same job the
                    // first click does — without it the look pipeline
                    // discards everything.
                    if !self.input.mouse_captured {
                        self.set_capture(true);
                    }
                    // Read the layer per button rather than per frame: a
                    // modifier pressed earlier in this very batch has to be
                    // in force for the button that follows it.
                    let context = self.pad_context();
                    match button {
                        // The two modifiers. Neither acts on the way down —
                        // what a tap meant is only knowable on release, and
                        // a hold means the layer, so both wait.
                        gilrs::Button::Select | gilrs::Button::LeftTrigger => {
                            self.pad.held.insert(button);
                            self.pad.used.remove(&button);
                        }
                        // The analog triggers mirror the mouse buttons, and
                        // a mouse button does not dig through an open panel.
                        gilrs::Button::RightTrigger2 => {
                            if !matches!(context, gamepad::Context::Panel) {
                                self.drill_held = true;
                            }
                        }
                        gilrs::Button::LeftTrigger2 => {
                            if !matches!(context, gamepad::Context::Panel) {
                                self.place_at_target();
                            }
                        }
                        // The one action with no keyboard twin: on a
                        // keyboard you press the letter itself, so there is
                        // no `KeyCode` for "the key I am pointing at".
                        gilrs::Button::South
                            if matches!(context, gamepad::Context::Typing) =>
                        {
                            let picked = self
                                .active
                                .as_ref()
                                .map(|active| active.osk.picked());
                            if let (Some(character), Some(active)) = (picked, &mut self.active) {
                                active.terminal.type_char(character);
                            }
                            self.refresh_terminal();
                        }
                        _ => {
                            // Anything pressed while a modifier is held is
                            // what that modifier was held *for*, so its tap
                            // action is cancelled.
                            if matches!(context, gamepad::Context::Second) {
                                self.pad.used.insert(gilrs::Button::Select);
                            }
                            if matches!(context, gamepad::Context::Palette) {
                                self.pad.used.insert(gilrs::Button::LeftTrigger);
                            }
                            if let Some(code) = gamepad::key_for(button, context) {
                                self.pad.down.insert(button, code);
                                self.handle_press(code);
                                self.input.press(code);
                            }
                        }
                    }
                }
                gamepad::Change::Release(button) => match button {
                    gilrs::Button::RightTrigger2 => self.drill_held = false,
                    // A modifier let go without having been used was a tap,
                    // and a tap is its own action: SELECT shows the scheme,
                    // LB swaps first and third person.
                    gilrs::Button::Select | gilrs::Button::LeftTrigger => {
                        self.pad.held.remove(&button);
                        // Used as a layer: the buttons under it already did
                        // the work, and the tap action is not owed.
                        if self.pad.used.remove(&button) {
                            continue;
                        }
                        // What a tap means is declared once, in `gamepad`,
                        // so the reachability test sees the same surface the
                        // player does. `None` is the help panel, which is
                        // `main`'s own and resolves to no key at all.
                        match gamepad::TAPS.iter().find(|(tapped, _, _)| *tapped == button) {
                            Some((_, Some(code), _)) => self.handle_press(*code),
                            Some((_, None, _)) => {
                                self.pad.help = !self.pad.help;
                                self.refresh_pad_help();
                            }
                            None => {}
                        }
                    }
                    _ => {
                        if let Some(code) = self.pad.down.remove(&button) {
                            // The same Enter release rule the keyboard has:
                            // letting go stops picking a lock.
                            if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
                                if let Some(active) = &mut self.active {
                                    active.picking = false;
                                }
                            }
                            self.input.release(code);
                        }
                    }
                },
            }
        }

        // The sticks. Movement merges into the same axes the keys drive;
        // look feeds the same accumulator the mouse fills, in its units.
        let (move_x, move_forward) = self.pad.left_stick();
        self.input
            .set_pad_axes(glam::Vec3::new(move_x, 0.0, move_forward));
        // A held tilt walks a panel's cursor. Before this the stick was
        // dead in every menu and the d-pad was the only way to move — which
        // is not how any console game reads.
        if matches!(self.pad_context(), gamepad::Context::Panel) {
            let tilt = self.pad.left_stick().1;
            if tilt.abs() > gamepad::CURSOR_TILT {
                self.pad.repeat -= dt;
                if self.pad.repeat <= 0.0 {
                    let code = if tilt > 0.0 { KeyCode::ArrowUp } else { KeyCode::ArrowDown };
                    self.handle_press(code);
                    self.pad.repeat = if self.pad.fresh {
                        gamepad::REPEAT_FIRST
                    } else {
                        gamepad::REPEAT_AGAIN
                    };
                    self.pad.fresh = false;
                }
            } else {
                self.pad.repeat = 0.0;
                self.pad.fresh = true;
            }
        }

        let (look_x, look_up) = self.pad.right_stick();
        if look_x != 0.0 || look_up != 0.0 {
            if !self.input.mouse_captured {
                self.set_capture(true);
            }
            // Screen y grows downward; stick up looks up.
            self.input.add_mouse_delta(
                look_x * gamepad::LOOK_SPEED * dt,
                -look_up * gamepad::LOOK_SPEED * dt,
            );
        }
    }

    /// Show or clear the controller overlay.
    /// Redraw the on-screen keyboard, or take it down.
    ///
    /// Sits under the terminal rather than over it: the point of the board is
    /// to type into a line you can still read.
    fn refresh_osk(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.osk.open || !active.terminal.open {
            active.renderer.clear_overlay(OSK_SLOT);
            return;
        }
        let pixels = osk::render_osk(&active.osk);
        let (width, height) = active.renderer.size();
        let panel_width = osk::OSK_WIDTH as f32 * osk::OSK_SCALE;
        let panel_height = osk::OSK_HEIGHT as f32 * osk::OSK_SCALE;
        let terminal_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            OSK_SLOT,
            &active.context.device,
            &active.context.queue,
            (osk::OSK_WIDTH, osk::OSK_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 + terminal_height) / 2.0 + 8.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    fn refresh_pad_help(&mut self) {
        let show = self.pad.help;
        let Some(active) = &mut self.active else { return };
        if !show {
            active.renderer.clear_overlay(PAD_SLOT);
            return;
        }
        let pixels = gamepad::render_pad_help();
        let (width, height) = active.renderer.size();
        let panel_width = gamepad::PAD_WIDTH as f32 * gamepad::PAD_SCALE;
        let panel_height = gamepad::PAD_HEIGHT as f32 * gamepad::PAD_SCALE;
        active.renderer.set_overlay(
            PAD_SLOT,
            &active.context.device,
            &active.context.queue,
            (gamepad::PAD_WIDTH, gamepad::PAD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    fn handle_press(&mut self, code: KeyCode) {
        // The handheld, while open, owns the keyboard the same way the shop
        // does: Enter must open a feed, never start a dig.
        if self.active.as_ref().is_some_and(|active| active.device.open) {
            let (roster_len, selected) = {
                let Some(active) = &self.active else { return };
                let from = active.player.eye_position();
                let roster = active.mining.roster(from);
                (roster.len(), active.device.selected(&roster))
            };
            let on_kestrel_page = self
                .active
                .as_ref()
                .is_some_and(|active| active.device.page == device::Page::Kestrel);
            // The arcade takes the keyboard for itself: its controls are
            // *held* rather than pressed, read straight off the input state
            // every frame, so the only thing this handler must do on that
            // page is keep out of the way. Enter opening a drone feed
            // mid-firefight would be a comedy.
            let on_arcade_page = self
                .active
                .as_ref()
                .is_some_and(|active| active.device.page == device::Page::Arcade);
            if on_arcade_page {
                match code {
                    KeyCode::Tab => {
                        let Some(active) = &mut self.active else { return };
                        active.device.turn_page();
                    }
                    KeyCode::KeyV | KeyCode::Escape => {
                        let Some(active) = &mut self.active else { return };
                        active.device.close();
                    }
                    _ => {}
                }
                return;
            }
            match code {
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let rows = if on_kestrel_page {
                        self.active
                            .as_ref()
                            .map_or(0, |active| Self::scout_rows(active).len())
                    } else {
                        roster_len
                    };
                    let Some(active) = &mut self.active else { return };
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    active.device.move_cursor(delta, rows);
                }
                KeyCode::Enter | KeyCode::NumpadEnter if on_kestrel_page => {
                    self.order_kestrel();
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    if let Some(machine) = selected {
                        self.open_feed(machine);
                    }
                }
                KeyCode::KeyR => {
                    if let Some(machine) = selected {
                        self.open_feed(machine);
                        self.take_or_release_control();
                    }
                }
                KeyCode::Tab => {
                    let Some(active) = &mut self.active else { return };
                    active.device.turn_page();
                }
                KeyCode::KeyV | KeyCode::Escape => {
                    // Put it down. The overlay stays up until the unit has
                    // finished dropping out of frame — see `frame`.
                    let Some(active) = &mut self.active else { return };
                    active.device.close();
                }
                _ => {}
            }
            return;
        }

        // The fabricator panel, while open, owns the keyboard the same way
        // the shop does.
        if self.active.as_ref().is_some_and(|active| active.printer.open) {
            match code {
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let Some(active) = &mut self.active else { return };
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    active.printer.move_cursor(delta);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => self.start_print(),
                KeyCode::KeyE | KeyCode::Escape => {
                    let Some(active) = &mut self.active else { return };
                    active.printer.close();
                    active.renderer.clear_overlay(PRINT_SLOT);
                }
                _ => {}
            }
            return;
        }

        // The terminal owns the keyboard outright while it is open, and it
        // has to come first: every other panel here answers to single letters,
        // and a console that dropped a key because the letter happened to
        // mean something elsewhere would be unusable.
        if self.active.as_ref().is_some_and(|active| active.terminal.open) {
            match code {
                KeyCode::Escape => {
                    let Some(active) = &mut self.active else { return };
                    active.terminal.close();
                    active.osk.close();
                    active.renderer.clear_overlay(TERM_SLOT);
                    active.renderer.clear_overlay(OSK_SLOT);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => self.run_typed_command(),
                KeyCode::Backspace => {
                    let Some(active) = &mut self.active else { return };
                    active.terminal.backspace();
                }
                KeyCode::Delete => {
                    let Some(active) = &mut self.active else { return };
                    active.terminal.delete();
                }
                // The pad's own keyboard, raised and put away on the one
                // key no letter can produce. A keyboard player never needs
                // it and never sees it.
                KeyCode::Tab => {
                    let Some(active) = &mut self.active else { return };
                    if active.osk.open {
                        active.osk.close();
                        active.renderer.clear_overlay(OSK_SLOT);
                    } else {
                        active.osk.open();
                    }
                    self.refresh_osk();
                }
                // While the board is up the arrows walk it; otherwise they
                // do what they always did — the caret and the history.
                KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let Some(active) = &mut self.active else { return };
                    if active.osk.open {
                        let (dx, dy) = match code {
                            KeyCode::ArrowLeft => (-1, 0),
                            KeyCode::ArrowRight => (1, 0),
                            KeyCode::ArrowUp => (0, -1),
                            _ => (0, 1),
                        };
                        active.osk.move_cursor(dx, dy);
                        self.refresh_osk();
                        return;
                    }
                    match code {
                        KeyCode::ArrowLeft => active.terminal.move_caret(-1),
                        KeyCode::ArrowRight => active.terminal.move_caret(1),
                        KeyCode::ArrowUp => active.terminal.recall(-1),
                        _ => active.terminal.recall(1),
                    }
                }
                KeyCode::Home => {
                    let Some(active) = &mut self.active else { return };
                    active.terminal.caret_home();
                }
                KeyCode::End => {
                    let Some(active) = &mut self.active else { return };
                    active.terminal.caret_end();
                }
                KeyCode::PageUp => {
                    let Some(active) = &mut self.active else { return };
                    active.terminal.scroll_by(terminal::WINDOW as i32 / 2);
                }
                KeyCode::PageDown => {
                    let Some(active) = &mut self.active else { return };
                    active.terminal.scroll_by(-(terminal::WINDOW as i32) / 2);
                }
                _ => {}
            }
            return;
        }

        // The bank's ledger: two columns, one cursor, deposit and draw.
        if self.active.as_ref().is_some_and(|active| active.banks.open) {
            match code {
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let Some(active) = &mut self.active else { return };
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    let town = active.banks.town.unwrap_or_default();
                    let rows = active
                        .banks
                        .rows(town, active.mining.fleet.base.as_ref().map(|base| &base.stockpile))
                        .len();
                    active.banks.move_cursor(delta, rows);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => self.move_at_the_bank(true),
                KeyCode::Backspace | KeyCode::Delete => self.move_at_the_bank(false),
                KeyCode::KeyE | KeyCode::Escape => {
                    let Some(active) = &mut self.active else { return };
                    active.banks.close();
                    active.renderer.clear_overlay(VAULT_SLOT);
                }
                _ => {}
            }
            return;
        }

        // The electrolyser's panel, on the fabricator's pattern.
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.electrolyser.open)
        {
            match code {
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let Some(active) = &mut self.active else { return };
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    active.electrolyser.move_cursor(delta);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => self.start_run(),
                KeyCode::KeyE | KeyCode::Escape => {
                    let Some(active) = &mut self.active else { return };
                    active.electrolyser.close();
                    active.renderer.clear_overlay(FUEL_SLOT);
                }
                _ => {}
            }
            return;
        }

        // The ward: two rows, one key each.
        if self.active.as_ref().is_some_and(|active| active.clinic.open) {
            match code {
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let Some(active) = &mut self.active else { return };
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    active.clinic.move_cursor(delta);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => self.take_the_ward(),
                KeyCode::KeyE | KeyCode::Escape => {
                    let Some(active) = &mut self.active else { return };
                    active.clinic.close();
                    active.renderer.clear_overlay(WARD_SLOT);
                }
                _ => {}
            }
            return;
        }

        // The wellhead's panel. One row and one key: a hole is a decision,
        // not a menu.
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.well_panel.open)
        {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => self.spud_in(),
                KeyCode::KeyE | KeyCode::Escape => {
                    let Some(active) = &mut self.active else { return };
                    active.well_panel.close();
                    active.renderer.clear_overlay(WELL_SLOT);
                }
                _ => {}
            }
            return;
        }

        // A live feed: the sticks belong to the machine.
        if self.active.as_ref().is_some_and(|active| active.device.feed().is_some()) {
            match code {
                KeyCode::KeyR => self.take_or_release_control(),
                KeyCode::Escape => self.close_feed(),
                KeyCode::KeyV => {
                    let Some(active) = &mut self.active else { return };
                    active.device.open_list();
                }
                _ => {}
            }
            return;
        }

        // The beacon console, while open, owns the keyboard for the same
        // reason the shop does.
        if self.active.as_ref().is_some_and(|active| active.board.open) {
            let Some(active) = &mut self.active else { return };
            // Taken out and put back so the console's own data can be lent to
            // `confirm` alongside a mutable borrow of the ledger beside it.
            let Some((here, postings, runs)) = active.console.take() else {
                active.board.close();
                return;
            };
            let rows = board::Board::rows_with_runs(here.centre, &postings, &active.ledger, &runs);
            // The ballot page owns the keyboard on its own terms: its rows
            // are seats rather than postings, and its Enter stands for
            // office rather than signing for freight.
            if active.board.page == board::Page::Ballot {
                match code {
                    KeyCode::Tab => active.board.turn_page(),
                    KeyCode::ArrowUp | KeyCode::ArrowDown => {
                        let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                        active.board.move_cursor(delta, office::OFFICES.len());
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter => {
                        Self::put_your_name_in(active, &here);
                    }
                    KeyCode::KeyE | KeyCode::Escape => {
                        active.board.close();
                        active.renderer.clear_overlay(BOARD_SLOT);
                    }
                    _ => {}
                }
                active.console = Some((here, postings, runs));
                return;
            }
            match code {
                KeyCode::Tab => active.board.turn_page(),
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    active.board.move_cursor(delta, rows.len());
                }
                // A trade run settles against the network and the base pile,
                // not against the ledger, so it takes a different path.
                KeyCode::Enter | KeyCode::NumpadEnter
                    if matches!(
                        active.board.selected(&rows),
                        Some(board::Row::Ship { .. })
                    ) =>
                {
                    let Some(board::Row::Ship { good, to }) = active.board.selected(&rows) else {
                        unreachable!("just matched a trade run")
                    };
                    let now = active.journal.tick();
                    let pile = active
                        .mining
                        .fleet
                        .base
                        .as_mut()
                        .map(|base| &mut base.stockpile);
                    active
                        .board
                        .ship(&here, good, to, pile, &mut active.economy, now);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    // Snapshot the sweep record first: the closure and the
                    // base pile both live inside the fleet.
                    let swept: std::collections::HashSet<vx_agent::Sector> =
                        active.mining.fleet.surveyed_sectors().collect();
                    let pile = active
                        .mining
                        .fleet
                        .base
                        .as_mut()
                        .map(|base| &mut base.stockpile);
                    active.board.confirm(
                        &here,
                        &postings,
                        &mut active.ledger,
                        pile,
                        &mut active.wallet,
                        &|at| swept.contains(&vx_agent::Sector::containing(at.0, at.1)),
                    );
                }
                KeyCode::KeyE | KeyCode::Escape => {
                    active.board.close();
                    active.renderer.clear_overlay(BOARD_SLOT);
                }
                _ => {}
            }
            active.console = Some((here, postings, runs));
            return;
        }

        // The welcome panel is the first thing a new player sees, so while it
        // is up it owns the keyboard outright.
        if self.active.as_ref().is_some_and(|active| active.intro.open) {
            let Some(active) = &mut self.active else { return };
            match code {
                KeyCode::ArrowUp => active.intro.scroll_by(-1),
                KeyCode::ArrowDown => active.intro.scroll_by(1),
                KeyCode::KeyE | KeyCode::Escape | KeyCode::Enter => {
                    active.intro.open = false;
                    active.renderer.clear_overlay(INTRO_SLOT);
                    intro::mark_seen();
                }
                _ => {}
            }
            return;
        }

        // The operator's console outranks everything: it is the tool you
        // reach for when the game underneath is misbehaving.
        #[cfg(feature = "gold")]
        {
            if code == KeyCode::F10 && self.gold_enabled {
                if let Some(active) = &mut self.active {
                    active.gold.toggle();
                    if !active.gold.open {
                        active.gold_hash = None;
                        active.renderer.clear_overlay(GOLD_SLOT);
                    }
                }
                return;
            }
            if self.active.as_ref().is_some_and(|active| active.gold.open) {
                self.gold_key(code);
                return;
            }
        }

        // The lockbox panel, while open, owns the keyboard.
        if self.active.as_ref().is_some_and(|active| active.permit_panel.open) {
            let Some(active) = &mut self.active else { return };
            match code {
                KeyCode::KeyE | KeyCode::Escape => {
                    active.permit_panel.close();
                    active.picking = false;
                    active.renderer.clear_overlay(PERMIT_SLOT);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => active.picking = true,
                _ => {}
            }
            return;
        }

        // The chest panel, while open, owns the keyboard.
        if self.active.as_ref().is_some_and(|active| active.home_panel.open) {
            let Some(active) = &mut self.active else { return };
            match code {
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let rows = active.homestead.chest.entries().count();
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    active.home_panel.move_cursor(delta, rows);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let base = active
                        .mining
                        .fleet
                        .base
                        .as_mut()
                        .map(|base| &mut base.stockpile);
                    active.home_panel.confirm(&mut active.homestead, base);
                }
                KeyCode::KeyE | KeyCode::Escape => {
                    active.home_panel.close();
                    active.renderer.clear_overlay(HOME_SLOT);
                }
                _ => {}
            }
            return;
        }

        // The shop, while open, owns the keyboard: Enter must trade, never
        // fall through to `start_mining`.
        if self.active.as_ref().is_some_and(|active| active.shop.open) {
            let Some(active) = &mut self.active else { return };
            match code {
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    let pile = active
                        .mining
                        .fleet
                        .base
                        .as_ref()
                        .map(|base| &base.stockpile);
                    let market = active
                        .counter
                        .map(|site| active.economy.market(&site, active.journal.tick()).clone());
                    let Some(market) = market else { return };
                    let rows = shop::Shop::rows(
                        pile,
                        &active.wallet,
                        &market,
                        &active.garage,
                        &active.arsenal,
                        &active.intrusion,
                        active.skills.level(skills::SECURITY),
                        &active.offers,
                    )
                    .len();
                    let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                    active.shop.move_cursor(delta, rows);
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    let pile = active
                        .mining
                        .fleet
                        .base
                        .as_mut()
                        .map(|base| &mut base.stockpile);
                    let Some(site) = active.counter else { return };
                    let now = active.journal.tick();
                    // The market is cloned out so the mail context can borrow
                    // the whole economy mutably beside it; sales write through
                    // `market_mut` below, so nothing here loses a write.
                    let mut market = active.economy.market_mut(&site, now).clone();
                    let offers = active.offers.clone();
                    let security = active.skills.level(skills::SECURITY);
                    let mut post = shop::MailContext {
                        economy: &mut active.economy,
                        mailbox_held: active.homestead.mailbox.total(),
                        now,
                    };
                    let compact_before = active.wallet.credits();
                    // A town with paper out on you will not deal with you.
                    let closed = active.warrants.pending_in(site.centre);
                    active.shop.confirm(
                        pile,
                        &mut active.wallet,
                        &mut market,
                        &mut active.garage,
                        &mut active.arsenal,
                        &mut active.intrusion,
                        security,
                        &offers,
                        Some(&mut post),
                        active.reputation.compact(),
                        !closed,
                    );
                    *active.economy.market_mut(&site, now) = market;
                    // Honest trade is how a county comes to know you: a
                    // trickle per sale, seasons to matter.
                    if active.wallet.credits() > compact_before {
                        let earned = active.wallet.credits() - compact_before;
                        let band = active.reputation.with_compact(reputation::TRADE_COMPACT);
                        Self::note_band(active, "THE TOWNS", band);
                        // And how one person comes to know you. The faction
                        // ledger is the county's opinion; this is the opinion
                        // of whoever was actually on the other side of the
                        // counter, which is the one that opens their door.
                        Self::book_the_trade(active, &site, earned);
                    }
                }
                KeyCode::KeyE | KeyCode::Escape => {
                    active.shop.close();
                    active.renderer.clear_overlay(SHOP_SLOT);
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Escape => self.set_capture(false),
            KeyCode::KeyE => self.interact(),
            KeyCode::KeyF => {
                if let Some(active) = &mut self.active {
                    active.mode = active.mode.toggled();
                    // Entering walk mode drops the player from wherever the
                    // camera was, so clear any stale fall speed.
                    active.player.velocity = glam::DVec3::ZERO;
                    log::info!("movement mode: {:?}", active.mode);
                }
            }
            KeyCode::KeyV => {
                if let Some(active) = &mut self.active {
                    active.device.open_list();
                }
            }
            KeyCode::KeyC => {
                if let Some(active) = &mut self.active {
                    active.view = active.view.cycled();
                    log::info!("view: {:?}", active.view);
                }
            }
            KeyCode::KeyT => {
                if let Some(active) = &mut self.active {
                    active.terminal.toggle();
                    if !active.terminal.open {
                        // The board belongs to the line it types into.
                        active.osk.close();
                        active.renderer.clear_overlay(TERM_SLOT);
                        active.renderer.clear_overlay(OSK_SLOT);
                    }
                }
            }
            KeyCode::KeyL => {
                if let Some(active) = &mut self.active {
                    active.optics.cycle();
                    let line = active.optics.label().unwrap_or("LIGHTS OFF");
                    active.greeting = Some((line.to_string(), Instant::now()));
                }
            }
            KeyCode::F3 => {
                if let Some(active) = &mut self.active {
                    active.debug_open = !active.debug_open;
                    if !active.debug_open {
                        active.renderer.clear_overlay(DEBUG_SLOT);
                    }
                }
            }
            KeyCode::F5 => self.save_world(),
            KeyCode::KeyM => self.mark_target(),
            KeyCode::Tab => {
                if let Some(active) = &mut self.active {
                    active.mining.cycle_method();
                    if let Some(status) = active.mining.status() {
                        log::info!("{status}");
                    }
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => self.start_mining(),
            KeyCode::KeyN => {
                if let Some(active) = &mut self.active {
                    active.map.visible = !active.map.visible;
                    if !active.map.visible {
                        active.renderer.clear_overlay(MAP_SLOT);
                    } else {
                        active.map.invalidate();
                    }
                }
            }
            KeyCode::BracketLeft => {
                if let Some(active) = &mut self.active {
                    active.map.zoom_out();
                    active.map.invalidate();
                }
            }
            KeyCode::BracketRight => {
                if let Some(active) = &mut self.active {
                    active.map.zoom_in();
                    active.map.invalidate();
                }
            }
            KeyCode::KeyG => {
                if let Some(active) = &mut self.active {
                    let at = active.camera.position;
                    let (x, z) = (at.x.floor() as i32, at.z.floor() as i32);
                    if active.mining.dispatch_scan(x, z) {
                        log::info!("flier dispatched to scan this sector");
                    } else {
                        log::info!("the flier is busy");
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(active) = &mut self.active {
                    active.mining.cancel(&mut active.world);
                    active.journal.record(Command::Cancel);
                    log::info!("mining plan cancelled");
                }
            }
            // Number keys pick a block to build with.
            // The belt: blocks on one to six, then the equipment — the
            // launcher on seven, the fabricator on eight, the electrolyser on
            // nine and the pump on zero. Six used to be unreachable, which
            // quietly made the chest unplaceable.
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit8
            | KeyCode::Digit9
            | KeyCode::Digit0 => {
                if let Some(active) = &mut self.active {
                    let slot = match code {
                        KeyCode::Digit1 => 0,
                        KeyCode::Digit2 => 1,
                        KeyCode::Digit3 => 2,
                        KeyCode::Digit4 => 3,
                        KeyCode::Digit5 => 4,
                        KeyCode::Digit6 => 5,
                        KeyCode::Digit8 => 6,
                        KeyCode::Digit9 => 7,
                        _ => 8,
                    };
                    if slot < active.palette.len() {
                        active.selected = slot;
                        let name = &active.world.registry().get_or_air(active.palette[slot]).name;
                        log::info!("selected {name}");
                    }
                }
            }
            // The launcher, in and out of hand.
            KeyCode::Digit7 => {
                if let Some(active) = &mut self.active {
                    if active.arsenal.owned {
                        active.arsenal.equipped = !active.arsenal.equipped;
                        active.digging = None;
                        active.greeting = Some((
                            if active.arsenal.equipped {
                                format!("LAUNCHER OUT. {} SLUGS", active.arsenal.ammo)
                            } else {
                                "LAUNCHER SLUNG".to_string()
                            },
                            Instant::now(),
                        ));
                    } else {
                        active.greeting = Some((
                            "NO LAUNCHER. THE SHOP COUNTER SELLS THEM".into(),
                            Instant::now(),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    /// Mark a corner of a body on whatever the player is looking at.
    fn mark_target(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some(hit) = raycast_solid(
            &active.world,
            active.world.registry(),
            active.camera.position,
            active.camera.forward(),
            Self::REACH,
        ) else {
            return;
        };
        active.mining.mark(&mut active.world, hit.block);
        match active.mining.status() {
            Some(status) => log::info!("{status}"),
            None => log::info!("marked {:?}; mark a second corner", hit.block),
        }
    }

    /// Put a drone on the selected plan.
    fn start_mining(&mut self) {
        let Some(active) = &mut self.active else { return };
        if active.mining.is_running() {
            return;
        }
        let area = active.mining.area();
        let crew = active.garage.owned(garage::DRONE);
        if crew == 0 {
            active.greeting = Some((
                "NO DRONES. BUY ONE AT THE SHOP COUNTER".into(),
                Instant::now(),
            ));
            log::info!("no drones owned");
            return;
        }
        // Ask before dispatching, rather than letting a drone find out block
        // by block and stall halfway through somebody's kitchen wall.
        if let Some(area) = area {
            let blocked = active
                .permits
                .borrow()
                .blocked_span(area.min, area.max);
            if let Some(claim) = blocked {
                active.greeting = Some((
                    format!("THAT DIG CROSSES {}", claim.label),
                    Instant::now(),
                ));
                log::info!("dispatch refused: the area overlaps {}", claim.label);
                return;
            }
        }
        match active.mining.start(&mut active.world, crew) {
            Some(method) => {
                if let Some(area) = area {
                    active.journal.record(Command::Dispatch { area, method, crew });
                }
                log::info!("digging: {} with a crew of {crew}", method.name());
            }
            None => log::info!("nothing marked to dig"),
        }
    }

    /// Press E at a piece of town furniture: the counter opens the shop, the
    /// console at the foot of the mast opens the beacon board.
    ///
    /// The town behind a console is *derived* from where the console is, not
    /// stored with it — the same lattice the world was generated from answers
    /// "which town is this", which is why a beacon works in a town that was
    /// built on the spot two seconds ago.
    fn interact(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some(hit) = raycast_solid(
            &active.world,
            active.world.registry(),
            active.camera.position,
            active.camera.forward(),
            Self::REACH,
        ) else {
            // Nothing solid in reach: maybe a person is. A surrendered
            // holder outranks a chat — hands up is an offer, and E accepts
            // it. The board pays per head, which is the quiet track's
            // payoff: a shelter cleared without a body.
            if let Some(active) = &mut self.active {
                if let Some(pay) = active
                    .garrisons
                    .arrest_near(active.player.position, 3.0)
                {
                    active.wallet.earn(pay);
                    let line = format!("TAKEN IN - {pay} CREDITS FROM THE BOARD");
                    active.terminal.say(terminal::Kind::Note, line.clone());
                    active.greeting = Some((line, Instant::now()));
                    // Civic service to one people, betrayal to the other.
                    let compact = active.reputation.with_compact(reputation::CAPTURE_COMPACT);
                    Self::note_band(active, "THE TOWNS", compact);
                    let holdouts =
                        active.reputation.with_holdouts(reputation::CAPTURE_HOLDOUTS);
                    Self::note_band(active, "THE SHELTERS", holdouts);
                    return;
                }
            }
            self.chat_up_the_street();
            return;
        };
        match active.world.registry().get_or_air(hit.id).name.as_str() {
            "engine:counter" => {
                // A counter is a *local* market now, so it has to know which
                // town it stands in — derived from where it is, exactly as the
                // beacon console below does it.
                let column = (hit.block.x, hit.block.z);
                match active
                    .world
                    .towns_near(column, vx_world::town::REACH)
                    .into_iter()
                    .next()
                {
                    Some(site) => {
                        // Price the mail shelf while we are here: for each
                        // good, the cheapest other town in radio range with a
                        // parcel to spare. Nearest wins a tie — `towns_near`
                        // is already sorted by distance, and strict `<` keeps
                        // the first seen.
                        let now = active.journal.tick();
                        let neighbours =
                            active.world.towns_near(site.centre, RADIO_RANGE);
                        let home = vx_world::town::home_site().centre;
                        let mut offers = Vec::new();
                        for good in 0..economy::GOODS.len() {
                            let mut best: Option<shop::Offer> = None;
                            for other in neighbours
                                .iter()
                                .filter(|other| other.centre != site.centre)
                            {
                                let market = active.economy.market(other, now);
                                if market.stock(good) < economy::PARCEL as f32 {
                                    continue;
                                }
                                let unit_price = market.price(good);
                                if best
                                    .as_ref()
                                    .is_some_and(|held| unit_price >= held.unit_price)
                                {
                                    continue;
                                }
                                best = Some(shop::Offer {
                                    good,
                                    source: *other,
                                    unit_price,
                                    freight: economy::Shipment::travel_ticks(
                                        other.centre,
                                        home,
                                    ) / 10,
                                });
                            }
                            offers.extend(best);
                        }
                        active.offers = offers;
                        active.counter = Some(site);
                        active.shop.open_at_counter();
                    }
                    None => log::warn!("a counter at {column:?} with no town behind it"),
                }
            }
            "engine:beacon" => {
                let column = (hit.block.x, hit.block.z);
                let Some(site) = active
                    .world
                    .towns_near(column, vx_world::town::REACH)
                    .into_iter()
                    .next()
                else {
                    log::warn!("a beacon at {column:?} with no town behind it");
                    return;
                };
                let neighbours = active.world.towns_near(site.centre, RADIO_RANGE);
                let postings = beacon::postings_for(&site, &neighbours);
                // What this town will pay somebody to carry, and where to: the
                // goods it is sitting on that a neighbour is short of.
                let now = active.journal.tick();
                let mut runs = Vec::new();
                for good in 0..economy::GOODS.len() {
                    let Some(target) = neighbours
                        .iter()
                        .filter(|other| other.centre != site.centre)
                        .find(|other| active.economy.market(other, now).wants(good))
                    else {
                        continue;
                    };
                    runs.push(board::Run {
                        good,
                        to: target.centre,
                        name: target.name.to_string(),
                    });
                }
                active.ledger.visit(site.centre);
                active.console = Some((site, postings, runs));
                active.board.open_at_beacon();
            }
            "engine:permit_box_i" | "engine:permit_box_ii" | "engine:permit_box_iii" => {
                let claim = active.permits.borrow().claim_here(hit.block);
                match claim {
                    Some(claim) => active.permit_panel.open_at(hit.block, claim),
                    None => log::warn!("a lockbox at {:?} with no claim behind it", hit.block),
                }
            }
            "engine:printer" => {
                active.printer.open_at(hit.block);
            }
            "engine:electrolyser" => {
                active.electrolyser.open_at(hit.block);
            }
            "engine:pump" => {
                // One switch, so no panel: a machine with a single state does
                // not need a screen to say what that state is. It says it in
                // the line it gives you and in the water coming out the top.
                let running = active.pumps.contains(&hit.block);
                let line: String = if running {
                    active.pumps.retain(|other| *other != hit.block);
                    "PUMP OFF".into()
                } else {
                    active.pumps.push(hit.block);
                    // Wake whatever it is standing in, so the first lift has
                    // somewhere to come from and somewhere to go.
                    journal::wake_water(&mut active.water, &mut active.world, hit.block);
                    "PUMP RUNNING - IT LIFTS WHAT IT CAN REACH OUT OF THE TOP".into()
                };
                active.journal.record(Command::Pump {
                    at: hit.block,
                    on: !running,
                });
                active.terminal.say(terminal::Kind::Note, line.clone());
                active.greeting = Some((line, Instant::now()));
            }
            "engine:wellhead" => {
                active.well_panel.open_at(hit.block);
            }
            "engine:ward_cot" => {
                active.clinic.open_at(hit.block);
            }
            "engine:vault" => {
                // Which town's strongroom this is comes from where it stands,
                // exactly as the counter derives its market — a vault and a
                // market in the same town must never disagree about which
                // town that is.
                match active
                    .world
                    .towns_near((hit.block.x, hit.block.z), vx_world::town::REACH)
                    .into_iter()
                    .next()
                {
                    Some(site) => active.banks.open_at(hit.block, site.centre),
                    None => {
                        active.greeting =
                            Some(("THIS VAULT BELONGS TO NO TOWN".into(), Instant::now()))
                    }
                }
            }
            "engine:supply_cache" => {
                // Whatever the crate holds goes straight onto the pile — the
                // one store this game has — and the crate goes with it. The
                // order is journalled because it changes the ground.
                if active.mining.fleet.base.is_none() {
                    active.greeting =
                        Some(("NO BASE PILE TO HAUL IT TO".to_string(), Instant::now()));
                    return;
                }
                match break_block(&mut active.world, &active.events, hit.block) {
                    Ok(_) => {
                        active.journal.record(Command::Salvage { at: hit.block });
                        Self::pay_out_cache(active, hit.block);
                    }
                    Err(error) => {
                        log::debug!("could not open the cache at {:?}: {error}", hit.block);
                    }
                }
            }
            "engine:chest" => {
                active.home_panel.open_at_chest();
            }
            "engine:mailbox" => {
                // Collect in place, no panel: everything moves into the chest,
                // which works whether the chest block stands or is packed — a
                // stockpile is a stockpile wherever it lives.
                let moved = active.homestead.collect();
                let line = if moved > 0 {
                    format!("COLLECTED {moved} GOODS INTO YOUR CHEST")
                } else {
                    "MAILBOX EMPTY".to_string()
                };
                active.greeting = Some((line, Instant::now()));
            }
            _ => {}
        }
    }

    /// Turn this frame's movement intent — keys or the pad's stick, through
    /// the axes both feed — into one tick of pilot intent.
    ///
    /// Movement is camera-relative, exactly like walking: forward is where the
    /// machine is looking, strafe is that rotated a quarter turn. The result
    /// is a cardinal, because the simulation underneath moves in whole cells.
    fn pilot_command(
        camera: &Camera,
        input: &InputState,
        cutting: bool,
    ) -> vx_agent::PilotCommand {
        use vx_agent::Heading;

        let axes = input.movement_axes();
        let forward = camera.forward_level();
        let looking = Heading::from_look(forward.x, forward.z);

        // Forward wins over strafe when both are held: one cardinal per tick,
        // and picking the one the player is facing is the least surprising.
        let heading = if axes.z.abs() > 0.1 {
            looking.map(|heading| if axes.z > 0.0 { heading } else { heading.rotated(2) })
        } else if axes.x.abs() > 0.1 {
            looking.map(|heading| heading.rotated(if axes.x > 0.0 { -1 } else { 1 }))
        } else {
            None
        };

        vx_agent::PilotCommand {
            heading,
            cut: cutting,
            climb: if axes.y > 0.1 {
                1
            } else if axes.y < -0.1 {
                -1
            } else {
                0
            },
        }
    }

    /// Has the ground under the player's own feet arrived back?
    ///
    /// Piloting a machine far away re-centres streaming on it, which unloads
    /// the player's own chunks. Stepping physics before they come back drops
    /// the body through a world that is not there yet.
    fn body_chunk_ready(&self) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.world.is_loaded(chunk_at(active.player.position))
        })
    }

    /// Master override: take the wheel of whatever is being watched, or give
    /// it back. Keeps the handheld's idea of control and the simulation's in
    /// step by doing both in one place.
    /// One row of the handheld's kestrel page: a standing order, or a job
    /// for the coil if the machine is standing next to something worth
    /// working on.
    fn scout_rows(active: &Active) -> Vec<(String, ScoutRow)> {
        let mut rows: Vec<(String, ScoutRow)> = [
            ("ORBIT OVERHEAD", journal::ScoutOrder::Orbit),
            ("SORTIE WHERE I LOOK", journal::ScoutOrder::Sortie { x: 0, z: 0 }),
            ("PERCH HERE", journal::ScoutOrder::Perch { x: 0, z: 0 }),
            ("FLY VANGUARD", journal::ScoutOrder::Vanguard),
            ("DOCK", journal::ScoutOrder::Dock),
        ]
        .into_iter()
        .map(|(label, order)| (label.to_string(), ScoutRow::Stand(order)))
        .collect();

        if active.intrusion.job.is_some() {
            rows.push(("CALL THE COIL OFF".into(), ScoutRow::Abort));
            return rows;
        }
        // What is the machine actually standing next to? A short scan around
        // it, because reach is the whole rule: the operator can be anywhere
        // the link carries, the machine has to be *there*.
        let Some((_, machine_at, _)) = Self::intruder(active) else {
            return rows;
        };
        let span = intrusion::REACH.ceil() as i32;
        let centre = vx_core::BlockPos::new(
            machine_at.x.floor() as i32,
            machine_at.y.floor() as i32,
            machine_at.z.floor() as i32,
        );
        let registry = active.world.registry();
        let roost_id = registry.id_of("engine:roost");
        let security = active.skills.level(skills::SECURITY);
        for dx in -span..=span {
            for dy in -span..=span {
                for dz in -span..=span {
                    let at = centre.offset([dx, dy, dz]);
                    let id = active.world.block(at);
                    if let Some(tier) = permits::tier_of(registry, id) {
                        rows.push((
                            "HACK THIS LOCK".into(),
                            ScoutRow::Hack(journal::IntrudeOrder::Lock {
                                x: at.x,
                                y: at.y,
                                z: at.z,
                            }),
                        ));
                        let _ = tier;
                        return rows;
                    }
                    if roost_id == Some(id) {
                        for grade in intrusion::Grade::ALL {
                            if security < grade.min_security() {
                                continue;
                            }
                            rows.push((
                                format!("{} THE WATCH BOX", grade.label()),
                                ScoutRow::Hack(journal::IntrudeOrder::Roost {
                                    x: at.x,
                                    y: at.y,
                                    z: at.z,
                                    grade,
                                }),
                            ));
                        }
                        return rows;
                    }
                }
            }
        }
        rows
    }

    /// Enter on the kestrel page: run the selected row.
    fn order_kestrel(&mut self) {
        let chosen = {
            let Some(active) = &self.active else { return };
            let rows = Self::scout_rows(active);
            rows.get(active.device.cursor()).cloned()
        };
        let Some((label, row)) = chosen else { return };
        match row {
            ScoutRow::Abort => {
                let Some(active) = &mut self.active else { return };
                active.intrusion.abort();
                active
                    .journal
                    .record(Command::Intrude(journal::IntrudeOrder::Abort));
                active.device.feedback = Some("CALLED OFF".into());
            }
            ScoutRow::Hack(order) => self.begin_intrusion(order, label),
            ScoutRow::Stand(order) => self.order_scout(order, label),
        }
    }

    /// Hand a standing order to the scout, journalling it if it is taken.
    fn order_scout(&mut self, order: journal::ScoutOrder, label: String) {
        let Some(active) = &mut self.active else { return };
        let Some(kestrel) = &mut active.mining.kestrel else {
            active.device.feedback = Some("NO KESTREL ON THE PACK".into());
            return;
        };
        let player = active.player.position;
        let ahead = player + (active.camera.forward_level() * 30.0).as_dvec3();
        // The orders carrying a place are filled in here, from where the
        // player actually is and is looking, and the *filled-in* order is
        // what the log records.
        let order = match order {
            journal::ScoutOrder::Sortie { .. } => journal::ScoutOrder::Sortie {
                x: ahead.x.floor() as i32,
                z: ahead.z.floor() as i32,
            },
            journal::ScoutOrder::Perch { .. } => journal::ScoutOrder::Perch {
                x: player.x.floor() as i32,
                z: player.z.floor() as i32,
            },
            other => other,
        };
        let mode = match order {
            journal::ScoutOrder::Dock => vx_agent::KestrelMode::Docked,
            journal::ScoutOrder::Orbit => vx_agent::KestrelMode::Orbit,
            journal::ScoutOrder::Sortie { x, z } => vx_agent::KestrelMode::Sortie {
                x,
                z,
                linger: vx_agent::kestrel::SORTIE_LINGER,
            },
            journal::ScoutOrder::Perch { x, z } => vx_agent::KestrelMode::Perch { x, z },
            journal::ScoutOrder::Vanguard => vx_agent::KestrelMode::Vanguard,
        };
        if kestrel.order(mode) {
            // Accepted orders go in the log; a refused one changed nothing
            // and records nothing.
            active.journal.record(Command::Scout(order));
            active.device.feedback = Some(label);
        } else {
            active.device.feedback = Some(format!(
                "CELL RECHARGING - READY IN {}S",
                kestrel.cooldown / 8
            ));
        }
    }

    /// Put the coil on something. Refusals are the intrusion module's to
    /// name, so the panel says exactly what is missing.
    fn begin_intrusion(&mut self, order: journal::IntrudeOrder, label: String) {
        let Some(active) = &mut self.active else { return };
        let target = match order {
            journal::IntrudeOrder::Abort => return,
            journal::IntrudeOrder::Lock { x, y, z } => {
                let at = vx_core::BlockPos::new(x, y, z);
                let Some(tier) = permits::tier_of(active.world.registry(), active.world.block(at))
                else {
                    active.device.feedback = Some("THAT IS NOT A LOCK".into());
                    return;
                };
                intrusion::Target::Lock { at, tier }
            }
            journal::IntrudeOrder::Roost { x, y, z, grade } => intrusion::Target::Roost {
                at: vx_core::BlockPos::new(x, y, z),
                grade,
            },
        };
        let Some((frame, machine_at, _)) = Self::intruder(active) else {
            active.device.feedback = Some("NO MACHINE TO SEND".into());
            return;
        };
        let centre = glam::DVec3::new(
            f64::from(target.at().x) + 0.5,
            f64::from(target.at().y) + 0.5,
            f64::from(target.at().z) + 0.5,
        );
        let attempt = intrusion::Attempt {
            frame,
            fitted: active.garage.fitted(frame.coil()),
            security: active.skills.level(skills::SECURITY),
            reach: (machine_at - centre).length() as f32,
            link: (machine_at - active.player.eye_position()).length() as f32,
            target,
        };
        match intrusion::refuse(&attempt) {
            Some(reason) => active.device.feedback = Some(reason),
            None => {
                active.intrusion.begin(&attempt);
                active.journal.record(Command::Intrude(order));
                active.device.feedback = Some(label);
            }
        }
    }

    fn take_or_release_control(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some((machine, taking)) = active.device.toggle_control() else {
            return;
        };
        if taking {
            if active.mining.take_control(machine) {
                // Recorded only once the simulation has granted it, so the
                // log never claims a wheel that was refused.
                active.journal.record(Command::Wheel {
                    machine: Some(journal::MachineTag::any(machine)),
                });
                active.device.feedback = Some("CONTROL TAKEN".into());
                log::info!("took control of {machine:?}");
            } else {
                // The simulation refused; do not leave the handheld claiming
                // a wheel it does not have.
                active.device.toggle_control();
                active.device.feedback = Some("CONTROL REFUSED".into());
            }
        } else {
            active.mining.release_control();
            active.journal.record(Command::Wheel { machine: None });
            active.last_pilot = None;
            active.device.feedback = Some("CONTROL RELEASED".into());
            log::info!("handed {machine:?} back");
        }
    }

    /// Take a machine's feed, parking the player's own view.
    fn open_feed(&mut self, machine: mining::MachineRef) {
        let Some(active) = &mut self.active else { return };
        if active.device.feed().is_none() {
            active.body_yaw = active.camera.yaw;
            active.body_pitch = active.camera.pitch;
        }
        active.device.view(machine);
        active.view = ViewMode::Fpv;
    }

    /// Hang up: hand any driven machine back and return to the body.
    fn close_feed(&mut self) {
        let Some(active) = &mut self.active else { return };
        if active.device.feed().is_none() {
            return;
        }
        if active.device.hand_back().is_some() {
            active.mining.release_control();
            // Hanging up is a release like any other, and the log has to hear
            // about it or replay keeps driving a machine nobody is holding.
            active.journal.record(Command::Wheel { machine: None });
            active.last_pilot = None;
        }
        active.camera.yaw = active.body_yaw;
        active.camera.pitch = active.body_pitch;
        active.view = ViewMode::FirstPerson;
        // The body's own ground may have streamed out while we were away.
        active.resuming = true;
        active.renderer.clear_overlay(FEED_SLOT);
        active.renderer.clear_overlay(DEVICE_SLOT);
    }

    /// Rebuild and upload the handheld panel while it is open.
    fn refresh_device(&mut self) {
        let Some(active) = &mut self.active else { return };
        // Drawn while the unit is anywhere in frame, not only while it is
        // open: the screen has to stay on its glass all the way down.
        if !active.device.showing() {
            return;
        }
        let from = active.player.eye_position();
        let roster = active.mining.roster(from);
        // The handheld's map: where you are, what you own, the towns you have
        // found and the traffic in the air — over the same fog the corner
        // minimap uses.
        let now = active.journal.tick();
        let centre = (
            active.player.position.x.floor() as i32,
            active.player.position.z.floor() as i32,
        );
        let mut markers = vec![map::Marker {
            x: centre.0,
            z: centre.1,
            colour: map::colour::PLAYER,
            radius: 2,
        }];
        for town in active.ledger.visited() {
            markers.push(map::Marker {
                x: town.0,
                z: town.1,
                colour: map::colour::TOWN,
                radius: 2,
            });
        }
        for pin in active.ledger.pins() {
            markers.push(map::Marker {
                x: pin.0,
                z: pin.1,
                colour: map::colour::CONTRACT,
                radius: 2,
            });
        }
        for load in active.economy.shipments() {
            let (x, z) = load.position_at(now);
            markers.push(map::Marker {
                x: x as i32,
                z: z as i32,
                colour: map::colour::TRADE,
                radius: 1,
            });
        }
        for position in active.mining.drone_positions() {
            markers.push(map::Marker {
                x: position.x,
                z: position.z,
                colour: map::colour::DRONE,
                radius: 1,
            });
        }
        for flier in &active.mining.fleet.fliers {
            markers.push(map::Marker {
                x: flier.position.x,
                z: flier.position.z,
                colour: map::colour::FLIER,
                radius: 1,
            });
        }
        // The scout's marks, dimming as they age — a stale report should
        // not read like a live one.
        let mark_now = active.journal.tick();
        for mark in active.marks.live(mark_now) {
            markers.push(map::Marker {
                x: mark.position.x as i32,
                z: mark.position.z as i32,
                colour: map::colour::mark_aged(mark.age(mark_now)),
                radius: 1,
            });
        }
        let country = device::Country {
            world: &active.world,
            explored: &active.map,
            centre,
            markers: &markers,
        };
        let scout_rows: Vec<String> = Self::scout_rows(active)
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        let job_line = active
            .intrusion
            .job
            .map(|job| format!("COIL WORKING {:.0}%", job.fraction() * 100.0))
            .or_else(|| {
                // Nothing running: say what the town's eye is doing instead,
                // which is the number that decides whether anything should be.
                active
                    .roost
                    .as_ref()
                    .map(|roost| format!("TOWN EYE {}", roost.status()))
            });
        let scout = active.mining.kestrel.as_ref().map(|kestrel| device::ScoutReadout {
            state: mining::kestrel_state(kestrel),
            endurance: kestrel.endurance,
            cooldown: kestrel.cooldown,
            rows: scout_rows,
            job: job_line,
        });
        // On the arcade page the readout *is* the game: same buffer, same
        // glass, and `device` never has to learn what a corridor is.
        let mut pixels = if active.device.page == device::Page::Arcade {
            arcade::render(&active.arcade)
        } else {
            device::render_device(&active.device, &roster, Some(&country), scout.as_ref())
        };
        // The screen comes on as the unit arrives.
        device::dim(&mut pixels, active.device.raise);

        // And it lands on the glass rather than in the middle of the screen:
        // the four corners of the model's own screen face, projected through
        // the very camera matrix the frame was drawn with. Look around while
        // it is up and the readout goes with it, because it is *on* it.
        let (width, height) = active.renderer.size();
        let rect = if active.view.draws_viewmodel() {
            let corners = device::screen_corners(
                active.camera.local_position(),
                active.camera.forward(),
                active.camera.right(),
                active.device.raise,
            );
            device::screen_rect(
                active.camera.view_projection(),
                corners,
                (width as f32, height as f32),
            )
        } else {
            // Third person draws no viewmodel, so there is no glass out
            // there to put this on: the readout goes back to the middle of
            // the frame rather than floating where a unit is not.
            Some(device::centred(width as f32, height as f32))
        };
        let Some(rect) = rect else {
            // Still swinging up from somewhere off-frame: nothing to draw
            // yet, and a rectangle derived from a point behind the eye would
            // be a rectangle in the wrong place.
            active.renderer.clear_overlay(DEVICE_SLOT);
            return;
        };
        active.renderer.set_overlay(
            DEVICE_SLOT,
            &active.context.device,
            &active.context.queue,
            (device::DEVICE_WIDTH, device::DEVICE_HEIGHT),
            &pixels,
            rect,
        );
    }

    /// Rebuild and upload the banner sitting over a live feed.
    fn refresh_feed(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some(machine) = active.device.feed() else { return };
        let from = active.player.eye_position();
        let Some(listing) = active.mining.listing(machine, from) else { return };
        let strength = device::signal(listing.distance);
        let piloting = active.device.is_piloting();
        let pixels = device::render_feed_banner(&listing, piloting, strength);
        let (width, _) = active.renderer.size();
        let panel_width = device::BANNER_WIDTH as f32 * device::BANNER_SCALE;
        let panel_height = device::BANNER_HEIGHT as f32 * device::BANNER_SCALE;
        active.renderer.set_overlay(
            FEED_SLOT,
            &active.context.device,
            &active.context.queue,
            (device::BANNER_WIDTH, device::BANNER_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: 12.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Rebuild and upload the shop panel while it is open. Closing clears the
    /// slot in the key handler, so an unset slot keeps frames byte-identical.
    fn refresh_shop(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.shop.open {
            return;
        }
        let pile = active
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| &base.stockpile);
        let Some(site) = active.counter else { return };
        let market = active.economy.market(&site, active.journal.tick()).clone();
        let pixels = shop::render_shop(
            &active.shop,
            pile,
            &active.wallet,
            &site,
            &market,
            &active.garage,
            &active.arsenal,
            &active.intrusion,
            active.skills.level(skills::SECURITY),
            &active.offers,
            !active.warrants.pending_in(site.centre),
        );
        let (width, height) = active.renderer.size();
        let panel_width = shop::SHOP_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = shop::SHOP_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            SHOP_SLOT,
            &active.context.device,
            &active.context.queue,
            (shop::SHOP_WIDTH, shop::SHOP_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Rebuild and upload the welcome panel while it is open.
    fn refresh_intro(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.intro.open {
            return;
        }
        let pixels = intro::render_intro(&active.intro);
        let (width, height) = active.renderer.size();
        let panel_width = intro::INTRO_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = intro::INTRO_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            INTRO_SLOT,
            &active.context.device,
            &active.context.queue,
            (intro::INTRO_WIDTH, intro::INTRO_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// One key into the operator's console.
    #[cfg(feature = "gold")]
    fn gold_key(&mut self, code: KeyCode) {
        let Some(active) = &mut self.active else { return };
        let telemetry = Self::gold_telemetry(active);
        let rows = active.gold.rows(&telemetry);
        drop(telemetry);
        match code {
            KeyCode::Escape | KeyCode::F10 => {
                active.gold.toggle();
                active.gold_hash = None;
                active.renderer.clear_overlay(GOLD_SLOT);
            }
            KeyCode::Tab => active.gold.cycle_tab(),
            KeyCode::ArrowUp | KeyCode::ArrowDown => {
                let delta = if code == KeyCode::ArrowUp { -1 } else { 1 };
                // A slider being held turns the vertical axis into the value.
                if let Some((key, pending)) = active.gold.sliding {
                    let step = rows
                        .get(active.gold.cursor)
                        .and_then(|row| match row.action {
                            gold::RowAction::Slider { step, .. } => Some(step),
                            _ => None,
                        })
                        .unwrap_or(0.05);
                    active.gold.sliding =
                        Some((key, (pending - delta as f32 * step).max(0.0)));
                } else {
                    active.gold.move_cursor(delta, rows.len());
                }
            }
            KeyCode::KeyX => {
                // Reset the focused tunable to its default — as an order, so
                // the journal knows the physics changed back.
                if let Some(gold::RowAction::Slider { key, .. }) =
                    rows.get(active.gold.cursor).map(|row| row.action.clone())
                {
                    let default = tuning::Tuning::default().get(key).unwrap_or(0.0);
                    Self::gold_order(
                        active,
                        journal::Admin::SetTuning { key: key.into(), value: default },
                    );
                    active.gold.sliding = None;
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                match rows.get(active.gold.cursor).map(|row| row.action.clone()) {
                    Some(gold::RowAction::Slider { key, .. }) => {
                        match active.gold.sliding.take() {
                            // Second press commits the pending value as an
                            // order. (The note wants hold-to-slide; with a
                            // keyboard standing in for the pad, press-adjust-
                            // press is the same gesture without key-held
                            // tracking. The pad backend maps hold onto this.)
                            Some((held, pending)) if held == key => {
                                Self::gold_order(
                                    active,
                                    journal::Admin::SetTuning {
                                        key: held.into(),
                                        value: pending,
                                    },
                                );
                            }
                            _ => {
                                let current = active
                                    .movement
                                    .tuning
                                    .get(key)
                                    .unwrap_or(0.0);
                                active.gold.sliding = Some((key, current));
                            }
                        }
                    }
                    Some(gold::RowAction::Order(order)) => {
                        // The teleport-ahead row is a placeholder until now:
                        // the real target is fifty blocks down the camera's
                        // level heading, decided at the moment of the order.
                        let order = if let journal::Admin::Teleport { x: 0, y: 0, z: 0 } = order
                        {
                            let ahead = active.camera.forward_level();
                            let target = active.player.position + (ahead * 50.0).as_dvec3();
                            journal::Admin::Teleport {
                                x: target.x.floor() as i32,
                                y: (target.y.floor() as i32).max(1) + 4,
                                z: target.z.floor() as i32,
                            }
                        } else {
                            order
                        };
                        Self::gold_order(active, order);
                    }
                    Some(gold::RowAction::Note) => {
                        // Two informational rows carry a verb: the hash, and
                        // the time advance.
                        if active.gold.tab() == gold::Tab::World {
                            match active.gold.cursor {
                                1 => {
                                    active.gold_hash =
                                        Some(vx_world::world_hash(&active.world));
                                    active.gold.feedback = Some("HASHED".into());
                                }
                                3 => Self::gold_advance(active, 80),
                                _ => {}
                            }
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Journal an operator's order and apply it live.
    ///
    /// The live application must match the replay arm exactly for everything
    /// `Rebuilt` carries, or the oracle and the session disagree — that
    /// discipline is the whole panel.
    #[cfg(feature = "gold")]
    fn gold_order(active: &mut Active, order: journal::Admin) {
        use journal::Admin;
        active.journal.record(Command::Admin(order.clone()));
        let line = match &order {
            Admin::Give { good, amount } => {
                match active.mining.fleet.base.as_mut() {
                    Some(base) => {
                        base.stockpile.add(good.clone(), *amount);
                        format!("GAVE {amount} {}", shop::display_name(good))
                    }
                    // Recorded anyway: replay applies the same no-base rule.
                    None => "NO BASE SET - NOTHING GIVEN".into(),
                }
            }
            Admin::SpawnMachine { kind, count } => {
                active.garage.grant(kind, *count);
                format!("{} +{count}", kind.to_uppercase())
            }
            Admin::Teleport { x, y, z } => {
                active.player.position =
                    glam::DVec3::new(*x as f64 + 0.5, *y as f64, *z as f64 + 0.5);
                active.player.velocity = glam::DVec3::ZERO;
                active.player.on_ground = false;
                // The ground there may not be streamed in yet; the gate that
                // already protects feed hang-ups protects this too.
                active.resuming = true;
                format!("MOVED TO {x} {y} {z}")
            }
            Admin::SetStat { key, value } => {
                if key == "stamina" {
                    active.movement.stamina =
                        (*value as f32).min(active.movement.tuning.stam_max);
                    "STAMINA FILLED".into()
                } else if key == "credits" {
                    active.wallet.earn(*value);
                    format!("+{value} CR")
                } else if let Some(skill) = key.strip_prefix("xp:") {
                    active.skills.add_xp(skill, *value);
                    format!("+{value} XP {}", skill.to_uppercase())
                } else {
                    format!("UNKNOWN STAT {key}")
                }
            }
            Admin::SetStock { x, z, good, amount } => {
                let now = active.journal.tick();
                let site = active
                    .world
                    .towns_near((*x, *z), vx_world::town::REACH)
                    .into_iter()
                    .next();
                match (site, economy::good_index(good)) {
                    (Some(site), Some(index)) => {
                        let market = active.economy.market_mut(&site, now);
                        let held = market.stock(index);
                        if held < *amount as f32 {
                            market.deposit(index, *amount as f32 - held);
                        } else {
                            market.withdraw(index, held - *amount as f32);
                        }
                        format!("{} STOCK -> {amount}", shop::display_name(good))
                    }
                    _ => "NO SUCH TOWN OR GOOD".into(),
                }
            }
            Admin::SetTuning { key, value } => {
                if active.movement.tuning.set(key, *value) {
                    format!("{} = {value:.2}", tuning::label(key))
                } else {
                    format!("UNKNOWN TUNABLE {key}")
                }
            }
        };
        active.gold.feedback = Some(line);
    }

    /// Advance the simulation by whole journal ticks, on demand.
    ///
    /// Bounded per press: this is the economy fast-forward exposed as a
    /// button, and an unbounded burst would freeze the frame for as long as
    /// the operator's ambition.
    #[cfg(feature = "gold")]
    fn gold_advance(active: &mut Active, ticks: u32) {
        active.journal.record(Command::Advance { ticks });
        active.mining.advance(&mut active.world, &active.events, ticks);
        let command = active.last_move.unwrap_or_default();
        for _ in 0..ticks {
            movement::advance_journal_tick(
                &mut active.movement,
                &mut active.player,
                &active.world,
                command,
            );
        }
        active.gold.feedback = Some(format!("ADVANCED {ticks} TICKS"));
    }

    /// What the panel reads. References only; computed fresh per use.
    #[cfg(feature = "gold")]
    fn gold_telemetry<'a>(active: &'a Active) -> gold::Telemetry<'a> {
        let column = (
            active.player.position.x as i32,
            active.player.position.z as i32,
        );
        let town = active
            .world
            .towns_near(column, vx_world::town::REACH)
            .into_iter()
            .next();
        let stocks = match &town {
            Some(site) => {
                // A read, so the catch-up write is fine: the books were going
                // to be caught up by the next reader anyway.
                let mut books = active.economy.clone();
                let market = books.market(site, active.journal.tick()).clone();
                // Built by walking the catalogue, so a new good appears in
                // the panel the day it is added rather than the day someone
                // notices the panel crashing.
                let mut stocks = [0.0; economy::GOODS.len()];
                for (good, stock) in stocks.iter_mut().enumerate() {
                    *stock = market.stock(good);
                }
                stocks
            }
            None => [0.0; economy::GOODS.len()],
        };
        gold::Telemetry {
            tick: active.journal.tick(),
            position: active.player.position,
            stance: active.movement.stance.label(),
            stamina: active.movement.stamina,
            credits: active.wallet.credits(),
            bounty: active.permits.borrow().bounty,
            drones: active.garage.owned(garage::DRONE),
            fliers: active.garage.owned(garage::FLIER),
            base_total: active
                .mining
                .fleet
                .base
                .as_ref()
                .map(|base| base.stockpile.total())
                .unwrap_or(0),
            town_name: town.as_ref().map(|site| site.name.to_string()),
            town_centre: town.as_ref().map(|site| site.centre),
            stocks,
            tuning: &active.movement.tuning,
            world_hash: active.gold_hash,
        }
    }

    /// Rebuild and upload the console while it is open.
    #[cfg(feature = "gold")]
    fn refresh_gold(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.gold.open {
            return;
        }
        let pixels = {
            let telemetry = Self::gold_telemetry(active);
            gold::render_gold(&active.gold, &telemetry)
        };
        let (width, height) = active.renderer.size();
        let panel_width = gold::GOLD_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = gold::GOLD_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            GOLD_SLOT,
            &active.context.device,
            &active.context.queue,
            (gold::GOLD_WIDTH, gold::GOLD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Turn a refused edit into a line for the player, and a mark against them
    /// if anybody was looking.
    ///
    /// Only refusals naming *this* block count: bedrock returns the same error
    /// without ever emitting an event, so a stale refusal would otherwise be
    /// read as fresh and the player told that bedrock belongs to the mayor.
    ///
    /// The witness check is the flag the frame already computed. Being seen is
    /// the whole rule — crouch, get behind something, and the town never knows.
    fn charge_for_refusal(active: &mut Active, at: vx_core::BlockPos, points: u64) -> Option<String> {
        let mut permits = active.permits.borrow_mut();
        let line = permits.refusal_for(at).map(|refusal| refusal.line())?;
        let witnesses = usize::from(active.watched);
        if permits.caught(points, witnesses) {
            Some(format!("{line} - SEEN"))
        } else {
            Some(line)
        }
    }

    /// Rebuild and upload the lockbox panel while it is open.
    fn refresh_permit(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.permit_panel.open {
            return;
        }
        let pixels = permits::render_permit(&active.permit_panel, &active.permits.borrow());
        let (width, height) = active.renderer.size();
        let panel_width = permits::PERMIT_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = permits::PERMIT_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            PERMIT_SLOT,
            &active.context.device,
            &active.context.queue,
            (permits::PERMIT_WIDTH, permits::PERMIT_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Rebuild and upload the chest panel while it is open.
    fn refresh_home(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.home_panel.open {
            return;
        }
        let pixels = homestead::render_homestead(&active.home_panel, &active.homestead);
        let (width, height) = active.renderer.size();
        let panel_width = homestead::HOME_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = homestead::HOME_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            HOME_SLOT,
            &active.context.device,
            &active.context.queue,
            (homestead::HOME_WIDTH, homestead::HOME_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Rebuild and upload the beacon board while it is open.
    fn refresh_board(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.board.open {
            return;
        }
        // Taken out so the books can be borrowed mutably alongside it.
        let Some((here, postings, runs)) = active.console.take() else {
            return;
        };
        let now = active.journal.tick();
        let market = active.economy.market(&here, now).clone();
        let traffic: Vec<(i32, i32)> = active
            .economy
            .shipments()
            .iter()
            .map(|load| {
                let (x, z) = load.position_at(now);
                (x as i32, z as i32)
            })
            .collect();
        let view = board::TradeView {
            world: &active.world,
            explored: &active.map,
            traffic: &traffic,
        };
        let civic = civic_snapshot(&here, &active.warrants, &active.charters, &active.elections);
        let day = (active.journal.tick() / schedule::TICKS_PER_DAY) as u32;
        let bounty = active.permits.borrow().bounty;
        let ballot = ballot_snapshot(&here, &active.elections, &active.friends, bounty, day);
        let pixels = board::render_board(
            &active.board,
            &board::Counter {
                here: &here,
                postings: &postings,
                runs: &runs,
                market: &market,
                civic: &civic,
                ballot: &ballot,
            },
            &active.ledger,
            &active.wallet,
            Some(&view),
        );
        active.console = Some((here, postings, runs));
        let (width, height) = active.renderer.size();
        let panel_width = board::BOARD_WIDTH as f32 * board::BOARD_SCALE;
        let panel_height = board::BOARD_HEIGHT as f32 * board::BOARD_SCALE;
        active.renderer.set_overlay(
            BOARD_SLOT,
            &active.context.device,
            &active.context.queue,
            (board::BOARD_WIDTH, board::BOARD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Rebuild and upload the HUD panel. Cheap enough to run every frame,
    /// which the fast-moving drill bar wants anyway.
    fn refresh_hud(&mut self) {
        let Some(active) = &mut self.active else { return };

        // Every toast is also a log line. A toast lasts three seconds and
        // then the thing it said is gone — which is fine for "you levelled
        // up" and useless for "the crew ran dry while you were in a cave".
        // The terminal is where it stops being useless.
        if let Some((line, at)) = &active.greeting {
            if active.logged != Some(*at) {
                active.logged = Some(*at);
                let line = line.clone();
                active.terminal.say(terminal::Kind::Note, line);
            }
        }

        let level_up = active.level_up.as_ref().and_then(|(skill, level, at)| {
            (at.elapsed().as_secs_f32() < 2.5).then(|| (skill.clone(), *level))
        });
        let greeting = active.greeting.as_ref().and_then(|(line, at)| {
            (at.elapsed().as_secs_f32() < 3.0).then(|| line.clone())
        });
        let content = hud::HudContent {
            condition: active.health.readout(),
            dose: active.dose.readout(),
            dark: active.dark.present().map(|it| {
                let (taken, of) = it.wounds();
                format!("{} - {taken}/{of}", it.mood.name())
            }),
            deputies: active.posse.active() + active.garrisons.hunting(),
            skills: &active.skills,
            time: active.clock,
            reconnecting: active.resuming,
            status: active.mining.status(),
            drilling: active.digging.map(|(_, progress)| progress),
            level_up,
            greeting,
            bounty: active.permits.borrow().bounty,
            watched: active.watched,
            movement: hud::MovementReadout {
                stance: active.movement.stance.label(),
                stamina: active.movement.stamina_fraction(),
                load: active.last_move.map_or(0.0, |command| {
                    command.load as f32 / 255.0
                }),
            },
            ammo: active
                .arsenal
                .equipped
                .then_some(active.arsenal.ammo),
            panicking: active.villagers.panicking(),
            kestrel: active.mining.kestrel.as_ref().map(|kestrel| {
                let seconds = if kestrel.cooldown > 0 {
                    kestrel.cooldown / 8
                } else {
                    kestrel.endurance / 8
                };
                format!("KESTREL {} {}S", mining::kestrel_state(kestrel), seconds)
            }),
            optic: active.optics.label(),
            fuel: {
                let spare = active
                    .mining
                    .fleet
                    .base
                    .as_ref()
                    .map_or(0, |base| base.stockpile.count(fuel::CELL));
                let burners = active.mining.burners();
                active.mining.tank.readout(burners, spare)
            },
        };
        let pixels = hud::render_hud(&content);

        let (_, screen_height) = active.renderer.size();
        let margin = 12.0;
        let width = hud::HUD_WIDTH as f32 * hud::HUD_SCALE;
        let height = hud::HUD_HEIGHT as f32 * hud::HUD_SCALE;
        active.renderer.set_overlay(
            HUD_SLOT,
            &active.context.device,
            &active.context.queue,
            (hud::HUD_WIDTH, hud::HUD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: margin,
                y: screen_height as f32 - height - margin,
                width,
                height,
            },
        );
    }

    /// Draw the terminal when it is open.
    /// Assemble the F3 snapshot and draw it. The panel is a pure function
    /// of this struct; everything here only *reads*, which is the whole
    /// contract — diagnostics must never perturb what they report.
    fn refresh_debug(&mut self) {
        let fps = self.fps;
        let Some(active) = &mut self.active else { return };
        if !active.debug_open {
            return;
        }
        // A steady cadence rather than every frame: the roster walk below
        // is cheap, but a diagnostics panel that costs frames would be
        // lying about the number it is proudest of.
        if !self.frames.is_multiple_of(6) {
            return;
        }

        let at = active.player.position;
        let feet = vx_core::BlockPos::new(at.x.floor() as i32, at.y.floor() as i32, at.z.floor() as i32);
        let aimed = raycast_solid(
            &active.world,
            active.world.registry(),
            active.camera.position,
            active.camera.forward(),
            Self::REACH,
        )
        .map(|hit| {
            active
                .world
                .registry()
                .get_or_air(hit.id)
                .name
                .to_uppercase()
        });
        let rows = active.mining.roster(at);
        let worst_machine = rows
            .iter()
            .map(|row| row.condition)
            .max()
            .unwrap_or(wear::Condition::Fresh)
            .name();
        let spare = active
            .mining
            .fleet
            .base
            .as_ref()
            .map_or(0, |base| base.stockpile.count(fuel::CELL));
        let now = active.journal.tick();

        let content = debug::DebugContent {
            fps,
            position: (at.x as f32, at.y as f32, at.z as f32),
            chunk: {
                let chunk = feet.chunk();
                (chunk.x, chunk.z)
            },
            yaw: active.camera.yaw.to_degrees(),
            pitch: active.camera.pitch.to_degrees(),
            aimed,
            chunks_loaded: active.world.loaded_chunk_count(),
            chunks_drawn: active.renderer.visible_chunk_count(),
            triangles: active.renderer.triangle_count(),
            edits: active.world.edit_count(),
            composites: active.world.composite_count(),
            tick: now,
            log_entries: active.journal.entries().len(),
            day: (now / schedule::TICKS_PER_DAY) as u32,
            hhmm: active.clock.hhmm(),
            burners: active.mining.burners(),
            fuel_cells: u64::from(active.mining.tank.cells()) + spare,
            worst_machine,
            panicking: active.villagers.panicking(),
            marks: active.marks.live(now).count(),
            shots: active.shots.len(),
            deputies: active.posse.active(),
            squads: active.garrisons.squads.len(),
            hunting: active.garrisons.hunting(),
            belief: active
                .posse
                .called_out()
                .then(|| (active.posse.belief.total(), active.posse.belief.confidence())),
            hits: (active.health.hits(), health::MAX_HITS),
            bounty: active.permits.borrow().bounty,
            compact: active.reputation.compact().name(),
            holdouts: active.reputation.holdouts().name(),
            wells: (
                active.mining.wells.len(),
                active.mining.wells.producing().count(),
            ),
            rads: (active.last_rads, active.dose.rads),
            medkits: active.health.medkits(),
            dark: (
                active.dark.heat(),
                active
                    .dark
                    .present()
                    .map_or(stalker::Mood::Asleep, |it| it.mood)
                    .name(),
            ),
        };

        let pixels = debug::render_debug(&content);
        let (width, _height) = active.renderer.size();
        let panel_width = debug::DEBUG_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = debug::DEBUG_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            DEBUG_SLOT,
            &active.context.device,
            &active.context.queue,
            (debug::DEBUG_WIDTH, debug::DEBUG_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                // Top-right, clear of the HUD's top-left column.
                x: width as f32 - panel_width - 12.0,
                y: 12.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    fn refresh_terminal(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.terminal.open {
            return;
        }
        // The caret blinks on the frame counter rather than on wall time, so
        // two captures of one state are identical — the same rule every other
        // panel here follows.
        let blink = (self.frames / 30).is_multiple_of(2);
        let pixels = terminal::render_terminal(&active.terminal, blink);
        let (width, height) = active.renderer.size();
        let panel_width = terminal::TERM_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = terminal::TERM_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            TERM_SLOT,
            &active.context.device,
            &active.context.queue,
            (terminal::TERM_WIDTH, terminal::TERM_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Draw the bank's ledger when it is open.
    fn refresh_vault(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.banks.open {
            return;
        }
        let Some(town) = active.banks.town else { return };
        let name = active
            .world
            .towns_near(town, 4)
            .into_iter()
            .next()
            .map_or_else(
                || "VAULT".to_string(),
                |site| format!("{}{} VAULT", site.name.head(), site.name.tail()),
            );
        let pile = active
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| &base.stockpile);
        let pixels = bank::render_vault(&active.banks, town, &name, pile);
        let (width, height) = active.renderer.size();
        let panel_width = bank::VAULT_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = bank::VAULT_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            VAULT_SLOT,
            &active.context.device,
            &active.context.queue,
            (bank::VAULT_WIDTH, bank::VAULT_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Draw the electrolyser's panel when it is open.
    fn refresh_electrolyser(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.electrolyser.open {
            return;
        }
        let dry_shore = active
            .electrolyser
            .at
            .is_none_or(|at| !electrolysis::water_near(&active.world, at));
        let pile = active
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| &base.stockpile);
        let level = active.skills.level(skills::FABRICATION);
        let pixels =
            electrolysis::render_electrolyser(&active.electrolyser, pile, level, dry_shore);
        let (width, height) = active.renderer.size();
        let panel_width = electrolysis::FUEL_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = electrolysis::FUEL_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            FUEL_SLOT,
            &active.context.device,
            &active.context.queue,
            (electrolysis::FUEL_WIDTH, electrolysis::FUEL_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Draw the ward when its door is open.
    fn refresh_clinic(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.clinic.open {
            return;
        }
        let town = active
            .clinic
            .at
            .and_then(|at| {
                active
                    .world
                    .towns_near((at.x, at.z), vx_world::town::REACH)
                    .into_iter()
                    .next()
            })
            .map_or_else(|| "THE FRONTIER".to_string(), |site| site.name.to_string());
        let content = clinic::ClinicContent {
            town,
            cursor: active.clinic.cursor,
            condition: (active.health.hits(), health::MAX_HITS),
            rads: active.dose.rads,
            medkits: active.health.medkits(),
            credits: active.wallet.credits(),
            feedback: active.clinic.feedback.clone(),
        };
        let pixels = clinic::render_clinic(&content);
        let (width, height) = active.renderer.size();
        let panel_width = clinic::WARD_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = clinic::WARD_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            WARD_SLOT,
            &active.context.device,
            &active.context.queue,
            (clinic::WARD_WIDTH, clinic::WARD_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Draw the wellhead panel when it is open.
    fn refresh_well(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.well_panel.open {
            return;
        }
        let Some(at) = active.well_panel.at else { return };
        let pile = active
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| &base.stockpile);
        let content = well::WellContent {
            at,
            trace: vx_world::reservoir::reservoir_under(active.world.seed(), at.x, at.z).is_some(),
            hole: active.mining.wells.at(at).copied(),
            refusal: well::refuse(&active.mining.wells, at, pile),
            feedback: active.well_panel.feedback.clone(),
        };
        let pixels = well::render_well(&content);
        let (width, height) = active.renderer.size();
        let panel_width = well::WELL_WIDTH as f32 * shop::SHOP_SCALE;
        let panel_height = well::WELL_HEIGHT as f32 * shop::SHOP_SCALE;
        active.renderer.set_overlay(
            WELL_SLOT,
            &active.context.device,
            &active.context.queue,
            (well::WELL_WIDTH, well::WELL_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Draw the fabricator panel when it is open.
    fn refresh_printer(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.printer.open {
            return;
        }
        let level = active.skills.level(skills::FABRICATION);
        let pile = active
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| &base.stockpile);
        let pixels = printer::render_printer(&active.printer, pile, level, &active.wallet);
        let (width, height) = active.renderer.size();
        let panel_width = printer::PRINT_WIDTH as f32 * printer::PRINT_SCALE;
        let panel_height = printer::PRINT_HEIGHT as f32 * printer::PRINT_SCALE;
        active.renderer.set_overlay(
            PRINT_SLOT,
            &active.context.device,
            &active.context.queue,
            (printer::PRINT_WIDTH, printer::PRINT_HEIGHT),
            &pixels,
            vx_render::OverlayRect {
                x: (width as f32 - panel_width) / 2.0,
                y: (height as f32 - panel_height) / 2.0,
                width: panel_width,
                height: panel_height,
            },
        );
    }

    /// Rebuild and upload the minimap overlay, on the redraw throttle.
    fn refresh_minimap(&mut self) {
        let Some(active) = &mut self.active else { return };
        if !active.map.visible || !active.map.should_redraw() {
            return;
        }

        // On a feed the map follows the machine — it is what you are looking
        // through — and the parked body becomes a marker so you can see how
        // far you have sent it.
        let feed_at = active
            .device
            .feed()
            .and_then(|machine| active.mining.machine_position(machine));
        let centre = match feed_at {
            Some(at) => (at.x, at.z),
            None => {
                let camera = active.camera.position;
                (camera.x.floor() as i32, camera.z.floor() as i32)
            }
        };

        let mut markers = Vec::new();
        if feed_at.is_some() {
            markers.push(map::Marker {
                x: active.player.position.x.floor() as i32,
                z: active.player.position.z.floor() as i32,
                colour: map::colour::PLAYER,
                radius: 2,
            });
        }
        if let Some(base) = &active.mining.fleet.base {
            markers.push(map::Marker {
                x: base.position.x,
                z: base.position.z,
                colour: map::colour::BASE,
                radius: 2,
            });
        }
        for ping in active.mining.fleet.pings() {
            markers.push(map::Marker {
                x: ping.position.x,
                z: ping.position.z,
                colour: map::colour::PING,
                radius: 2,
            });
        }
        for position in active.mining.drone_positions() {
            markers.push(map::Marker {
                x: position.x,
                z: position.z,
                colour: map::colour::DRONE,
                radius: 1,
            });
        }
        for flier in &active.mining.fleet.fliers {
            markers.push(map::Marker {
                x: flier.position.x,
                z: flier.position.z,
                colour: map::colour::FLIER,
                radius: 1,
            });
        }
        if let Some(kestrel) = active
            .mining
            .kestrel
            .as_ref()
            .filter(|kestrel| kestrel.aloft())
        {
            markers.push(map::Marker {
                x: kestrel.craft.position.x,
                z: kestrel.craft.position.z,
                colour: map::colour::FLIER,
                radius: 1,
            });
        }
        // The scout's reports, dimming with age.
        let mark_now = active.journal.tick();
        for mark in active.marks.live(mark_now) {
            markers.push(map::Marker {
                x: mark.position.x as i32,
                z: mark.position.z as i32,
                colour: map::colour::mark_aged(mark.age(mark_now)),
                radius: 1,
            });
        }
        for town in active.ledger.visited() {
            markers.push(map::Marker {
                x: town.0,
                z: town.1,
                colour: map::colour::TOWN,
                radius: 2,
            });
        }
        // Trade traffic, wherever it is. Watching a load crawl across the map
        // toward a town you sold to is half the pleasure of the network.
        let now = active.journal.tick();
        for load in active.economy.shipments() {
            let (x, z) = load.position_at(now);
            markers.push(map::Marker {
                x: x as i32,
                z: z as i32,
                colour: map::colour::TRADE,
                radius: 1,
            });
        }
        // Contract pins draw over the fog on purpose: a posting can name a
        // town you have never seen, and going to find it is the job.
        for pin in active.ledger.pins() {
            markers.push(map::Marker {
                x: pin.0,
                z: pin.1,
                colour: map::colour::CONTRACT,
                radius: 3,
            });
        }
        // The player draws last, so nothing covers you.
        markers.push(map::Marker {
            x: centre.0,
            z: centre.1,
            colour: map::colour::PLAYER,
            radius: 1,
        });

        let pixels = map::render_map(&active.world, &active.map, centre, &markers);
        let (screen_width, _) = active.renderer.size();
        let margin = 12.0;
        let panel = map::MAP_SIZE as f32;
        active.renderer.set_overlay(
            MAP_SLOT,
            &active.context.device,
            &active.context.queue,
            (map::MAP_SIZE, map::MAP_SIZE),
            &pixels,
            vx_render::OverlayRect {
                x: screen_width as f32 - panel - margin,
                y: margin,
                width: panel,
                height: panel,
            },
        );
    }

    /// Write the world to disk, reporting how it went.
    fn save_world(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some(save) = active.save.as_ref() else {
            return;
        };
        match save.save_world(&mut active.world) {
            Ok(0) => log::info!("nothing to save"),
            Ok(count) => log::info!("saved {count} chunks to {}", save.root().display()),
            Err(error) => log::error!("could not save the world: {error}"),
        }
        match active.map.save(save.root()) {
            Ok(()) => log::info!("saved the map ({} explored chunks)", active.map.explored_count()),
            Err(error) => log::error!("could not save the map: {error}"),
        }
        if let Err(error) = active.skills.save(save.root()) {
            log::error!("could not save the skill sheet: {error}");
        }
        if let Err(error) = active.wallet.save(save.root()) {
            log::error!("could not save the wallet: {error}");
        }
        if let Err(error) = clock::save(active.clock, save.root()) {
            log::error!("could not save the clock: {error}");
        }
        if let Err(error) = active.ledger.save(save.root()) {
            log::error!("could not save the posting ledger: {error}");
        }
        if let Err(error) = active.homestead.save(save.root()) {
            log::error!("could not save the homestead: {error}");
        }
        if let Err(error) = active.permits.borrow().save(save.root()) {
            log::error!("could not save the permits: {error}");
        }
        // The region files above are the keyframe; the journal is everything
        // ordered since. Both are written every save for now — the journal
        // earns its keep as a determinism oracle first, and only once it has
        // been proving itself against real sessions is it worth letting it
        // *replace* region writes on disk.
        // Keyframe only once the tail is long enough to be worth bounding.
        // Regions are written every save regardless, so loading never depends
        // on the journal — it accumulates alongside as the record `--replay`
        // checks the simulation against.
        if active.journal.wants_keyframe() {
            active.journal.keyframed(vx_world::world_hash(&active.world));
        }
        if let Err(error) = active.journal.save(save.root()) {
            log::error!("could not save the command journal: {error}");
        }
        if let Err(error) = active.garage.save(save.root()) {
            log::error!("could not save the garage: {error}");
        }
        if let Err(error) = active.arsenal.save(save.root()) {
            log::error!("could not save the arsenal: {error}");
        }
        if let Err(error) = active.intrusion.save(save.root()) {
            log::error!("could not save the intrusion kit: {error}");
        }
        if let Err(error) = active.printer.save(save.root()) {
            log::error!("could not save the fabricator: {error}");
        }
        if let Err(error) = active.optics.save(save.root()) {
            log::error!("could not save the optics kit: {error}");
        }
        if let Err(error) = active.electrolyser.save(save.root()) {
            log::error!("could not save the electrolyser: {error}");
        }
        if let Err(error) = active.mining.tank.save(save.root()) {
            log::error!("could not save the fleet's tank: {error}");
        }
        // The pile, which everything else in the game already remembered
        // better than the game did: the wallet, the town's books and the
        // house chest all survived a save and the one pile the shop sells out
        // of did not. It also feeds the tank above it, so a forgotten pile is
        // a fleet that stops working.
        if let Err(error) = pile::save(&active.mining.fleet, save.root()) {
            log::error!("could not save the base pile: {error}");
        }
        if let Err(error) = active.mining.wear.save(save.root()) {
            log::error!("could not save the wear ledger: {error}");
        }
        if let Err(error) = active.mining.wells.save(save.root()) {
            log::error!("could not save the wells: {error}");
        }
        if let Err(error) = active.arcade.save(save.root()) {
            log::error!("could not save the arcade: {error}");
        }
        if let Err(error) = active.health.save(save.root()) {
            log::error!("could not save the player's condition: {error}");
        }
        // The one part of the weather that outlives a session: which stands
        // something cleared, and how far back they have come.
        if let Err(error) = active.stands.save(save.root()) {
            log::error!("could not save the disturbed stands: {error}");
        }
        if let Err(error) = active.warrants.save(save.root()) {
            log::error!("could not save the towns' warrants: {error}");
        }
        if let Err(error) = active.elections.save(save.root()) {
            log::error!("could not save the towns' elections: {error}");
        }
        if let Err(error) = active.charters.save(save.root()) {
            log::error!("could not save the charters: {error}");
        }
        if let Err(error) = active.reputation.save(save.root()) {
            log::error!("could not save the player's reputation: {error}");
        }
        if let Err(error) = active.banks.save(save.root()) {
            log::error!("could not save the town vaults: {error}");
        }
        if let Err(error) = active.friends.save(save.root()) {
            log::error!("could not save the friendship ledger: {error}");
        }
        match active.economy.save(save.root()) {
            Ok(()) => log::info!("saved {} town markets", active.economy.tracked()),
            Err(error) => log::error!("could not save the town books: {error}"),
        }
    }

    fn report_framerate(&mut self) {
        self.frames += 1;
        let elapsed = self.last_report.elapsed();
        if elapsed.as_secs_f32() < 1.0 {
            return;
        }

        self.fps = self.frames as f32 / elapsed.as_secs_f32();
        if let Some(active) = &self.active {
            let fps = self.fps;
            active.window.set_title(&format!(
                "gamingg - {fps:.0} fps - {} - {}/{} chunks drawn - {} tris",
                match active.mode {
                    MovementMode::Fly => "fly",
                    MovementMode::Walk => "walk",
                },
                active.renderer.visible_chunk_count(),
                active.renderer.loaded_chunk_count(),
                active.renderer.triangle_count()
            ));
        }
        self.frames = 0;
        self.last_report = Instant::now();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.active.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("gamingg")
            .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("could not create a window: {error}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        let (context, surface) =
            match pollster::block_on(GpuContext::for_window(window.clone(), size.width, size.height))
            {
                Ok(pair) => pair,
                Err(error) => {
                    log::error!("could not initialise the graphics device: {error}");
                    event_loop.exit();
                    return;
                }
            };

        let mut renderer = Renderer::new(&context, surface.format(), size.width, size.height);

        // Open the save directory. A failure here is not fatal — the world is
        // still playable, it just will not persist — so it is reported and
        // play continues.
        let save_root = vx_platform::paths::saves_dir().join(&self.world_name);
        let save = match WorldSave::create(&save_root) {
            Ok(save) => Some(save),
            Err(error) => {
                log::error!(
                    "could not open {}: {error}; this session will not be saved",
                    save_root.display()
                );
                None
            }
        };

        // An existing world keeps its own seed, so reloading gives back the
        // same terrain regardless of what --seed says.
        let seed = save
            .as_ref()
            .filter(|save| save.exists())
            .and_then(|save| match save.read_meta() {
                Ok(seed) => {
                    log::info!("loading existing world (seed {seed})");
                    Some(seed)
                }
                Err(error) => {
                    log::error!("could not read the world metadata: {error}");
                    None
                }
            })
            .unwrap_or(self.seed);

        let mut world = World::new(seed);
        let streamer = ChunkStreamer::new(StreamingConfig {
            render_distance: self.view_distance,
            ..StreamingConfig::default()
        })
        .with_worker(streaming::GenWorker::new(world.generator().clone()));

        // Blocks the player can build with, resolved by name so this survives
        // any future id changes.
        let palette: Vec<BlockId> = [
            "engine:stone",
            "engine:dirt",
            "engine:grass",
            "engine:sand",
            "engine:container",
            "engine:chest",
            "engine:printer",
            "engine:electrolyser",
            "engine:pump",
        ]
        .iter()
        .filter_map(|name| world.registry().id_of(name))
        .collect();

        // Pregenerate the whole spawn area before the first playable frame, so
        // the world never visibly assembles itself around the player. This is
        // deliberately synchronous: a legible pause up front beats terrain
        // popping in around your first steps. Progress goes to the title bar —
        // the one place we can paint before the render loop exists.
        let home = vx_world::town::home_site();
        let spawn_block = vx_world::town::spawn_position(&home);
        let spawn_chunk = vx_core::ChunkPos::new(
            spawn_block.x.div_euclid(vx_core::CHUNK_SIZE),
            spawn_block.z.div_euclid(vx_core::CHUNK_SIZE),
        );
        let to_prepare = streaming::chunks_in_range(spawn_chunk, self.view_distance);
        let total = to_prepare.len();
        for (done, pos) in to_prepare.into_iter().enumerate() {
            streaming::load_or_generate(&mut world, save.as_ref(), pos);
            if done % 16 == 0 {
                window.set_title(&format!("gamingg — preparing the world {done}/{total}"));
            }
        }
        window.set_title("gamingg");

        // The hometown is held resident forever — the "spawn chunks" idea.
        // Coming home never hitches on regeneration, and the villagers' ground
        // is always real. `unload_beyond` honours the pin.
        let reach = home.core_half + vx_world::town::SKIRT;
        world.pin_span(
            vx_core::BlockPos::new(-reach, 0, -reach),
            vx_core::BlockPos::new(reach, 0, reach),
        );

        // You wake up in your own house, facing the door.
        let player = PlayerBody {
            position: glam::DVec3::new(
                spawn_block.x as f64 + 0.5,
                spawn_block.y as f64,
                spawn_block.z as f64 + 0.5,
            ),
            ..PlayerBody::default()
        };

        let camera = Camera {
            position: player.eye_position(),
            // The door is east of the bed. +x is yaw ninety degrees.
            yaw: std::f32::consts::FRAC_PI_2,
            aspect: size.width as f32 / size.height.max(1) as f32,
            ..Camera::default()
        };
        renderer.update_camera(&context.queue, &camera);

        let mut map = MapState::new();
        let mut skills = Skills::new();
        let mut wallet = Wallet::new();
        let mut clock = TimeOfDay::default();
        let mut ledger = beacon::Ledger::new();
        let mut journal = CommandLog::new();
        let mut economy = economy::Economy::new();
        let mut garage = garage::Garage::new();
        let mut homestead = homestead::Homestead::new();
        let mut town_permits = permits::Permits::new();
        let mut rack = arsenal::Arsenal::default();
        let mut kit = intrusion::Intrusions::default();
        let mut press = printer::Printer::default();
        let mut eyes = optics::Optics::default();
        let mut bath = electrolysis::Electrolyser::default();
        let mut tank = fuel::Tank::default();
        let mut crew_wear = wear::Wear::default();
        let mut holes = well::Wells::default();
        // A bare fleet to read the saved pile into, so the base and its goods
        // can be grafted onto the real one below.
        let mut base_pile = vx_agent::Fleet::new();
        let mut cabinet = arcade::Arcade::default();
        let mut stands = succession::Ledger::default();
        let mut warrants = warrant::Docket::default();
        let mut elections = ballot::Register::default();
        let mut charters = charter::Charters::default();
        let mut condition = health::Health::default();
        let mut name = reputation::Reputation::default();
        let mut vaults = bank::Bank::default();
        let mut friends = disposition::Disposition::default();
        if let Some(save) = &save {
            map.load(save.root());
            skills.load(save.root());
            wallet.load(save.root());
            clock = clock::load(save.root());
            ledger.load(save.root());
            journal = CommandLog::load(save.root());
            economy.load(save.root());
            garage.load(save.root());
            homestead.load(save.root());
            town_permits.load(save.root());
            rack.load(save.root());
            kit.load(save.root());
            press.load(save.root());
            eyes.load(save.root());
            bath.load(save.root());
            // The tank travels with the world it was burned in: replay
            // re-derives it from tick zero, so a session that reloaded with
            // an empty one would run dry at a different tick than its own
            // journal says it did.
            tank.load(save.root());
            crew_wear.load(save.root());
            // And the pile the tank burns out of, for the tank's own reason:
            // no pile is no fuel, no fuel is a fleet that does not turn, and
            // a fleet that does not turn cuts different ground.
            pile::load(&mut base_pile, save.root());
            // And the holes, for the tank's reason exactly: a well puts
            // goods on the pile the fleet burns, so a reload that forgot one
            // would dig a different hole than the journal says it dug.
            holes.load(save.root());
            // The cabinet keeps what a cabinet keeps: the cartridge, the
            // record and how deep anybody ever got.
            cabinet.load(save.root());
            // And which stands are still coming back, because a burn that a
            // reload forgot would be a burn that never healed.
            stands.load(save.root());
            // And whose paperwork is out on you, for the same reason: a
            // warrant a reload forgot would be a warrant that never happened.
            warrants.load(save.root());
            // And which seats you hold, because a badge a reload forgot would
            // be an election that never happened.
            elections.load(save.root());
            // And which towns you founded. The ground is in the region files
            // already; the world only has to be told to answer for them.
            charters.load(save.root());
            charters.tell(&mut world);
            condition.load(save.root());
            name.load(save.root());
            vaults.load(save.root());
            friends.load(save.root());
        }
        if self.sheriff {
            // The hometown's badge, which is what this override always meant:
            // before stage 40 an office had no town on it and this quietly
            // opened every lock on the frontier.
            let home = vx_world::town::home_site();
            town_permits.take_office(home.centre, permits::Office::Sheriff);
            log::info!("wearing {}'s sheriff's badge", home.name.head());
        }
        let shared_permits: permits::Shared =
            std::rc::Rc::new(std::cell::RefCell::new(town_permits));

        // The bus's first production subscriber since stage 2.x. It hooks the
        // events rather than the call sites, so the player's drill, a drone's
        // cutter and anything added later are all covered without one caller
        // changing. Built here, before the world is moved into `Active`.
        let mut events = EventBus::new();
        permits::install(&mut events, shared_permits.clone(), world.registry());
        // The dev override starts you with a crew, so the swarm tests and the
        // capture flags still mean what they meant before machines cost money.
        if self.crew > 0 {
            garage.grant(garage::DRONE, self.crew);
        }

        self.active = Some(Active {
            movement: movement::Movement::default(),
            move_ticks: movement::Ticker::default(),
            last_move: None,
            last_pilot: None,
            window,
            context,
            surface,
            renderer,
            world,
            streamer,
            camera,
            player,
            mode: MovementMode::Walk,
            events,
            save,
            palette,
            selected: 0,
            mining: {
                let mut mining = Mining::default();
                mining.ensure_flier(camera.position);
                mining.tank = tank;
                // Like the tank: replay re-derives it from tick zero, and a
                // reload that started fresh would hand back a worn-out
                // fleet's youth.
                mining.wear = crew_wear;
                mining.wells = holes;
                // The container you placed is still standing in the region
                // file; this is what makes it mean something again.
                mining.fleet.base = base_pile.base;
                mining
            },
            map,
            skills,
            digging: None,
            level_up: None,
            clock,
            view: ViewMode::default(),
            device: device::Device::new(),
            body_yaw: 0.0,
            body_pitch: 0.0,
            // True at boot as well as after a feed: the ground is guaranteed
            // by pregeneration, but the physics gate costs nothing and closes
            // the door on any future reordering of this function.
            resuming: true,
            player_rig: Rig::player(),
            stalker_rig: Rig::stalker(),
            handheld_rig: Rig::handheld(),
            hand_rig: Rig::hand_drill(),
            trade_rig: Rig::flier(),
            drill_spin: 0.0,
            villagers: Villagers::new(),
            villager_rigs: Villagers::rigs(),
            health: condition,
            posse: hostile::Posse::default(),
            garrisons: garrison::Garrisons::default(),
            reputation: name,
            jam_warned: false,
            debug_open: false,
            warrant_check: 0.0,
            greeting: None,
            wallet,
            shop: shop::Shop::new(),
            board: board::Board::new(),
            ledger,
            journal,
            economy,
            garage,
            homestead,
            home_panel: homestead::HomePanel::default(),
            permits: shared_permits,
            permit_panel: permits::PermitPanel::default(),
            watched: false,
            picking: false,
            #[cfg(feature = "gold")]
            gold: gold::Gold::default(),
            #[cfg(feature = "gold")]
            gold_hash: None,
            offers: Vec::new(),
            intro: {
                let mut panel = intro::Intro::new();
                panel.open = !intro::seen();
                panel
            },
            counter: None,
            console: None,
            // Deliberately absurd, so the first frame always scans.
            last_scan: (i32::MAX, i32::MAX),
            last_network: u64::MAX,
            arsenal: rack,
            shots: Vec::new(),
            falls: Vec::new(),
            water: Vec::new(),
            pumps: Vec::new(),
            pump_step: 0,
            fires: Vec::new(),
            stands,
            warrants,
            elections,
            charters,
            audio: audio::Audio::open(),
            launcher_rig: Rig::launcher(),
            shake: 0.0,
            shake_phase: 0.0,
            marks: scout::Marks::default(),
            intrusion: kit,
            printer: press,
            optics: eyes,
            electrolyser: bath,
            well_panel: well::Panel::default(),
            clinic: clinic::Clinic::default(),
            arcade: cabinet,
            dark: stalker::TheDark::default(),
            dose: dose::Dose::default(),
            dose_check: 0.0,
            last_rads: 0.0,
            banks: vaults,
            friends,
            terminal: terminal::Terminal::default(),
            osk: osk::Osk::default(),
            logged: None,
            roost: {
                // The box stands on the hometown security office roof; the
                // roost is its tenant. Derived from the office's claim so
                // worldgen stays the one authority on where that is.
                let site = vx_world::town::home_site();
                vx_world::town::plan::buildings(&site)
                    .into_iter()
                    .find(|building| building.role == vx_world::town::plan::Role::Security)
                    .map(|office| {
                        roost::Roost::new(vx_core::BlockPos::new(
                            office.max.x - 2,
                            office.max.y,
                            office.max.z - 2,
                        ))
                    })
            },
        });

        self.last_frame = Instant::now();
        self.last_report = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                // Save before tearing down, or everything built this session
                // is lost on quit.
                self.save_world();
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                if let Some(active) = &mut self.active {
                    if active
                        .surface
                        .resize(&active.context.device, size.width, size.height)
                    {
                        active
                            .renderer
                            .resize(&active.context, size.width, size.height);
                        active.camera.aspect =
                            size.width as f32 / size.height.max(1) as f32;
                    }
                }
            }

            WindowEvent::Focused(false) => {
                // Keys held when focus is lost would otherwise stay down.
                self.input.clear_keys();
                self.set_capture(false);
            }

            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                match event.state {
                    ElementState::Pressed => {
                        // Typing comes from the platform's own text for the
                        // key, not from the scancode: a scancode is a
                        // *position*, and reading letters off positions is how
                        // a console ends up unusable on half the world's
                        // keyboards. Repeats count here — holding backspace
                        // or a letter should do what it looks like it does.
                        let typing = self
                            .active
                            .as_ref()
                            .is_some_and(|active| active.terminal.open);
                        if typing {
                            if let Some(text) = event.text.as_ref() {
                                let characters: Vec<char> = text.chars().collect();
                                if let Some(active) = &mut self.active {
                                    for character in characters {
                                        if !character.is_control() {
                                            active.terminal.type_char(character);
                                        }
                                    }
                                }
                            }
                            if event.repeat {
                                self.handle_press(code);
                            }
                        }
                        // Key repeat re-fires this, so anything that toggles
                        // state must ignore repeats or it flickers.
                        if !event.repeat {
                            self.handle_press(code);
                        }
                        self.input.press(code);
                    }
                    ElementState::Released => {
                        // Letting go of Enter stops picking, the same way
                        // letting go of the mouse stops the drill.
                        if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
                            if let Some(active) = &mut self.active {
                                active.picking = false;
                            }
                        }
                        self.input.release(code);
                    }
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Released {
                    if button == MouseButton::Left {
                        // Letting go stops the drill mid-block; progress on
                        // that block is abandoned, like lifting a real tool.
                        self.drill_held = false;
                    }
                    return;
                }
                // The first click captures the pointer; once captured, the
                // held left button runs the drill. Without the capture step,
                // the click that focuses the window would also start cutting.
                if !self.input.mouse_captured {
                    if button == MouseButton::Left {
                        self.set_capture(true);
                    }
                    return;
                }
                match button {
                    MouseButton::Left => self.drill_held = true,
                    MouseButton::Right => self.place_at_target(),
                    _ => {}
                }
            }

            WindowEvent::RedrawRequested => self.frame(),

            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        // Raw motion, so mouse-look is unaffected by pointer acceleration and
        // does not stop at the screen edge.
        if let DeviceEvent::MouseMotion { delta } = event {
            self.input.add_mouse_delta(delta.0 as f32, delta.1 as f32);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(active) = &self.active {
            active.window.request_redraw();
        }
    }
}
