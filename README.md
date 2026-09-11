# gamingg

A modular voxel engine in Rust, Linux-first, built for mods distributed through
Steam Workshop.

## Provenance

This is **original code**. It contains no Minecraft source, no decompiled game
code, and no Mojang assets, and it is not a fork of `mcp940` or any other
decompiled-source repository — redistributing decompiled game code violates
both the MCP license and the Minecraft EULA.

The 1.12-era conventions this borrows are design *reference points* only
(16×16 chunk columns, 256-block world height, ~1m cubic voxels). All textures
and content are original; placeholder art is generated procedurally until real
art exists.

## Status

M3 stage 10a is in, and it started by finding that the game's opening loop was
not implemented. **Machines are earned now.** A flier used to arrive the moment
the world opened and a whole crew of drones appeared, free, on every dispatch —
so there was nothing to mine *for* and the economy had no sink. You keep one
free starter flier; every drone after that is bought with credits at a shop
counter, and each one costs half again what the last did. Mine by hand, sell
your ore, buy your first drone: that is the opening.

**And the maps read like paper maps.** Pick a trade run at a beacon console and
it draws a map of the route — this town, the destination pinned, live caravans
crawling between them, and everything you have never walked pitch black — with
a bearing under it: `IRONREACH - NW 206M`. If you have not been there, it is a
pin sitting in the dark and you have to go and find it. `Tab` on the handheld
turns to the same map centred on you, with your machines, the towns you have
found and the traffic in the air.

Before that, stage 9b — **the towns are in business.** Every town keeps books now:
a mining camp pulls ore and stone out of the ground and burns through timber, a
refinery eats that ore and turns out **copper bars** worth far more than what
went into them, and a depot chews through finished goods and ships them onward.
So somewhere always has too much of something and somewhere else always wants
it.

Prices move with that. Ore is cheap at a mine tripping over it and dear at a
refinery that needs it — and a counter now knows which town it stands in, so
walk in and dump forty loads on a small market and you will watch the price fall
out from under you. That is not a random walk; it is what happens when you flood
a place.

**The towns haul to each other on their own**, whether you are watching or not,
which slowly closes the very gap you were making money on. And you can put your
own drones on the same routes: walk up to the mast, pick a run, send a load out
of your pile, and get paid at the far end when it lands.

A load in the air is not simulated — it left here at this tick and lands there
at that one, so where it is right now is a sum. A hundred of them crossing the
map cost nothing, and when one passes near you it is drawn as a real machine.
Same story for the towns: one two kilometres away is not ticking away in the
background, it just knows when it was last looked at and works out the gap in
one go when you finally walk in the door.

Before that, stage 9a — engine work rather than new toys, and it started by finding
a real bug. `World::block` reports unloaded chunks as air, and drones read the
world through it, so a machine working near the edge of the streamed-in set saw
air where there was rock. That made its decisions depend on which chunks were
resident, and residency follows *your camera* — so the same dispatch could dig
a different hole depending on where you happened to stand. Operations now pin
the ground they will read, and a test digs the same body with four chunks
resident and with none and holds the results identical. It fails without the
pins.

That fix unlocked the round's real idea. Ask who edits this world: you break a
few hundred blocks by hand; one drone moves tens of thousands, and moves exactly
the same ones every time. Writing that to disk is writing down the output of a
deterministic function whose input was one line. So the game now keeps a
**command journal** — mine that area this way, run this many ticks — and
replaying it re-derives every block those orders produced. A fixture mine of
several hundred blocks is described by fewer than a dozen entries.

The prize is `--replay`, which rebuilds a saved world from its journal and
compares content hashes against the ground on disk. It needs no GPU and no
window, and it covers worldgen, the agents and the editing path at once.

Also in: a **crew** of drones instead of one — the job board was built for
contention and had never run any of it — plus the seed tree and a body id on
every chunk, so a planet is additive later rather than a rewrite.

And the geometry got twenty-one times smaller. A merged quad used to cost four
36-byte vertices and six indices; it is now **eight bytes**, with the six
vertices of its two triangles synthesised in the vertex shader. Positions are
chunk-local, which also pulls the `f32` precision wall out of the mesh data —
it was baked into every vertex, so walking far enough would have degraded the
geometry itself and not just the camera. Verified the only way this can be
verified: a real town capture through the new path is byte-identical to the
same capture taken before it.

Before that, stage 8: The sun moves. A day runs twenty real minutes and the light,
the sky and the fog all turn with it — golden hour, dusk, real dark, dawn —
with the hour on the HUD and saved beside the world, so you come back to the
time you left. The townsfolk keep those hours: at sundown they walk home,
stand down inside for the night, and are back out on the plaza in the morning.

The village became a **frontier**. Towns are built from shipping containers
and corrugated metal now, rusted and patched, with grated catwalks and a radio
mast in the middle of each one. And there is more than one of them: towns sit
on a 512-block lattice across the whole world, each with its own name, size and
trade — depot, mine or refinery. Your hometown is still pinned at the origin,
authored, and byte-identical in every seed you ever roll; everything past its
skirt is derived from the seed.

The mast does something. Press `E` at the console at its foot and the town
posts work: haul so much stone to Redfork, sweep a sector out at such-and-such.
Take a job and it pins the target on your map **even if you have never been
there and the ground around it is still black** — and freight is signed for at
the far end, so the walk is the job. Nothing about that is pre-generated:
worldgen is a pure function of the seed, so "which towns are within two
kilometres" is a few hundred hashes and loads no chunks. A beacon can name a
town that does not exist as a single block until you walk over and it builds
itself exactly where the posting said it would.

Before that, stage 7: Press `C` and the camera swings over your shoulder — there
is a body there now, hi-vis jacket and hard hat, and the camera slides in
close rather than through the rock when you back into a wall. Press `V` and
the handheld lists the fleet: pick a machine, look through its eyes while it
carries on working, or take the master override and drive it yourself, cutter
and all. Its mining job goes back on the board while you have the wheel and it
picks the work up again when you hand it back. The world streams around
whichever eyes you are using, so you can drive a drone clear across the map
and watch the whole trip. And the townsfolk can see you now — properly see
you, with the rock in between counting — so they turn to watch, stop their
strolling, greet only someone actually in view, and keep looking where you
*were* for a few seconds after you slip out of sight.

Before that, stage 6: Every world now starts in the **same village**: an authored
town at the origin, identical whatever the seed — plaza, paths, three houses
and a supply shop — blended into whatever terrain the seed grew around it.
Villagers stroll the plaza on deterministic rounds and greet you when you
walk up. Press E at the shop counter to trade for real: sell the ore your
flier ferried home for credits, buy drill-power and cargo upgrades that take
effect immediately and stack on top of your skill levels. Credits and
upgrades persist in their own small save file. The wilderness grew greenery
too — trees (felled whole when a drone cuts any part of one) and grass
tufts, the engine's first non-cube shape.

Before that, stage 5: The world has real relief, meshes across all cores, draws
only what the camera can see, and hides copper ore in the rock with occasional
outcrops breaking the surface. Mark a body and the game works out how to mine
it — a level adit into a hillside, a diagonal decline, or a benched open pit —
and a ground drone cuts the excavation, drives in, digs the ore out and hauls
it to the mine mouth. A flying drone sweeps sectors and drops pings on ore
buried underground, and once you place a container it ferries every pile home
automatically. A fog-of-war minimap paints in as you walk — and as the flier
sweeps — with live dots for you, the drones, the pings and the base.

The machines now look like machines: the ground drone is a squat rust-orange
digger rig with a spinning nose drill and chunky treads, the flier carries a
rotor, and both glide between ticks and turn to face their travel. You hold a
compact boring drill instead of breaking blocks by clicking — hold the button
and it chews through by hardness, faster as your Mining level climbs. Skills
(Mining, Prospecting, Logistics) level RuneScape-style: prospecting deepens
the flier's scanner, logistics grows every cargo hold, and it is all drawn on
a bitmap-font HUD with XP bars and a level-up shout. You can walk on it,
build in it, and it survives quitting — skills included.

| Crate | What it does | State |
|---|---|---|
| `vx-core` | Block registry, coordinate spaces, event bus | Done |
| `vx-world` | Chunk storage, worldgen, ore, town lattice, biomes and flora, raycast, line of sight, physics, editing, content hashes, saves | Done |
| `vx-mesh` | Greedy meshing + crossed-quad plants, packed into 8-byte quads | Done |
| `vx-render` | wgpu renderer, camera, frustum culling, instanced objects, 2D overlays, bitmap font, offscreen capture | Done |
| `vx-platform` | Input state, XDG paths | Done |
| `vx-app` | Window, walk/fly/third-person camera, tick-based player movement, streaming, day/night clock, HUD, rigs, skills, villagers, awareness, shop, wallet, garage, handheld, beacon board, town economy, maps, command journal, `gamingg` binary | Done |
| `vx-agent` | Job board, flow fields, mine planning, spoil-heap shapes, scanner, flier + fleet, manual piloting you can crash | Done |
| `vx-mod-api` / `vx-mod` | Mod ABI, manifests, WASM host | later |
| `vx-steam` | Steam Workshop mod source | M4 |

### Known rough edges

- **A town grows in buildings and capacity, not in people.** `people::PEOPLE`
  is a constant that the ballot, the friendship ledger and the villagers all
  count on, so a grown town has new sheds and a better output rate and exactly
  the same three residents standing in it. Making population per-town is its
  own round, and it is the obvious next one.
- **A town never shrinks.** Growth is measured against lifetime earnings and is
  deliberately a ratchet, so a place that boomed and then went broke keeps
  every shed it put up. That is the right call while nothing can un-build a
  building, and it means a long enough game ends with every town it can see at
  the top of its table.
- **Three growth buildings, and then a town is finished.** `growth_plan` is
  three authored blueprints per speciality dropped into three fixed pockets of
  the plaza, so two grown depots look alike and the third threshold is the end
  of what a town can become.
- **A shed goes up while you are not looking, never while you are.** The stamp
  needs the chunks resident, so a town with its ground unloaded waits — which
  is right — but it also means you will essentially never *watch* a building
  appear. You walk in and it is there. The `TOWN` verb says `ONE PENDING` so at
  least the gap is legible.
- A **flier crash is not on the oracle**, because a flier is not on the wire.
  Machines are bought live and `Command::SpawnMachine` is an admin cheat whose
  replay arm is a no-op, so a replayed session has diggers (the dispatch order
  carries the crew count) and no fliers at all. Everything else about losing a
  machine re-derives correctly — a drone shot, flattened or ground down by
  neglect replays exactly, and there is a test that says so — but a machine
  you fly into a hillside is a live-only loss. Putting the fleet on the wire
  is a journal bump and its own round.
- Only the flier can be crashed. A ground drone keeps the standability rule
  that stops a hand-driven machine stranding itself, which also means it
  cannot be driven off a cliff, and the kestrel is absent from both the wear
  and the integrity ledgers by the same argument that has kept it out of the
  fuel loop since stage 14.
- A wreck is stripped, never rebuilt. Walking out to a hulk gets you its cargo
  and six spare parts; there is no way to pay to put the machine back
  together where it lies.
- Machines do not collide with each other, only with the ground. Two fliers
  can occupy the same cell all day.
- A spoil heap the crew cannot stand beside does not get finished. Stacking
  reuses the cut's neighbourhood, and `PLACE_OFFSETS` has nothing below it on
  purpose — a drone cannot fill the cell under its own feet without lifting
  itself — so once a shaft's walls are up, the cells inside the top course
  have no station left to work from and the heap comes out hollow. It is
  visible in the `--heap` capture, and it is a truthful picture of the rule
  rather than a bug in it: what is missing is a machine that can reach *down*.
- The crew has one spoil heap at a time, and it is part of the dig rather than
  a standing order of its own: `Mining::operation` is a single `Option`, so
  ordering a second heap posts more courses onto the same crew. Cancel the
  dispatch and the heap goes with it.
- Footings are poured flat under a town's levelled plateau; a building on a
  slope would want stepped footings, and no building stands on a slope yet.
- A town's vault charges no fee and pays no interest, so banking is pure
  convenience rather than a decision with a price on it.
- A save costs about six milliseconds, which is most of a frame, so an
  autosave is a small visible hitch. Moving it off the main thread is its own
  round; the number is measured in `--keeping` rather than guessed at.
- The kept fallback holds region files by hard link, so a region damaged *in
  place* by the disk itself is damaged in the backup too. Guarding that means
  copying the whole world every save, which is a stall rather than a backup.
- SIGTERM and SIGINT are unhandled: a `kill` or a Ctrl-C in a terminal loses
  whatever the autosave has not written yet. Closing the lid, closing the
  window and the window being taken away are all covered.
- The sonar cannot draw a marker through rock. The object pass is opaque
  geometry with a depth test, and a second pass just for hologram markers is
  more renderer than the feature is worth, so ore with no exposed face is
  counted on the scope rather than lit in the world.
- The drill's cage is drawn on whatever is aimed at within reach, whether or
  not the trigger is down, and there is still no crosshair for the moments
  when nothing is in reach at all.
- Forts are raised whole or dropped in segments; nothing between, and no
  rubble where a wall fell. Nobody defends them either — that waits for
  hostiles.
- Fuel is one number for the whole fleet rather than a tank per machine, so a
  drone cannot be stranded far from base with the rest still working. Machines
  also refuel from the pile at any distance: there is no tanker run yet.
- Nothing burns fuel except the mining fleet — not the kestrel, which runs on
  its own cell and cooldown, and not the player.
- A bunker's room pool is small, and it shows: after enough of them the
  furniture repeats even though the arrangements never do. That is content to
  author, not code to write.
- The coil's spine is a run of straight corridors, not a true golden spiral;
  ruin is not modelled at all, so every bunker is intact; and a room on the
  bottom floor is the same room as one at the top — depth means nothing yet.
- Underground light is column depth, not light transport: a lit shaft does not
  spill light into the gallery beside it, and the lamp casts no shadows. Cheap,
  pure, and wrong in ways that mostly read as atmosphere.
- The visors are full-screen transforms with no battery, no grain and no wear;
  thermal paints every machine warm, including your own cold wreckage.
- A cave mouth on flat ground is a pothole you notice at your feet, not a
  landmark; the mouths that read from a distance are the ones in hillsides.
- Bounty never decays. The town's memory is perfect and permanent, which will
  want a statute of limitations once the warrant chain exists.
- The player cannot hold an office in ordinary play, so the sheriff's override
  is exercised only by the `--sheriff` development flag until the ballot box
  lands. It is built and tested; it is simply not reachable yet.
- Villager sight is a cone about their heading with a close-range exception, and
  the cone width is a guess that has had no real playtesting.
- A drone caught digging somebody's wall on the far side of town is not traced
  back to you: the witness check is on the player's position. Convenient, and
  arguably correct, but it is a hole a patient player could drive through.
- The mantle arc is an interpolation, not an animation: the body slides up a
  ledge rather than climbing it, the same way villagers glide rather than walk.
  Stance poses are the standing rig scaled vertically — the parts are boxes and
  there are no joints to bend.
- Swimming slows you and stops you sprinting, and that is all it does. There is
  nothing to do underwater and no aquatic machine to do it with.
- Slide friction is one constant for every surface, so gravel, metal grating and
  a wet town street all feel the same. Per-block friction is a small registry
  change and a large tuning job.
- No view bob, camera roll on strafe, sprint FOV shift or landing dip. The
  cheapest way to make movement feel good and the fastest way to make people
  motion sick, so they want individual toggles rather than arriving on by
  default.
- Toggle-versus-hold for sprint, crouch and prone is not configurable, and input
  is not remappable.
- The third-person camera does not swing clear of the body during a slide, so a
  slide into a wall can fill the frame with your own back. An existing rough
  edge, now reachable at higher speed.
- A 2.2 m mantle lets the player leave a hole a ground drone cannot drive out
  of. The planner does not warn about it yet.
- The played session walks by holding forward, working along a face when it is
  stuck, and — since stage 50 — sweeping the ground it can see when that stops
  working. It is still not a route planner: it can only see as far as the
  chunks that are loaded, so a detour longer than about thirty-five blocks is
  one it has to find in pieces, and a route with a doorway in it still needs
  the doorway named as a waypoint, which is why the house's, the shop's and
  each town's gateway are.
