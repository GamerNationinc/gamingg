//! A played session, with nobody at the keyboard.
//!
//! # Why this exists
//!
//! Until stage 48 there was no way to *play* this game without a person sat in
//! front of it. Every verb in the loop — walking, aiming, drilling, opening the
//! counter, selling — is a method on `App`, and `App` owns an
//! `Arc<winit::Window>`; `main.rs` has no test module and cannot usefully have
//! one, and `vx-app` is a binary crate, so no integration test can reach any of
//! it either. The consequences were exactly what you would predict. Walking is
//! proved to a micrometre three thousand kilometres from spawn. Drone mining is
//! proved end to end on ground nobody arranged. Selling is proved against a
//! hand-built pile and a bare market. **Nothing joined them up**, and the join
//! is where the bug was: a block mined with no base container declared
//! evaporated in silence.
//!
//! A `Session` is the headless twin of the player's half of `Active`, in the
//! shape `journal::Rebuilt` already established for replay. It is deliberately
//! *not* a simulation of the game — it drives the game's own functions:
//! [`movement::advance_journal_tick`] for the body, [`crate::drill`] for the
//! bit, [`Mining::advance`] for the fleet, and the real [`Shop::confirm`] for
//! the counter. A harness that re-implemented any of those would be a test of
//! itself.
//!
//! # What it is not
//!
//! Not a bot, and not a pathfinder. [`Session::walk_to`] holds forward and lets
//! stage 47's vault and mantle do the climbing, and it is allowed to fail:
//! [`Arrival::Stuck`] is a *finding*, not a panic. Whether you can walk out of
//! your own front door in a straight line is a thing worth learning rather than
//! asserting.

use glam::{DVec3, Vec3};

use std::path::Path;

use vx_agent::Stockpile;
use vx_core::{BlockPos, EventBus};
use vx_world::{break_block, raycast_solid, PlayerBody, World, WorldSave};

use crate::drill::{self, Deposited};
use crate::economy::Economy;
use crate::journal::{Command, CommandLog};
use crate::mining::Mining;
use crate::movement::{self, MoveCommand, Movement};
use crate::shop::Shop;
use crate::skills::{self, Skills};
use crate::wallet::{self, Wallet};

/// How far the player can reach, matching `App::REACH`.
pub const REACH: f32 = 5.0;

/// Frames a second the drill is held at, matching the frame loop's own step.
/// The bit is charged per frame, not per journal tick, so a session that wants
/// the live game's drilling times has to hold at the live game's rate.
pub const DRILL_HZ: f32 = 60.0;

/// How close to a target counts as arrived, in blocks.
///
/// Level distance only: a target on a ledge you have climbed onto is arrived
/// at, and one directly below you through the floor is not somewhere the walk
/// can take you anyway.
pub const ARRIVED: f64 = 1.4;

/// Chunks kept loaded around the walker.
///
/// There is no streamer here, so the session does the streamer's one job.
/// Three is enough for the body's sweep and the drill's reach with room over.
pub const KEEP_LOADED: i32 = 3;

/// How a walk ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrival {
    /// Got there, in this many journal ticks.
    Reached { ticks: u32 },
    /// Ran out of budget, or stopped making ground, and gave up here.
    ///
    /// Not an error. A wall, a locked gate, a cliff and a tree are all real
    /// answers to "can you walk there", and the point of playing the loop is
    /// to find out which one you meet.
    Stuck { at: DVec3, ticks: u32 },
}

impl Arrival {
    pub fn reached(self) -> bool {
        matches!(self, Arrival::Reached { .. })
    }

    pub fn ticks(self) -> u32 {
        match self {
            Arrival::Reached { ticks } | Arrival::Stuck { ticks, .. } => ticks,
        }
    }
}

/// Everything a played session holds. The player's half of `Active`, minus the
/// window and everything that only exists to draw.
pub struct Session {
    pub world: World,
    pub events: EventBus,
    pub player: PlayerBody,
    pub movement: Movement,
    /// Where the head is pointed. The camera is not journalled — only the
    /// quantised angles inside a `MoveCommand` reach the simulation — so the
    /// session carries the same two floats the camera would.
    pub yaw: f32,
    pub pitch: f32,
    pub mining: Mining,
    pub wallet: Wallet,
    pub skills: Skills,
    pub economy: Economy,
    pub shop: Shop,
    pub journal: CommandLog,
    /// The hold in progress, exactly as `Active::digging` carries it.
    pub digging: Option<(BlockPos, f32)>,
    /// Ticks the session has advanced.
    pub tick: u64,
    /// Blocks that came out of the ground with nowhere to go. The bug this
    /// round found, counted rather than assumed.
    pub lost: u64,
    last_move: Option<MoveCommand>,
    loaded: Option<vx_core::ChunkPos>,
    /// The save this session was opened from, if any. Held so ground the
    /// walker reaches later still comes off disk rather than being
    /// regenerated over the top of what a previous session dug.
    save: Option<WorldSave>,
}

impl Session {
    /// Open a world and stand in the doorway of the house, exactly where the
    /// game puts a new player.
    pub fn open(seed: u64) -> Self {
        let home = vx_world::town::home_site();
        let spawn = vx_world::town::spawn_position(&home);
        let mut world = World::new(seed);
        let chunk = spawn.chunk();
        world.load_around(chunk, KEEP_LOADED);

        let player = PlayerBody {
            position: DVec3::new(
                f64::from(spawn.x) + 0.5,
                f64::from(spawn.y),
                f64::from(spawn.z) + 0.5,
            ),
            ..PlayerBody::default()
        };

        Session {
            world,
            events: EventBus::new(),
            player,
            movement: Movement::default(),
            // Facing +x, out of the door, the way `App::resumed` aims a new
            // camera.
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            mining: Mining::default(),
            wallet: Wallet::new(),
            skills: Skills::default(),
            economy: Economy::new(),
            shop: Shop::new(),
            journal: CommandLog::new(),
            digging: None,
            tick: 0,
            lost: 0,
            last_move: None,
            loaded: Some(chunk),
            save: None,
        }
    }

    /// Write the session to a save directory, the way quitting does.
    ///
    /// The same set of files `App::save_world` writes, minus the ones a
    /// `Session` does not own. The world's modified chunks are the keyframe;
    /// the journal is the oracle beside them.
    pub fn save_to(&mut self, root: &Path) -> std::io::Result<()> {
        let save = WorldSave::create(root)
            .map_err(|error| std::io::Error::other(format!("{error}")))?;
        save.write_meta(self.world.seed())
            .map_err(|error| std::io::Error::other(format!("{error}")))?;
        save.save_world(&mut self.world)
            .map_err(|error| std::io::Error::other(format!("{error}")))?;
        self.journal.save(root)?;
        self.wallet.save(root)?;
        self.skills.save(root)?;
        self.economy.save(root)?;
        self.mining.tank.save(root)?;
        crate::pile::save(&self.mining.fleet, root)?;
        crate::whereabouts::save(
            crate::whereabouts::Whereabouts {
                position: self.player.position,
                yaw: self.yaw,
                pitch: self.pitch,
            },
            root,
        )?;
        Ok(())
    }

    /// Open a session back up from a save directory.
    ///
    /// A second constructor rather than a flag on [`Session::open`], because
    /// the two genuinely differ: `open` generates a world from a seed and
    /// stands the player in the doorway, and this one reads the seed off the
    /// save, pulls the chunks back through it rather than regenerating them,
    /// and restores everything that was written beside them.
    pub fn load_from(root: &Path) -> std::io::Result<Session> {
        let save = WorldSave::create(root)
            .map_err(|error| std::io::Error::other(format!("{error}")))?;
        let seed = save
            .read_meta()
            .map_err(|error| std::io::Error::other(format!("{error}")))?;

        let mut session = Session::open(seed);
        // Where you were standing, before anything is loaded around it: the
        // ground is pulled in around wherever the body is going to be, and a
        // body restored *after* the chunks are chosen stands in unloaded air
        // and falls through the world.
        if let Some(at) = crate::whereabouts::load(root) {
            session.player.position = at.position;
            session.yaw = at.yaw;
            session.pitch = at.pitch;
        }
        // Pull the saved ground back in place of the generated ground. The
        // same free function the live game's boot uses, so a chunk that was
        // dug in the last session comes back dug.
        let around: Vec<vx_core::ChunkPos> = session.world.loaded_chunks().collect();
        for pos in around {
            session.world.unload_beyond(pos, i32::MAX);
        }
        let here = BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32,
            session.player.position.z.floor() as i32,
        )
        .chunk();
        for dx in -KEEP_LOADED..=KEEP_LOADED {
            for dz in -KEEP_LOADED..=KEEP_LOADED {
                let pos = vx_core::ChunkPos::new(here.x + dx, here.z + dz);
                crate::streaming::load_or_generate(&mut session.world, Some(&save), pos);
            }
        }
        session.loaded = Some(here);
        session.save = Some(save);

