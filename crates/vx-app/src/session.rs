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
    /// Slugs in the air, stepped on the crew's own clock — the same list
    /// `Rebuilt` carries, so a shot the log records is a shot the replay
    /// flies.
    pub shots: Vec<crate::arsenal::Shot>,
    /// The pilot input last written down, so `Pilot` is recorded on change
    /// only — it is a held input and `Advance` counts the ticks it covers.
    /// `App` keeps the same field for the same reason.
    last_pilot: Option<vx_agent::PilotCommand>,
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
    /// The machines the player actually owns. A `Session` had none until
    /// stage 52, which is why nothing had ever played the loop this round is
    /// about: you cannot buy a drone without a shed to put it in.
    pub garage: crate::garage::Garage,
    pub journal: CommandLog,
    /// The hold in progress, exactly as `Active::digging` carries it.
    pub digging: Option<(BlockPos, f32)>,
    /// The drill mod's two switches, exactly as `Active::drillmod` carries
    /// them — and the whole point of having them here is that they change
    /// nothing about what a session does. See `the_drill_mod_is_a_lens_not_a_lever`.
    pub drillmod: crate::drillmod::Switches,
    /// The last sonar reading, so a fixture can photograph one.
    pub ping: Option<crate::sonar::Reading>,
    /// What the player is carrying. Stage 55: the pack the load byte has
    /// always been describing and never actually held.
    pub pack: crate::pack::Pack,
    /// What would not fit, lying on the ground where it came out.
    pub drops: crate::drops::Drops,
    /// The capacity last put on the wire, so `Carry` is recorded on change
    /// rather than on every tick.
    carried: Option<u64>,
    /// Which save this session is on, for the manifest.
    pub generation: u64,
    /// Ticks the session has advanced.
    pub tick: u64,
    /// Blocks that would not fit in the pack and are lying on the floor.
    ///
    /// Until stage 55 this counted blocks *lost* — mined with no container
    /// declared, and destroyed. Nothing is destroyed any more, so it counts
    /// what is waiting to be walked back to instead.
    pub left: u64,
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
            last_pilot: None,
            shots: Vec::new(),
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
            garage: crate::garage::Garage::new(),
            journal: CommandLog::new(),
            digging: None,
            drillmod: crate::drillmod::Switches::default(),
            ping: None,
            pack: crate::pack::Pack::new(),
            drops: crate::drops::Drops::new(),
            carried: None,
            generation: 0,
            tick: 0,
            left: 0,
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
        self.mining.wear.save(root)?;
        self.mining.integrity.save(root)?;
        self.mining.wrecks.save(root)?;
        crate::pile::save(&self.mining.fleet, root)?;
        crate::fleet::save(&self.mining.fleet, root)?;
        crate::dig::save(&self.mining, root)?;
        self.garage.save(root)?;
        self.drillmod.save(root)?;
        crate::pack::save(&self.pack, root)?;
        crate::drops::save(&self.drops, root)?;
        crate::pumps::save(&[], root)?;
        crate::whereabouts::save(
            crate::whereabouts::Whereabouts {
                position: self.player.position,
                yaw: self.yaw,
                pitch: self.pitch,
            },
            root,
        )?;
        // The stamp, last, exactly as the live game writes it: a session that
        // sealed differently from the game would be a session testing a
        // different save format.
        self.generation += 1;
        crate::keeping::seal(root, self.generation)?;
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
        session.drillmod.load(root);
        // Refuse a save that never finished writing, exactly as the live boot
        // does — one discipline, not two.
        if let crate::keeping::Verdict::Torn { generation, disagreed } =
            crate::keeping::inspect(root)
        {
            log::warn!("save {generation} did not finish ({}); falling back", disagreed.join(", "));
            let _ = crate::keeping::roll_back(root);
        }
        if let crate::keeping::Verdict::Whole { generation } = crate::keeping::inspect(root) {
            session.generation = generation;
        }
        // Pull the saved ground back in place of the generated ground. The
        // same free function the live game's boot uses, so a chunk that was
        // dug in the last session comes back dug.
        //
        // Every generated chunk has to go *first*, and this used to try that
        // with `unload_beyond(pos, i32::MAX)` — which retains everything,
        // because the radius is squared into a limit no chunk is outside. So
        // nothing was dropped, `load_or_generate` returned early on chunks
        // that were "already loaded", and the reload served generated terrain
        // over the top of the save. Nothing noticed while the only reloads
        // tested were of ground nobody had dug.
        session.world.unload_all();
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
        session.mining.wear.load(root);
        session.mining.integrity.load(root);
        session.mining.wrecks.load(root);
        session.garage.load(root);
        // The pile, the air side and the crew, in the one order that works —
        // and through the same function the live game boots with, because
        // this having been its own hand-written copy is exactly how the live
        // one came to drop the goods a broken container was holding. See
        // `keeping::restore_the_fleet`.
        crate::keeping::restore_the_fleet(&mut session.mining, &mut session.world, root);
        // And what the player themselves is carrying, plus whatever they left
        // on the floor. Through one function for the same reason as above:
        // two hand-written copies of a restore is how the last one drifted.
        let (pack, drops) = crate::keeping::restore_the_pack(root);
        session.pack = pack;
        session.drops = drops;
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

    /// Put the current carrying capacity on the wire, if it has moved.
    ///
    /// Replay has no wallet and no skill sheet — the shop counter and the
    /// fabricator's upgrade rows are live-only — so the size of the pack has
    /// to be *told* to it or replay would fill a stock-sized pack and drop
    /// what a fitted player kept. Recorded on change rather than per tick,
    /// which for most sessions means exactly once. See [`Command::Carry`].
    pub fn note_capacity(&mut self) {
        let now = self.capacity();
        if self.carried == Some(now) {
            return;
        }
        self.carried = Some(now);
        self.journal.record(Command::Carry {
            capacity: now.min(u64::from(u32::MAX)) as u32,
        });
    }

    /// What the player can carry, in `pack::UNIT`s.
    ///
    /// One helper, called from here and from `App::frame` alike. It used to be
    /// two copies of the same arithmetic a thousand lines apart, which is
    /// precisely how the live game and the headless one drifted.
    pub fn capacity(&self) -> u64 {
        crate::pack::capacity(
            self.skills.level(skills::LOGISTICS),
            self.wallet.upgrade(wallet::PACK),
            self.wallet.upgrade(wallet::EXO),
        )
    }

    /// What the pack weighs, as the byte the journal carries.
    ///
    /// The pack on your back, at last — not the pile in a container somewhere
    /// across the map, which is what this measured for fifty-four stages.
    pub fn load_byte(&self) -> u8 {
        crate::pack::load_byte(
            &self.pack,
            self.capacity(),
            self.wallet.upgrade(wallet::EXO),
        )
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
        // Anything the tick killed comes off the roster before the orders are
        // written, the way `App::frame` does it — a crash is a consequence of
        // orders already recorded, so nothing new goes on the wire here.
        self.bury_the_downed();
        self.journal.record(Command::Advance { ticks });
        for _ in 0..ticks {
            self.tick += 1;
            movement::advance_journal_tick(
                &mut self.movement,
                &mut self.player,
                &self.world,
                held,
            );
            // Slugs step on the same clock as everything else, through the
            // same function the live game and the replay both call, and **in
            // the same place in the tick** — after the body, before the next
            // one. Their sweeps go through the crew, because since stage 57 a
            // machine is a thing a round can destroy.
            let sweeps = crate::arsenal::advance_shots(
                &mut self.shots,
                &mut self.world,
                &self.movement.tuning,
            );
            for sweep in &sweeps {
                self.mining
                    .under_fire(sweep.from, sweep.to, crate::integrity::SLUG_HIT);
            }
            self.bury_the_downed();
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
    /// Put at least `cells` canisters of fuel on the fleet's pile.
    ///
    /// **Through the bath, not around it.** This used to add canisters to the
    /// pile directly, which is a cheat a fixture can afford right up until the
    /// moment somebody asks the journal to reproduce the session: the crew
    /// burns fuel every tick it works, a replay with a dry tank runs no crew
    /// at all, and the ground comes back untouched. Stage 56's oracle asked
    /// that question for the first time and got a hole-free hill back.
    ///
    /// So the fixture banks its fuel the way the game does — a run of the
    /// electrolyser, which has been an order since stage 20 — and both sides
    /// do the same arithmetic over the same pile.
    pub fn fuel_the_fleet(&mut self, cells: u64) {
        let mut banked = 0;
        while banked < cells {
            let remaining = cells - banked;
            // The largest run that fits, and the smallest there is when none
            // does: a fixture would rather overshoot than run the crew dry.
            let index = crate::electrolysis::RUNS
                .iter()
                .enumerate()
                .filter(|(_, run)| u64::from(run.cells) <= remaining)
                .max_by_key(|(_, run)| run.cells)
                .map_or(0, |(index, _)| index);
            let Some(run) = crate::electrolysis::run(index) else {
                return;
            };
            if let Some(base) = self.mining.fleet.base.as_mut() {
                base.stockpile.take("engine:copper_bar", run.bars);
                base.stockpile
                    .add(crate::fuel::CELL.to_string(), u64::from(run.cells));
            }
            self.journal.record(Command::Electrolyse { run: index as u32 });
            banked += u64::from(run.cells);
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

    /// Buy one machine of a kind, if the wallet can stand it.
    ///
    /// Straight through [`crate::garage::Garage::buy`], which is where the
    /// rising price and the one-kestrel rule live: the session buys what a
    /// player buys, at what a player pays.
    pub fn buy(&mut self, kind: &str) -> bool {
        self.garage.buy(&mut self.wallet, kind)
    }

    /// How many ground drones the shed holds.
    pub fn crew(&self) -> u32 {
        self.garage.owned(crate::garage::DRONE)
    }

    /// Mark a body and dispatch on a *named* method, if this ground offers it.
    ///
    /// The plain [`Session::dispatch`] takes whatever the planner ranks first,
    /// which is right for play and wrong for measurement: an adit and a
    /// decline move different amounts of rock, so comparing one crew on a
    /// decline with two on an adit says nothing about the crew. Cycling to a
    /// chosen method is exactly what the player's own key does.
    /// Put the crew on a spoil heap over `area`, and journal the order.
    ///
    /// `dispatch_using`'s twin. The shape is *installed* rather than searched
    /// for — see the note on `Command::Heap`'s replay arm — so a session and
    /// its replay build the same tower even if the ground's offer list moves.
    pub fn heap_using(
        &mut self,
        area: vx_agent::VoxelAabb,
        shape: vx_agent::HeapShape,
    ) -> Option<vx_agent::HeapPlan> {
        let crew = self.crew();
        let plan = self.mining.start_heap(&mut self.world, area, shape, crew)?;
        self.journal.record(Command::Heap { area, shape, crew });
        Some(plan)
    }

    /// Take the wheel of a machine, writing the order down.
    ///
    /// The headless twin of `App::toggle_control`, and the same order in the
    /// same place: recorded only once the simulation has *granted* control, so
    /// the log never claims a wheel that was refused.
    pub fn take_wheel(&mut self, machine: crate::mining::MachineRef) -> bool {
        if !self.mining.take_control(machine) {
            return false;
        }
        self.journal.record(Command::Wheel {
            machine: Some(crate::journal::MachineTag::any(machine)),
        });
        true
    }

    /// Hold a pilot input for `ticks`, then let go.
    ///
    /// Records on change only, exactly as `App::frame` does — `Pilot` is a
    /// held input and `Advance` counts the ticks it covers. Getting that
    /// wrong is what stage 49a's whole round was about.
    pub fn fly(&mut self, command: vx_agent::PilotCommand, ticks: u32) {
        if self.last_pilot != Some(command) {
            self.journal.record(Command::piloting(command));
            self.last_pilot = Some(command);
        }
        self.mining.set_pilot_command(command);
        self.advance(ticks, self.command(0));
    }

    /// Put a slug down a bearing from a stated muzzle, writing the order down.
    ///
    /// The headless twin of `App::fire`, through the same
    /// [`crate::arsenal::launch`] and the same order — `Fire` carries the
    /// muzzle and the two quantised angles, and everything the slug then does
    /// is re-derived on both sides from those.
    ///
    /// The muzzle is stated rather than taken from the body because the
    /// muzzle is *on the wire* either way, so a shot from a named place is
    /// exactly as reproducible as one from the eye — and a test that wants a
    /// round in the air does not have to walk a body across a valley to get
    /// one, which would load chunks the log never mentions.
    pub fn fire_from(&mut self, muzzle: DVec3, target: DVec3) {
        let along = (target - muzzle).normalize();
        // Quantised through the very function the live game quantises a look
        // with, so live fire and replay dequantise to the same line.
        let quantised = MoveCommand::looking(
            0,
            f32::atan2(along.x as f32, -(along.z as f32)),
            (along.y as f32).asin(),
        );
        self.journal.record(Command::Fire {
            muzzle: muzzle.to_array(),
            yaw_q: quantised.yaw_q,
            pitch_q: quantised.pitch_q,
        });
        crate::arsenal::launch(
            &mut self.shots,
            &mut self.movement,
            muzzle,
            quantised.yaw_q,
            quantised.pitch_q,
        );
    }

    /// Every machine lost so far, with what is still on it.
    pub fn wrecks(&self) -> &crate::wrecks::Wrecks {
        &self.mining.wrecks
    }

    /// Strip the hulk within reach, writing the order down.
    ///
    /// The headless twin of `App::strip_a_wreck`, through the same
    /// [`Command::Salvage`] and the same [`crate::wrecks::recover`].
    pub fn strip_a_wreck(&mut self) -> Option<crate::wrecks::Wreck> {
        let index = self.mining.wrecks.near(self.player.position)?;
        let at = self.mining.wrecks.iter().nth(index).map(|hulk| hulk.at)?;
        self.journal.record(Command::Salvage { at });
        let wreck = self.mining.wrecks.strip(index)?;
        self.mining.integrity.forget(wreck.machine);
        let capacity = self.capacity();
        crate::wrecks::recover(
            &wreck,
            &mut self.pack,
            &mut self.drops,
            capacity,
            &self.world,
        );
        // The garage shrinks here, as it does in the live game: `Mining` has
        // never heard of the roster and is not going to.
        Some(wreck)
    }

    /// Take every machine lost since this was last called off the books.
    ///
    /// `App::bury_the_downed`'s twin. Called from [`Session::advance`] so a
    /// played session's roster shrinks by itself, the way the live game's
    /// does inside `App::frame`.
    fn bury_the_downed(&mut self) {
        for wreck in self.mining.take_downed() {
            let kind = match wreck.machine {
                crate::mining::MachineRef::Digger(_) => crate::garage::DRONE,
                crate::mining::MachineRef::Flier(_) => crate::garage::FLIER,
                crate::mining::MachineRef::Kestrel => crate::garage::KESTREL,
            };
            self.garage.lose(kind, 1);
            self.left += 1;
        }
    }

    /// How far up the heap has got, if the crew is stacking one.
    pub fn heap_progress(&self) -> Option<(u64, u64)> {
        self.mining.heap_progress()
    }

    pub fn dispatch_using(
        &mut self,
        area: vx_agent::VoxelAabb,
        method: vx_agent::MineMethod,
    ) -> Option<vx_agent::MineMethod> {
        let crew = self.crew();
        self.mining.mark(&mut self.world, area.min);
        self.mining.mark(&mut self.world, area.max);
        // Bounded: the planner offers at most one plan per method, so a full
        // lap of the list is the most that can ever be needed.
        for _ in 0..vx_agent::MineMethod::ALL.len() {
            if self.mining.selected_plan().is_some_and(|plan| plan.method == method) {
                break;
            }
            self.mining.cycle_method();
        }
        // **And write it down.** `App::start_mining` has recorded a
        // `Command::Dispatch` since stage 9; this model of a played session
        // never did, so every oracle test that ran a crew was replaying a
        // journal with no crew in it — the replay dug nothing, and the only
        // reason no test caught that is that no test had ever asked a *crew*
        // to change the ground and then checked the log. Stage 56's heap is
        // the first order that does, and this is what it found.
        let method = self.mining.start(&mut self.world, crew)?;
        if let Some(area) = self.mining.area() {
            self.journal.record(Command::Dispatch { area, method, crew });
        }
        Some(method)
    }

    /// Let the crew work for a while, and say how much landed on the pile.
    ///
    /// The player stands still: this is the drones' clock, not the walker's,
    /// and the journal hears about it the same way `advance` tells it.
    pub fn work(&mut self, ticks: u32) -> u64 {
        let before = self.pile().map_or(0, |pile| pile.total());
        self.advance(ticks, self.command(0));
        self.pile()
            .map_or(0, |pile| pile.total())
            .saturating_sub(before)
    }

    /// Everything the crew is holding but has not delivered yet.
    ///
    /// The number that makes a conservation check possible: blocks cut are on
    /// the pile, in a hopper, or at the mine mouth waiting for a ferry, and
    /// any that are in none of those have been lost.
    pub fn in_transit(&self) -> u64 {
        self.mining
            .operation_snapshot()
            .map_or(0, |dig| {
                dig.stockpile.total()
                    + dig.drones.iter().map(|drone| drone.cargo.total()).sum::<u64>()
            })
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
            // A fresh block under the bit is a drill use, and the sonar
            // goes out — the same rule the live game drills by, through the
            // same function, so a played session exercises the real ping
            // rather than a second copy of it.
            if carried.is_none() && self.drillmod.ping {
                self.ping = Some(self.sonar_at(hit.block));
            }
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

            self.note_capacity();
            let capacity = self.capacity();
            let landed = drill::deposit(
                &mut self.pack,
                &mut self.drops,
                capacity,
                &self.world,
                hit.id,
                hit.block,
                false,
            );
            if matches!(landed, Deposited::Dropped(_)) {
                self.left += 1;
            }
            let xp = (hardness * skills::MINING_XP_PER_HARDNESS) as u64;
            self.skills.add_xp(skills::MINING, xp);
            return Some(landed);
        }
        None
    }

    /// Tip the pack into the fleet's pile, and say how many things moved.
    ///
    /// The other half of a pack: something has to empty it, or a carrying
    /// limit is just a shorter game. `None` means there is nowhere to tip it —
    /// no container has declared a base yet — which is the one place
    /// [`drill::NO_BASE`] still has a job.
    ///
    /// Journalled, because it moves the pile the fleet burns fuel out of and
    /// the shop sells from, and both of those change what ground gets cut.
    /// **Payload-free** on purpose: replay re-derives the manifest from its
    /// own pack rather than trusting a number written in a file, so a
    /// hand-edited log cannot conjure goods. See [`crate::journal`].
    pub fn stow(&mut self) -> Option<u64> {
        self.mining.fleet.base.as_ref()?;
        self.note_capacity();
        self.journal.record(Command::Stow);
        Some(crate::pack::tip(&mut self.pack, &mut self.mining.fleet))
    }

    /// Read the four metres of ground round a block, exactly as the live
    /// game does.
    pub fn sonar_at(&self, at: BlockPos) -> crate::sonar::Reading {
        crate::sonar::ping(&self.world, self.world.registry(), at)
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

        // The session's own shed, not a throwaway: a sale and a purchase have
        // to see the same machines, or buying a drone out of the takings is a
        // drone that exists only until the panel closes.
        let mut shed = std::mem::take(&mut self.garage);
        let mut rack = crate::arsenal::Arsenal::default();
        let mut kit = crate::intrusion::Intrusions::default();
        let security = self.skills.level(skills::SECURITY);

        // The shelf lists one sell row per kind on the pile, ahead of
        // everything you can buy, so walking the cursor from the top sells
        // each kind in turn until the sell rows run out — or until the
        // counter runs out of money, which since stage 52 it can. A stack the
        // till cannot finish leaves its row on the shelf, so the loop has to
        // notice a pass that sold nothing rather than pressing Enter for ever.
        let mut stalled = 0;
        loop {
            let held = self.mining.fleet.held();
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
            if self.mining.fleet.held() == held {
                stalled += 1;
                // Two passes that moved nothing: this counter has bought
                // everything it can afford, and the rest of the load stays on
                // the pile for another town.
                if stalled >= 2 {
                    break;
                }
            } else {
                stalled = 0;
            }
        }

        self.garage = shed;
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

    /// **The drill mod is a lens, not a lever.**
    ///
    /// The whole claim of stage 53, as a test. Two sessions on the same
    /// seed play the same orders — one with the cage and the sonar on, one
    /// with both off — and the world they leave behind and the journal they
    /// wrote must be identical, byte for byte and hash for hash.
    ///
    /// This is what buys the feature its freedom. Because it draws and
    /// reads and never writes, it needs no order on the wire, no journal
    /// version bump and no keyframe; a session recorded with it on replays
    /// against a build with it off. If a pretty effect could ever change
    /// what happened, this goes red, and it should.
    #[test]
    fn the_drill_mod_is_a_lens_not_a_lever() {
        let played = |cage: bool, ping: bool| {
            let mut session = ready();
            session.drillmod = crate::drillmod::Switches { cage, ping };
            // Cut a few blocks out of the ground under the doorstep: enough
            // holds for the sonar to fire on several fresh blocks, which is
            // the code path being cleared of suspicion.
            for step in 0..4 {
                let under = BlockPos::new(
                    session.player.position.x.floor() as i32,
                    session.player.position.y.floor() as i32 - 1 - step,
                    session.player.position.z.floor() as i32,
                );
                session.look_at(under);
                session.drill_through(seconds(6.0));
                session.advance(seconds(0.5), MoveCommand::default());
            }
            let hash = vx_world::world_hash(&session.world);
            let orders = format!("{:?}", session.journal.entries());
            // What you cut goes in your pack now, not into a container across
            // the map — see `pack`.
            let carried = session.pack.total();
            (hash, orders, carried)
        };

        let lit = played(true, true);
        let dark = played(false, false);
        assert_eq!(lit.0, dark.0, "the drill mod changed the ground");
        assert_eq!(lit.1, dark.1, "the drill mod wrote to the journal");
        assert_eq!(lit.2, dark.2, "the drill mod changed what was mined");
        // And it really did dig something, or the three assertions above
        // are three ways of comparing nothing to nothing.
        assert!(lit.2 > 0, "nothing was mined, so nothing was proved");
    }

    /// **The oracle now agrees about the goods, not just the ground.**
    ///
    /// An assertion that could not have been written yesterday.
    /// `Command::Break`'s replay arm broke the block and banked nothing, from
    /// stage 6 to stage 55 — so replay's pile was short every rock the player
    /// had ever cut by hand, and nothing noticed because nothing downstream
    /// looked. It matters: the pile is what the fleet's fuel burns out of,
    /// and a fleet that stops cuts different ground.
    ///
    /// So this sinks a real shaft by hand and tips the haul into a container,
    /// then replays the log over a fresh world and asks for the same four
    /// things: the same ground, the same pack, the same floor, and the same
    /// pile. It found two live bugs on the way in, both of which are now
    /// fixed and neither of which anything else was looking at:
    /// `Command::Break` banking nothing, and `Command::Place` of a container
    /// not declaring the base — so every replayed `Stow` tipped into nowhere.
    ///
    /// The *full* pack half of the rule is proved at the wire in
    /// `journal::tests::a_small_frame_drops_what_it_cannot_hold`, where the
    /// frame can be made small without digging sixty-four blocks first.
    #[test]
    fn a_session_that_mines_and_stows_replays_to_the_same_goods() {
        // `ready` rather than `dug_in`: a base container and nothing else, so
        // every edit to the world is an order in the log. A crew would bring
        // fuel with it, and fuel is *not* journalled — the pile it burns out
        // of is restored from `pile.dat`, not replayed — which is a separate
        // hole and not this test's business.
        let mut session = ready();
        session.note_capacity();

        let start = session.player.position;
        // Off the doorstep first: the house stands on `engine:footing`, which
        // is fortified and cannot be cut, so a shaft sunk where you spawn is
        // one catwalk deep and then nothing.
        let away = start + DVec3::new(9.0, 0.0, 0.0);
        assert!(
            session.walk_to(away, seconds(30.0)).reached(),
            "could not step off the doorstep"
        );
        let mut cut = 0;
        // Straight down, one block at a time, letting the body fall into each
        // hole before cutting the next — the shaft a player actually digs.
        for _ in 0..24 {
            let under = BlockPos::new(
                session.player.position.x.floor() as i32,
                session.player.position.y.floor() as i32 - 1,
                session.player.position.z.floor() as i32,
            );
            session.look_at(under);
            if session.drill_through(seconds(6.0)).is_some() {
                cut += 1;
            }
            session.advance(seconds(1.0), MoveCommand::default());
        }
        assert!(cut >= 8, "only {cut} blocks were cut, so little is proved");
        let tipped = session.stow().expect("no container to tip into");

        let ground = vx_world::world_hash(&session.world);
        let pack = session.pack.clone();
        let floor = session.drops.clone();
        let pile: Vec<(String, u64)> = session
            .pile()
            .map(|pile| {
                pile.entries()
                    .map(|(name, count)| (name.to_string(), count))
                    .collect()
            })
            .unwrap_or_default();

        // Replay the orders over a world generated from the same seed, from
        // where the player actually stood.
        let mut fresh = vx_world::World::new(session.world.seed());
        fresh.load_around(
            BlockPos::new(
                start.x.floor() as i32,
                start.y.floor() as i32,
                start.z.floor() as i32,
            )
            .chunk(),
            KEEP_LOADED,
        );
        let events = vx_core::EventBus::new();
        let rebuilt = crate::journal::replay_from(&session.journal, &mut fresh, &events, start);

        assert_eq!(
            vx_world::world_hash(&fresh),
            ground,
            "the replay dug a different hole"
        );
        assert_eq!(rebuilt.pack, pack, "the replay is carrying something else");
        assert_eq!(rebuilt.drops, floor, "the replay left a different floor");
        let replayed: Vec<(String, u64)> = rebuilt
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| {
                base.stockpile
                    .entries()
                    .map(|(name, count)| (name.to_string(), count))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            replayed, pile,
            "the replay tipped a different pile ({tipped} moved live)"
        );
    }

    /// **The oracle, for a crew that makes the world bigger.**
    ///
    /// Every replay test before this one had an easier job than it looked:
    /// the crew could only ever *remove* blocks, so "the same ground" meant
    /// "the same holes". Stage 56 is the first time an order puts blocks back,
    /// and a heap is far less forgiving than a hole — a hole that is dug in a
    /// different order is the same hole, while a tower stacked in a different
    /// order can be a different tower, or can strand the machine building it.
    ///
    /// So: dig a real body with a real crew, order a real heap out of the
    /// spoil, run it, and demand the same world hash from the log.
    ///
    /// **Three separate bugs stood between this test and green**, and none of
    /// them were in the shapes:
    ///
    /// 1. The board handed out the *apex* first. Courses were posted at
    ///    `-1_000 - (courses - step)`, which rises with height, so the highest
    ///    course outranked the floor and a drone was sent seven blocks up to a
    ///    cell with nothing under it. See
    ///    `vx_agent::operation::tests::a_heap_is_claimed_from_the_ground_up`.
    /// 2. [`Session::dispatch_using`] never wrote `Command::Dispatch` down.
    ///    `App::start_mining` has recorded it since stage 9; this model of a
    ///    played session did not, so a replayed crew was never dispatched.
    /// 3. [`Session::fuel_the_fleet`] conjured canisters straight onto the
    ///    pile. The crew burns fuel every tick it works, so the replay's crew
    ///    stood on a dry tank and cut nothing.
    ///
    /// Two of those three were invisible until an order asked a *crew* to
    /// change the ground and then checked the log — which is the first time
    /// this stage did it.
    #[test]
    fn a_crew_that_stacks_a_heap_replays_to_the_same_ground() {
        let mut session = dug_in();
        // Somewhere clear beside the workings, on ground the crew can reach.
        let base = session
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| base.position)
            .expect("no base");
        // Right beside the workings, which is what a spoil heap is and where
        // the player asked for one. Sited across the town from the dig it
        // feeds, the crew would have to cross the town's own ditch to reach
        // it — a real interaction, and not the one under test here.
        let footprint = vx_agent::VoxelAabb::new(
            BlockPos::new(base.x + 4, base.y, base.z + 4),
            BlockPos::new(base.x + 8, base.y, base.z + 8),
        );

        // Cut first, so the drones are carrying spoil worth stacking.
        session.work(8 * 30);
        let plan = session
            .heap_using(footprint, vx_agent::HeapShape::Pyramid)
            .expect("a pyramid beside the base");
        session.work(8 * 120);

        let stacked = session.heap_progress().map_or(0, |(done, _)| done);
        eprintln!(
            "ordered a {} of {} blocks; the crew stacked {stacked}",
            plan.shape.name(),
            plan.volume
        );
        assert!(stacked > 0, "the crew never stacked a single block");

        let ground = vx_world::world_hash(&session.world);
        let start = session.player.position;

        let mut fresh = vx_world::World::new(session.world.seed());
        fresh.load_around(
            BlockPos::new(
                start.x.floor() as i32,
                start.y.floor() as i32,
                start.z.floor() as i32,
            )
            .chunk(),
            KEEP_LOADED,
        );
        let events = vx_core::EventBus::new();
        let rebuilt = crate::journal::replay_from(&session.journal, &mut fresh, &events, start);

        assert_eq!(
            vx_world::world_hash(&fresh),
            ground,
            "the replay built a different heap"
        );
        assert_eq!(
            rebuilt.mining.heap_progress().map(|(done, _)| done),
            Some(stacked),
            "the replay stacked a different number of blocks"
        );
    }

    /// **The oracle, for a machine you lost.**
    ///
    /// Put a slug through one of your own drones, walk out to what is left of
    /// it, strip it, and demand that the log reproduces all three: the same
    /// ground, the same pack, and the same hole in the roster.
    ///
    /// **Nothing new went on the wire for this.** A machine dying is a
    /// consequence of `Fire` and `Advance`, both recorded since stage 13; the
    /// salvage is `Command::Salvage`, which has meant "strip the thing at this
    /// position" since stage 19. If this passes, damage really is being
    /// re-derived rather than remembered, which is the claim `integrity.rs`
    /// makes in its own module note.
    ///
    /// It also checks the change this stage made to replay's `Advance` arm.
    /// Shot sweeps used to be dropped there — "the craters are the part the
    /// hash checks" — and that stopped being true the moment a sweep could
    /// destroy a machine, because a destroyed machine stops cutting.
    #[test]
    fn a_machine_you_lose_replays_to_the_same_wreck() {
        let mut session = dug_in();
        // Shot before the crew goes underground: a slug stops at the first
        // rock it meets, so a drone down a decline is behind cover. This is a
        // test about the log, not about ballistics through a hillside.
        let drone = crate::mining::MachineRef::Digger(0);
        let centre = session
            .mining
            .machine_eye(drone)
            .expect("the crew never turned up");
        let owned_before = session.garage.owned(crate::garage::DRONE);

        // Straight down from above, where the air is certainly clear — and
        // **without moving the body**. A teleport is not an order, so a test
        // that walks the player about by assignment loads chunks the log
        // never mentions, and the replay then holds a different set of ground
        // to hash. `fire_from` states the muzzle instead, which *is* on the
        // wire.
        let _ = centre;
        for _ in 0..24 {
            let centre = match session.mining.machine_position(drone) {
                Some(at) => glam::DVec3::new(
                    f64::from(at.x) + 0.5,
                    f64::from(at.y) + 0.5,
                    f64::from(at.z) + 0.5,
                ),
                None => break,
            };
            session.fire_from(centre + glam::DVec3::new(0.0, 8.0, 0.0), centre);
            // One tick: a slug crosses eight blocks in far less, and the
            // drone is walking, so a longer wait is a longer lead to miss by.
            session.work(1);
            if !session.wrecks().is_empty() {
                break;
            }
        }
        assert_eq!(session.wrecks().len(), 1, "the drone shrugged off two dozen rounds");
        assert_eq!(
            session.garage.owned(crate::garage::DRONE),
            owned_before - 1,
            "the roster did not shrink"
        );

        // Walk out to it and strip it. The body is put beside the hulk rather
        // than pathed to it — this test is about the log, not about walking —
        // and then **put back**, because where the body has been is what
        // decided which chunks this world is holding, and the replay's world
        // loads around the position it is handed. Leave the body somewhere it
        // never walked and the two sides hash different sets of ground for a
        // reason that has nothing to do with the feature.
        let at = session.wrecks().iter().next().expect("no hulk").at;
        let stood = session.player.position;
        session.player.position = glam::DVec3::new(
            f64::from(at.x) + 0.5,
            f64::from(at.y),
            f64::from(at.z) + 0.5,
        );
        session.strip_a_wreck().expect("nothing within reach");
        session.player.position = stood;
        assert!(session.wrecks().is_empty(), "the hulk stayed");
        let carried = session.pack.total();
        assert!(carried > 0, "stripping a hulk yielded nothing");

        let ground = vx_world::world_hash(&session.world);
        let start = session.player.position;
        let mut fresh = vx_world::World::new(session.world.seed());
        fresh.load_around(
            BlockPos::new(
                start.x.floor() as i32,
                start.y.floor() as i32,
                start.z.floor() as i32,
            )
            .chunk(),
            KEEP_LOADED,
        );
        let events = vx_core::EventBus::new();
        let rebuilt = crate::journal::replay_from(&session.journal, &mut fresh, &events, start);

        assert_eq!(
            vx_world::world_hash(&fresh),
            ground,
            "the replay left different ground"
        );
        assert!(
            rebuilt.mining.wrecks.is_empty(),
            "the replay is still holding a hulk the session stripped"
        );
        assert_eq!(
            rebuilt.pack.total(),
            carried,
            "the replay carried a different haul off the wreck"
        );
    }

    /// A hulk, and what is on it, survives a save.
    #[test]
    fn a_wreck_is_still_lying_there_after_a_reload() {
        let directory = scratch("wrecks");
        let (at, aboard) = {
            let mut session = ready();
            session.wallet.earn(5_000);
            session.fuel_the_fleet(24);
            assert!(session.buy(crate::garage::FLIER));
            session.ensure_flier();
            let flier = crate::mining::MachineRef::Flier(0);
            assert!(session.take_wheel(flier));
            session.fly(
                vx_agent::PilotCommand {
                    climb: -1,
                    ..Default::default()
                },
                8 * 40,
            );
            let hulk = session.wrecks().iter().next().expect("no hulk").clone();
            session.save_to(&directory).expect("could not save");
            (hulk.at, hulk.haul().iter().map(|(_, n)| n).sum::<u64>())
        };

        let back = Session::load_from(&directory).expect("could not load");
        assert_eq!(back.wrecks().len(), 1, "the hulk evaporated over a save");
        let hulk = back.wrecks().iter().next().expect("no hulk");
        assert_eq!(hulk.at, at);
        assert_eq!(hulk.haul().iter().map(|(_, n)| n).sum::<u64>(), aboard);

        std::fs::remove_dir_all(&directory).ok();
    }

    /// Nothing is conjured: what the crew stacked came out of the ground it
    /// dug, not out of nowhere. The fourth term in the conservation identity
    /// this game has kept since stage 52.
    #[test]
    fn a_heap_is_built_out_of_what_was_dug() {
        let mut session = dug_in();
        let base = session
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| base.position)
            .expect("no base");
        let footprint = vx_agent::VoxelAabb::new(
            BlockPos::new(base.x + 4, base.y, base.z + 4),
            BlockPos::new(base.x + 8, base.y, base.z + 8),
        );
        session.work(8 * 30);
        session
            .heap_using(footprint, vx_agent::HeapShape::Pyramid)
            .expect("a pyramid beside the base");
        session.work(8 * 120);

        let (stacked, _) = session.heap_progress().expect("no heap running");
        // Vacuous while `stacked` is zero, and that is the point of saying so:
        // this asserts the *conservation*, which holds whether or not the
        // crew managed to reach the footprint. The building itself is proved
        // in `operation::tests`, and the played route is the ignored test
        // above.
        let plan = session.mining.heap().expect("no plan").clone();
        let standing = plan
            .cells
            .iter()
            .filter(|cell| session.world.is_solid(**cell))
            .count() as u64;
        assert!(
            standing >= stacked,
            "the crew claims {stacked} stacked but only {standing} cells are solid"
        );
        // And every drone is still somewhere it can stand — nobody built
        // themselves into their own tower.
        for drone in session.mining.drone_positions() {
            assert!(
                !session.world.is_solid(drone),
                "a drone is inside a block at {drone:?}"
            );
        }
    }

    /// The other half: with the ping on, a played session really does come
    /// back with a reading, through the same function the live game uses.
    #[test]
    fn a_played_session_hears_the_ground_it_drills() {
        let mut session = ready();
        let under = BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32 - 1,
            session.player.position.z.floor() as i32,
        );
        session.look_at(under);
        session.drill_through(seconds(6.0));

        let reading = session.ping.as_ref().expect("the drill never pinged");
        assert_eq!(reading.centre, under);
        assert!(reading.cells > 0, "the ground under the doorstep read as empty");

        // And with the switch off, no ping — which is what "toggleable"
        // means when it is written down rather than described.
        let mut quiet = ready();
        quiet.drillmod.ping = false;
        quiet.look_at(under);
        quiet.drill_through(seconds(6.0));
        assert!(quiet.ping.is_none(), "the sonar fired with its switch off");
    }

    /// Both switches come back off a save, like everything else that is a
    /// choice the player made.
    #[test]
    fn the_switches_come_back_with_the_world() {
        let root = std::env::temp_dir().join("gamingg-session-drillmod");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");

        let mut session = ready();
        session.drillmod.cage = false;
        session.save_to(&root).expect("save");

        let reloaded = Session::load_from(&root).expect("load");
        assert!(!reloaded.drillmod.cage, "the cage came back on");
        assert!(reloaded.drillmod.ping, "the sonar came back off");
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

    /// What stage 48 fixed, and what stage 55 made moot.
    ///
    /// This used to assert that mining with no container declared reported
    /// `NoBase` and yielded nothing — the best answer available while the only
    /// pile in the game stood in a box across the map. There is a pack now, so
    /// a new player who walks out of the door and mines *keeps what they cut*,
    /// container or no container.
    #[test]
    fn mining_before_you_own_anything_still_fills_your_pack() {
        let mut session = Session::open(SEED);
        assert!(session.pile().is_none(), "a new player already has a pile");

        let under = BlockPos::new(
            session.player.position.x.floor() as i32,
            session.player.position.y.floor() as i32 - 1,
            session.player.position.z.floor() as i32,
        );
        session.look_at(under);
        let landed = session.drill_through(600);

        assert!(
            matches!(landed, Some(Deposited::Packed(_))),
            "a block mined with no container went somewhere else: {landed:?}"
        );
        assert_eq!(session.pack.total(), 1, "the block did not reach the pack");
        assert_eq!(session.left, 0, "nothing should have hit the floor");
        assert!(session.pile().is_none(), "a pile appeared out of nowhere");

        // And a container standing changes nothing about the swing: what you
        // break is yours until you tip it in.
        let mut kept = ready();
        let under = BlockPos::new(
            kept.player.position.x.floor() as i32,
            kept.player.position.y.floor() as i32 - 1,
            kept.player.position.z.floor() as i32,
        );
        kept.look_at(under);
        let landed = kept.drill_through(600);
        assert!(
            matches!(landed, Some(Deposited::Packed(_))),
            "a block mined over a declared pile did not land in the pack: {landed:?}"
        );
        assert_eq!(kept.pile().map(|pile| pile.total()), Some(0));

        assert_eq!(kept.left, 0);
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

    /// The rough edge this round exists to delete, as one test.
    ///
    /// It has been in the README since stage 16: *"mining sixty blocks makes
    /// you walk at 0.55x until you sell, with the pack sat in a container
    /// somewhere else entirely."* So the assertion has two halves, and the
    /// first one is the one that would have failed yesterday — a container
    /// full of ore across the map must not slow you down at all, and what is
    /// actually on your back must.
    #[test]
    fn what_you_are_carrying_is_what_slows_you_down() {
        let stroll = |set: &dyn Fn(&mut Session)| {
            let mut session = ready();
            set(&mut session);
            let start = session.player.position;
            // Straight down the path, well short of anything to climb.
            let target = start + DVec3::new(10.0, 0.0, 0.0);
            let arrival = session.walk_to(target, seconds(20.0));
            (arrival, session.load_byte())
        };

        let (light, light_load) = stroll(&|_| {});
        let (in_a_box, box_load) = stroll(&|session| {
            // Sixty-four ore in a container standing somewhere else entirely.
            if let Some(base) = session.mining.fleet.base.as_mut() {
                base.stockpile.add("engine:copper_ore".to_string(), 64);
            }
        });
        let (on_your_back, back_load) = stroll(&|session| {
            for _ in 0..64 {
                session.pack.stow("engine:copper_ore", session.capacity());
            }
        });

        assert!(
            light.reached() && in_a_box.reached() && on_your_back.reached(),
            "the stroll did not finish"
        );
        assert_eq!(light_load, 0, "an empty pack is not an empty load");
        assert_eq!(
            box_load, 0,
            "a pile in a container across the map still weighed on the player"
        );
        assert_eq!(
            in_a_box.ticks(),
            light.ticks(),
            "goods you are nowhere near changed how fast you walk"
        );
        assert!(back_load > 0, "sixty-four ore on your back weighed nothing");
        assert!(
            on_your_back.ticks() > light.ticks(),
            "a full pack ({} ticks) was no slower than an empty one ({} ticks)",
            on_your_back.ticks(),
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
                Some(Deposited::Packed(_)) => mined += 1,
                Some(Deposited::Dropped(_)) => break,
                Some(other) => panic!("a mined block went nowhere good: {other:?}"),
                None => break,
            }
        }
        let carried = session.pack.total();
        eprintln!(
            "mined {mined} blocks in {:.1}s of holding; pack now {carried} \
             (load byte {})",
            frames as f32 / DRILL_HZ,
            session.load_byte()
        );
        assert!(mined > 0, "stood at an outcrop and could not cut any of it");
        assert_eq!(session.left, 0, "{} blocks would not fit", session.left);

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

        // --- Tipping the pack ------------------------------------------
        // New in stage 55, and the reason the walk home means anything: what
        // you cut is on your back until you put it down. The counter sells
        // out of the fleet's pile, so nothing can be sold until the pack has
        // been emptied into a container.
        let carrying = session.pack.total();
        let tipped = session.stow().expect("no container to tip into");
        eprintln!("tipped {tipped} of {carrying} into the container");
        assert_eq!(tipped, carrying, "the pack did not empty");
        assert!(session.pack.is_empty(), "the pack kept something back");
        assert_eq!(
            session.pile().map_or(0, |pile| pile.total()),
            carrying,
            "the goods did not reach the pile"
        );
        assert_eq!(session.load_byte(), 0, "an emptied pack still weighed");

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
    /// A session with a container down, a drone bought and a crew already
    /// cutting: the state this round is about.
    fn dug_in() -> Session {
        let mut session = ready();
        // Credits the honest way is a whole play; the shed is what is under
        // test here, so the wallet is seeded and the drone is *bought*.
        session.wallet.earn(5_000);
        // Declaring a base is the moment the fleet starts wanting fuel —
        // `Mining::fuelled` reports a machine fuelled only while there is *no*
        // base — so a crew on a dry pile never turns a wheel. Stage 50 found
        // that trap; this is it, avoided.
        session.fuel_the_fleet(24);
        assert!(session.buy(crate::garage::DRONE), "could not buy a drone");
        assert_eq!(session.crew(), 1);

        // A body under the ground beside the house, big enough to take a
        // while and small enough to stay inside the loaded chunks.
        let base = session
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| base.position)
            .expect("no base");
        let area = vx_agent::VoxelAabb::new(
            BlockPos::new(base.x + 4, base.y - 6, base.z + 4),
            BlockPos::new(base.x + 9, base.y - 2, base.z + 9),
        );
        assert!(
            session
                .dispatch_using(area, vx_agent::MineMethod::Decline)
                .is_some(),
            "the dispatch never started"
        );
        session
    }

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

    /// A crew set to work survives a save, and keeps working.
    ///
    /// The bug this round is named for. `Mining::operation` was a private
    /// field that no save file anywhere named, so buying drones, marking a
    /// body and setting them cutting was work you lost the moment you quit —
    /// the hole stayed dug and nothing was in it. Every drone, its cargo, its
    /// claimed job and the board itself have to come back, or a restored crew
    /// stands idle on work nobody posted.
    #[test]
    fn a_dig_in_progress_survives_a_save() {
        let directory = scratch("dig");
        let (crew, cut, at, transit) = {
            let mut session = dug_in();
            session.work(8 * 20);
            let dig = session
                .mining
                .operation_snapshot()
                .expect("the crew was never dispatched");
            assert!(
                dig.board.entries.iter().any(|(_, held)| held.is_some()),
                "nobody had claimed a job to test the claim with"
            );
            session.save_to(&directory).unwrap();
            (
                dig.drones.len(),
                session.pile().map_or(0, |pile| pile.total()),
                dig.drones.iter().map(|drone| drone.position).collect::<Vec<_>>(),
                session.in_transit(),
            )
        };

        let mut session = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();

        let dig = session
            .mining
            .operation_snapshot()
            .expect("the dispatch did not survive the save");
        assert_eq!(dig.drones.len(), crew, "the crew came back a different size");
        assert_eq!(
            dig.drones.iter().map(|drone| drone.position).collect::<Vec<_>>(),
            at,
            "the crew came back somewhere else"
        );
        assert!(
            dig.board.entries.iter().any(|(_, held)| held.is_some()),
            "the claims did not come back"
        );
        assert_eq!(session.pile().map_or(0, |pile| pile.total()), cut);
        assert_eq!(
            session.in_transit(),
            transit,
            "goods in a hopper were lost or doubled across the save"
        );

        // And it is still a *working* crew, not a museum piece.
        let before = session.pile().map_or(0, |pile| pile.total()) + session.in_transit();
        session.work(8 * 30);
        let after = session.pile().map_or(0, |pile| pile.total()) + session.in_transit();
        assert!(
            after > before,
            "the restored crew cut nothing: {before} -> {after}"
        );
    }

    /// A restored crew digs the same hole, because it holds the same ground.
    ///
    /// The subtle half of the fix. A drone reads the world to decide what to
    /// cut and an unloaded chunk reads as *air*, so a dispatch whose span is
    /// not resident digs differently depending on where the player is
    /// standing. `Mining::start` pins against exactly that; a restore that
    /// forgot the pin would hand the replay oracle an excavation whose outcome
    /// depended on the camera.
    #[test]
    fn a_restored_crew_holds_its_own_ground() {
        let directory = scratch("pinned");
        {
            let mut session = dug_in();
            session.work(8 * 20);
            session.save_to(&directory).unwrap();
        }
        let mut session = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();

        let dig = session.mining.operation_snapshot().expect("no dispatch");
        // Every job region the crew is going to read is resident. Unloaded
        // ground is the thing that would read as air.
        for (job, _) in &dig.board.entries {
            for corner in [job.region.min, job.region.max] {
                assert!(
                    session.world.chunk(corner.chunk()).is_some(),
                    "the crew's ground at {corner:?} was not held after a reload"
                );
            }
        }
        // And it stays held while the crew works, rather than being evicted by
        // the streamer the first time the player moves.
        session.work(8 * 10);
        let dig = session.mining.operation_snapshot().expect("no dispatch");
        for (job, _) in &dig.board.entries {
            assert!(
                session.world.chunk(job.region.min.chunk()).is_some(),
                "the crew's ground was let go while it was still working"
            );
        }
    }

    /// The same dispatch writes the same bytes twice — the refusal lists come
    /// off `HashSet`s, whose order is not stable, so they are sorted on the
    /// way out. A save that churned would make every quit a fresh write.
    #[test]
    fn the_same_dispatch_writes_the_same_bytes() {
        let directory = scratch("stable");
        let mut session = dug_in();
        session.work(8 * 20);
        crate::dig::save(&session.mining, &directory).unwrap();
        let first = std::fs::read(directory.join("dig.dat")).unwrap();
        crate::dig::save(&session.mining, &directory).unwrap();
        let second = std::fs::read(directory.join("dig.dat")).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(first, second);
    }

    /// A missing or damaged file is no dispatch at all, never a world that
    /// refuses to open — and a cancelled dig stays cancelled rather than being
    /// resurrected by a stale file from two saves ago.
    #[test]
    fn a_missing_or_damaged_dig_is_no_dig_at_all() {
        let directory = scratch("damaged");
        let mut session = ready();
        crate::dig::load(&mut session.mining, &mut session.world, &directory);
        assert!(!session.mining.is_running(), "a missing file invented a crew");

        std::fs::write(directory.join("dig.dat"), b"NOPE and then some").unwrap();
        crate::dig::load(&mut session.mining, &mut session.world, &directory);
        assert!(!session.mining.is_running(), "a damaged file invented a crew");

        // And a save taken with nothing running says so out loud.
        crate::dig::save(&session.mining, &directory).unwrap();
        let mut fresh = ready();
        crate::dig::load(&mut fresh.mining, &mut fresh.world, &directory);
        std::fs::remove_dir_all(&directory).ok();
        assert!(!fresh.mining.is_running(), "an idle save invented a crew");
    }

    /// Every sector you have already swept survives a save.
    ///
    /// A sweep burns HHO off the pile, so a survey is bought and paid for —
    /// and `pile.dat` wrote the base and nothing else, so the receipt was
    /// binned every time. You came back to a fleet that had never scanned
    /// anything, re-flew ground you had already covered, spent the fuel twice,
    /// and had no way of knowing.
    #[test]
    fn the_sectors_you_paid_to_scan_survive_a_save() {
        let directory = scratch("surveys");
        let (pings, depth) = {
            let mut session = ready();
            session.fuel_the_fleet(12);
            session
                .scan_sector((146, 30), 8 * 900)
                .expect("the flier never finished its sweep");
            let pings = session.pings();
            assert!(!pings.is_empty(), "the sweep found nothing to save");
            session.save_to(&directory).unwrap();
            (pings, session.mining.fleet.scan_depth)
        };

        let session = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(session.pings(), pings, "the pings did not come back");
        assert_eq!(session.mining.fleet.scan_depth, depth);
        assert!(
            !session.mining.fleet.fliers.is_empty(),
            "the fleet came back with no fliers"
        );
    }

    /// Goods are conserved across a save, wherever they happen to be.
    ///
    /// The obvious way to get the crew's persistence wrong is to write a
    /// drone's cargo *and* an operation stockpile that already counted it, and
    /// come back with twice the ore. The obvious way to get it wrong in the
    /// other direction is to drop a hopper on the floor. Neither: cut, save
    /// mid-haul, reload, and the pile plus everything in transit adds up to
    /// exactly what it did.
    #[test]
    fn nothing_is_lost_or_doubled_by_saving_mid_haul() {
        let directory = scratch("conserved");
        let (piled, carried) = {
            let mut session = dug_in();
            // Long enough that somebody is part way through a run.
            session.work(8 * 60);
            let carried = session.in_transit();
            assert!(carried > 0, "nobody was carrying anything to test with");
            session.save_to(&directory).unwrap();
            (session.pile().map_or(0, |pile| pile.total()), carried)
        };

        let session = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(
            session.pile().map_or(0, |pile| pile.total()),
            piled,
            "the pile changed across a save"
        );
        assert_eq!(
            session.in_transit(),
            carried,
            "goods in transit were lost or doubled"
        );
    }

    /// Breaking your own container keeps the goods.
    ///
    /// It used to destroy them: `main.rs` took the pile back out of the fleet,
    /// logged how much was "set aside", and let it fall out of scope — so one
    /// stray click on your own container was the single most expensive
    /// mistake available. The goods are held instead, and the next container
    /// picks them up. Held, not doubled: the total never moves.
    #[test]
    fn breaking_the_container_holds_the_goods_rather_than_eating_them() {
        let mut session = ready();
        if let Some(base) = session.mining.fleet.base.as_mut() {
            base.stockpile.add("engine:copper_ore".to_string(), 31);
        }
        let held = session.mining.fleet.held();
        assert_eq!(held, 31);

        let at = session
            .mining
            .fleet
            .base
            .as_ref()
            .map(|base| base.position)
            .expect("no base");
        let broken = session.mining.fleet.clear_base();
        assert_eq!(broken, 31, "the break did not report what it held");
        assert!(session.mining.fleet.base.is_none());
        assert_eq!(
            session.mining.fleet.held(),
            31,
            "breaking the container destroyed the goods"
        );

        // A new container anywhere picks them up, once.
        session.place_base(BlockPos::new(at.x, at.y, at.z + 1));
        assert_eq!(
            session.pile().map_or(0, |pile| pile.total()),
            31,
            "the goods did not come back with the new container"
        );
        assert_eq!(session.mining.fleet.held(), 31, "the goods were doubled");
        assert_eq!(
            session.mining.fleet.orphaned().total(),
            0,
            "the goods are still in holding as well as on the pile"
        );
    }

    /// And goods in holding survive a save, so quitting between breaking a
    /// container and placing the next one is not a way to lose a barrow —
    /// nor, in the other direction, to walk away with two.
    #[test]
    fn goods_waiting_for_a_container_survive_a_save() {
        let directory = scratch("orphan");
        {
            let mut session = ready();
            if let Some(base) = session.mining.fleet.base.as_mut() {
                base.stockpile.add("engine:copper_ore".to_string(), 17);
            }
            session.mining.fleet.clear_base();
            assert_eq!(session.mining.fleet.orphaned().total(), 17);
            session.save_to(&directory).unwrap();
        }
        let mut session = Session::load_from(&directory).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(
            session.mining.fleet.held(),
            17,
            "goods waiting for a container did not survive the save"
        );
        assert!(session.mining.fleet.base.is_none(), "a base reappeared");

        let home = vx_world::town::chest_position(&session.home());
        session.place_base(BlockPos::new(home.x, home.y, home.z));
        assert_eq!(session.pile().map_or(0, |pile| pile.total()), 17);
        assert_eq!(session.mining.fleet.held(), 17, "the goods were doubled");
    }

    /// Cancelling a dispatch does not mint or destroy anything either: the
    /// crew stands down, the hole stays dug, and the pile is untouched.
    #[test]
    fn cancelling_a_dispatch_conserves_the_pile() {
        let mut session = dug_in();
        session.work(8 * 40);
        let piled = session.pile().map_or(0, |pile| pile.total());
        session.mining.cancel(&mut session.world);
        assert!(!session.mining.is_running(), "the dispatch survived a cancel");
        // `cancel` keeps the fleet, which is what the pile lives on.
        assert_eq!(
            session.pile().map_or(0, |pile| pile.total()),
            piled,
            "cancelling changed the pile"
        );
    }

    /// Two drones cut faster than one — the whole reason to spend the money.
    ///
    /// Controlled: the same ground, the same method, the same window. The
    /// first `--payroll` run compared a lone drone on a decline against a pair
    /// on an adit and reported the pair as *slower*, which was a fact about
    /// the planner rather than the crew.
    #[test]
    fn two_drones_cut_faster_than_one() {
        fn cut_with(crew: u32, at: BlockPos) -> u64 {
            let mut session = ready();
            session.wallet.earn(50_000);
            session.fuel_the_fleet(200);
            for _ in 0..crew {
                assert!(session.buy(crate::garage::DRONE), "could not buy a drone");
            }
            assert_eq!(session.crew(), crew);
            let face = vx_agent::VoxelAabb::new(
                BlockPos::new(at.x, at.y - 10, at.z),
                BlockPos::new(at.x + 10, at.y - 2, at.z + 10),
            );
            assert!(
                session
                    .dispatch_using(face, vx_agent::MineMethod::Decline)
                    .is_some(),
                "no decline on this ground"
            );
            let before = session.pile().map_or(0, |pile| pile.total()) + session.in_transit();
            session.advance(8 * 90, session.command(0));
            session.pile().map_or(0, |pile| pile.total()) + session.in_transit() - before
        }

        let base = vx_world::town::chest_position(&Session::open(SEED).home());
        let at = BlockPos::new(base.x + 4, base.y, base.z + 4);
        let one = cut_with(1, at);
        let two = cut_with(2, at);
        assert!(one > 0, "a single drone cut nothing at all");
        assert!(
            two > one,
            "two drones ({two}) did not beat one ({one}) on the same ground"
        );
    }

    /// The census: everything a saved session writes is accounted for.
    ///
    /// Both directions, which is the point. A file on the list that is missing
    /// means a subsystem quietly stopped saving; a file on disk that is *not*
    /// on the list means a new one arrived without anybody deciding whether it
    /// should persist — which is exactly how the pile, the player and the crew
    /// each went three stages before anyone noticed. The table in `main.rs`'s
    /// module docs is the human-readable half of the same promise.
    #[test]
    fn the_census_covers_every_saved_subsystem() {
        const EXPECTED: [&str; 18] = [
            "log.dat",
            "wallet.dat",
            "player.dat",
            "economy.dat",
            "fuel.dat",
            "wear.dat",
            "integrity.dat",
            "wrecks.dat",
            "pile.dat",
            "fleet.dat",
            "dig.dat",
            "garage.dat",
            "whereabouts.dat",
            "drillmod.dat",
            "pack.dat",
            "drops.dat",
            "pumps.dat",
            "manifest.dat",
        ];

        let directory = scratch("census");
        let mut session = dug_in();
        session.work(8 * 20);
        session.save_to(&directory).unwrap();

        let written: std::collections::BTreeSet<String> = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".dat"))
            .collect();
        std::fs::remove_dir_all(&directory).ok();

        for name in EXPECTED {
            assert!(
                written.contains(name),
                "{name} was not written: a subsystem stopped saving"
            );
        }
        for name in &written {
            assert!(
                EXPECTED.contains(&name.as_str()),
                "{name} is saved but not in the census: decide whether it should persist, \
                 then add it here and to main.rs's table"
            );
        }
    }

    /// **A patch you marked out but had not sent anybody at survives.**
    ///
    /// `dig.dat` carried a *running* dispatch from stage 52 and nothing else,
    /// so the two corners you picked by eye — which is a decision, and the
    /// step every dispatch starts from — were binned by the save that was
    /// meant to be protecting them.
    #[test]
    fn a_patch_you_marked_but_never_dug_survives_a_save() {
        let directory = scratch("marked");
        let mut session = ready();
        let at = session.player.position;
        let corner = BlockPos::new(at.x.floor() as i32 + 3, at.y.floor() as i32 - 1, at.z.floor() as i32);
        let far = BlockPos::new(corner.x + 4, corner.y - 2, corner.z + 4);
        session.mining.mark(&mut session.world, corner);
        session.mining.mark(&mut session.world, far);
        let (marked, _) = session.mining.marked();
        assert_eq!(marked, [corner, far], "the fixture did not mark anything");
        let area = session.mining.area().expect("two corners make an area");

        session.save_to(&directory).expect("save");
        drop(session);

        let reloaded = Session::load_from(&directory).expect("load");
        assert_eq!(
            reloaded.mining.marked().0,
            [corner, far],
            "the corners were binned by the save"
        );
        assert_eq!(
            reloaded.mining.area(),
            Some(area),
            "the area came back as a different area"
        );
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
            matches!(landed, Some(Deposited::Packed(_))),
            "a block mined after a reload went nowhere: {landed:?}"
        );
        assert_eq!(session.left, 0);
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