- Placing your first base container is the moment your fleet starts wanting
  fuel: `Mining::fuelled` reports a machine as fuelled only while there is *no*
  base to burn from, so declaring one quietly grounds the flier unless there is
  HHO on the pile. Nothing says so. It is the same shape of trap as the ore
  that used to vanish when you mined with no container, and it wants the same
  answer — a line on the screen — rather than a rule change.
- The pile itself persists now (`pile.dat`), but its *deposits* are not on the
  journal any more than hand-mining's are, so a replayed session's pile
  diverges from a live one's for the same reason the wallet does. Two holes of
  one family; they want one round between them.
- A town's ditch is crossable only at its gateways. Anywhere else it is three
  blocks deep with a two-block mantle available, which is the point of a ditch
  — but it also means a player who drops into one has to walk it round to a
  gate, and nothing on the screen says that is what has happened.
- Drops do not fall while you watch. A block cut out of a ceiling settles onto
  the floor under it the moment it is shed, which is the part that matters, but
  it arrives there rather than tumbling — and a block cut out over a shaft it
  cannot find a floor in within twelve blocks simply stays where it was cut.
- The pack panel lists at most twelve kinds and says how many more there are.
  A pack with more kinds than that in it is a pack you should have tipped, but
  the panel still cannot show you all of it.
- Selling still sells out of the *base pile*, not the pack, so a haul is two
  actions: tip it into a container, then trade. That is the right shape for a
  container you own and the wrong one for a counter in another town, where you
  have to be carrying it and there is nowhere to put it down.
- Saves store a full chunk snapshot per modified chunk — about 24 KiB whether
  one block changed or ten thousand. An edit journal would cost bytes instead.
  Harmless until worlds get large.
- Water is alpha-blended without depth sorting. Fine while water is the only
  translucent block; looks wrong the moment two transparent surfaces overlap.
- Saving happens on quit and on demand, not periodically. A crash loses the
  session.
- Selling "everything" at a counter sells your **fuel** too: HHO is a traded
  good and the shop is glad to take it, and it is the same pile the fleet burns
  out of. Sell up without thinking and your crew stops, with a full job board
  and an empty tank. The played loop hit this on its first run.
- A crew is fielded when a dispatch *starts*, so a drone bought while a job is
  running joins the next one rather than that one. Nothing says so on screen.
- A town's till refills on the same closed-form clock the stock does, so a
  counter you drained recovers whether or not you are anywhere near it. That is
  deliberate — a town trades when you are not watching — but it does mean
  waiting is a strategy, and nothing tells you how long.
- A save remembers where you were standing and which way you were facing, and
  deliberately not your stance or your speed — you always arrive upright and
  still. So a save taken mid-slide reloads as a stop, and one taken in a place
  you could only have reached by falling reloads with the fall already over.
- Chunk culling uses each chunk's full 256-block height. A tighter bound around
  the blocks actually present would cull more.
- Ore uses one tile for every block, so a large exposed body shows a visibly
  repeating pattern. Real art or per-block texture variation fixes it.
- Rig parts are lit like everything else but cast no shadows and have no
  animation beyond spin and yaw — no suspension, no articulation. Villagers
  and the third-person player glide rather than walk: no leg animation yet.
- The third-person body is a static pose that yaws with the camera, and the
  camera does not swing round to avoid it — you can stand nose-first against a
  wall and watch your own back fill the frame.
- One machine can be piloted at a time, by design. No squad orders, no
  waypoint autopilot, no saved view or pilot state: quitting returns you to
  first person with nothing driven.
- Villager awareness is sight only — no hearing, and nothing reacts to a
  machine cutting rock next to it beyond turning to look.
- Villagers are not persisted — their deterministic stroll restarts each
  run. Cosmetic only, since nothing about them accumulates yet.
- A town ignores its surroundings beyond height blending and the sea/slope
  siting gates: a plateau can still abut open water, and no road leads from one
  town to the next.
- Only the town you are standing in has people in it. Distant towns are
  architecture until you arrive — deliberate, to keep the frame budget flat
  however many exist, but it means nothing happens in a town you are not in.
- **The town books are not covered by `--replay`.** The network's reach follows
  the player, and where the player stood is not in the journal, so a replay
  rebuilds the ground but not the markets. Journalling the network's windows
  would close it and is not done.
- The economy only runs for towns within radio range of the player. That is the
  level-of-detail — the simulation is as wide as it needs to be — but it does
  mean a far corner of the frontier is frozen until somebody goes near it.
- Routing is greedy nearest-deficit matching rather than a real min-cost flow.
  Fine at a few dozen towns; revisit if the traffic disappoints.
- One load in the air at a time per dispatch window, and no contract can fail,
  expire, or be robbed.
- Villagers do not react to their town's fortunes — a boom town and a starving
  one look identical on the ground.
- Nothing can be intercepted and nothing shoots: caravans pass overhead
  untouchable. Weapons, interception and bounty are stage 10b, designed in
  ROADMAP.
- The starter flier is free and unlosable, so you can always scan and ferry even
  with an empty garage and no credits.
- A damaged `garage.dat` costs you your whole bought fleet. It is the harshest
  of the tolerant loaders, and the alternative — refusing to open the world — is
  worse.
- The console's trade map picks its zoom to fit the route, so a very long run
  draws a very coarse map.
- There is no artificial light. Night is genuinely dark outside, and a lamp
  block is the obvious next thing the sun uniform wants.
- Leaves do not decay when a trunk is felled by the player's drill (drones
  fell whole trees; the handheld drill still breaks one block at a time).
  Felled wood has nowhere to go but the pile until stage 9's crafting.
- The viewmodel drill is drawn in world space at a camera offset, so it can
  clip into a wall you stand against. A real fix renders it in a separate
  near-scaled pass; cosmetic until then.
- Skill effects apply on the next frame's `apply_skills`, so a capacity raise
  reaches existing drones mid-run. Deliberate — retroactive upgrades feel
  good — but it means capacity is not per-drone state anywhere.
- A drone's grade shapes the ramps it cuts but is not yet a limit on what it can
  drive: the flow field allows one block of climb per step, so a 1:1 staircase is
  traversable by anything. Making grade a real constraint needs a field that
  tracks how far a drone has run since it last climbed.
- The command journal records world edits and dispatches, not the whole session:
  the fleet, surveys, the base and piloting are not in it yet, so `--replay`
  rebuilds the *ground* rather than the entire game state.
- Region files are still written on every save, so the journal does not yet make
  saves smaller — it earns its keep as a determinism oracle first. Letting it
  replace region writes is where the disk win lives, and the keyframe machinery
  is in place for it.
- Replay is deterministic on the same binary and platform. Agent movement uses
  `f32`, so **cross-platform replay is not promised** and should not be claimed.
- Chunk pinning holds an operation's whole working span resident for the life of
  the job. Correct, but a very large marked area pins a lot of chunks.
- A running excavation is not saved, and neither is the fleet. The holes are —
  block edits go through the same path a player's do — but quitting mid-dig
  loses the drone, the job board, the surveys and the base declaration. The
  journal is the obvious way to fix this and does not do it yet.
- Each chunk carries its own small origin uniform and bind group. Fine at a few
  hundred chunks; a single buffer with dynamic offsets is the tidier answer if
  view distances grow.
- The minimap draws explored-but-unloaded ground from the generator, so your
  edits and mine holes only show on it while their chunks are loaded. The
  trade is that the map stores nothing but the explored set.
- One flier. The fleet is shaped for more; nothing exercises it.
- No combat, health or hostiles. Piloting and NPC senses are the
  foundation they will stand on; neither is exercised by anything yet.
- No inventory.

## Design notes

**The simulation/presentation split is the networking seam.** `vx-world` owns
all authoritative state and never references rendering or windowing. Multiplayer
is not implemented, but adding it later means transporting the existing command
interface over a socket rather than restructuring world logic.

**Chunk storage is palette-compressed.** A chunk is 65 536 blocks; storing a
`BlockId` each would cost 128 KiB. Instead blocks index a per-chunk palette,
packed at the narrowest bit width that fits it. An untouched air chunk collapses
to zero bits and an empty buffer.

**Block ids are not stable across runs.** They are assigned in registration
order, so the mod set changes them. Anything persisted to disk must key on the
namespaced string name (`engine:stone`), never the numeric id.

**All `BlockView` coordinates are absolute world coordinates**, never
chunk-local. Mixing the two makes geometry silently vanish.

**Terrain height comes from three fields through splines, not summed octaves.**
Summing octaves drives the result toward its mean, so everything lands mid-range
and the world reads as flat terraces — the old version had ~22 blocks of relief.
Continentalness, erosion and peaks/valleys are sampled independently and each
mapped through a piecewise-linear curve, which can spend a wide output range on
a narrow input band. A cliff is a steep segment; a plain is a flat one, flat
because it was authored that way. Domain warping bends the sample space so
nothing lines up with the lattice. Relief is now ~100 blocks.

**Outcrops are a consequence, not a feature.** Ore deposits are irregular blobs
buried at depth. Most never reach the top of the rock. A few sit high enough that
the blob pushes up through the soil, and that is an outcrop — nothing generates
them directly. Deposit centres come from a jittered lattice, so they are a pure
function of the seed and the ones near a chunk can be gathered without a global
list. Ore only ever replaces stone or overburden, which is what stops it hanging
in mid-air.

**For a machine that drives, the mining method is the access problem.** A ground
drone can change height by one block per step, so how an excavation is shaped
decides whether the ore can be reached and hauled out at all — the same question
real operations answer when they choose between an adit, a decline and a pit. An
adit is a level tunnel into a hillside and only works where the body meets a
slope, which is exactly what the outcrop rule produces; a decline is the
general-purpose ramp; a pit's benches are its own haul road but its volume grows
with the cube of depth. A vertical shaft is deliberately absent: nothing that
drives can climb out of one, so it belongs to the flying drone as a later unlock.

**Methods are ranked on cost, not on dug blocks.** Ranking on volume alone would
never choose an adit — a decline can always start part-way down a slope and cut a
shorter tunnel. But the excavation is dug once and the haul is made on every load
forever, so a plan is charged for the height its loaded drone has to climb. On
the reference body that puts the adit ahead at 2160 against the decline's 2355,
even though the decline moves 66 fewer blocks of rock. The player can override.

**A body is mined in benches, not as a box.** Clearing a box layer by layer
leaves vertical walls, and a drone that has worked to the floor of one cannot
climb four blocks back out — it ends up standing on its own ore with a full load
and nowhere to take it. Each layer of the stope reaches one block further toward
the access than the layer beneath, which leaves a staircase down one wall. The
bottom bench is the body's true footprint, so all the ore still comes out; the
extra is waste cut to make the ramp.

**A drone cuts level with itself and above, never diagonally below.** Cutting
down-and-across looks like the obvious way to descend and is what strands it: it
carves a full-height notch and destroys the floor of every cell in it, including
the ones needed to advance. Descending is by undermining — cutting the block
directly underfoot, allowed only when there is solid ground under *that*, so the
drop is one block and the step back up exists. Every hole it makes for itself, it
can climb out of.

**Tunnels are two blocks tall for the same reason.** Two is the tallest a drone
can cut standing on its own floor. A three-block corridor needs somebody a block
off the ground to cut the roof, and on a level tunnel there is nowhere to stand.

**The scanner samples real blocks, not the deposit function.** For each surface
column it walks up to 24 blocks down (deeper as Prospecting levels), then clusters adjacent
hits into one ping per body. Reading the pure deposit lattice would have been
cheaper and always right about generated terrain — and wrong about everything
else: a mined-out body must honestly stop pinging, and player-placed ore must
ping. Depth-0 pings are outcrops anyone can eyeball; the depth-1-to-24 band is
what the scanner is for, and in practice that band matters most on gentle
terrain — in steep country shallow bodies usually breach a hillside somewhere,
so your eyes really are nearly as good as the instrument there.

**The flier's movement is trivial by design.** It flies at a fixed clearance
over whichever column it is over, one block per tick horizontally, climbing
first before entering a column whose ground is too close — so it is never
inside terrain, and a cliff costs climb time instead of a collision. No flow
fields, no reachability: not needing them is the fantasy of the flier, and it
is also what makes it nearly free to simulate.

**The overlay draws only when set.** The minimap goes through a generic 2D
pass — any RGBA image at any screen rectangle, no depth test — and headless
captures and the culling tests never set one, so every pixel-equality
guarantee stands byte-for-byte. The same pass is deliberately the first brick
of the terminal screen and anything else that is a flat picture.

**One owner for where the camera sits.** `Camera` has no look target — the
view is built from position plus yaw/pitch alone — so third person needed no
renderer change at all, only a different position chosen after movement. The
controllers own orientation and the body; `view::camera_placement` owns
placement, which is why first person, over-the-shoulder and a drone's feed
differ in exactly one place instead of being threaded through every
controller. The pull-in casts back along `-forward` and clamps to a minimum:
a ray starting inside a solid reports distance zero, and without the floor the
camera would collapse onto the player's own head.

**A piloted machine can do nothing an autonomous one could not.** The override
is a change of driver, not of physics. Held keys become a `PilotCommand`
sampled once per frame and re-issued on every simulation tick, so a driven
drone moves one cell per tick obeying the same standability and one-block-step
rule the flow field enforces, falls when it cuts its own floor, cannot reach
through rock, and cannot undermine anywhere the planner would refuse to. A
piloted flier still climbs before it enters a column. Break that and "every
route in is a route out" — the invariant every mine plan rests on — stops
being true the moment a player touches a machine.

**Jobs are shared; surveys are not.** Taking a drone *releases* its claimed job
back to the board, so another drone can pick the work up while you joyride, and
hand-back needs no bookkeeping at all — the drone goes idle and claims lazily
on its next tick. A flier's survey lives only in that flier, with no board to
hand it back to, so taking one **stashes** its state and restores it on
hand-back instead. Releasing it the way a job is released would silently throw
the whole sweep away.

**The eye and the body come from one interpolation.** Machines move a whole
cell per tick and are drawn gliding between ticks; the FPV camera reads the
*same* interpolated point the rig is drawn at, lifted to eye height, so the
view can never drift or stutter against the hull it is bolted to.

**Piloting far away unloads your own ground.** Streaming follows the camera,
which on a feed is the machine — that is what makes driving across the map
work, and it needed no new code. The consequence needed care: `unload_beyond`
drops the player's own chunks while they are away, so hanging up does *not*
resume physics until `world.is_loaded` says their ground is back. Without it
the first step after a long-range flight drops the body through a world that
has not arrived yet. The HUD says `RECONNECTING` while it waits.

**Exploration is earned by being somewhere.** Driving a drone around does not
paint the fog-of-war map. A remote camera is not the player, and letting a
driven digger reveal terrain would quietly take the scouting job away from the
flier, whose swept sectors still count.

**NPC memory is the feature, not an optimisation.** An observer that forgets
the instant line of sight breaks snaps its head away the moment you step behind
a tree. One that keeps watching where you *were* for a few seconds reads as
somebody who saw something — and it is the shape a hostile will need to hunt
rather than twitch. The cost control falls out of the same mechanism: one
villager re-casts per update on a round robin keyed to an update counter (not
wall time, so the bit-identical determinism survives), and between turns they
face the remembered spot. Three villagers today and thirty later cost the same.

**The hometown is a drawing; the frontier is a dice roll.** `town::home_site()`
returns before any hashing happens, so the starting town is seed-*independent*
by construction rather than merely seed-stable — same plot, same name, same
plan, every world. Every other town comes off a jittered 512-block lattice with
one splitmix64 stream per property, the same idiom ore deposits and trees
already use. Buildings are origin-relative ASCII layer blueprints, so one
static set of plans stamps at any site; they are never saved, because they
regenerate.

**The height field must not feed itself.** `height_at` used to *be* the village
override — its last line blended the plateau in. With one town that is fine.
With a lattice of them, siting town N+1 would measure ground that town N had
already flattened, and the world would depend on the order towns were
considered in. So the field is split: `natural_height_at` is the seed's own
terrain and the only height a siting test may consult, and `height_at` gathers
the towns that could reach a column and blends their plateaus on top. Sites are
gathered **once per chunk** and threaded through column filling, flora and
stamping; a test asserts siting never reads the blended field.