        session.journal = crate::journal::CommandLog::load(root);
        session.wallet.load(root);
        session.skills.load(root);
        session.economy.load(root);
        session.mining.tank.load(root);
        crate::pile::load(&mut session.mining.fleet, root);
        Ok(session)
    }

    /// The town the house stands in.
    pub fn home(&self) -> vx_world::town::TownSite {
        vx_world::town::home_site()
    }

    /// The fleet's pile, if a container has declared one.
    pub fn pile(&self) -> Option<&Stockpile> {
        self.mining.fleet.base.as_ref().map(|base| &base.stockpile)
    }

    /// What the pack weighs, as the byte the journal carries.
    ///
    /// The same sum `App::frame` makes: the pile *is* the load, because there
    /// is no player inventory and everything routes through the base.
    pub fn load_byte(&self) -> u8 {
        let carried = self.pile().map_or(0, |pile| pile.total());
        let capacity = wallet::pack_capacity(
            skills::capacity(
                vx_agent::DEFAULT_CAPACITY,
                self.skills.level(skills::LOGISTICS),
            ),
            self.wallet.upgrade(wallet::PACK),
        );
        movement::load_byte(carried, capacity)
    }

    /// The command a held bitfield makes right now, looking where the head is
    /// looking and carrying what the pack is carrying.
    pub fn command(&self, bits: u16) -> MoveCommand {
        MoveCommand::looking(bits, self.yaw, self.pitch).laden(self.load_byte())
    }

    /// Where the eye is.
    pub fn eye(&self) -> DVec3 {
        self.player.eye_position()
    }

    /// Which way the head is pointed, in the camera's own convention.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch).normalize()
    }

    /// Advance the simulation, recording it the way the live game records it.
    ///
    /// This is `journal::apply`'s `Command::Advance` arm and `App::frame`'s
    /// tick seam, agreeing: the held command is journalled only when it
    /// changes, the fleet advances once for the whole span, and the body takes
    /// one journal tick at a time.
    pub fn advance(&mut self, ticks: u32, held: MoveCommand) {
        if ticks == 0 {
            return;
        }
        if self.last_move != Some(held) {
            self.journal.record(Command::moving(held));
            self.last_move = Some(held);
        }
        self.mining.advance(&mut self.world, &self.events, ticks);
        self.journal.record(Command::Advance { ticks });
        for _ in 0..ticks {
            self.tick += 1;
            movement::advance_journal_tick(
                &mut self.movement,
                &mut self.player,
                &self.world,
                held,
            );
        }
        self.keep_chunks_loaded();
    }

    /// The streamer's one job, done by hand: the ground under the walker
    /// exists before the walker gets there.
    fn keep_chunks_loaded(&mut self) {
        let here = BlockPos::new(
            self.player.position.x.floor() as i32,
            self.player.position.y.floor() as i32,
            self.player.position.z.floor() as i32,
        )
        .chunk();
        if self.loaded == Some(here) {
            return;
        }
        match &self.save {
            // Saved ground first: a hole dug last session is still a hole.
            Some(save) => {
                for dx in -KEEP_LOADED..=KEEP_LOADED {
                    for dz in -KEEP_LOADED..=KEEP_LOADED {
                        let pos = vx_core::ChunkPos::new(here.x + dx, here.z + dz);
                        crate::streaming::load_or_generate(&mut self.world, Some(save), pos);
                    }
                }
            }
            None => {
                self.world.load_around(here, KEEP_LOADED);
            }
        }
        self.loaded = Some(here);
    }

    /// Point the head at a block's centre.
    pub fn look_at(&mut self, block: BlockPos) {
        let target = DVec3::new(
            f64::from(block.x) + 0.5,
            f64::from(block.y) + 0.5,
            f64::from(block.z) + 0.5,
        );
        self.look_toward(target);
    }

    /// Point the head at a place.
    pub fn look_toward(&mut self, target: DVec3) {
        let to = target - self.eye();
        let level = (to.x * to.x + to.z * to.z).sqrt();
        // The camera's yaw convention: +x is a quarter turn, and forward is
        // `(sin yaw, _, -cos yaw)`.
        self.yaw = (to.x as f32).atan2(-to.z as f32);
        self.pitch = (to.y as f32)
            .atan2(level as f32)
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    }

    /// Walk to a place: blunder at it, and when blundering stops working,
    /// stop and work out a way round before blundering at it again.
    ///
    /// # Why there are two halves
    ///
    /// [`Session::blunder_to`] is what a player pointing at the horizon and
    /// holding W actually gets, sidesteps included, and it is enough for
    /// everything inside a town. It is not enough for two hundred blocks of
    /// open country: stage 50's first haul walked out of Stonehaven, crossed
    /// forty blocks of hill, met a ridge at (-136, -110) and spent
    /// twenty-four sidesteps pacing the same piece of it. Widening the
    /// sidestep did not help and neither did doubling the allowance, because
    /// the shape of the problem is not "there is a wall in front of me", it
    /// is "the way through is not in the direction I am facing".
    ///
    /// So when the legs run out of ideas the walker sweeps the ground it can
    /// actually see — [`crate::afoot::Ground`], the drones' breadth-first
    /// search with a body's rules rather than a machine's — and walks to
    /// whichever piece of it lies nearest the place it is going. Then it
    /// blunders on from there. That gets it round anything smaller than the
    /// loaded box, which on this frontier is most things.
    ///
    /// It is still not a pathfinder for the *player*: the game has no route
    /// planner and a person on foot does not get one. It is a person walking
    /// up to a ridge, looking along it, and going the way it opens.
    pub fn walk_to(&mut self, target: DVec3, budget: u32) -> Arrival {
        /// How many times the walker will stop and look before giving up.
        const THINKS: u32 = 8;

        let mut spent = 0;
        let mut tried = Vec::new();
        for _ in 0..THINKS {
            match self.blunder_to(target, budget - spent) {
                Arrival::Reached { ticks } => {
                    return Arrival::Reached {
                        ticks: spent + ticks,
                    }
                }
                Arrival::Stuck { at, ticks } => {
                    spent += ticks;
                    if spent >= budget {
                        return Arrival::Stuck { at, ticks: spent };
                    }
                    match self.feel_a_way_round(target, budget - spent, &mut tried) {
                        Some(ticks) => spent += ticks,
                        // Nothing loaded around the walker is any closer than
                        // where it is standing. That is genuinely stuck.
                        None => {
                            return Arrival::Stuck {
                                at: self.player.position,
                                ticks: spent,
                            }
                        }
                    }
                }
            }
        }
        Arrival::Stuck {
            at: self.player.position,
            ticks: spent,
        }
    }

    /// Walk to the loaded ground nearest the target, and report what it cost.
    /// `None` when there is nowhere better to stand than here.
    ///
    /// The sweep runs **from the walker outward**, not from the target: the
    /// target is two hundred blocks away and not loaded, so a sweep from it
    /// would label nothing. Sweeping from here labels every cell a body could
    /// reach on foot, and the best of those is simply the reachable cell
    /// closest to where it is going — which is the honest answer to "look
    /// along the ridge and go the way it opens".
    fn feel_a_way_round(
        &mut self,
        target: DVec3,
        budget: u32,
        tried: &mut Vec<BlockPos>,
    ) -> Option<u32> {
        /// Half-width of the ground the walker considers, in blocks. Kept
        /// inside `KEEP_LOADED` chunks so every cell in it is real ground
        /// rather than the air an unloaded chunk reads as.
        const LOOK: i32 = 36;
        /// Half-height. Deep enough for a gully, shallow enough that the
        /// sweep stays cheap.
        const RISE: i32 = 20;
        /// Cells between the waypoints the route is boiled down to. Every
        /// cell would be a waypoint a stride apart, which the walker would
        /// spend its whole budget arriving at.
        const STRIDE: usize = 6;
        /// How far apart two ideas have to be to count as different ones, in
        /// blocks. Also the least sideways ground a shoulder-of-the-hill
        /// detour has to be worth before it is worth walking.
        const SHRUG: i32 = 8;

        self.keep_chunks_loaded();
        let here = BlockPos::new(
            self.player.position.x.floor() as i32,
            self.player.position.y.floor() as i32,
            self.player.position.z.floor() as i32,
        );
        let ground = crate::afoot::Ground::sweep(&self.world, here, LOOK, RISE);

        let level = |x: i32, z: i32| {
            let (dx, dz) = (f64::from(x) + 0.5 - target.x, f64::from(z) + 0.5 - target.z);
            dx * dx + dz * dz
        };
        let start = level(here.x, here.z);
        // Somewhere the walker has already been sent and got stuck from is not
        // somewhere to send it again. Without this the walker paces: the ledge
        // to the south looks best from the ridge, the ridge looks best from
        // the ledge, and it spends the whole budget between them.
        let fresh = |cell: &BlockPos, tried: &[BlockPos]| {
            !tried.iter().any(|old| {
                let (dx, dz) = (f64::from(cell.x - old.x), f64::from(cell.z - old.z));
                dx * dx + dz * dz < f64::from(SHRUG * SHRUG)
            })
        };
        // Ties are broken by position throughout, so the same ground gives the
        // same answer every run — the walker is inside the replayed
        // simulation and may not wander differently between them.
        let order = |a: &(f64, BlockPos), b: &(f64, BlockPos)| {
            a.0.total_cmp(&b.0)
                .then_with(|| (a.1.x, a.1.y, a.1.z).cmp(&(b.1.x, b.1.y, b.1.z)))
        };

        let closer = ground
            .reachable()
            .filter(|cell| fresh(cell, tried))
            .map(|cell| (level(cell.x, cell.z), cell))
            .filter(|(score, _)| *score < start - 1.0)
            .min_by(order)
            .map(|(_, cell)| cell);

        // Nothing in sight is closer. That is a *ledge*, not a dead end: the
        // way on is round the shoulder of the hill, and the first half of
        // going round is going the wrong way. So take the furthest ground
        // that is at least broadly the right way and look again from there —
        // which is exactly what a person does when the direct line runs into
        // a bank they cannot climb.
        let best = match closer {
            Some(cell) => cell,
            None => {
                let bearing = {
                    let (dx, dz) = (target.x - self.player.position.x, target.z - self.player.position.z);
                    let length = (dx * dx + dz * dz).sqrt().max(1.0);
                    (dx / length, dz / length)
                };
                ground
                    .reachable()
                    .filter(|cell| fresh(cell, tried))
                    .map(|cell| {
                        let (dx, dz) = (
                            f64::from(cell.x - here.x),
                            f64::from(cell.z - here.z),
                        );
                        // Negated, so the *most* sideways progress sorts
                        // first under the same comparator.
                        (-(dx * bearing.0 + dz * bearing.1), cell)
                    })
                    .filter(|(along, _)| *along < -f64::from(SHRUG))
                    .min_by(order)
                    .map(|(_, cell)| cell)?
            }
        };
        tried.push(best);

        let route = ground.route_to(best)?;
        let mut spent = 0;
        let legs: Vec<BlockPos> = route
            .iter()
            .enumerate()
            .filter(|(index, _)| index % STRIDE == 0)
            .map(|(_, cell)| *cell)
            .chain(std::iter::once(best))
            .collect();
        for cell in legs {
            if spent >= budget {
                break;
            }
            let leg = DVec3::new(
                f64::from(cell.x) + 0.5,
                f64::from(cell.y),
                f64::from(cell.z) + 0.5,
            );
            match self.blunder_to(leg, budget - spent) {
                Arrival::Reached { ticks } => spent += ticks,
                Arrival::Stuck { ticks, .. } => {
                    spent += ticks;
                    break;
                }
            }
        }
        Some(spent)
    }

    /// Walk toward a place, holding forward and letting the movement system do
    /// the rest.
    ///
    /// Deliberately naive. There is no pathfinder in this game — the drones
    /// have flow fields and the player has legs — so this is what a player
    /// pointing at the horizon and holding W actually gets, including getting
    /// stopped by things. It gives up when the budget runs out or when a long
    /// stretch of walking has not closed any distance, and says where it was
    /// standing when it did.
    fn blunder_to(&mut self, target: DVec3, budget: u32) -> Arrival {
        // A slice is an eighth of a second of game time: long enough that a
        // vault or a mantle finishes inside one, short enough that a wall is
        // noticed almost at once.
        const SLICE: u32 = 8;
        /// Slices of no ground gained before the walker decides it is stuck
        /// on something and tries going round.
        const STALL: u32 = 4;
        /// How far off the bearing a detour goes.
        ///
        /// A right angle, not the sixty degrees this started with. Sixty is
        /// enough to clear the corner of a building, and that is all this
        /// walker was ever asked to do — but sixty degrees still has a
        /// forward component, so against a *ridge* it skims the face and
        /// bumps it again every few strides, and a hundred-block detour buys
        /// twenty blocks of sideways. A right angle walks the face.
        const DETOUR_TURN: f32 = std::f32::consts::FRAC_PI_3;
        /// The turn a walker makes when stepping aside has stopped working.
        ///
        /// A sixty-degree detour is an answer to a wall in front of you. It is
        /// no answer at all to a *room* — and this frontier has rooms in it
        /// nobody put a door on the far side of. The first haul that made it
        /// out of town walked into a bunker at (-136, -110), forty blocks
        /// short of its destination, and spent twenty-four detours pacing the
        /// same three walls of the same dead end, because turning one way by
        /// sixty degrees over and over only ever traces a room's inside.
        /// Getting out of a dead end means going back the way you came in.
        const BACK_OUT: f32 = 2.4;
        /// How long it commits to a detour before looking at the target
        /// again. Long enough to get past a building; short enough that it
        /// does not wander off.
        const DETOUR_SLICES: u32 = 56;
        /// How many times it will try going round before admitting it cannot
        /// get there. Twelve was enough for a walk across town; a haul to the
        /// next town crosses open country with mountains in it, and a single
        /// ridge can cost half a dozen on its own.
        const DETOURS: u32 = 24;

        let level = |a: DVec3, b: DVec3| {
            let (dx, dz) = (a.x - b.x, a.z - b.z);
            (dx * dx + dz * dz).sqrt()
        };

        let mut ticks = 0;
        let mut best = level(self.player.position, target);
        let mut since_progress = 0;
        // Which way it tries first, and how many tries are left. Alternating
        // sides is what turns "walk into the cliff, sidestep, walk into the
        // cliff" into working along a face until it ends.
        let mut side = 1.0f32;
        let mut turn = DETOUR_TURN;
        let mut detour_left = 0;
        let mut detours_used = 0;

        while ticks < budget {
            if level(self.player.position, target) <= ARRIVED {
                return Arrival::Reached { ticks };
            }

            // Re-aim every slice: the ground turns underfoot, and a body that
            // has been shoved sideways by a wall has to point at the target
            // again.
            self.look_toward(DVec3::new(target.x, self.eye().y, target.z));
            if detour_left > 0 {
                self.yaw += side * turn;
                detour_left -= 1;
            }

            // Forward, and a jump only once the walk has stopped getting
            // anywhere.
            //
            // Holding jump the whole way is how the first played run ended up
            // stood on the shop's roof selling ore through it: with the button
            // down, every wall in town is a staircase. Stage 47's automatic
            // vault already handles kerbs and benches without asking, so a
            // walk holds forward and reaches for the jump only when it is
            // stuck — which is also what a person does.
            // Never on the final approach. A doorway is one block wide and
            // two high, and a walker that reaches for the jump the moment it
            // brushes a jamb climbs the wall beside the door and ends up on
            // the roof — which is how the first `--play` run tried to sell
            // its ore through the shop's ceiling.
            let closing = level(self.player.position, target) < 3.0;
            let bits = if since_progress >= 1 && detour_left == 0 && !closing {
                movement::FWD | movement::JUMP
            } else {
                movement::FWD
            };
            let held = self.command(bits);
            let step = SLICE.min(budget - ticks);
            self.advance(step, held);
            ticks += step;

            let now = level(self.player.position, target);
            if now < best - 0.05 {
                best = now;
                since_progress = 0;
                // A detour that worked does not count against the next one.
                //
                // `detours_used` used to be a *lifetime* cap on one call, so a
                // five-hundred block haul to the next town was punished for a
                // fence it had already climbed in the first fifty — twelve
                // obstacles total, however far it went. Resetting on real
                // progress keeps the cap meaning what it should: twelve tries
                // at the thing in front of you, not twelve for the journey.
                detours_used = 0;
                continue;
            }
            // A detour is expected to lose ground; it is not a stall.
            if detour_left > 0 {
                continue;
            }
            since_progress += 1;
            if since_progress < STALL {
                continue;
            }
            // Stuck on something the legs cannot answer. Try going round it.
            // Terrain in this game has three-block steps in it, which is above
            // anything a body can mantle — the honest response to one is to
            // walk along it, not to keep bouncing off it.
            if detours_used >= DETOURS {
                return Arrival::Stuck {
                    at: self.player.position,
                    ticks,
                };
            }
            detours_used += 1;
            // Step aside twice, then back out. Stepping aside answers a wall;
            // backing out is the only thing that answers a room.
            turn = if detours_used % 3 == 0 { BACK_OUT } else { DETOUR_TURN };
            detour_left = DETOUR_SLICES;
            since_progress = 0;
            // Commit to a side for two tries before trying the other one. A
            // cliff face runs for a long way, and a walker that alternates
            // every time only ever paces the same twenty blocks of it.
            if detours_used % 2 == 0 {
                side = -side;
            }
        }

        if level(self.player.position, target) <= ARRIVED {
            Arrival::Reached { ticks }
        } else {
            Arrival::Stuck {
                at: self.player.position,
                ticks,
            }
        }
    }

    /// Walk a route, one leg at a time, stopping at the first leg that fails.
    ///
    /// The concession this harness makes to not having a pathfinder, and the
    /// same one a person makes without noticing: you do not set off on the
    /// bearing of somewhere a hundred and seventy blocks away while you are
    /// still stood in your kitchen. You walk out of the door first.
    ///
    /// Legs are aimed at in order and the budget is shared across them, so a
    /// route that is mostly a straight line costs what the straight line
    /// costs.
    pub fn walk_route(&mut self, legs: &[DVec3], budget: u32) -> Arrival {
        let mut spent = 0;
        for leg in legs {
            if spent >= budget {
                return Arrival::Stuck {
                    at: self.player.position,
                    ticks: spent,
                };
            }
            match self.walk_to(*leg, budget - spent) {
                Arrival::Reached { ticks } => spent += ticks,
                Arrival::Stuck { at, ticks } => {
                    return Arrival::Stuck {
                        at,
                        ticks: spent + ticks,
                    }
                }
            }
        }
        Arrival::Reached { ticks: spent }
    }

    /// Just outside the front door, on the plaza lane.
    ///
    /// The first leg of every route that leaves the house, and the last leg
    /// of every route that comes back to it.
    pub fn doorstep(&self) -> DVec3 {
        let door = vx_world::town::door_position(&self.home());
        DVec3::new(
            f64::from(door.x) + 1.5,
            f64::from(door.y),
            f64::from(door.z) + 0.5,
        )
    }

    /// The gateway of a town's mini-star that faces `target`, as a place to
    /// walk to.
    ///
    /// Every town has been walled since stage 32, and a rampart does not care
    /// which way you were going: a haul that sets off on the bearing of the
    /// next town along walks out of its own front door, across the plaza, and
    /// into the inside of its own wall. The first `--haul` run did exactly
    /// that and stopped dead at (-28, -27), which is the trace, not terrain.
    ///
    /// The four gateways sit on the cardinal axes because that is where the
    /// roads are, so leaving is a two-part move — reach the gate that faces
    /// where you are going, then set off — and arriving is the same move
    /// backwards. The y is the caller's: `walk_to` measures on the level and
    /// the ground under a gateway is whatever the fort cut it to.
    pub fn gateway_toward(&self, site: &vx_world::town::TownSite, target: DVec3) -> DVec3 {
        let fort = vx_world::fort::fort_for(site);
        let bearing = (
            target.x - f64::from(site.centre.0),
            target.z - f64::from(site.centre.1),
        );
        let gate = fort
            .gateways()
            .into_iter()
            .max_by(|a, b| {
                let dot = |(x, z): (i32, i32)| {
                    let (dx, dz) = (
                        f64::from(x - site.centre.0),
                        f64::from(z - site.centre.1),
                    );
                    let length = (dx * dx + dz * dz).sqrt().max(1.0);
                    (dx * bearing.0 + dz * bearing.1) / length
                };
                dot(*a).total_cmp(&dot(*b))
            })
            .unwrap_or(site.centre);
        // A pace past the trace, so the waypoint is out in the gateway rather
        // than in the thickness of the wall itself.
        let (dx, dz) = (
            f64::from(gate.0 - site.centre.0),
            f64::from(gate.1 - site.centre.1),
        );
        let length = (dx * dx + dz * dz).sqrt().max(1.0);
        DVec3::new(
            f64::from(gate.0) + 0.5 + dx / length * 2.0,
            self.player.position.y,
            f64::from(gate.1) + 0.5 + dz / length * 2.0,
        )
    }

    /// How long a leg of open country is before the route puts another
    /// waypoint in.
    ///
    /// Each leg gets its own detour allowance, so short legs mean a walker
    /// that gives up less easily — but a waypoint dropped blind onto a
    /// hillside can also be somewhere unreachable, and then the walk stops at
    /// it. Ninety blocks is what the two-hundred-block haul wants; thirty was
    /// measurably worse.
    pub const LEG: f64 = 90.0;

    /// A walk across country: out of one town by its gate, over the hills in
    /// legs, in through another town's gate, and up to the place itself.
    ///
    /// This is the shape every long walk in the game has, and it was written
    /// out by hand three times in the played loop before it was worth naming.
    /// Both gates matter and for the same reason: a bearing taken on a plaza
    /// points at the inside of your own rampart, and a bearing taken in open
    /// country points at the outside of theirs. `leaving` and `entering` are
    /// each optional because a seam on a hillside is in no town at all.
    pub fn cross_country(
        &self,
        leaving: Option<&vx_world::town::TownSite>,
        to: DVec3,
        entering: Option<&vx_world::town::TownSite>,
    ) -> Vec<DVec3> {
        let mut legs = Vec::new();
        if let Some(site) = leaving {
            legs.push(self.gateway_toward(site, to));
        }
        let from = *legs.last().unwrap_or(&self.player.position);
        // The far gate is chosen by the bearing of the *approach*, not of the
        // town's centre: you come in at whichever gate is on your side.
        let arrive = entering.map(|site| self.gateway_toward(site, from));
        let last = arrive.unwrap_or(to);
        let steps = ((last - from).length() / Self::LEG).ceil().max(1.0) as i32;
        for step in 1..=steps {
            let along = f64::from(step) / f64::from(steps);
            legs.push(from + (last - from) * along);
        }
        if arrive.is_some() {
            legs.push(to);
        }
        legs
    }

    /// The route into the shop and up to the counter: the doorway first, then
    /// the customer's side of the counter run.
    ///
    /// The counter is *inside* a building, so walking at the counter's own
    /// coordinates walks into the shop's north wall — which is exactly what
    /// the first played run did.
    ///
    /// Takes the town rather than assuming the hometown, because the whole
    /// point of hauling is that you sell somewhere else: a refinery is short
    /// of ore and pays for it, a mine is sitting on a hill of the stuff and
    /// does not.
    pub fn counter_route(&self, site: &vx_world::town::TownSite) -> [DVec3; 3] {
        let door = vx_world::town::shop_door_position(site);
        let stand = vx_world::town::counter_stand_position(site);
        [
            // Outside, square on to the doorway.
            DVec3::new(
                f64::from(door.x) + 0.5,
                f64::from(door.y),
                f64::from(door.z) - 2.5,
            ),
            // The doorway itself: aimed *at* the gap rather than through it,
            // so the walk threads it instead of cutting the corner into the
            // wall beside it.
            DVec3::new(
                f64::from(door.x) + 0.5,
                f64::from(door.y),
                f64::from(door.z) + 0.5,
            ),
            // And the customer's side of the counter run.
            DVec3::new(
                f64::from(stand.x) + 0.5,
                f64::from(stand.y),
                f64::from(stand.z) + 0.5,
            ),
        ]
    }

    /// Put the fleet's flier in the air over the player, if it has none.
    pub fn ensure_flier(&mut self) {
        let at = self.player.position;
        self.mining.ensure_flier(at);
    }

    /// Order a sweep of the sector containing a column, and fly it to
    /// completion.
    ///
    /// This is the game's actual ore-finder — the paid flier's sector scan,
    /// not the kestrel, which watches people and machines and is documented
    /// as finding no ore at all. The sweep is a serpentine lawnmower path over
    /// 64×64 columns, reading real blocks down to the scanner's depth, so the
    /// **chunks must be loaded first** or an unread column reads as "no ore"
    /// rather than as an error.
    ///
    /// Returns how many ticks it took, or `None` if it never finished inside
    /// the budget — which is what a dry tank looks like from out here.
    pub fn scan_sector(&mut self, at: (i32, i32), budget: u32) -> Option<u32> {
        // Every column the sweep will read, plus the swath overhang.
        let sector = vx_agent::Sector::containing(at.0, at.1);
        let corner = sector.min_column();
        let across = vx_agent::SECTOR_SIZE / vx_core::CHUNK_SIZE;
        for cx in -1..=across {
            for cz in -1..=across {
                let chunk = BlockPos::new(
                    corner.0 + cx * vx_core::CHUNK_SIZE,
                    0,
                    corner.1 + cz * vx_core::CHUNK_SIZE,
                )
                .chunk();
                self.world.load_around(chunk, 0);
            }
        }

        self.ensure_flier();
        if !self.mining.dispatch_scan(at.0, at.1) {
            return None;
        }
        let held = self.command(0);
        let mut ticks = 0;
        while ticks < budget {
            if self.mining.fleet.is_surveyed(sector) {
                return Some(ticks);
            }
            self.advance(8, held);
            ticks += 8;
        }
        None
    }

    /// What the sweep found: one ping per ore body it saw.
    pub fn pings(&self) -> Vec<vx_agent::Ping> {
        self.mining.fleet.pings()
    }

    /// Put fuel on the pile, the way buying or printing HHO cells does.
    ///
    /// Needed before a scan, and the reason is a trap worth knowing: the
    /// fleet burns fuel *out of the base pile*, and `Mining::fuelled` returns
    /// true only while there is **no** base at all. So declaring your first
    /// container — the thing the game now tells you to do before mining — is
    /// also the thing that grounds your flier, unless there is fuel on the
    /// pile for it.
    pub fn fuel_the_fleet(&mut self, cells: u64) {
        if let Some(base) = self.mining.fleet.base.as_mut() {
            base.stockpile.add(crate::fuel::CELL.to_string(), cells);
        }
    }

    /// Declare the fleet's base where a container stands.
    ///
    /// The live game does this when the player *places* an `engine:container`;
    /// the session places the block and declares it in one, because the
    /// placement rules are `App`'s business and the pile is what the loop
    /// needs.
    pub fn place_base(&mut self, at: BlockPos) {
        let container = self
            .world
            .registry()
            .id_of("engine:container")
            .expect("no container block in the registry");
        let _ = self.world.set_block(at, container);
        self.journal.record(Command::Place {
            at,
            block: "engine:container".to_string(),
        });
        self.mining.fleet.set_base(at);
    }

    /// Hold the trigger on whatever the head is pointed at until it breaks.
    ///
    /// Returns what became of the block, or `None` if the bit never bit —
    /// nothing in reach, or a block with no hardness at all.
    pub fn drill_through(&mut self, budget_frames: u32) -> Option<Deposited> {
        let power = drill::power_of(
            self.skills.level(skills::MINING),
            self.wallet.upgrade(wallet::DRILL),
        );
        let dt = 1.0 / DRILL_HZ;

        for _ in 0..budget_frames {
            let hit = raycast_solid(
                &self.world,
                self.world.registry(),
                self.eye(),
                self.forward(),
                REACH,
            )?;
            let hardness = self.world.registry().get(hit.id).and_then(|def| def.hardness)?;

            let step = drill::bite(hardness, power, dt);
            let carried = match &self.digging {
                Some((target, progress)) if *target == hit.block => Some(*progress),
                _ => None,
            };
            let bite = drill::advance_bite(carried, step);
            self.digging = Some((hit.block, bite.progress));
            if !bite.through {
                continue;
            }

            self.digging = None;
            if break_block(&mut self.world, &self.events, hit.block).is_err() {
                return None;
            }
            self.journal.record(Command::Break { at: hit.block });

            let landed = drill::deposit(
                self.mining
                    .fleet
                    .base
                    .as_mut()
                    .map(|base| &mut base.stockpile),
                self.world.registry(),
                hit.id,
                false,
            );
            if landed == Deposited::NoBase {
                self.lost += 1;
            }
            let xp = (hardness * skills::MINING_XP_PER_HARDNESS) as u64;
            self.skills.add_xp(skills::MINING, xp);
            return Some(landed);
        }
        None
    }

    /// Is the player standing somewhere they could actually trade?
    ///
    /// The same question `App::interact` asks, and asked the same way: look at
    /// the counter and see whether the counter is what you hit. A plain
    /// distance check is not the same thing and is wrong in a way that
    /// matters — the first played run finished on the shop's *roof*, four
    /// blocks above the till and comfortably inside any radius you like,
    /// which a raycast calls what it is.
    pub fn at_the_counter(&mut self, site: &vx_world::town::TownSite) -> bool {
        let counter = vx_world::town::counter_position(site);
        self.look_at(counter);
        let Some(hit) = raycast_solid(
            &self.world,
            self.world.registry(),
            self.eye(),
            self.forward(),
            REACH,
        ) else {
            return false;
        };
        self.world
            .registry()
            .get(hit.id)
            .is_some_and(|def| def.name == "engine:counter")
    }

    /// Sell everything sellable over the counter, through the real shelf.
    ///
    /// Goes through [`Shop::confirm`] rather than straight to `sell_all`, so
    /// the row ordering, the closed-counter rule and the reputation shading
    /// are all on the path a player's Enter key takes.
    ///
    /// Returns the credits earned.
    pub fn sell_everything(&mut self) -> u64 {
        let home = self.home();
        self.sell_everything_at(&home)
    }

    /// Sell everything sellable at a named town's counter.
    ///
    /// Nothing here is hometown-specific: `Economy` keys its books on
    /// `site.centre`, and a town's opening stock comes from its speciality —
    /// so the same pile is worth different money in different places, which
    /// is the only reason to walk anywhere with it.
    pub fn sell_everything_at(&mut self, site: &vx_world::town::TownSite) -> u64 {
        let site = *site;
        let now = self.journal.tick();
        let before = self.wallet.credits();

        let mut shed = crate::garage::Garage::default();
        let mut rack = crate::arsenal::Arsenal::default();
        let mut kit = crate::intrusion::Intrusions::default();
        let security = self.skills.level(skills::SECURITY);

        // The shelf lists one sell row per kind on the pile, ahead of
        // everything you can buy, so walking the cursor from the top sells
        // each kind in turn until the sell rows run out.
        loop {
            let mut market = self.economy.market(&site, now).clone();
            let rows = {
                let pile = self.mining.fleet.base.as_ref().map(|base| &base.stockpile);
                Shop::rows(
                    pile,
                    &self.wallet,
                    &market,
                    &shed,
                    &rack,
                    &kit,
                    security,
                    &[],
                )
            };
            // Sell rows come first on the shelf, and `open_at_counter` puts
            // the cursor at the top — so opening the panel and pressing
            // Enter sells a kind, exactly as a player does it, and the loop
            // ends when there is no sell row left to be first.
            if !rows.iter().any(|row| matches!(row, crate::shop::Row::Sell(_))) {
                break;
            }
            self.shop.open_at_counter();
            self.shop.confirm(
                self.mining
                    .fleet
                    .base
                    .as_mut()
                    .map(|base| &mut base.stockpile),
                &mut self.wallet,
                &mut market,
                &mut shed,
                &mut rack,
                &mut kit,
                security,
                &[],
                None,
                crate::reputation::Standing::Neutral,
                true,
            );
            *self.economy.market_mut(&site, now) = market;
        }

        self.wallet.credits() - before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed the screenshots use, and the one
    /// `crates/vx-agent/tests/real_terrain.rs` pins its outcrop against — so a
    /// failure here and a bad screenshot are the same bug.
    const SEED: u64 = 2024;

    /// A journal tick is a sixty-fourth of a second, so a budget in seconds
    /// reads like one.
    fn seconds(count: f32) -> u32 {
        (count * 64.0) as u32
    }

    fn centre(block: BlockPos) -> DVec3 {
        DVec3::new(
            f64::from(block.x) + 0.5,
            f64::from(block.y),
            f64::from(block.z) + 0.5,
        )
    }

    /// A session standing in the doorway with a pile declared, which is what a
    /// player has after the one gesture the game never told them to make.
    fn ready() -> Session {
        let mut session = Session::open(SEED);
        let chest = vx_world::town::chest_position(&session.home());
        session.place_base(BlockPos::new(chest.x, chest.y, chest.z));
        session
    }

    #[test]
    fn a_new_player_starts_in_the_house_with_the_counter_up_the_path() {
        let session = Session::open(SEED);
        let home = session.home();
        let spawn = vx_world::town::spawn_position(&home);
        let counter = vx_world::town::counter_position(&home);

        assert_eq!(
            (spawn.x, spawn.z),
            (-14, 9),
            "spawn moved; the run's geometry is stale"
        );
        // Standing, not falling through the floor and not inside anything.
        assert!(
            !vx_world::collides(&session.world, &session.player.aabb()),
            "a new player spawns inside geometry at {:?}",
            session.player.position
        );
        // The shop really is up the path, and close enough to be the first
        // thing you do.
        let walk = ((counter.x - spawn.x) as f64).hypot((counter.z - spawn.z) as f64);
        assert!(walk < 20.0, "the counter is {walk} blocks from the door");
    }

    /// The bug the round was for. Mine with no container down and the ore is
    /// gone — but now it is *reported* gone, which is the difference between
    /// a rule and a hole.
    #[test]
    fn mining_without_a_base_says_so_instead_of_eating_the_ore() {
        let mut session = Session::open(SEED);
        assert!(session.pile().is_none(), "a new player already has a pile");

        // Any solid block underfoot will do; the rule is about the pile, not
        // about what was mined.
        let under = BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32 - 1,
            session.player.position.z.floor() as i32,
        );
        session.look_at(under);
        let landed = session.drill_through(600);

        assert_eq!(
            landed,
            Some(Deposited::NoBase),
            "the drill did not report where the block went"
        );
        assert_eq!(session.lost, 1, "the loss went uncounted");

        // And with a container down, the very same dig lands on the pile.
        let mut kept = ready();
        let under = BlockPos::new(
            kept.player.position.x.floor() as i32,
            kept.player.position.y.floor() as i32 - 1,
            kept.player.position.z.floor() as i32,
        );
        kept.look_at(under);
        let landed = kept.drill_through(600);
        assert!(
            matches!(landed, Some(Deposited::Piled(_))),
            "a block mined over a declared pile did not land on it: {landed:?}"
        );
        assert_eq!(kept.pile().map(|pile| pile.total()), Some(1));
        assert_eq!(kept.lost, 0);
    }

    /// The drill's pace, felt rather than calculated: a block of stone under
    /// the house takes about as long as the constants say it should.
    #[test]
    fn a_block_takes_the_time_the_drill_says_it_takes() {
        let mut session = ready();
        let under = BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32 - 1,
            session.player.position.z.floor() as i32,
        );
        let hardness = session
            .world
            .registry()
            .get(session.world.block(under))
            .and_then(|def| def.hardness)
            .expect("the floor has no hardness");
        let power = drill::power_of(1, 0);
        let expected = drill::seconds_for(hardness, power);

        session.look_at(under);
        let mut frames = 0;
        let mut broke = false;
        while frames < 1_200 {
            frames += 1;
            if session.drill_through(1).is_some() {
                broke = true;
                break;
            }
        }
        assert!(broke, "the floor never gave up");
        let took = frames as f32 / DRILL_HZ;
        assert!(
            (took - expected).abs() < 0.05,
            "a block of {hardness} hardness took {took}s, not {expected}s"
        );
    }

    /// The walk the game opens with: out of the door and up the path to the
    /// counter, holding forward, no pathfinder.
    #[test]
    fn you_can_walk_from_the_door_to_the_counter() {
        let mut session = ready();
        let home_town = session.home();
        let mut route = vec![session.doorstep()];
        route.extend_from_slice(&session.counter_route(&home_town));
        let arrival = session.walk_route(&route, seconds(60.0));
        assert!(
            arrival.reached(),
            "could not walk to the counter: {arrival:?}"
        );
        assert!(
            session.at_the_counter(&home_town),
            "arrived but out of reach at {:?}",
            session.player.position
        );
    }

    /// A loaded pack is slower. The whole shape of the loop — go out light,
    /// come back heavy — depends on it, and it had never been measured end to
    /// end.
    #[test]
    fn a_loaded_pack_walks_slower_than_an_empty_one() {
        let run = |load: u64| {
            let mut session = ready();
            let chest = vx_world::town::chest_position(&session.home());
            if load > 0 {
                if let Some(base) = session.mining.fleet.base.as_mut() {
                    base.stockpile.add("engine:copper_ore".to_string(), load);
                }
            }
            let _ = chest;
            let start = session.player.position;
            // Straight down the path, well short of anything to climb.
            let target = start + DVec3::new(10.0, 0.0, 0.0);
            let arrival = session.walk_to(target, seconds(20.0));
            (arrival, (session.player.position - start).length())
        };

        let (light, _) = run(0);
        let (heavy, _) = run(64);
        assert!(light.reached() && heavy.reached(), "the stroll did not finish");
        assert!(
            heavy.ticks() > light.ticks(),
            "a full pack ({} ticks) was no slower than an empty one ({} ticks)",
            heavy.ticks(),
            light.ticks()
        );
    }

    /// Selling drains the pile, pays the board price, and moves the price for
    /// whoever sells next.
    #[test]
    fn selling_the_haul_pays_and_moves_the_price() {
        let mut session = ready();
        if let Some(base) = session.mining.fleet.base.as_mut() {
            base.stockpile.add("engine:copper_ore".to_string(), 40);
        }
        let site = session.home();
        let now = session.journal.tick();
        let before = crate::shop::sell_price(session.economy.market(&site, now), "engine:copper_ore")
            .expect("copper ore is not sellable");

        let earned = session.sell_everything();

        assert_eq!(earned, 40 * before, "paid {earned} for 40 at {before}");
        assert_eq!(session.wallet.credits(), earned);
        assert_eq!(
            session.pile().map(|pile| pile.total()),
            Some(0),
            "the pile still holds something after selling everything"
        );

        let after = crate::shop::sell_price(session.economy.market(&site, now), "engine:copper_ore")
            .expect("copper ore stopped being sellable");
        assert!(
            after < before,
            "forty loads moved the price from {before} to {after}"
        );
    }

    /// **The round's reason for existing: a whole loop, played.**
    ///
    /// Leave the house, walk out of town to the copper that breaks the
    /// surface on this seed, cut it out by hand, carry it home slower than
    /// you left, and sell it over the counter for money that is really in the
    /// wallet afterwards. Every step through the game's own functions.
    ///
    /// It prints its own log, because the numbers — how far, how long, how
    /// much — are the point as much as the pass is. Run it with
    /// `cargo test -p vx-app -- --nocapture the_whole_loop` to read them.
    #[test]
    fn the_whole_loop_can_be_played_from_the_door_to_the_counter() {
        let mut session = Session::open(SEED);
        let home = session.home();
        let spawn = vx_world::town::spawn_position(&home);
        let counter = vx_world::town::counter_position(&home);
        let chest = vx_world::town::chest_position(&home);

        // The one gesture nothing ever told a new player to make. Without it
        // every block mined below is thrown away — which is the bug this
        // round found, and is exactly why the pile is declared here rather
        // than assumed.
        session.place_base(BlockPos::new(chest.x, chest.y, chest.z));
        eprintln!("spawned at {spawn:?}, pile declared at {chest:?}");

        // Find the copper that breaks the surface out east — the body
        // `crates/vx-agent/tests/real_terrain.rs` pins on this same seed.
        let outcrop = (146, 30);
        session
            .world
            .load_around(BlockPos::new(outcrop.0, 0, outcrop.1).chunk(), 2);
        let body = vx_agent::find_body(&session.world, outcrop, 48)
            .expect("no outcrop where the fixtures say there is one");

        // The block a person would actually walk up to: the highest ore in
        // the body with open sky over it.
        let exposed = body
            .blocks()
            .filter(|pos| vx_agent::is_ore(&session.world, *pos))
            .filter(|pos| {
                session
                    .world
                    .block(BlockPos::new(pos.x, pos.y + 1, pos.z))
                    .is_air()
            })
            .max_by_key(|pos| pos.y)
            .expect("the body never reaches daylight");
        let out = ((exposed.x - spawn.x) as f64).hypot((exposed.z - spawn.z) as f64);
        eprintln!("outcrop at {exposed:?}, {out:.0} blocks from the door");

        // --- Leaving ---------------------------------------------------
        let stand = DVec3::new(
            f64::from(exposed.x) + 0.5,
            f64::from(exposed.y) + 1.0,
            f64::from(exposed.z) + 0.5,
        );
        let went = session.walk_route(&[session.doorstep(), stand], seconds(240.0));
        eprintln!(
            "walked out: {went:?} -> {:?}",
            session.player.position.round()
        );
        assert!(
            went.reached(),
            "could not walk from the house to the ore: {went:?} \
             (stopped {:.0} blocks short)",
            (session.player.position - stand).length()
        );
        let out_ticks = went.ticks();

        // --- Collecting ------------------------------------------------
        let mut mined = 0u64;
        let mut frames = 0u32;
        for _ in 0..24 {
            let Some(next) = body
                .blocks()
                .filter(|pos| vx_agent::is_ore(&session.world, *pos))
                .filter(|pos| {
                    (DVec3::new(
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ) - session.eye())
                    .length()
                        < f64::from(REACH) - 0.5
                })
                .min_by_key(|pos| -pos.y)
            else {
                break;
            };
            session.look_at(next);
            let before = frames;
            let mut landed = None;
            while frames < before + 600 {
                frames += 1;
                if let Some(outcome) = session.drill_through(1) {
                    landed = Some(outcome);
                    break;
                }
            }
            match landed {
                Some(Deposited::Piled(_)) => mined += 1,
                Some(other) => panic!("a mined block went nowhere good: {other:?}"),
                None => break,
            }
        }
        let carried = session.pile().map_or(0, |pile| pile.total());
        eprintln!(
            "mined {mined} blocks in {:.1}s of holding; pack now {carried} \
             (load byte {})",
            frames as f32 / DRILL_HZ,
            session.load_byte()
        );
        assert!(mined > 0, "stood at an outcrop and could not cut any of it");
        assert_eq!(session.lost, 0, "{} blocks were thrown away", session.lost);

        // --- Coming home -----------------------------------------------
        let back = session.walk_route(&session.counter_route(&home), seconds(300.0));
        eprintln!(
            "walked home: {back:?} -> {:?}",
            session.player.position.round()
        );
        assert!(back.reached(), "could not get home to the counter: {back:?}");
        assert!(session.at_the_counter(&home), "home but out of reach of the counter");
        // On the shop's floor, not on its roof. An earlier run of this very
        // loop finished four blocks above the till, comfortably inside any
        // radius you like, trying to sell ore down through the ceiling.
        assert!(
            (session.player.position.y - f64::from(counter.y)).abs() < 1.5,
            "traded from {:.1}, and the counter is at {}",
            session.player.position.y,
            counter.y
        );
        eprintln!(
            "out {out_ticks} ticks laden 0, home {} ticks laden {}",
            back.ticks(),
            session.load_byte()
        );

        // --- The trade -------------------------------------------------
        let site = session.home();
        let now = session.journal.tick();
        let price =
            crate::shop::sell_price(session.economy.market(&site, now), "engine:copper_ore");
        let earned = session.sell_everything();
        eprintln!(
            "sold the haul for {earned} CR at {price:?} a block; wallet now {}",
            session.wallet.credits()
        );

        assert!(earned > 0, "the counter paid nothing for a full pack");
        assert_eq!(session.wallet.credits(), earned);
        assert_eq!(
            session.pile().map_or(0, |pile| pile.total()),
            0,
            "the counter left goods on the pile"
        );
        eprintln!(
            "journal: {} entries over {} ticks",
            session.journal.entries().len(),
            session.tick
        );
    }

    /// A scratch directory that cleans up after itself, like `wear.rs`'s.
    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "vx-session-{name}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// **The bug this round exists to fix.**
    ///
    /// Everything else a player owns survives a save: the wallet, the town's
    /// shifted prices, the chest in the house. The one pile the shop actually
    /// sells out of did not — `Fleet` has no save of any kind, and
    /// `App::save_world` names the tank, the wear ledger and the wells and
    /// stops. So you mined, saved, reloaded, and the goods were gone.
    #[test]
    fn the_pile_survives_a_save_and_a_reload() {
        let directory = scratch("pile");
        let (before, at) = {
            let mut session = ready();
            if let Some(base) = session.mining.fleet.base.as_mut() {
                base.stockpile.add("engine:copper_ore".to_string(), 37);
                base.stockpile.add("engine:stone".to_string(), 4);
            }
            session.save_to(&directory).unwrap();
            (
                session.pile().map(|pile| pile.total()),
                session.mining.fleet.base.as_ref().map(|base| base.position),
            )
        };
        assert_eq!(before, Some(41));

        let reloaded = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();

        assert_eq!(
            reloaded.mining.fleet.base.as_ref().map(|base| base.position),
            at,
            "the base was not declared after a reload"
        );
        let pile = reloaded.pile().expect("no pile after a reload");
        assert_eq!(pile.count("engine:copper_ore"), 37, "the ore did not come back");
        assert_eq!(pile.count("engine:stone"), 4);
        assert_eq!(pile.total(), 41);
    }

    /// Where you were standing survives a save.
    ///
    /// It did not. `App`'s boot planted the body at `spawn_position` on every
    /// load, unconditionally, and nothing ever wrote a position for it to read
    /// instead — so a player who walked to another town, sold their load and
    /// quit came back in their own kitchen with the walk to do again. It
    /// stayed hidden because every save/load test there had ever been ran at
    /// the spawn, where being put back at the spawn is indistinguishable from
    /// working. This one deliberately stands somewhere else first.
    #[test]
    fn where_you_stood_survives_a_save() {
        let directory = scratch("whereabouts");
        let spawn = {
            let session = Session::open(SEED);
            session.player.position
        };
        let (stood, yaw, pitch) = {
            let mut session = ready();
            // The customer's side of the counter, up the path from the house:
            // somewhere a player really ends a session, and not the bed.
            let counter = vx_world::town::counter_stand_position(&session.home());
            session.player.position = DVec3::new(
                f64::from(counter.x) + 0.5,
                f64::from(counter.y),
                f64::from(counter.z) + 0.5,
            );
            session.look_at(vx_world::town::counter_position(&session.home()));
            session.save_to(&directory).unwrap();
            (session.player.position, session.yaw, session.pitch)
        };
        assert_ne!(stood, spawn, "the test never left the spawn");

        let reloaded = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(
            reloaded.player.position, stood,
            "the reload put the body back at the spawn instead of where it was"
        );
        assert_eq!(reloaded.yaw, yaw, "it came back facing somewhere else");
        assert_eq!(reloaded.pitch, pitch);
    }

    /// And the ground is loaded around wherever that turns out to be.
    ///
    /// The order matters and is easy to get wrong: the chunks are pulled in
    /// around the body's position, so a body restored *after* the chunks are
    /// chosen stands in unloaded air, which reads as nothing to stand on and
    /// drops it through the world.
    #[test]
    fn a_reloaded_body_has_ground_under_it() {
        let directory = scratch("footing");
        {
            let mut session = ready();
            let counter = vx_world::town::counter_stand_position(&session.home());
            session.player.position = DVec3::new(
                f64::from(counter.x) + 0.5,
                f64::from(counter.y),
                f64::from(counter.z) + 0.5,
            );
            session.save_to(&directory).unwrap();
        }
        let mut session = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        let landed = session.player.position;
        // A second of standing still. If the chunks under it were not there,
        // this is the second it spends falling.
        session.advance(64, session.command(0));
        assert!(
            (session.player.position.y - landed.y).abs() < 0.5,
            "the reloaded body fell from {landed:?} to {:?}",
            session.player.position
        );
    }

    /// You can walk back into your own town.
    ///
    /// The other half of the loop, and the half nothing had ever walked: every
    /// test until stage 50 either stayed inside the walls or left and stopped.
    /// A town is ringed by a rampart with four gateways and a three-block
    /// ditch, and until the causeway went in the gateway had the ditch across
    /// it — so this walk was impossible for a body that mantles 2.2, at every
    /// town, including your own.
    #[test]
    fn you_can_walk_home_from_outside_the_walls() {
        let mut session = ready();
        let home = session.home();
        let counter = vx_world::town::counter_position(&home);
        let till = DVec3::new(
            f64::from(counter.x) + 0.5,
            session.player.position.y,
            f64::from(counter.z) + 0.5,
        );

        // Stand well outside the wall, on the far side of the ditch, and walk
        // in. Sixty blocks out is past the trace, the ditch and the bastions.
        let outside = DVec3::new(till.x + 60.0, till.y, till.z + 60.0);
        let out = session.cross_country(Some(&home), outside, None);
        let left = session.walk_route(&out, 8 * 400);
        assert!(left.reached(), "could not get out of my own town: {left:?}");

        let mut back = session.cross_country(None, till, Some(&home));
        back.extend_from_slice(&session.counter_route(&home));
        let home_again = session.walk_route(&back, 8 * 600);
        assert!(
            home_again.reached(),
            "could not get back into my own town: {home_again:?}, stopped at {:?}",
            session.player.position
        );
        assert!(
            session.at_the_counter(&home),
            "got into town but not to the counter"
        );
    }

    /// A cross-country route leaves by a gate and arrives by one, and every
    /// leg of it is a place — not a bearing taken from a plaza.
    #[test]
    fn a_route_between_towns_goes_gate_to_gate() {
        let session = ready();
        let home = session.home();
        let elsewhere = *session
            .world
            .towns_near((0, 0), 4_000)
            .iter()
            .find(|site| !site.is_home())
            .expect("no other town on this frontier");
        let counter = vx_world::town::counter_position(&elsewhere);
        let to = DVec3::new(
            f64::from(counter.x) + 0.5,
            session.player.position.y,
            f64::from(counter.z) + 0.5,
        );

        let legs = session.cross_country(Some(&home), to, Some(&elsewhere));
        assert!(legs.len() >= 3, "a two-hundred block route in {} legs", legs.len());
        assert_eq!(legs.last().copied(), Some(to), "the route does not end at the counter");

        let near = |leg: &DVec3, site: &vx_world::town::TownSite| {
            vx_world::fort::fort_for(site)
                .gateways()
                .into_iter()
                .any(|(x, z)| {
                    let (dx, dz) = (leg.x - f64::from(x), leg.z - f64::from(z));
                    (dx * dx + dz * dz).sqrt() < 4.0
                })
        };
        assert!(near(&legs[0], &home), "the route did not leave by a gate");
        assert!(
            legs.iter().any(|leg| near(leg, &elsewhere)),
            "the route did not arrive by a gate"
        );
        // No leg is longer than the leg length, so each gets its own detour
        // allowance and a long walk is not one enormous bearing.
        for pair in legs.windows(2) {
            assert!(
                (pair[1] - pair[0]).length() <= Session::LEG + 1.0,
                "a leg of {:.0} blocks",
                (pair[1] - pair[0]).length()
            );
        }
    }

    /// The second-order half of the same bug: with the base gone, the *next*
    /// block mined reported `NoBase` and evaporated too — stage 48's silent
    /// loss, reappearing across a save boundary.
    #[test]
    fn a_reloaded_session_can_still_mine() {
        let directory = scratch("remine");
        {
            let mut session = ready();
            session.save_to(&directory).unwrap();
        }
        let mut session = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();

        let under = BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32 - 1,
            session.player.position.z.floor() as i32,
        );
        session.look_at(under);
        let landed = session.drill_through(600);
        assert!(
            matches!(landed, Some(Deposited::Piled(_))),
            "a block mined after a reload went nowhere: {landed:?}"
        );
        assert_eq!(session.lost, 0);
    }

    /// The things that already worked, so the fix cannot quietly break them.
    #[test]
    fn the_wallet_and_the_town_books_survive_too() {
        let directory = scratch("books");
        let (credits, price) = {
            let mut session = ready();
            if let Some(base) = session.mining.fleet.base.as_mut() {
                base.stockpile.add("engine:copper_ore".to_string(), 30);
            }
            let earned = session.sell_everything();
            assert!(earned > 0);
            let site = session.home();
            let now = session.journal.tick();
            let price = crate::shop::sell_price(
                session.economy.market(&site, now),
                "engine:copper_ore",
            );
            session.save_to(&directory).unwrap();
            (session.wallet.credits(), price)
        };

        let mut reloaded = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(reloaded.wallet.credits(), credits, "the wallet forgot");
        let site = reloaded.home();
        let now = reloaded.journal.tick();
        assert_eq!(
            crate::shop::sell_price(reloaded.economy.market(&site, now), "engine:copper_ore"),
            price,
            "the town forgot the price a sale moved"
        );
    }

    /// The flier finds ore the eye cannot: buried bodies, read through real
    /// blocks down to the scanner's depth.
    ///
    /// It also exercises the trap: the fleet burns fuel out of the base pile,
    /// and `fuelled()` is true only while there is *no* base — so the moment
    /// you declare one, your flier stops flying unless you have put HHO on
    /// it. The `fuel_the_fleet` call below is not scaffolding, it is what a
    /// player has to do.
    #[test]
    fn a_scan_finds_ore_and_needs_fuel_to_do_it() {
        let mut session = ready();
        session.fuel_the_fleet(4);
        let ticks = session
            .scan_sector((146, 30), 8 * 900)
            .expect("the sweep never finished");
        let pings = session.pings();
        eprintln!("swept in {ticks} ticks, {} pings", pings.len());
        assert!(!pings.is_empty(), "a sector with a known outcrop pinged nothing");
        // A ping names a place with ore under it, at a depth the scanner can
        // actually reach.
        for ping in &pings {
            assert!(
                ping.depth >= 0 && ping.depth <= vx_agent::SCAN_DEPTH + 12,
                "a ping claims a depth of {} blocks",
                ping.depth
            );
            assert!(ping.ore_columns > 0);
        }

        // And with no fuel on the pile the same order goes nowhere: the
        // flier is grounded, not slow.
        let mut dry = ready();
        assert!(
            dry.scan_sector((146, 30), 8 * 120).is_none(),
            "a dry fleet finished a sweep"
        );
    }

    /// Why you would walk anywhere: a refinery is short of ore and pays for
    /// it, a mine is sitting on a hill of the stuff and does not.
    #[test]
    fn a_refinery_pays_more_for_ore_than_a_mine_does() {
        use vx_world::town::Speciality;
        let sites = vx_world::town::towns_near(SEED, (0, 0), 4_000, &|_, _| 90);
        let mine = sites
            .iter()
            .find(|site| site.speciality == Speciality::Mine)
            .expect("no mine on this frontier");
        let refinery = sites
            .iter()
            .find(|site| site.speciality == Speciality::Refinery)
            .expect("no refinery on this frontier");

        let paid = |site: &vx_world::town::TownSite| {
            let mut session = ready();
            if let Some(base) = session.mining.fleet.base.as_mut() {
                base.stockpile.add("engine:copper_ore".to_string(), 20);
            }
            session.sell_everything_at(site)
        };
        let at_mine = paid(mine);
        let at_refinery = paid(refinery);
        eprintln!(
            "20 ore: {at_refinery} CR at the refinery {:?}, {at_mine} CR at the mine {:?}",
            refinery.centre, mine.centre
        );
        assert!(
            at_refinery > at_mine,
            "a refinery paid {at_refinery} and a mine {at_mine} for the same load"
        );
    }

    /// A long walk is not punished for a fence it already climbed.
    ///
    /// `detours_used` was a lifetime cap on one call, so a haul to the next
    /// town — three to five times further than anything this walker had been
    /// proved over — spent its whole allowance early and gave up short.
    #[test]
    fn a_long_walk_is_not_punished_for_obstacles_it_got_past() {
        let mut session = ready();
        let start = session.player.position;
        // Far enough that the walk meets real ground rather than the plaza.
        let target = start + DVec3::new(90.0, 0.0, 0.0);
        let arrival = session.walk_to(target, seconds(400.0));
        assert!(
            arrival.reached(),
            "gave up on a ninety-block walk: {arrival:?}"
        );
    }

    /// The oracle, on a *played* session rather than a scripted one: the same
    /// play produces the same journal and the same ground, twice.
    #[test]
    fn the_same_play_plays_the_same_way_twice() {
        let play = || {
            let mut session = ready();
            let counter = vx_world::town::counter_position(&session.home());
            session.walk_to(centre(counter), seconds(20.0));
            let under = BlockPos::new(
                session.player.position.x.floor() as i32,
                session.player.position.y.floor() as i32 - 1,
                session.player.position.z.floor() as i32,
            );
            session.look_at(under);
            session.drill_through(600);
            let area = vx_agent::VoxelAabb::new(
                BlockPos::new(-40, 40, -40),
                BlockPos::new(40, 110, 40),
            );
            (
                session.player.position,
                session.journal.entries().len(),
                vx_world::region_hash(&session.world, area.min, area.max),
            )
        };
        assert_eq!(play(), play(), "the same play diverged");
    }
}