**Town separation falls out of the constants, not out of a rejection pass.**
Jitter is clamped to the middle half of each 512-block cell, so two neighbouring
towns are at least 256 blocks apart, against a maximum reach of 58. No
cross-cell comparison is needed, which is what lets a site be answered from one
cell's hash alone — and that in turn is what makes enumerating three kilometres
of frontier arithmetic instead of a search.

**Nothing about the frontier is pre-generated.** Because worldgen is pure in the
seed, "where are the towns within two kilometres?" is a few hundred hashes and
loads no chunks, no disk, no memory. A beacon can post a job at a town that does
not exist as a single block; walk over and it builds itself exactly where the
posting said. The only thing *stored* is which towns the player has actually
stood in — player knowledge, in its own tolerant file, not world truth.

**A map marker needs no visibility check, and that is the hook.** Markers are
stamped in a pass after the terrain, with no test against the explored set, so
a contract pin draws over blacked-out ground for free. Accepting a delivery
pins a town you have never seen and the black around it stays black until you
walk there.

**The sun is pushed, never read.** Time of day is a uniform at
`@group(2) @binding(0)` holding direction, sky and light, written through
`Renderer::set_sun` beside `update_camera`. No render path reads a clock, which
is what keeps headless captures byte-identical and the culling pixel tests
honest — and it is why `--time`, `--dawn` and `--night` can light a capture
exactly. The fog colour *is* the sky colour, one value, so the horizon cannot
disagree with the clear colour the way two matching constants eventually would.

**The clock is an input to the town, not a global.** `Villagers::update` takes
the hour as an argument, so the bit-identical determinism test still passes a
fixed noon while sundown genuinely sends everyone indoors.

**Vegetation is not ground, and machines know it.** Trees are solid blocks,
so the planners had to learn that a canopy is not a hilltop: `ground_height`
walks down through vegetation, or a pit surveys its rim six blocks up in the
branches. And a crown hanging into a bench from a tree rooted elsewhere has
no cell to stand and prune it from — so cutting *any* part of a tree fells
the connected whole into the cargo bed (through the same cancellable event
path as every edit), and standing vegetation never counts as outstanding
work. Take the trunk, take the tree.

**The cross shape is the engine's first non-cube.** A grass tuft is two
diagonal quads, each emitted twice with reversed winding because the terrain
pipeline culls back faces — a plant must carry its own back or vanish from
half the compass. Cross blocks never merge into the greedy sweep and never
cull a neighbour's face; they are non-solid, so you walk through them and
the raycast ignores them.

**Selling drains the fleet's base pile.** The shop buys whatever the flier
ferried home, which closes the loop the whole game is about: scan → mine →
ferry → sell → upgrade → dig deeper. There is deliberately no separate
player inventory yet (stage 7); the pile *is* the wealth. Prices and
upgrade lines are name-keyed tables, so the shelf grows by adding rows.

**Bought upgrades multiply on top of skill effects.** `wallet.dat` is its
own small file beside `player.dat` (one concern per file, no migrations,
same tolerant loader — a corrupt wallet is an empty wallet, never a failed
world). A fresh wallet is the exact identity, and upgrades reach machines
already in the field on the next frame. Retroactive upgrades feel good;
that is deliberate. The shop counter itself has no hardness: you cannot
drill the economy out of the town.

**Villagers are deterministic clockwork.** Waypoints and pauses come from
hashing the villager's index and stroll leg — no RNG state, nothing saved,
two runs fed the same frame times are bit-identical. Greetings use
hysteresis (speak at 3 m, re-arm at 5 m), so walking up gets one line, not
a line per frame.

**The font is data, the HUD is pixels.** Text is a hand-set 5×7 bitmap —
A–Z, digits, punctuation, seven bytes a glyph — stamped into any RGBA buffer.
The HUD composites a small panel on the CPU exactly the way the minimap does
and ships it through a second overlay slot; the renderer never learns what a
glyph is. The same font is the terminal screen's and the shop's, later.

**A rig is data too: a handful of cuboids.** Machines are parts — centre,
size, tile, maybe a spin axis — turned into instanced objects each frame, so
they ride the existing one-draw-call path and per-object frustum culling with
zero new render code. The digger reads Motherload-ish on purpose: squat
rust-orange hull, pale cab, chunky treads, tapered steel drill that spins
while she cuts. The *vibe* is borrowed; every pixel is procedural and ours.
Rigs face +X and yaw to their travel; positions interpolate between
simulation ticks so machines glide instead of teleporting block to block.
The inverse-transpose normal fix from the review round is what keeps these
rotated, stretched parts lit correctly — this is the consumer it was fixed
for.

**The player's drill is the pickaxe role without the pickaxe.** Hold to dig:
progress accrues at `drill_power / hardness` per second against the block
under the crosshair, resets when you look away, and the break at 100% goes
through the same cancellable event path as a click ever did — a mod's veto
still holds. It is an original *compact boring drill*: the tunnel-company
name on the real one is a live trademark, so ours has none. Bedrock has no
hardness and stays undrillable.

**Skills are name-keyed entries, not an enum.** `"mining"`, `"prospecting"`,
`"logistics"` today; the Skyrim/Fallout-breadth combat and faction skills
planned for the village era are *rows in the same table*, not code changes —
the same reasoning as namespaced block names. The XP curve is the classic
each-level-costs-~10%-more shape with our own constants, levels 1–99,
precomputed once. Effects are small pure functions: Mining speeds the hand
drill, Prospecting deepens the flier's scan, Logistics grows every cargo
hold. `player.dat` persists XP by name and a corrupt file logs and starts
fresh — progress must never take the world down with it.

**The map's only state is the explored set.** Worldgen is a pure function of
`(seed, position)`, so explored-but-unloaded terrain is recomputed from the
height field on demand rather than cached as thumbnails. `explored.dat` is a
list of chunk coordinates and nothing else, and a corrupt one logs and starts
empty — player knowledge must never take the world down with it.

**Every visible outcrop has ore continuing beneath it.** A surface block only
shows ore when the body also fills the blocks below it. Without that rule a body
could graze the surface and leave a single speck leading nowhere, players would
learn that outcrops mean nothing, and the prospecting loop would die with them.
A test pins it.

**Worldgen is a pure function of `(seed, position)`.** It uses an integer hash
rather than a seeded RNG, so there is no sequence state to desynchronise:
chunks can generate in parallel in any order, and a saved world regenerates
identically. This is also why saves only store chunks a player has *changed* —
everything else comes back from the seed for free.

**Saves key blocks by namespaced name, never by numeric id.** Ids are assigned
in registration order, so installing one mod shifts all of them; a save keyed on
numbers would silently reload as the wrong blocks entirely.

**Collision resolves one axis at a time.** Resolving all three together and
pushing out along the shallowest overlap is the usual shortcut, and it is what
makes sliding along a wall snag on every block seam.

**Block edits are announced on the event bus before they happen**, through
`emit_cancellable`. Nothing listens yet — the point is that M3 mods will be able
to veto or alter an edit without any call site changing.

**Meshing runs on a worker pool, reading the world immutably.** `World` is plain
data with no interior mutability, so parallel reads need no locking — the shared
borrow the mesher was already written against was all that was required. A test
asserts the parallel result is byte-identical to the serial one.

**Objects are instanced from the start and share the terrain's fragment shader.**
The design calls for swarms, so there is one cube mesh, one instance buffer and
one draw call however many drones exist; per-object draws would be rewritten at
the first swarm. They share the terrain pass's depth buffer so drones and hills
occlude each other, and the *same* fragment entry point, so a drone can never
drift out of step with the light on the ground it is standing on.

**Frustum culling is derived from the same view-projection matrix the GPU uses**,
so it can never disagree with what actually gets clipped. A chunk's bounding box
comes from its `ChunkPos`, which relies on the mesher never emitting geometry
outside the chunk it was asked for. The test that matters renders a frame with
culling on and off and asserts the pixels are identical: skipping something
visible would show up immediately.

## Building and running

Requires Rust 1.94+.

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run --release -p vx-app          # opens a window
```

Every building in a town carries a lockbox saying who may edit it, ranked
sheriff over mayor over owner over guest. Your own house is yours; the streets
are the town's; past the town line you build where you like. Getting into
somebody else's means earning it, picking the lock, or drilling the box out —
and any of it only costs you a bounty if somebody actually sees you, which is
what crouching and going prone are for.

New players wake up inside their own house in the starting village, with a
storage chest, a mailbox for mail orders, and a one-time welcome panel whose
changelog is parsed straight out of `ROADMAP.md` (`--changelog` prints it).
The spawn area is pregenerated before the first frame and the hometown is held
resident permanently; `--view-distance <n>` (4–16) picks the streaming radius.

### The towns get bigger

Every town on the frontier keeps books, and until now those books were a
readout. The towns hauled to each other and **nobody paid for anything**: a
town's money went up on a timer against how full its shelves were, so a place
on the busiest corner of the map and a place nobody had ever shipped to ended
up with exactly the same till. And the whole network moved one wagon every four
in-game minutes, which is a map with freight drawn on it rather than a network.

Now the buyer pays, out of its own till, at its own price, the moment the load
leaves — and **a town that cannot cover a load does not order one**. That single
rule is what makes the money mean anything: a poor town stays short, its prices
stay high, and the first thing it can afford is the thing it needs most. Up to
four runs a window, never twice out of the same town, so one glutted mine cannot
take every wagon while the rest of the country sits still.

And a town that trades well **builds**. What a place has earned selling on, over
its whole life, is tracked separately from what is in the till — a till is spent,
and a town doing well is one money moves *through* — and each time that number
crosses a threshold the town puts up another building: a second warehouse, a
holding tank, a bunkhouse, a loading yard, chosen by what the place does for a
living. The new sheds are new capacity, so growing makes it easier to grow. Walk
back into a town you sold to a week ago and the square is fuller than you left it.

It is a ratchet. A town that has a good decade and then a bad year keeps what it
built, because the measure is what it earned and not what it currently has.

The buildings are real blocks, written into the world the same way anything you
build yourself is — terrain generation is untouched, so nothing about the ground
or the world hash moves. Which means they are **deferred**: a town three
kilometres away grows on its books wherever it is, and the sheds go up the next
time somebody is near enough for there to be somewhere to put them. Nothing is
lost by the wait; the `TOWN` verb tells you when a building has been earned and
is still pending.

Two things had to be fixed to make any of it checkable. The freight network had
exactly one caller — the windowed game — so a headless session had never run it
and no test had ever watched it end to end. And selling had never been written
down at all, which was harmless for eight stages for exactly one reason: a
town's books could not move a block. They can now, so the counter is on the
journal, and a session that trades its way into a new building replays to the
same ground *and* the same books.

### A build you can just run

`dist/` holds a stripped release binary for x86-64 Linux, for testing on a Steam
Deck without a toolchain. One file, no assets — the shaders are
compiled in. It needs **glibc 2.39 or newer** (SteamOS 3.7+ is fine, 3.6 is not);
`dist/README.md` has the check and the rest of the Deck notes.

```sh
./dist/gamingg-linux-x86_64
```

### The handheld is a thing you hold

The fleet uplink used to be a rectangle that appeared in front of your face.
It is now an object: a cased unit with a bezel, a stub aerial and a strap over
the forearm, and pressing `V` swings it up into view over about a third of a
second, with the screen fading on as it arrives. Press it again and the unit
drops back out of frame — it leaves rather than vanishing.

The readout is the same readout, drawn on the unit's own glass. Every frame
the four corners of the screen face are put through the very camera matrix the
world was drawn with, and the panel lands in the rectangle that comes back, so
looking around while it is up moves the screen with the thing carrying it. The
rectangle is *fitted* to the glass rather than stretched onto it: the case is
tipped toward you, so its projection is a trapezoid, and stretching text into
that would squash it by however far the unit happens to be tilted.

And while it is up, your drill and your launcher are away. You have two hands
and both of them are holding the thing — which is what turns checking on your
drones into a decision rather than a free pause.

### The water moves now

Drive a gallery into the side of a lake and the lake comes in after you. It
runs down the tunnel, finds the low ground, and settles out flat — and it keeps
coming for as long as the hole is open, because the sea is bigger than you are.

Inland it is a different story: a pond, a cave pool, anything above the tide
line holds a fixed amount of water and every drop of it is counted. Cut a
channel out of one and it goes down by exactly what ran out. That is how you
get at the floor of a flooded cave — drain it somewhere lower and walk in.

The clever part costs nothing. Every block in this game is already cut into
sixty-four little cells so that a damaged block can say which bits of it are
missing. A wet block uses the same sixty-four to say how full it is. Sixty-four
steps of fill, which looks smooth rather than steppy, on machinery that was
already there.

And you can print a **pump** at the fabricator. Stand it in the water, press
`E`, and it lifts what it can reach out of a spout on its top — which is the
one thing gravity will not do for you. Fill a cistern up a rise, or get back a
gallery you flooded on yourself.

### You can put a tree on the ground

Get your drill on a trunk — **low, near the roots** — and hold it. You are not
chipping a block any more, you are cutting a **notch**, and the notch is cut
into the trunk's own cross-section rather than into the face of one block.

Cut about a third of the way through and the tree is *aimed*; keep going until
the holding wood on the far side is down to the corners and it lets go. That
is the real thing: a feller's face notch is 15–33% of the trunk's diameter and
the hinge behind it about a tenth, the notch aims the tree and the hinge steers
it down.

**It falls toward the side you cut from** — which is where you are standing, so
move. On a slope the lean has a say, and if you cut a hard leaner against its
lean the trunk splits and goes where it is heavy instead of where you aimed it.
Drill higher up a trunk and nothing special happens; you just take a block off
a tree. Cut low, or you are only nibbling.

A sapling will bruise you. An old-growth giant weighs as much as a truck and
brings down whatever is under it, including the trees beside it — a stem in the
arc comes over too if the one falling is carrying enough. Rock and steel stop
it dead and leave it **hung up**, leaning where it stopped. Everything else in
the way goes.

When it lands it is a line of **logs** along the way it fell, following the
ground. Off an emergent giant or an ancient it is **prime timber** instead,
which the fabricator mills into three times the planks. Ancient trees are rare,
older than anything built near them, and hard enough that a starter drill
barely marks the bark.

Fell the same tree the same way twice and it lands in exactly the same place
both times. The fall is worked out from the tree, the direction and the tick —
no physics engine having a different opinion on a different machine.

### Three forests, and the ground decides which

The country is no longer one kind of tree. Every column belongs to one of
three forests, and which one it is falls out of two things the terrain already
knows: how high the ground is, and how wet.

Down in the flat hollows — where the ground is level and everything around it
drains inward — is **peat bog**: thin crooked black spruce you can see clean
through, standing on a carpet of sphagnum instead of grass. Through the middle
elevations is the **hardwood cove**: broad crowns, an open floor to walk, and
one stand in a dozen carrying an **emergent giant** that stands a head above
everything else and can be picked out from a ridge away. Up high it is
**subalpine conifer** — dark tapered spires that thin as they climb, giving
way at the **treeline** to knee-high **krummholz** mats hugging the rock, and
then to nothing at all, which is what makes a summit look like a summit.

The bands are not drawn on. They wander by a few blocks, so a treeline
meanders the way a real one does, and cold air pools in hollows — so a deep
enough draw is colder than its height says, and spruce comes fingering down
the drainages into hardwood country.

Nothing about it is stored. Which forest a column grows is a function of the
seed and the column, worked out from the land's own shape rather than the
flattened plot a town sits on, so the same hillside grows the same wood
whichever direction you walk onto it from.

### There is weather, and it will burn you out

The sky goes over. The wind picks a direction and holds it, the light goes
flat and grey, and it starts to rain — and it rains *across* the map, so a
front comes at you over the hills rather than switching on where you happen to
be standing. It wets the ground down as it goes: hollows fill, the surplus runs
off downhill through the same water that has been moving since the last round,
and it drains away after. Type `WEATHER` at the terminal and it will tell you
what the sky is doing, which way it is blowing, and how dry the woods are.

Then the storm gets mean, and the lightning starts. It goes for the tall and
the lonely — whatever stands proud of the ground around it, which is the
emergent giant over a cove or the one spruce on a ridge, not the middle of a
flat wood. Most strikes do nothing at all. About one in fifty lights, and how
likely that is depends on how long it has been since it rained and on what the
bolt actually hit.

And then the woods go up. Fire runs **uphill and downwind**, because that is
what fire does: the chance of it taking the next block along is the no-wind
chance multiplied by the wind blowing that way and the ground tilting that way,
so the direction it spreads fastest is the sum of the two. A black spruce bog
is kindling with cones on. A damp hardwood cove barely catches. Subalpine
burns rarely and then totally. And it does not care whose wood it is: your
house, a shop's plank walls, the fabricator you left sitting in a clearing.
Cut yourself a firebreak or lose the lot.

One thing will not burn, and that is **ancient** wood. A grove of those old
things is the safest ground on the map, which is the point of having gone
looking for one.

The best part is that it grows back. Burnt ground remembers when it burnt — and
so does ground you cut — and over the seasons it comes back through weeds, then
a scrubby mess of saplings, then half-grown trees, then the forest that was
always there in the seed. Black spruce comes back fastest, hardwood next,
subalpine slowest. Nothing is stored except the ground something disturbed, so
a forest nobody has touched still costs the game nothing at all.

The whole chain — the sky, the rain, the strike, the fire, the regrowth — is
worked out from the world seed and the tick. Two machines on the same tick get
the same storm over the same hill, and a saved game replayed from its own
journal burns exactly the ground it burned the first time.

### Somebody runs the town

Every town on the frontier has a **mayor** and a **sheriff** now, and they are
people off its own roster rather than a word on a lockbox — with names, jobs
and temperaments, worked out from the town's seed so the same place is run by
the same pair however you arrive at it. Type `TOWN` at the terminal and it will
tell you who they are, what everybody who lives there is worth, what they are
doing right now, and where you stand with each of them.

**And the townsfolk work.** They have always had a trade and a place to be at
any hour; those two facts now mean something. When the schedule has somebody at
their bench they are putting goods on the town's books and credits in their own
pocket, on the same clock the market has always run on. Walk into a place on a
market day and its shelves genuinely differ from the same place at four in the
morning, because its people are at the square instead of at work. Nothing about
it is stored: a resident's purse is the shifts they have worked, worked out
from the seed and the tick, so every person in every town in the world has
money in their pocket and the save is not one byte bigger for it.

**Trade with somebody enough and they will trust you**, which is a different
thing from liking you. Friendship you buy with gifts and conversation. Trust
you buy with business — and once you have done enough of it, they will hand you
a key to their own door rather than leaving you to pick the lock. A trader who
never gave anybody a present in their life can still end up with the run of a
town.

### The sheriff has to ask somebody

Rack up enough bounty and the deputies used to simply appear. Now the sheriff
cannot act alone: he has to take it to the **mayor** and get a warrant signed,
and the mayor is a person with an opinion of you.

A proud one signs because the law is the law. A nervy one stalls. One you have
been good to for a season will find reasons to leave the paperwork in a drawer
— and a refusal is a **reprieve, not a pardon**, because he can be asked again
in a few minutes. What friendship buys you is time. Rob a bank vault and it is
signed before the ink is dry, whoever your friends are.

It costs you either way. The moment the paperwork is filed there is a **fine**,
and what your wallet cannot cover goes straight back onto the bounty — being
broke and in trouble is worse than being solvent and in trouble. And that
town's **counter shuts to you** while anything stands: not a worse price, a
closed door. So there is a whole middle now between "nothing happened" and
"there are four lads with guns coming over the hill", and you can talk, pay or
run your way out of it.

### The drone's camera comes off its own hull

Look through one of your machines and the camera was sat **inside it**. The
digger's was in its cab box. The flier's floated in the gap between its hull
roof and its rotor. The little scout's hovered above its own blades. You never
noticed because the camera happened to land in air pockets and you were looking
out through the back faces of your own machine.

It hangs under the nose now, on a mount, like a surveillance bird — measured
off each rig's actual parts so it is forward of the hull, below its floor,
above the skids and clear of anything that spins. Two things had to come with
it. The machine you are looking *through* is no longer drawn, because a camera
outside the hull sees the hull. And machines now **turn** smoothly instead of
snapping: their position was always glided between ticks and their heading
never was, which nothing could see while the camera ignored heading entirely —
bolt a camera to the nose and you feel it on every corner. Every drone you
merely watch turns better for it too.

**And the thing nobody meant.** When you take the wheel of a drone and cut with
it, the game **was not writing that down**. Not taking the wheel, not what you
were holding. Only the number of ticks that passed. So the book the game
replays your day out of had a hole in it exactly the shape of every hole you dug
by hand: load it back and that ground was untouched.

That is not a hypothetical. Before the fix, the same orders played out to one
world and replayed to another — two different hashes over the same session, on
purpose, in a test written to prove it. Both are recorded now: who has the
wheel, and what they are holding, folded onto the log only when it changes,
exactly the way walking already was. The test that proved the divergence now
proves the agreement, and it sits beside every other "replays to the same
ground" test in the file.

Where the pilot is *looking* is deliberately not recorded: it only points the
machine's drawn nose and never reaches a world edit. That is the sort of thing
it is easy to add by mistake, so it says so in the code.

### You can play it with nobody at the keyboard

Somebody asked me to play a round — walk out, dig something up, sell it — and
the first thing I found was that **nothing could**. Every verb in the loop is
welded to the window: walking, aiming, drilling, opening the counter. The
drone can be tested doing the whole job on real ground; you could not. So the
loop's three verbs had wildly different coverage — the walk is proved to a
micrometre three thousand kilometres from spawn, the drone's mining is proved
end to end on ground nobody arranged, and selling is proved against a
hand-built pile and a bare market. Nothing joined them up.

**And the join is where the bug was.** When you break a block by hand it goes
on your pile — except you have not *got* a pile until you place a container,
and there is none when the world opens. So a new player walked out, spent two
seconds a block chipping ore out of a hillside, and **every bit of it went
nowhere, silently.** No message, no line in the terminal, nothing. Every other
system in the game says so out loud — the printer, the bank, the chest, the
job board all tell you there is no base pile — and the one that produces the
goods was the only one that stayed quiet. It says so now, and the welcome
panel says it before you get there.

**So the game can be played without a person.** A session opens a world, puts
you in the doorway, and drives the same movement, the same drill and the same
shop counter the window does — not a simulation of them, the things
themselves. It walks by holding forward and letting the automatic vault do
the climbing; it aims by turning the head; it holds the trigger until the rock
gives up; it presses Enter on the shelf. It can also *fail*, and that is the
point of it: a walk that cannot get there reports where it stopped rather than
pretending.

Playing the whole loop on the shipped seed goes like this. Out of the front
door, 170 blocks east to the copper that breaks the surface on the hillside,
sixteen blocks of ore cut out at two seconds each, and home again — **slower
than you left, because the pile is the weight on your back.** Then over the
counter for 224 credits at the price that town was paying that day, and the
price drops for whoever sells next. Four pictures come out of it, one per beat.

Three things went wrong while playing it, all of them now fixed and all of
them the sort of thing only playing finds. The first run walked out of the
house and straight into the wall beside its own front door, because it set off
on the bearing of somewhere 170 blocks away while still stood in the kitchen —
so the door is a named place now, and so is the shop's. The second climbed a
three-block cliff by holding jump, which also meant it climbed the *shop*, and
finished stood on the roof selling ore down through the ceiling — so the walk
only reaches for the jump when it is genuinely stuck, and "am I at the counter"
is a raycast at the till, the same question the game asks, rather than a
distance. And the third could not get past that cliff at all until it learned
to walk along a face instead of bouncing off it.

### Nobody could ever walk into a town

The ask was simple enough: go scouting, dig something up, **save the game,
load it back**, then take the goods to a different village and sell them. Two
things fell out, and the second one had been sitting there since the day the
walls went up.

**Your pile does not survive saving.** You mine a load of ore, it goes on your
pile, you save, you quit, you come back — and it is gone. Not moved, gone. And
the game has also forgotten you ever put a container down, so the *next* block
you dig vanishes too, which is the silent loss from a couple of rounds back
turning up again the moment you reload. What makes it daft is what *does*
survive: your money, the prices you shifted at the counter, and every single
thing in the chest in your house. The one pile the shop actually sells out of
was the only thing in the game that forgot. It has its own little save file
now, the same as the fuel tank and the wear ledger, and it matters more than
"some ore went missing" — the fuel your drones burn comes off that pile, so a
pile that forgets is a fleet that stops.

**And then the haul could not get there.** Every town out here is walled, and
outside the wall is a ditch three blocks deep. There is one way through a
curtain wall and that is the gateway — and the ditch ran **straight across the
gateway**, because the note in the code said a causeway would be a cheat and a
drawbridge is a mechanism this game has not got. A body can pull itself up
about two blocks. Three is one too many. So the gate was a hole in the wall
with a trench in front of it, and **no player has been able to walk into a
town, or back into the one they started in, since forts shipped.** Nobody
noticed, because you wake up inside your own walls and everything that ever
got tested happened in there.

The note was wrong twice. A real fort built without a bridge is built with an
**uncut causeway** at the gate — a strip of ground left in place — which is a
fortification detail, not a compromise. And it made the entire town layer
unreachable on foot. The gates have their causeways now, and a test walks each
one from outside the ditch to inside the wall and fails the build if the ground
is broken anywhere along it.

**The walker also learned to stop and look.** It used to hold forward and
sidestep, which is what a person does and is plenty inside a town; two hundred
blocks of open country is another matter, and widening the sidestep or
doubling the allowance only ever got it pacing the same ridge for longer. So
when the legs run out of ideas it now sweeps the ground it can actually see —
the same breadth-first search the drones have used since the beginning, but
with a *body's* rules rather than a machine's: two blocks of headroom, a climb
of two, and a drop of six. That difference is the whole difference between a
wall and a staircase on a hillside, and the drones' own version had said 2,673
cells out of 218,000 were reachable and none of them any use.

The run, end to end: the flier sweeps a sector in 792 ticks and comes back with
two pings, the best of them 322 columns of ore under no overburden at all; 20
blocks cut and 25 goods on the pile; save, drop the whole session, load it back
— **25 goods still on it**; then 206 blocks over the hills to a depot that has
no ore of its own and pays **405 credits** for yours. Four pictures come out of
it, one per beat.

### You are carrying it now

Here's a thing nobody's said out loud in fifty-four rounds. **You have never
actually been carrying anything.** You swing the drill, the rock breaks, and the
copper teleports straight into a container that might be three hundred metres
away behind a hill. And then the game slows you down — for carrying it. The box.
That you aren't touching. That's been sitting in the known-problems list since
round sixteen like a mug nobody's picked up.

So: pockets. You mine a thing, it goes on you, and the screen tells you what
you've got — COPPER ORE 12 — on every single swing. Hit **I** and there's the
whole list, heaviest first, with a bar showing how loaded you are and where the
line is.

**Ore's heavy. Leaves are nothing.** A block of plain stone is the yardstick;
copper's about twice that, uranium three times, and a whole tree's worth of
leaves is barely worth noticing. So what you choose to haul home is an actual
decision now instead of a number going up. A fresh pack takes sixty-four stone —
exactly what it always did, so nothing you're used to got smaller — or twenty-one
of uranium.

The first stretch is free: under about a third full you walk normally. Past that
you feel it, and it gets worse the fuller you get, on exactly the same curve the
game has always used — it's just finally attached to something you can see. And
when it's completely full, the next rock you break **falls on the floor**. Little
block sitting there, turning slowly. Nothing is destroyed, nothing is wasted; go
back lighter and walk over it and it hops into your pockets. A block you knock
out of a ceiling lands on the ground underneath it rather than hanging in the
air at head height.

Get to a container, press **E**, and the whole lot tips in. And when your legs
have had enough there's an **exoskeleton** at the counter — five marks, a hundred
credits up to sixteen hundred, or print one at the fabricator — which carries
more *and* moves the "you're feeling it" line up so most of the pack is free.

Two things fell out of the floorboards on the way through. The status panel has
been quietly drawing rows off the bottom edge where nobody can see them, and one
of the bars on it was one bad afternoon away from **crashing the game outright**
— it wrote past the end of its own picture, and nothing had ever asked it to draw
everything at once. Fixed, with a test that lights every row.

And the replay check — the thing that proves a saved game plays back to the exact
same world — **has never once counted a rock you broke by hand.** Since round six.
It hid because nothing downstream looked, except that the pile it was short of is
the pile your drones burn fuel out of, and a fleet that stops digs a different
hole. While proving that one, the same test found a second: putting a container
down in a replay never *declared* it, so a replayed session had a box standing
there with nothing in it. Both fixed. The check is stricter than it has ever
been, and it now agrees with the game about the goods and not just the ground.

### Nothing you did is lost

You said this one was a gate: if progress doesn't save, it isn't really a game.
You were right, and it was worse than either of us thought. I went through
every single thing the game holds — a hundred and four of them — and every one
of the thirty-four files it writes, and checked what actually happens to each
when you quit.

**There was no autosave. None at all.** It wrote your world when you pressed
F5, when you typed SAVE, and when you closed the window. That was the lot —
and all three of those are you politely asking it to stop. Everything else took
the session with it: the thing crashing, running out of memory, being told to
shut down, you closing the lid. On a Deck those aren't accidents, that's just
how you stop playing. It saves itself now, every couple of minutes and right
after anything you'd hate to redo — a sale, a drone bought, a crew sent out, a
level, a town founded. And it saves when the lid closes, when the window goes,
and on its way out.

**When it did save, it could hurt you.** The ground was written carefully — to
a spare file, pushed to the disk properly, then swapped in — so a crash
couldn't catch it half done. The other thirty-four files weren't. Straight over
the top, one after another. So a power cut mid-save didn't just lose the last
bit: it could leave one file chopped off, and because every one of those files
is written to be forgiving, a chopped-off wallet doesn't complain, it just sets
your money to zero. Or it could leave your *money* from after you sold the ore
sitting next to your *ore* from before it, and nothing anywhere compared the
two. Every file goes to a spare name and gets swapped in now, and there's a
stamp written across the whole set at the very end — so a mixed-up save is
spotted and refused instead of quietly loaded, and there's a kept copy of the
last good one to drop back to. The broken one isn't binned either; it's set
aside in case it says something useful.

**And if a save failed, nobody told you.** Disk full, card popped out, folder
gone read-only — a line into a log you'll never open. Worse: if it couldn't set
the save up when you started, it silently never saved *at all* and still said
"WORLD WRITTEN OUT" when you asked. Now a failed save is red on screen and in
the terminal, the "written out" line is earned and tells you which save and how
long it took, and a world that can't be saved says so the moment you start it.

Then the things that were just plain being lost:

- **Break your container and everything in it was gone on reload.** The game
  wrote those goods down on every single save and threw them away every single
  time it read them back. It's been doing that since the feature shipped.
  There's even a test proving it works — it just tests a different bit of code
  than the one you play. There's one bit of code now, and both use it.
- **Clear out a bunker, save, and they're all back up.** Restocked, including
  the ones you'd put down. You keep the money, so you could sit there clearing
  the same shelter for pay all day. That's a thing you lost *and* a thing you
  could cheat with, out of one missing file.
- **A pump you built and switched on came back switched off.** Pump's there,
  water isn't moving, nothing says why.
- **Hack a watch box and the hack was gone on load** — but the charge you spent
  on it wasn't. You paid and got nothing.
- **Mark out a patch to dig and quit before sending the crew: gone.**

Last bit, the unglamorous one. There's a list in the code that's supposed to
say what gets saved. It had **six filenames in it that have never existed**.
Nothing was checking, because it was a comment. Now the game writes down what
it actually saved, and a test compares that against the list and against the
comment. It can't drift again.

I played the whole thing to prove it: cut two dozen blocks, broke the
container on them, saved, kept a fallback, tore the next save on purpose the
way a crash would, and watched it come back from the one before with all
twenty-four goods still there. A save takes about **six milliseconds** — the
first time this game has ever measured that, because there was nothing
measuring it, which is a poor place to be adding an autosave from.

### The drill knows what it's looking at

You wanted the drill to light the block up like something off an old arcade
cabinet, and you wanted it to ping the rock around it like sonar. Here's the
thing I found on the way in: **this game has never had a selection box.** No
little outline on the block you're pointing at, not even a crosshair. Fifty-odd
rounds, and the only way to know what was under the bit was to open the debug
panel and read the name off a text row.

So this isn't sparkle glued onto a highlight. It *is* the highlight.

**The cage.** Twelve bars of cyan light round whatever the bit is aimed at,
standing just off the faces so they read as projected rather than painted on.
Dim when you're only looking. Bright when you're cutting — bright enough to
read in a pitch-dark shaft, which is where you'll want it. And a plane of light
that rises through the block as you chew through it, so the progress bar is on
the rock instead of stuck in the corner of the screen. Free on any drill.
Nobody has to buy it.

**The ping.** Every time the bit touches a new block it reads four metres in
every direction and tells you what's in there. Ore in the walls around you gets
a magenta marker hanging against it. Ore that's still *inside* the rock gets
counted and reported on the scope instead — I'll be straight about why: the way
this renderer draws things, a marker inside a solid block is a marker nobody
can ever see. So the markers show you what you could reach out and touch, and
the readout tells you what's behind the wall and how far. `COPPER ORE 69 AT
1.4M`, and `45 IN ROCK`.

Two switches, one key each — `H` for the light show, `P` for the ping — and the
game remembers where you left them. Both are on the pad's second layer, and you
can type `DRILL` at the terminal if you'd rather.

Now the boring-but-important bit. **Neither of these things does anything to
the world.** They don't move a block, they don't move a credit, they don't
change a single number. It's a lens, not a lever. That matters because there's
a thing in here that replays a whole session back and proves it came out the
same, and there's now a test that plays the game twice — once with the light
show on, once with it off — and demands both come out byte for byte identical.
If a pretty effect could ever change what happens, that test goes red.

One thing got faster on the way past: the ray that works out what you're
pointing at used to be cast twice a frame and thrown away both times. That's
*why* there was never a selection box — nothing outside those two bits of code
could find out what the block was. It's cast once now, and everyone reads the
same answer.

### The robots dig, and the books balance

Then: the loop where you buy bots, they mine, and you get rich — with two rules
attached. Everything has to save, and there must be no way to glitch the numbers
upward. Both turned up more than the feature did.

**Your crew didn't survive saving.** Buy the drones, mark out a patch, set them
digging, save, quit — come back and they're gone. Not idle, gone; the hole still
there with nothing in it. Third round running I've found the same daft thing in
the same place: first the pile didn't survive a save, then *you* didn't, now the
crew. This one's the worst of the three, because leaving a drone working is the
entire point of owning one. They pick up where they stopped now — they don't dig
while the game's off, because that would wreck the thing that lets me replay a
session and prove it came out the same.

**Three other things weren't being written down either.** Every sector you'd
scanned — that you burned fuel to scan — forgotten. Everything the little scout
bird had seen — forgotten. And the radiation you'd soaked up got wiped on every
load, which was a free trip to the doctor's, available from the menu, any time
you liked.

So there's a **census** now: a list in the code of every single thing the game
holds, and for each one either the file it lives in or the reason it's
deliberately not saved — with a test that fails if something quietly stops
saving *or* if something new turns up that nobody decided about. That's the
fourth time this bug has cost a round; it shouldn't get a fifth.

**And there was an infinite money glitch in it.** A town's price drops as you
sell it more, but only so far — it bottoms out and never goes lower. Stone
bottoms out at one credit a block. And nothing anywhere checked whether the town
had any money. So the best strategy in the game was: point drones at any rock at
all and sell the rubble to the nearest counter for ever. Mindless, unlimited, and
better than every clever thing the economy offers.

Towns have a **till** now. Real money in it, that fills back up from what the
town actually trades — a place you've stripped bare earns slowly, a busy one
earns fast — and a counter can only pay you what's in it. Turn up with a barrow
worth two thousand credits at a shop holding a hundred and four, and you sell a
hundred and four credits' worth and carry the rest back out. That kills the
glitch, and it's what makes getting the maximum an actual puzzle: which town,
which goods, how far, and when to come back.

There's a **PAYROLL** readout for exactly that: what your crew cuts an hour,
what's in your container, what this counter would give you, and which town would
give you more — ranked on what they can actually *pay*, not on the price on the
board.

Two more things fell out on the way. Breaking your own container used to
**destroy everything in it** — one stray click was the most expensive mistake in
the game; the goods are held now and the next container picks them up. And the
played run found the worst bug of the round: **loading a save was serving freshly
generated ground on top of the ground on disk**, so a hole you'd dug came back
filled in and the drone standing in it was walled up.

The run, measured: 40 blocks by hand, sold for 1,124 credits; a drone for 250;
one drone cutting 15,150 blocks an hour; saved mid-dig and reloaded with the crew
still cutting and not a block lost or doubled; a second drone for 375; and two
drones cutting **31,830 an hour — 2.1×** — on the same method and matching
ground.

### Every load put you back in bed

Then: leave town, mine, **come back**, save, go to another town, trade. And
make sure saving works. Two things fell out of that, and the second one is the
worst kind, the sort that is obviously broken the moment somebody actually
tries it.

**Coming home had never been walked.** Going out is the easy direction — you
are empty, you know where your own door is, and the gate you leave by is the
one facing where you are going. Coming back has to find a gate from *outside*
while carrying the load that halves your walking speed, and nothing in this
game had ever done it: every test either stayed inside the walls or left and
stopped. It runs now, 424 ticks from the hole in the ground to your own
counter, in through your own gate.

**And then I saved at that counter, loaded it back, and woke up in bed.**

Not a quirk of the test harness — the game itself. It puts you at the spawn
every single time you load, whatever the save says, because nothing had ever
written down where you were. The ground you dug came back. Your pile came back.
Your money, your skills, the town's prices, the chest in your house, the fuel
tank and the wear on your machines all came back. *You* did not. Walk two
hundred blocks to another town, sell your load, quit for the night, come back
tomorrow — and you are stood in your own kitchen in Stonehaven with the whole
walk to do again.

It hid for exactly the same reason the ditch did: every save-and-load test ever
written started at the spawn, and at the spawn "put him back where he was" and
"put him at the spawn" look identical. The new one deliberately stands
somewhere else first, and it fails against the old code.

Where you are and where you are looking now go in their own little file. Not
your stance — a save taken mid-crawl should not put you back mid-crawl under a
ceiling somebody has since dug out — and not your speed, because reloading into
a fall you started before you quit is a way to die at a loading screen. You
arrive stood up and still, which is what every game that does this does. It is
stored at full precision, so a save three thousand kilometres from spawn does
not shuffle you a few centimetres every time you quit.

The fiddly part was *where in the boot* to read it. The game prepares the
ground around wherever you are going to be standing before it draws the first
frame — so reading your position after choosing that ground puts you in
unloaded air, which the physics reads as nothing to stand on, and you drop
through the world. There is a test that stands a freshly loaded body still for
a second and insists it has not moved.

The run, end to end: out to the seam the flier found, 20 blocks cut, home again
laden, saved and reloaded **at the counter, not in bed**, then 206 blocks over
the hills to a depot for 405 credits. Five pictures, one per beat.

### The body stays out of the wall

You could get inside the world. Not far, and not for long, but you could —
walk up flush to a rock face and the corner of you was in it, climb onto a
ledge and for about half a second you were *inside* the ledge, back the
over-the-shoulder camera into a corner and the lens went through the stone.
Three different paths, one cause: every collision test in the game answered
a yes-or-no question — *is this box inside a block* — and not one of them
ever asked *how much room is left*. So the millimetre of clearance the
collision sweep leaves against every surface was a promise nothing checked,
and in three places it was quietly not being kept.

**Climbing a ledge was the bad one.** A mantle is the one movement the
physics does not drive: the body is walked along a fixed arc, hand over the
edge and up, and the integrator is suspended while it happens. That was
meant to be an exception about *momentum*. It had become an exception about
*geometry* too — nothing along that arc was checked against the world at
all, only where it finished. Now every tick of the climb is checked, and a
blocked one lets go: you drop off the edge like a hand slipping, which is
what the sweep would have done anyway. On the way past, the arc was made to
be what its own comment always claimed — up first, *then* across, instead of
the two overlapping in the middle, which is exactly the window where you cut
the corner of the block you were climbing.

**The camera had a floor that outranked the wall.** The follow camera pulls
in when something is behind you, and it was never allowed closer than about
half a block so it could not collapse into the back of your own head. When a
wall was closer than that, the floor won — and put the lens a quarter of a
block into the rock. It now refuses instead: no clear air behind you means
the frame is first person, which is what you wanted anyway when your own
back is filling the screen.

**And standing up left no room at all.** Getting to your feet under a low
ceiling checked whether the taller you *collides* — and a head resting
exactly on the ceiling plane does not collide, by a hair, so the one gesture
in the game that grows your body was the one that gave it no clearance. It
takes the same millimetre as everything else now.

There are no new pictures in this one; a margin does not photograph. What
there is instead is the family of tests that never existed: walk into a wall
and the hull rests *exactly* a millimetre off the face, land on a floor and
it rests exactly a millimetre above it, climb a two-block bench and the hull
is outside the rock on every single tick of the climb, and sweep the camera
all the way round inside a three-block room and there is a full quarter block
of air past the lens at every angle.

### Everybody gets the good math

Stage 42 fixed the far-from-home jitter for *you*: your body, your feet, your
aim and the camera moved to double precision, and the renderer started drawing
relative to the chunk you stand in. It left everybody else on single
precision and wrote the limit down — a quarter of a block at three thousand
kilometres, invisible in a still. This stage moves the line.

Every body in the game now carries a double-precision position: the
townsfolk, the deputies and the shelters' holders, the thing in the dark, the
slugs in the air, the falling stems, the caravans, the kestrel's marks, what
a villager last saw and where a posse believes you are. Directions, headings,
ranges, speeds and the rigs' own geometry stay single: they are scale-free,
and the rule is the one stage 42 set — *take the difference wide, then
narrow it*. Every body is drawn in the camera's frame too, the way your own
body and the tool in your hand already were, so a villager's feet three
thousand kilometres out land on the same bytes as they do at spawn. The
per-object lighting pass, which darkens a body by the column it stands in,
now adds the render origin back before it reads the column — it had been
reading your own body's light from the wrong chunk since stage 42.

The muzzle of a shot is the one float that crosses the journal's wire, and it
crosses at double precision now: a round fired from three thousand kilometres
out starts at your eye rather than a quarter block from it. That is a new
journal version, so an older log restarts the oracle the way every older log
does. A downed caravan's load remembers its column at the same width, and an
old `arsenal.dat` reads its narrow columns and widens them.

The proofs are the stage-42 tests' siblings: the same posse called out on
the same floor at spawn and at three thousand kilometres walks the same walk
to a micrometre for two hundred and forty frames; the home town's people,
moved to a far centre, stroll the same stroll; the same two rounds leave the
same craters, block for block and bite for bite; and the player's rig built
through the camera at the origin and a hundred thousand chunks out comes out
bit-identical. On the way past: a falling stem that hit the stump of a tree
already down used to fell it again, and a stem that took a neighbour early in
its arc forgot it by the time it landed. Both fixed. And the handheld's roster
listed the kestrel twice.

### Winter is ground now

Stage 41 gave the country a year and drew one line through it: a season can
change the *colour* of things and never the *ground*. The leaves turn, the sky
goes grey, the fire season comes and goes, and no block moves — and the test
that says so named the one thing that would turn it red: snow that settles.

That is this stage. When it is cold enough and it is coming down, it is
**snow**, and it settles. The grass goes white, the sand goes white, the bog
goes white. Not a pile on top: the ground is the same height it was, your
doors still open and the paths are where they were. The surface block is
swapped for its snowed twin, and a snowed block is a block *name*, which is
what the region files, the world hash and the minimap all already read. Snow
lands on open ground only — a tuft on the grass counts as open sky; a roof, a
log, a wall or a canopy over the column does not.

And the lake **freezes**. Full, still water at or below sea level goes solid,
and ice is a solid translucent block and nothing else, so everything that
follows from a solid block follows: you walk across it, so do the drones, so
do the deputies coming for you. A pump on ice lifts nothing. An electrolyser
beside it says `NO WATER IN REACH`. Fire finds no fuel in snow or ice, so a
stand that burns in winter leaves ash on snow. The lake bed under the ice
keeps its light, because the mesher's roofs are opaque and ice is not.

Then the thaw. Snow goes back to exactly the bare block it lay on, ice goes
back to a full block of water, and the pump starts up. The cold itself is a *weather*, not a calendar entry: the
temperature is mostly the place's, a cove warmer than a summit in any month,
with the year leaning on it, so the high country freezes in midwinter, the
lowlands never do in summer, and the shoulders of the year cross the line on a
cold front and come back. `WEATHER` at the terminal says `AIR FREEZING` and
`SKY SNOW` when they apply.

It only happens near *you* — the same rule the rain follows — so when the game
replays your day from the book, it snows on the same columns and thaws the same
columns and the ground comes out identical. That is the test, and it is the
ordinary one this time rather than the inverted one: a winter journal replays
to the same snow and the same ice, and differs from the untouched country.
Stage 41's own test did not go red: *reading* the year still moves nothing.
The snow is not a reading; it is an automaton on the journal's side of the
line. And on the way past: the rain had been falling on a diagonal through
the player since stage 38, because its second hash draw read the same top
bits as its first. Both draws are finalised now.

### A town of your own, one way or the other

Getting voted in was the only way to run a town. Now there are two more.

**Found one.** Print a **town charter** at a fabricator — it is the dearest
thing on the ladder, and meant to be — walk out to flat, dry ground a good way
from anybody else, and say `FOUND IRON REACH` at the terminal. The names come
from the same book every town is named from; ask for a word that is not in it
and the terminal shows you the book. Then there is a town. All of it: the
tower, the shop, the clinic, the bank, the houses, the paths, the wall, and
three settlers who arrived with the charter and are already open for business.
It is built by exactly the code that builds every other town, so nobody can
tell it was founded. You hold the mayor's chair and the sheriff's, because it
is your town — and it is still a town, so it votes. A settler owes the founder
the roof over their head, which keeps you in through the quiet first terms;
after that you trade with them like anybody else.

The plot is levelled when the town goes up. Anything you dug there is filled.

**Or take one.** Get a warrant on you somewhere, and the deputies come. Put
every one of them down or into surrender, then walk to that town's console and
say `TAKE`. Both chairs are yours. So is the bill: the Compact will remember
it, every mayor within radio range signs a warrant on you by the next civic
tick, and the three people who live there now trust you exactly nothing. At
the next poll they will throw you straight back out unless you have paid the
fines and bought them back. A town you took is a town you have to keep.

`TOWN` and the console say which it was — `FOUNDED BY YOU - DAY 12`, `TAKEN -
DAY 30` — and, on the way past, every town's *sheriff* now comes up for a vote:
since stage 40 the first seat polled on a market day had been quietly closing
the ballot for the second.

### The country goes on

There used to be a place, about fifty minutes' walk in any one direction, where
the game quietly stopped working. Not with a crash: you would start sticking
to walls. The numbers the game used to say where you were had got too big to
be exact — a ruler with marks every foot, asked to measure a kitchen — and a
body that should have been a millimetre off a block face rounded into it. A
good way further out, the picture itself would have started to shake.

Nobody had walked that far because there was nothing out there. There is now:
towns, and drones to ferry you, and a frontier is only a frontier if it goes
on.

So it goes on. **The picture is drawn from where you are standing** rather
than from the middle of the world: every number the graphics card sees is "how
far from your chunk," never "how far from spawn," and the subtraction that
turns one into the other is done on exact integers before it ever touches a
float. And **you yourself** — your feet, your eyes, the part of you that bumps
into walls — are measured with the big ruler now, the one with the fine marks.
Everyone else in a town stays on the ordinary one, because they never leave
town.

Three thousand kilometres out the woods look like the woods, the tool in your
hand sits where it should, and you walk down a corridor exactly the way you
walk down one at home. There is a test that draws the same hill at spawn and
again sixteen hundred kilometres away and requires the two pictures to be
identical to the byte, and another that walks the same corridor at spawn and
at fifty thousand kilometres and requires the same footsteps to a
micrometre. Nothing about the world moved to make this true: the blocks are
where they were, the saves are what they were.

`--at 3000000,3000000` works as a capture flag, if you want to see for
yourself.

### The year turns

The game counts a **twenty-eight day year** — it always has, quietly, so that
villagers have birthdays. Now it runs the weather and the woods: a week each of
spring, summer, autumn and winter, one season to an election term. A full year
is about an hour of play.

**And you can see it.** The leaves turn — green, then gold, then bare, then
back. The hardwoods do most of the work, because that is what hardwoods do; the
spruce barely notice, because that is what spruce are like; the grass goes to
straw and then to dun; the moss in a bog goes rusty. The sky goes with them,
deep and hard in the summer and pale and flat in the winter, and the light
comes down with it.

**Summer is a fire season.** The ground dries right out and *stays* dry — a
shower in August is gone in a day where the same shower in April would have
kept things safe for a week. So when the lightning starts it actually bites,
and a hillside will run on you. Come winter the country is sodden, the strikes
do nothing, and you can be careless again. Nothing in the fire model changed to
make that happen; it reads how dry the fuel is, and how dry the fuel is now
knows what month it is.

**And the woods stop growing over the winter.** Burn a stand in October and
that is it: it sits there black until the spring, and only then starts coming
back through meadow and thicket to forest. The burn you set before the snow is
still staring at you in February — which is the whole point of having a year.

It does not make anything slower. A year of growing still fits in a year: the
woods simply do none of it in the dead of winter and twice as much in June. And
the season is derived from the tick like everything else here — nothing is
stored, no save file grew, and a stand cleared in autumn and a stand cleared in
spring come back as exactly the same forest, just months apart.

`WEATHER` at the terminal opens with the season, the day of the year and
whether the woods are in their fire season.

### You can stand for office

A town's mayor and sheriff used to have the job for ever. Now every town
**votes** — on its own market day, once a week, when everybody is down at the
square anyway.

And you can put your name in. Walk up to the beacon console, press `TAB` to
turn it to its **ballot page**, and there is a row for each seat: stand for it,
or take your name back off. The page also tells you how each resident is
leaning and how long you have until they vote.

**They vote on who has been good to them** — and specifically on business. The
trust you built selling ore across somebody's counter is a vote. Being liked
helps. A bounty in that town is steep enough to lose you a room you had already
bought, because the frontier will elect somebody it likes rather than somebody
it is frightened of. And if the sitting mayor has a warrant hanging over him he
looks weak, and it costs him.

A poll is a **referendum on you**, not a scrap between neighbours. In a town of
three, nobody swaps the man in the chair for the man beside him — they have
lived next door for years. What changes it is somebody turning up from outside
that the town would rather have. So if you never stand, the incumbent is
returned, for ever, and the town costs the save nothing at all.

Win the sheriff's badge and it is real: every lock in that town opens for you.
**Only that town.** A badge belongs to the place that issued it — which is a
thing that used to be wrong, and is fixed as of this round.

Win the mayor's seat and the town's own property is yours, and you decide the
warrants — except your own. **You cannot sign your own paper.** So the sheriff
takes it up the road to the next town's mayor, who is a different man with his
own opinion of you. A town you run is a good place to be. It is not a place
they cannot reach you.

### There is a game on the handheld

Print a `POCKET ARCADE` cartridge at a fabricator — it is dear, and it sits
high on the ladder, because it is the last thing you need and the first thing
you will want. It slots into the unit you already carry. Raise the handheld
with `V`, `Tab` round to the arcade page, and there is a corridor shooter
running on the glass: rock walls receding into the dark, something with two
eyes coming down the passage at you, a gun at the bottom of the screen and a
door out somewhere on the floor.

Find the door and the next floor is worse — more of them, faster, and less
ammunition lying about. A kill pays two rounds back, so pushing forward is
what keeps you loaded. When they finally get you the run ends, and the
cartridge remembers two things: the best score and the deepest floor you
reached.

Every floor is a number: the same cartridge deals the same floors in the same
order, which is what makes a score worth comparing. And every wall, every
enemy and every pixel of the status strip is computed here — no borrowed
assets, no borrowed engine, exactly like the terrain and the audio. It is not
a port of anything.

### The townsfolk have faces

Everybody in every town has eyes now, and the eyes go where their attention
does. That is not a new system: the townsfolk have turned to look at things
since stage 7, and the sighting the body turns toward is the same one the
pupils use — one perception, two tells. Walk past somebody working and you
will catch them tracking you without turning their head. Get far enough round
behind them and the gaze *saturates* rather than swivelling into the back of
their skull: past about thirty-five degrees the eyes give up and the body has
to turn, which is what makes the whole thing read as a person rather than a
pair of googly eyes.

And if you walk straight into somebody, they grunt at you and shift out of the
way. Three voices in a town rather than one recording, no line of sight
needed — being trodden on is not something you have to *see* coming — and
once per approach, with the same hysteresis a greeting uses. It says so on
screen as well as out loud: a machine with no sound device is a supported
machine here, so the tell had to survive one.

### Every town is walled

Stage 21 gave the big towns bastioned traces and left everybody else standing
in a field. Now the floor is a **mini star**: four short bastions on a low,
thin wall drawn in tight against the buildings, with the same ditch, the same
four gates on the four roads, and the same lockbox on each gate. It is a wall
a village could plausibly have built — and about a third of them have been let
go, so you will find as many gaps as walls out there.

Nothing on the frontier is unwalled any more. The `OPEN` trace is gone rather
than left in the table unreachable: a variant that says "this place never
bothered" is a story this world does not tell.

### There is a hospital in every town

Health has mended on a timer since the deputies arrived, and an arrest has
always put you back on your feet. Neither of those is a *place*. Every town
now has a clinic — a ward with two cots and a lockbox, claimed like any other
building — and the cot is **free**. Lie down and you are whole again, and it
**scrubs your radiation dose** with it, which is the answer to a uranium face
that the last round deliberately left open.

Free, because the cost is already the walk. A cot in town is worth nothing at
the bottom of a shaft forty minutes away, and that is exactly what makes the
**medkit** worth its forty-five credits: bought at the same counter, carried,
and spent with `patch` at the terminal wherever you happen to be bleeding.
Two hits back, five in a pocket, and a kit is never wasted on somebody who is
already whole.

### Uranium, oil and gas

Three new things are in the ground, and none of them behaves like copper.

**Uranium** is banded far below the overburden, so nobody meets it by walking
— you go and find it, in the dark, a long way down. It is worth more per block
than anything else in the world, it takes twice as long to cut, and it is
doing something to you the whole time you stand in it. Exposure is not a
timer: it is a sum over the bare uranium within five blocks of your body,
falling off with the square of the distance, so every lever you have over it
is a physical one. Back off between cuts. Wall the face back up. Send a
machine instead. Or print the **lead lining** at the workshop, which buys time
and never buys immunity — a fully lined suit still takes a third of the dose,
and nobody gets to live in a uranium face.

**Oil and gas** are not ore at all. A reservoir is a body hundreds of blocks
across on its own coarse lattice, deep, and worthless one block at a time —
digging into oil sand by hand gets you the smell of it and nothing else. What
makes it pay is a **wellhead**: printed at the fabricator, carried out to a
column with something under it, and left there. Spudding in costs casing and
cement up front, the string takes minutes to reach the crown, and then it
lifts on its own clock into your base pile while you go and do something else.
It is the first machine in this game that keeps paying — and, because a
reservoir is finite, the first one that stops.

A dry hole costs exactly what a good one costs. What is under a column is a
pure function of the seed, so a duster is a *place*, not a dice roll: the same
column is dry in every session of that world, and a player who learns the
ground has learned something true. The panel will tell you whether the mud log
shows a trace before you spend the casing. What fluid, how much, and how deep
are what the drilling is for.

Oil is what the towns pay for. Gas is what your fleet burns when there is no
lake for two kilometres: a canister of well gas is worth three fifths of a
canister of oxyhydrogen, and the tank reaches for the good stuff first and
falls back to gas on its own.

### Something down there heard that

The deep is not empty any more, and what is in it hunts by ear.

It is two brains, deliberately. A **director** knows exactly where you are and
is forbidden from saying so: everything it passes on is quantised to a
thirty-two metre cell — the same grade the shelters' director uses, because
there is one rule about lying to your own monsters and it is written once. The
**creature** takes that cell and closes the rest with the same occupancy
search a posse uses. It cannot walk to you, because it does not know where you
are. Break line of sight and its picture of you goes stale exactly the way
yours goes stale about it.

What attaches it to this game rather than to a haunted corridor: hints are
weighted by noise, and **machines are loud**. A drill chewing rock is a dinner
bell. A shot underground is louder still. So the pressure lands on the core
mining loop and the levers are all yours — run the swarm loud and rich, run it
short and quiet, or dig a decoy a valley over and work in the noise of your own
diversion. Heat fades when nothing is cutting, and it fades four times faster
in daylight: coming up is the reliable way out.

Nothing ever arrives within forty-eight metres of you. It comes from somewhere,
always, from the direction the noise came from, and it says so — every change
of mind lands a line, because a search nobody can perceive may as well be a
random walk. Sustained contact spends a budget: ninety seconds of it and the
director makes the thing break off for a minute, which is what turns a monster
into a rhythm. Eight rounds put it off for the night.

### Two peoples keep two books on you

The bounty board is a bill; getting arrested settles it. Your *name* is a
different thing, and nobody wipes that. The settled towns — the Compact —
remember every witnessed crime long after the fine cleared, and every honest
sale, capture turned in, and gift too. The people who hold the shelters —
the Holdouts — remember which of theirs you dragged to the board, and a
capture offends them *less* than a body: the board parades captures, graves
are quiet.

Standing runs Enemy → Cold → Neutral → Warm → Friend, and it moves like a
season, not a mood. With the towns it shades every counter's prices a few
percent either way. With the shelters, Neutral buys you the **truce**: walk
near a held hatch and you get "WALK ON - THIS GROUND IS HELD" and a few
seconds of grace instead of a volley — until you press in close, linger too
long, run a drill on their ground, or fire a shot. And once a shelter holds
a grudge, the spoofers you learned about in the hacking round turn up in
*their* hands: fly your kestrel over their ground and it marks nothing.
`standing` at the terminal shows both books and what they are costing you.

### The shelters are held

The bunkers stopped being free real estate. Come near a hatch and its
holders muster — two, three or four of them by the shelter's size, the same
squad every time because they are rolled from the bunker's own seed. They
run everything the deputies run: nerve, cover, fire discipline, the
searching. And they *path* now — hostiles route on the same flow fields the
mining drones drive on, so they come around the rockfall instead of
moonwalking into it, and an underground holder no longer teleports to the
meadow above.

What they hear is a zone, never a spot. A shot, or your drill running, tells
every shelter in earshot which 32-metre cell the noise came from — and
nothing finer, so they still have to come and look. Your drill is a dinner
bell: run the swarm loud and rich, run it slow and quiet, or dig decoy noise
a valley over and raid in the shadow of your own diversion. They move in
pairs, one walking while one watches.

Break a holder's nerve instead of their body and their hands go up — walk
over, press `E`, and they are taken in: the board pays 80 to 180 credits a
head by the shelter's size. A whole shelter can be cleared without a single
shot landing, if you can scare everyone in it. Clear it however you like and
it stays cleared: the caches below are yours.

### The law comes for you

Every crime you have ever been seen committing has been going on a tab. Push
that tab past the warrant threshold and the town stops writing and starts
sending: three deputies, fanned out, coming your way.

They do not know where you are — they *believe* where you are. Each squad
keeps a last-known position whose confidence fades, and spreads a map of
where you might have got to across the ground you could have walked. They
walk to the likeliest spot, and looking somewhere is what rules it out. So
they sweep, they cover ground, they double back, and when they run out of
places you could be they say so and go home. Break line of sight and the
clock starts; run far enough and hiding genuinely works. Duck behind the
nearest rock and it genuinely does not.

What you are playing against is their **nerve**, not their aim. Wound one and
their composure drops; miss one closely and it drops anyway, so you can pin
somebody without hitting them; put one down in front of the others and it
costs all of them badly. As it falls they stop fighting and start taking
cover, then start running, and finally put their hands up — except the proud
ones, who never surrender, and the nervous ones, who never bothered with
cover in the first place. That temperament is the same one the townsfolk
have had since they got names: the sheriff's steadiness and a drifter's
nerve are the same number, rolled once.

They will not shoot through each other, ever. If one ends up in another's
line, they step aside instead. And they say what they are doing out loud —
"check the far side", "taking cover", "trail is cold, spread out" — because
a clever enemy nobody can *see* being clever may as well be a random one.
Type `law` at the terminal for the roll call: what each deputy is doing, how
their nerve is holding, and what sort of person they are.

You can take six hits. There is no medkit: break contact, stay unhit for a
few seconds, and you start mending — so getting *away* is the heal. Take all
six and you go down, and going down in front of the law is an arrest, not a
grave: they take what you can pay of the bounty, write off the rest, and you
wake up at home with a clean sheet.

### Walls remember being shot

Blocks are still one metre, right up until something hits them. Then that
block — only that block — grows a 4×4×4 interior and loses exactly the cells
the hit took out. A slug leaves a bite where it actually struck; a blast
leaves a crater; the drill takes the layer nearest the bit each quarter of
the way through, so a rock face being worked visibly gets worked. Nothing
else in the world changes scale: the terrain, the towns, the forts and the
bunkers are the same one-metre grid they always were, and a block nobody has
damaged costs exactly what it always cost.

The good part is what falls out of it. **Rays read the cells; your feet do
not.** Shoot the same spot twice and the second round goes through the hole
the first one made — nobody wrote that, it is just that the cells stopped
being there and the ray noticed. Chip away at cover and it stops being cover,
a bit at a time, because every line-of-sight check in the game already runs
through the same raycast. But you can never *walk* through a wound: for
collision a damaged block is still a solid box until it dies outright. You
can shoot through a peephole; you can never fall through one.

Damage converges rather than piling up. Chew a block past the point where
there is anything left to hold and it becomes air, dropping its yield like
any other break — and anything a carve knocks loose goes with it, so you
never get crumbs floating in mid-air. Fire enough rounds at one wall and you
end up with a hole or a wall, never a museum of every shot you took.

Under the hood a wound is a single 64-bit number, one bit per cell, and
every operation on it is a shift or an AND or a population count — the sort
of instruction that is one cycle on the Steam Deck's chip. That is why
damage is cheap enough to be everywhere, and why two machines always agree
about what a wall looks like after a firefight.

### Machines wear out

Work costs machines. Every tick a drone actually digs or a flier actually
hauls is a tick of wear on that machine — and only work counts, so a fleet
parked in the garage ages not at all while one that dug all night is ruined.
They go **FRESH → WORN → FAILING → SEIZED**, losing a tick in five, then one
in two, then all of them.

The crew's pace comes off its *worst* machine, not an average, because an
average is something you have to be told and a worst is something you can
see. Type `fleet` and the roster names the bad one; the status line says so
out loud when something starts failing; and mending that one machine hands
the whole dig back. There is a nice wrinkle in it: a failing machine wears
*slower*, because it is working less, so machines decay towards death rather
than falling off a cliff.

Mending is what the workshop was for. **Spare parts** are printed low on the
fabricator's ladder — a fleet that cannot be mended is a fleet that dies of
old age, and you should never meet that wall before you can print your way
past it. `repair` at the terminal mends the worst machine, or name one:
`repair digger 2`.

This is the first thing a machine carries that the game's replay log has to
know about. Everything else you accumulate — credits, upgrades, friendships
— sits outside it deliberately, but a worn crew digs slower and how long a
crew dug is exactly what decides where the hole ended up, so wear lives
inside the simulation the log re-runs, right beside the fuel tank.

### The workshop

Upgrades used to be a shelf at the counter: five marks a line, cash only.
Now every line worth fitting also has a *part* you can print — a **drill
head**, a **cargo rack**, a **pack frame**, a **lamp reflector** — made out
of ore and time at your own fabricator. It is the same upgrade either way:
the same number, applied retroactively to machines already out in the field.
The counter is for people with money and the printer is for people with a
mine, which is the argument the fabricator has been making since it shipped.

Three of the lines are new. The **pack** lets you carry more before the
weight tells on your legs, the **lamp** throws further down a cave, and the
**press** makes every print finish sooner — and that last one is the single
thing the counter will not sell you. Rollers for the fabricator come out of
the fabricator. The panel shows what is fitted on each row (`3/5`), and
typing `kit` at the terminal prints the whole sheet: every line, what it
does, and what the next mark costs.

A repeat part never costs more materials than the first one did — the price
on the row is the price forever. What rises instead is the **skill** each
successive mark demands, so a fifth drill head is earned rather than bought.
That is not a balance whim: a recipe's cost is arithmetic the replay oracle
re-runs, so it has to be the same on both sides of a reload, while the
question *"may you start this print"* is only ever asked live. The same rule
cut a fuel-efficiency line from this round — machines burn fuel inside the
call replay re-runs, so an upgrade there would quietly change where the hole
ended up.

### Steam Deck: install once, play in Game Mode

Copy the `dist/` folder onto the Deck (Desktop Mode), then:

```sh
cd dist && ./install-steamdeck.sh
```

The installer verifies the binary against `SHA256SUMS`, installs it for the
current user (no root, nothing system-wide), and puts a launcher in the
applications menu. It ends by printing the two-step *Add a Non-Steam Game*
instructions — that registration is what makes the game appear in Game
Mode's library. `./install-steamdeck.sh --uninstall` removes it again;
saves stay.

**The controller works out of the box.** Steam Input's default Gamepad
layout presents the Deck as an ordinary pad, and the game reads pads
natively: left stick moves, right stick looks, and press `SELECT` any time
for the full scheme on screen. The stick is analog everywhere: on foot a
gentle push is a gentle walk, because the tilt rides the command as a
quantised byte the simulation replays. A key is all-or-nothing and always
walks at full pace, so keyboard play is exactly what it was. Buttons
drive the very same bindings as the keyboard — one implementation of every
rule, so every panel, the shop, the map and the handheld all answer to the
pad. The terminal still wants a keyboard for *typing*, but scrolls and
closes from the pad.

**You never need a keyboard.** The terminal — where you found towns, take
them, read the law and ask about the weather — types from an on-screen
keyboard the pad drives: `Y` inside the terminal raises it, the d-pad walks
the keys, `A` types, `X` rubs out, `Start` sends the line. And the left stick
is properly analog on foot now: a gentle push is a gentle walk, because the
speed rides the command the simulation replays rather than being thrown away
at the door.

**Every binding is reachable, and there is a test that says so.** Thirteen
buttons cannot name twenty actions, so two of them are *layers*: hold `LB`
for the block palette, hold `SELECT` for everything a hand does not need in
a hurry. Tap either and it does its own thing instead — `LB` swaps first and
third person, `SELECT` raises the scheme. A test walks every `KeyCode` the
game binds and fails the build if no button on any layer produces it, so a
control added without a pad path cannot ship.

| On foot | Action |
|---|---|
| Left stick | Move; click it to sprint |
| Right stick | Look; click it to turn the optics dial |
| `RT` | Drill / fire (hold) |
| `LT` | Place the selected block |
| `A` `B` | Jump, crouch |
| `X` `Y` | Use (trade, read, talk); the handheld uplink |
| `LB` (tap) | First or third person |
| `RB` | Cycle the mining method |
| D-pad ↑↓ | Mark an ore corner; the minimap |
| D-pad ←→ | Zoom the map |
| `Start` | Dispatch / hold to pick a lock |

| Hold `LB` — the palette | Hold `SELECT` — the second layer |
|---|---|
| D-pad: slots one to four | `Y` the terminal, `A` go prone |
| Face buttons: slots five to eight | `B` withdraw at a vault |
| `RB`, right stick: slots nine and ten | D-pad: scan, walk-or-fly, debug, save |

| In a panel | Looking through a machine |
|---|---|
| `A` confirm, `B` back out | `B` **hang up** |
| D-pad or left stick: the cursor | `Y` take or hand back the wheel |
| `Y` turn the page | `X` back to the roster |
| `LB` `RB` scroll the backlog | `A`, left stick: climb, descend |
| Left stick click: withdraw, delete | |

Building from source on Linux needs `libudev` headers for the pad backend
(`libudev-dev` on Debian/Ubuntu, `systemd-devel` on Fedora).

Controls:

| Input | Action |
|---|---|
| `WASD` | Move |
| `Space` | Jump (walk) / rise (fly). Held into a ledge above waist height, mantle it |
| `Left Shift` | Crouch (walk) / descend (fly) |
| `Left Ctrl` | Sprint |
| `Left Ctrl` + `Left Shift` | Slide, from a sprint. A slide-jump keeps the speed it built |
| `Z` | Go prone |
| — | Vaulting a waist-high ledge is automatic; you do not press anything |
| Click | Capture the mouse |
| Hold left button | Run the drill — harder rock takes longer |
| Hold left button (low on a trunk) | Cut a notch: about a third through aims the tree, and past the hinge it falls — toward the side you are cutting from |
| Right click | Place the selected block |
| `1`–`6` | Choose stone, dirt, grass, sand, the base container or your chest |
| `8` | Choose the fabricator, to place it (buy one at the counter first) |
| `7` | Take out or sling the slug launcher (once you own one) |
| Hold left button | (launcher out) Fire — slow, heavy rounds on a visible arc |
| `C` | Toggle first person and over-the-shoulder |
| `V` | Raise the handheld — a real unit that swings up into your hands, screen coming on as it arrives. Arrows pick, `Tab` turns the page, `V` again puts it down. Your drill is away while it is up |
| `Tab` | (in the handheld) Turn the page: fleet roster, map, kestrel command, arcade |
| `W` `S` `A` `D`, arrows or `Q` / `E`, `Space`, `Enter` | (on the arcade page) Walk, strafe, turn, shoot, start a run — keys only, so the pad reaches it too |
| `Enter` | (on the kestrel page) Give the selected standing order — orbit, sortie, perch, vanguard, dock — or, when the machine is standing at a lock or the town's watch box, set the coil on it |
| `Enter` | (in the uplink) Look through the selected machine |
| `R` | Take or release the master override of what you are watching |
| `Escape` | (on a feed) Hang up and return to your body |
| `E` | Trade at the shop counter: sell at that town's prices, buy drones and fliers, or order goods by mail from another town (arrows pick, `Enter` trades, `E` leaves) |
| `E` | Open your chest at home (arrows pick, `Enter` moves goods to the base pile), or collect the mailbox outside |
| `E` | Read a lockbox: who owns the building, where you stand with them, and the grade of the lock |
| `Enter` | (at a lockbox you have no right to) Hold to pick it. Needs the Security skill; you stand still and exposed while it runs |
| `Z` / `Left Shift` | Go prone or crouch — cover and a low profile are what stop anyone seeing what you are up to |
| `E` | Read the beacon console at the foot of a radio mast — take work, or sign for a delivery (arrows pick, `Enter` acts, `E` leaves) |
| `M` | Mark a corner of an ore body (two marks make an area) |
| `Tab` | Cycle the proposed mining method |
| `Enter` | Send a drone to dig it |
| `Backspace` | Cancel the marked plan, releasing the ground it held |
| `G` | Send the flier to scan the sector you are standing in |
| `N` | Toggle the minimap |
| `[` / `]` | Zoom the minimap out / in |
| `9` | The electrolyser, to place on a shore |
| `0` | The pump, once you have printed one |
| `E` | A pump you have placed: switch it on, and it lifts whatever water it can reach out of the top |
| `E` | A word with the nearest townsperson when nothing solid is in reach — or, if somebody nearby has their hands up, take them in for the board's pay |
| `T` | The terminal — type `help`; `who`, `talk` and `gift <good>` are the townsfolk's verbs, `kit` lists your upgrades, `repair` mends a machine, `patch` spends a medkit on you, `law` reads the deputies, `standing` your name with both peoples, `wells` every hole you have sunk, `weather` the sky, the wind and how dry the woods are, `town` who runs this place, when it votes and what it has out on you |
| `E` | A printed wellhead, once it is standing: the mud log says whether anything shows under this column, and `Enter` spuds the hole in |
| `E` | A cot in any town's clinic: rest for nothing, or buy a medkit for the road |
| `L` | Turn the optics dial: lamp, then any printed visor, then off |
| `F` | Toggle walking and flying |
| `F3` | The debug readout: FPS, position, chunks and triangles, journal tick, fleet and fuel, hostiles and their belief, your dose, your wells, how roused the deep is, your standings — diagnostics only, in every build, and it changes nothing it reports on |
| `F5` | Save |
| `F10` | The gold panel — the operator's console (dev builds only, see below) |
| `Escape` | Release the mouse |

Worlds are saved on quit to `$XDG_DATA_HOME/gamingg/saves/world`, selectable
with `--world <name>`. Reloading an existing world uses its stored seed, so
`--seed` only applies when creating a new one.

### The arsenal

The shop counter sells a compact slug launcher and boxes of slugs. It is
heavy (it rides your pack weight), loud (every sound in the game is
synthesized — a machine with no audio device simply plays nothing), and the
town treats it accordingly. The first shot inside a town's line gets you one
warning, once, per town. After that the rules are the rules: property you
break costs bounty scaled by how many people saw it; pointing the muzzle at
somebody panics them — each one either runs home or runs for the security
office, and an alarm that reaches the office is a signed report against you.
Caravans can be shot down mid-flight: the cargo drops where it fell and can
be salvaged, and the network logs the loss against your name whether or not
anyone watched — the manifest is its own witness. Every shot is journalled
with its muzzle and aim, so a firefight replays crater-for-crater under
`--replay`.

### The kestrel and the roost

The shop sells a palm-sized scout drone that rides your pack. It flies on a
small cell — endurance and cooldown are one budget, and the recharge costs
what the flight spent — and takes standing orders from the handheld's third
page: circle overhead, fly a sortie where you're looking, perch as a sentry
(a quarter of the drain), hold ahead of you, or come home. What it sees gets
a mark: a report of where a person or machine *was*, on both maps and over
the spot itself, fading after thirty seconds unless re-sighted. It reveals
contacts, never terrain — cartography stays the flier's paid trade.

The same machine sits in a box on the security office roof. Breach a lock or
fire a shot in town and it pops out, flies to the noise, and watches. The
first time it sees you, you are *observed* — the drone overhead is the
warning. Anything you do while observed is witnessed, on the same bounty
arithmetic as every other pair of eyes. It can be evaded (break its line of
sight until the mark fades), outlasted (its battery dies like yours, and
while it recharges the town is blind), or shot down — which is property
damage in front of the best witness in town. Cover now works in two
directions: walls hide you from people, roofs and canopy hide you from the
sky, and both are plain geometry.

### The fabricator

Everything you break is stock now. Every block yields itself onto the fleet's
base pile — the same pile the flier ferries into and the shop sells out of —
so the drill finally produces something, and a hillside is raw material.

Buy a fabricator at the counter, place it wherever you like (`8` on the belt,
broken to move it), and press `E` at it. It takes raw goods off the pile and
turns them into whatever the catalogue has a row for: slugs for the launcher,
copper bars, planks, metal wall panels, a charged cell that puts the kestrel
back in the air immediately, a spoofer coil, and — at the top of the ladder —
a kestrel or a ground drone. Buying a machine with credits and printing one
out of copper are two routes to the same machine: the counter is for people
with money, the fabricator is for people with a mine.

Materials come off the pile the moment you start, so a print is a decision
rather than a queue, and the Fabrication skill buys speed and unlocks the
harder patterns. Nothing here costs credits.

### The world below

The terrain used to be a height field: one surface height per column, solid
rock all the way down. Caves are the first genuinely three-dimensional thing
worldgen has ever done — winding tunnel galleries where two 3D noise fields
run near zero together, and rarer chambers deeper down, all of it a pure
function of the seed and the block position so chunks still generate in
parallel and a saved world still regenerates identically.

Tunnels thin as they approach the surface, so mouths exist — a hole in a
hillside you can walk into — but stay scarce enough to be worth marking on
the map. Nothing opens under a town, nothing opens into the sea, and the
bedrock floor keeps a margin of rock below the deepest gallery. Where a
tunnel cuts through an ore body it carves the vein with it, which leaves
copper showing in the cut faces: a cave is where hand-mining is pleasant,
ore at the surface of a wall instead of under twenty blocks of overburden.

Drones already cope: a machine over a void settles to the cave floor, and a
mine plan that meets a cavity simply finds part of its digging already done.

### Lights in the dark

The world below is genuinely dark now. Every face bakes how much sky its
column can see, so daylight dies a few blocks under a roof and is gone twenty
under rock — house interiors read as shade, caves as night. Machines and
people darken with the ground they stand on.

Against that: the suit's hand lamp, on `L` — a warm cone thrown from wherever
you are looking, short and honest. Everything better comes off the
fabricator, not the counter: a **high beam** that throws nearly twice as far,
a **night vision visor** that amplifies what little light there is into green
geometry, and a **thermal visor** that ignores light entirely and paints
warm machinery and warm bodies against cold rock. One key cycles the dial
through what you own; the HUD names what you are looking through.

### The terminal

Press `T` and a console opens. It is the first place in this game you type,
and it answers questions the panels never could — `status`, `fleet`, `where`,
`pile`, `bank` — and takes a few orders besides: `dig`, `cancel`, `survey`,
`lights`, `scout sortie -40 120`. `help` lists the lot, and anything it does
not know it refuses by name rather than by silence.

The real reason it exists is the scrollback. Every toast the game shows you
lasts three seconds and then the thing it said is gone, which is fine for "you
levelled up" and useless for "the crew ran dry while you were down a cave".
The terminal keeps four hundred lines of them, so the message you missed is
still there when you come up.

Nothing it does is new. An order typed at the prompt goes out through the very
same call the keys use, so the journal only ever sees one kind of dispatch —
a console that recorded its own orders would be a second implementation of
every rule in this game, and the first one to drift.

### The townsfolk

Every town's three villagers have names now, and lives to go with them. WRENA
THE ASSAYER is at her workplace through working hours, on the square from ten
to four on the town's own market day, home before eight — and always with her
own clock, twenty minutes fast or slow of the town's, so nobody moves in
lockstep. Where a person stands at any hour is a pure function of who they
are and what day it is: no simulation runs until you look, and a stakeout
tonight tells you where the sheriff will be tomorrow.

Walk up and press `E` (or type `talk`) and they say something *true*: the ore
price in their line is the market's live price, the bounty they warn you
about is the board's real number, and "the fleet is dry" is your tank's own
flag. `who` lists the roster — name, temperament, how well they know you, and
where they are right now.

Friendship is a ledger. Gifts score by their tables — two loved goods and one
hated per person, derived from their trade, authored for the hometown trio —
with two gifts a week counted, birthdays tripled, and a first chat each day
worth a little. Get witnessed breaching a lock and everyone in that town
holds it against you, scaled off the same bill the sheriff charged. The tiers
open things that already exist: market talk at Acquainted, a bearing to a
bunker at Trusted (stage 19's loot loop, fed by talking), and at Close a key
to that person's own door — granted through the permit system, not around it.

`Talk` is journalled and replays as a no-op, because what talking moves lives
in its own file, like permits grants. `gift <good>` is not: the good comes
off the base pile on both sides of the oracle, and only what it *earned*
stays outside the hash.

### Walls, and what they are for

Towns build what they can justify. A hamlet stays open; a middling town raises
a plain ring; the big ones earn a **bastioned trace** — the star-fort geometry
that exists because cannon exist. Low, thick, angled walls with no dead
ground: every face covered by the guns of another face, which is what the star
shape *is*. Four bastions on a working fort, six with deep re-entrant angles
on a county seat.

The trace is a radius that varies with angle, so "how far is this column from
the wall" is one cheap subtraction — and wall, walkway, parapet and ditch are
just bands of that one number. Gates sit on the four cardinal axes because
that is where the roads already ran, and each gate carries a lockbox like any
other door. About a quarter of walled towns have let theirs go: a deterministic
pass drops whole segments, so breaches exist to find. A perfect wall is a worse
story than a broken one.

The wall rides the terrain rather than sitting at the plaza's level — out at
the trace the town's plateau has already blended most of the way back to
natural ground, and a wall pinned to the square would hang in the air on the
downhill side.

### Foundations

Buildings are founded now, the way real ones are: a **deep strip** under
every load-bearing wall and a **shallow slab** under the floor between. What
sets the depth is not how tall the building is but what it is protecting — a
shed runs two courses, the radio tower four, the bank's strongroom five. The
open plaza runs none, because paving is a surface rather than a structure.

Footings are poured in the same fortified stuff a bunker's shell is made of:
four hundred times the hardness of stone. That is not a wall — the number is
finite on purpose, so a determined player with a good drill can always get
through — but it prices undermining honestly, at about an afternoon a block.

This is the thing that makes every lock in the game mean something. Before it,
the way past a Tier Three vault door was a hole in the floor. A building's
claim now reaches the bottom of its own foundation too, so digging under one
is a permit crime as well as a long day. Star-fort curtains are founded the
same way, along their whole circuit — including under the gateways and under
the segments that have fallen down, so a ruin leaves its foundation in the
ground where the wall used to be.

### The bank, and the vault

Every town has a bank now, and it carries the heaviest lock in the game — the
Tier Three lockbox that has existed since stage 11 and never had anything
worth putting behind it. Inside is a **vault**: a strongroom that holds goods
for you, town by town.

Two things that solves. A trade run used to mean carrying everything and
selling it in one lump; now you can leave a load in the town you mean to sell
it in and come back when the price is right. And anything you cannot afford to
lose can sit somewhere with a door on it instead of in a container in a field.

Taking a strongroom door off its hinges carries the **maximum bounty in the
game** — several times what any other crime costs, and enough on its own to
put you over the warrant threshold before you have carried anything out of the
building. Every other crime here is against one person or one machine; a bank
holds what a whole town left with it. Picking the lock quietly still costs the
quiet price, which is the whole shape of the permits system — though Security
60 is its own toll.

Each town's books are separate — goods left in Stonehaven are in Stonehaven —
and the vault holds six thousand units. Unlike almost everything else in this
world its contents are not derived from anything: they are exactly what
somebody put there, which is the whole point of a bank.

### The fuel loop

Machines used to run forever on nothing, which made every cost in the game a
one-off: buy the drone, and it digs until the sun burns out. Now the fleet
burns **oxyhydrogen** — water taken apart into the two gases it is made of,
two parts hydrogen to one of oxygen, burned back into water in the cutting
gear. A canister runs one machine for five minutes; a crew of four gets
through one in a minute and a quarter. When the pile is empty the crew stops
where it stands and the HUD says so in red.

You make it yourself. Buy an **electrolyser**, and then find somewhere to put
it: it only works within two blocks of water, which is the first machine in
this game whose *position* decides whether it works at all, and which turns a
lake shore from scenery into somewhere worth building. Water is free. What is
not free is the copper you feed it as electrodes, and the minutes it takes.
Longer runs are cheaper by the canister.

The towns are in the same business. A depot banks cells and sells them; a mine
and a refinery burn through them; so oxyhydrogen is one more good on a network
that already knows how to make shortages — and it is the first shortage that
can stop you outright rather than just cost you money.

### The world below, and what is in it

Bunkers are buried out on their own lattice, far from any town: a concrete
stair head cut into a hillside, and behind it somebody's works. Every one is
laid out by a proportion system drawn from its own site hash — a golden-ratio
**coil** whose rooms shrink as the plan turns inward, a √2 **grid** that halves
like a barracks, or a √3 **hive** that branches three ways — and every room
size is a Fibonacci number, because those are the whole numbers that read as
golden on a block grid. The result is that no two bunkers in a world share a
layout, and none of it is stored: a bunker three kilometres away costs nothing
until you walk to it.

Each one faces its own way, too. Bunker *k* takes a bearing of *k* × 137.507°
— the golden angle, the irrational rotation — so no two hatches on a ridge
line point the same direction and the sequence never repeats, world-wide,
without a single byte written down.

The shell is the gate, and it is one number rather than a new mechanic:
bunker concrete has a hardness of 400 against stone's 1, so cutting in through
the wall is possible, and almost never worth it. The way in is the stair. Down
there are supply caches — the first goods in this game you cannot buy at any
price and cannot mine at any depth. What a crate holds is derived from where it
stands, so two visits agree; that it was opened is remembered by the crate not
being there any more.

### Hacking through machines

Once Security 10, the counter stocks spoofer coils. Fitted to a machine, a
coil lets that machine do the lock work while you stand somewhere else: a
light coil rides the kestrel and opens houses and shops, a heavy one rides a
real airframe and opens anything. The Security floors from the permits round
still apply — hardware says *where* the work can happen, your skill says
*what* work is possible, and neither substitutes for the other.

What you have bought is distance from the scene, never from the consequence.
The machine at the lock is watched by the same eyes a body would be, and if
it is spotted the bounty lands on your name. Worse: a machine working
unattended gets seized, and signing it out of the pound costs a fee at the
counter. One you are personally flying can at least be flown away. And the
link is a leash — stay within 120 m of your machine and the machine within
reach of the target, or the job stops where it stands.

The town's watch box is a target too, and there are three things to do to
it. **Blind** it and it stands down — though the town notices a dark box
soonest. **Silence** it and it flies its patrols, sees everything and files
nothing, which nobody notices until an offence goes strangely unpunished.
**Tap** it and its sightings mirror onto your handheld: the sheriff's eyes,
exactly, never better ones. Each grade wants more Security than the last, and
the tap needs a heavy coil. If subtlety is not your line, the box is still a
block — drill it out and the town is blind until it is re-boxed, at the same
price every other loud thing costs.

You can buy the same box for your own roof, at the price the sheriff paid. It
does not fly, because it has nothing to answer; it just watches your yard and
files what it sees to you.

### The gold panel

Development builds carry an operator's console behind the repo's first cargo
feature, `gold` (on by default; the `dist/` build is compiled with
`--no-default-features` and does not contain it). `F10` opens it, or launch
with `--gold` to start with it open. Five tabs — Player, Spawn, Town, World,
Tuning — driven Deck-shaped: `Tab` cycles tabs, arrows move the cursor and
adjust a held slider, `Enter` acts and commits, `X` resets a tunable to its
shipped default. Every mutation it makes is an ordinary journaled order
(`Command::Admin`), so a cheated session still replays to the same world hash
— a cheat is an order like any other. It is a keyboard console today; the
gamepad backend waits for real hardware.

Running windowed needs a Vulkan loader and drivers plus X11 and/or Wayland
client libraries.

### Rendering without a display

`--screenshot` renders one frame offscreen and exits, so it works over SSH and
in CI:

```sh
cargo run --release -p vx-app -- --screenshot frame.ppm --width 640 --height 360

# look at a specific place, which is how ore outcrops get eyeballed
cargo run --release -p vx-app -- --screenshot ore.ppm --at 146,30

# find the nearest outcrop, mine it out, and photograph the workings
cargo run --release -p vx-app -- --screenshot mine.ppm --at 146,30 --dig auto
cargo run --release -p vx-app -- --screenshot pit.ppm --at 146,30 --dig pit

# same, but framed close on the digger rig itself
cargo run --release -p vx-app -- --screenshot rig.ppm --at 146,30 --dig auto --close

# the starting village (identical in every world), villagers included
cargo run --release -p vx-app -- --screenshot village.ppm --at 0,0

# the same view with the supply shop panel open over it
cargo run --release -p vx-app -- --screenshot shop.ppm --at 0,4 --shop

# over the shoulder, with the player's body in shot
cargo run --release -p vx-app -- --screenshot third.ppm --at 0,10 --third-person

# the container town at dawn, mast against the sky (--night, --noon, --dusk too)
cargo run --release -p vx-app -- --screenshot town.ppm --at 0,22 --dawn

# from inside the roomiest cave pocket near a spot, facing down the gallery
cargo run --release -p vx-app -- --screenshot cave.ppm --at -244,-14 --cave

# the terminal, with a session's worth of log in it
cargo run --release -p vx-app -- --screenshot term.ppm --at 0,10 --terminal

# the nearest walled town, from above its own trace
cargo run --release -p vx-app -- --screenshot fort.ppm --at 1258,-148 --fort

# an excavation beside the bank, its footing in section
cargo run --release -p vx-app -- --screenshot footing.ppm --at 0,0 --footing

# a town's vault, ledger open
cargo run --release -p vx-app -- --screenshot vault.ppm --at 0,0 --vault

# an electrolyser on the nearest shore, panel open mid-run
cargo run --release -p vx-app -- --screenshot hho.ppm --at 200,200 --hho

# the nearest bunker to a spot: its hatch from outside, its works from within
cargo run --release -p vx-app -- --screenshot hatch.ppm --at 2026,364 --bunker
cargo run --release -p vx-app -- --screenshot works.ppm --at 2026,364 --bunker --close --optic lamp

# the same gallery by hand lamp, night vision or thermal
cargo run --release -p vx-app -- --screenshot lamp.ppm --at -244,-14 --cave --optic lamp
cargo run --release -p vx-app -- --screenshot nvg.ppm --at -244,-14 --cave --optic nvg
cargo run --release -p vx-app -- --screenshot heat.ppm --at 0,8 --time 0.95 --optic thermal

# a fabricator standing in frame with its panel open mid-print
cargo run --release -p vx-app -- --screenshot fab.ppm --at 13,9 --fab

# the beacon console, work posted and one contract already taken
cargo run --release -p vx-app -- --screenshot board.ppm --at 0,4 --board

# the console with its trade map: a pin in the dark and a bearing under it
cargo run --release -p vx-app -- --screenshot console.ppm --at 0,4 --board --height 620

# the handheld's map page
cargo run --release -p vx-app -- --screenshot handheld.ppm --at 0,10 --handheld-map

# the beacon console, now with the town's prices and what it is short of
cargo run --release -p vx-app -- --screenshot market.ppm --at 0,4 --board

# rebuild a saved world from its journal and check it against the ground on disk
cargo run --release -p vx-app -- --replay --world myworld

# look down a machine's gimbal, and at where the gimbal hangs
cargo run --release -p vx-app -- --screenshot gimbal.ppm --gimbal --at 0,10

# play a whole loop with nobody at the keyboard — out of the house, out to the
# ore, cut it, home, sold — and photograph all four beats. Writes
# play-01-door.ppm, play-02-out.ppm, play-03-ore.ppm and play-04-counter.ppm
cargo run --release -p vx-app -- --screenshot play.ppm --play --seed 2024 --at 146,30

# the whole loop through a save and out to a *different* town: scout with the
# flier, cut what it found, walk home laden, save and reload, then haul it over
# the hills and sell it at a counter that is not yours. Writes haul-01-ping.ppm,
# haul-02-cut.ppm, haul-03-home.ppm, haul-04-reloaded.ppm and haul-05-sold.ppm
cargo run --release -p vx-app -- --screenshot haul.ppm --haul --seed 2024 --at 146,30

# the crew loop, played and measured: earn a drone by hand, dispatch it, save
# mid-dig and reload with it still cutting, sell up and buy a second, and print
# the books. Writes payroll-01-byhand.ppm through payroll-04-books.ppm
cargo run --release -p vx-app -- --screenshot payroll.ppm --payroll --seed 2024 --at 146,30

# the pack: cut a real adit until it will hold no more, leave what would not
# fit on the floor of the drift, tip the lot into a container and walk the
# drops back up. Writes pack-01-dropped.ppm through pack-04-adit.ppm
cargo run --release -p vx-app -- --screenshot pack.ppm --pack --seed 2024 --at 146,30

# put a bigger crew on the next dispatch
cargo run --release -p vx-app -- --drones 8

# a gallery cut into a lake, and the same gallery once it has filled
cargo run --release -p vx-app -- --screenshot cut.ppm --at 0,10 --flood cut
cargo run --release -p vx-app -- --screenshot level.ppm --at 0,10 --flood level

# a tree mid-fall, and the logs it left
cargo run --release -p vx-app -- --screenshot swing.ppm --at 0,10 --fell swing
cargo run --release -p vx-app -- --screenshot logs.ppm --at 0,10 --fell down

# rain over the country, at a tick the seed actually rains on
cargo run --release -p vx-app -- --screenshot storm.ppm --at 0,10 --storm

# a stand alight running upslope, and the same ground once it has come back
cargo run --release -p vx-app -- --screenshot fire.ppm --at 0,10 --fire burning
cargo run --release -p vx-app -- --screenshot after.ppm --at 0,10 --fire after

# the beacon console with its offices, and the same console with a warrant on it
cargo run --release -p vx-app -- --screenshot town.ppm --at 0,10 --town
cargo run --release -p vx-app -- --screenshot warrant.ppm --at 0,10 --warrant

# the console's ballot page, and the ballot page of a town that elected you
cargo run --release -p vx-app -- --screenshot ballot.ppm --at 0,10 --ballot
cargo run --release -p vx-app -- --screenshot elected.ppm --at 0,10 --elected

# a stand of each forest, found rather than hard-coded
cargo run --release -p vx-app -- --screenshot cove.ppm --at 0,10 --forest cove
cargo run --release -p vx-app -- --screenshot high.ppm --at 0,10 --forest high
cargo run --release -p vx-app -- --screenshot bog.ppm --at 0,10 --forest bog
cargo run --release -p vx-app -- --screenshot mats.ppm --at 0,10 --forest treeline

# the same cove at three points in the year — same seed, same camera, only the
# tick differs, so the leaves and the sky are the whole of the difference
cargo run --release -p vx-app -- --screenshot summer.ppm --at 0,10 --forest cove --season summer --time 0.5
cargo run --release -p vx-app -- --screenshot autumn.ppm --at 0,10 --forest cove --season autumn --time 0.5
cargo run --release -p vx-app -- --screenshot winter.ppm --at 0,10 --forest cove --season winter --time 0.5

# three thousand kilometres from spawn, with the hand lamp on at dusk
cargo run --release -p vx-app -- --screenshot far.ppm --at 3000000,3000000 --time 0.5
cargo run --release -p vx-app -- --screenshot far-lamp.ppm --at 3000000,3000000 --time 0.85 --optic lamp

# a town founded at --at (or the first open plot east of it), framed from its wall,
# and the console of a town whose chairs you seized
cargo run --release -p vx-app -- --screenshot founded.ppm --at 600,40 --founded --time 0.5
cargo run --release -p vx-app -- --screenshot frost.ppm --at 0,10 --frost --time 0.5
cargo run --release -p vx-app -- --screenshot thaw.ppm --at 0,10 --thaw --time 0.5
cargo run --release -p vx-app -- --screenshot far-posse.ppm --at 3000000,3000000 --posse --close --time 0.5
cargo run --release -p vx-app -- --screenshot taken.ppm --at 0,10 --taken

# the pocket arcade, mid-fight, on the raised handheld
cargo run --release -p vx-app -- --screenshot arcade.ppm --at 0,10 --arcade

# the handheld's fleet roster and a live feed banner
cargo run --release -p vx-app -- --screenshot uplink.ppm --at 0,10 --device
```

`--dig` runs a whole excavation headlessly against generated terrain — then
sweeps the surrounding sector with the flier and draws the pings, the aircraft
and the minimap into the frame. It is both a screenshot tool and the fastest
way to see whether a change still produces a mine a drone can drive and a
survey the scanner can fly.

The render tests do the same thing and assert on the pixels. Both run against a
software Vulkan driver, so no GPU is required:

```sh
sudo apt-get install mesa-vulkan-drivers
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo test --workspace
```

Tests skip themselves rather than failing when no Vulkan adapter exists at all.

## Licence

MIT OR Apache-2.0.
