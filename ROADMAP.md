# gamingg — roadmap

Where this project has got to, and where it is going. Written to be picked up
cold: if you have never seen the repository, start here, then read `README.md`
for how to build and run it.

**What it is.** An original voxel engine and game in Rust, Linux-first, aimed
at a commercial release with mods distributed through Steam Workshop. A
frontier mining game: you prospect, you send machines into the ground, you haul
what they find back to town and sell it.

**Branch.** All work lands on `claude/custom-mcp940-build-18pfok`.

---

## House rules

Three invariants constrain every stage. Most of the hard bugs so far have been
violations of one of them.

1. **Worldgen is a pure function of `(seed, position)`.** No sequence state, no
   global RNG. Chunks generate in parallel in any order and a saved world
   regenerates identically — which is also why saves only store chunks a player
   has *changed*. It is why the flying drone's surveys, the ore lattice, the
   forest and (as of stage 8) the map of towns can all be *derived* rather than
   stored.
2. **Agents are bit-identical given the same inputs.** Villager strolls,
   perception and drone routes come from hashing, never from wall-clock time or
   an RNG the caller cannot reproduce. Anything time-dependent takes the time
   as a parameter.
3. **No wall-clock reads inside render paths.** The renderer draws what it was
   explicitly pushed. That is what keeps the headless captures reproducible and
   the pixel-equality tests honest.

Two more that matter at the seams:

- **Persisted data is keyed by name, never by numeric id.** Block ids are
  assigned in registration order, so a mod shifts them; a save keyed on numbers
  would silently reload as the wrong blocks. Same reasoning drives name-keyed
  skills, stockpiles and upgrades.
- **Crate boundaries hold.** `vx-world` knows nothing of rendering. `vx-agent`
  knows nothing of rendering *or* game fiction — no quests, no economy. Game
  fiction lives in `vx-app`.

## Licensing constraints

Written down because they are easy to forget and expensive to get wrong.

- **This is original code.** No Minecraft source, no decompiled game code, no
  Mojang assets. It is not a fork of `mcp940` or any decompiled-source
  repository — redistributing decompiled game code violates both the MCP
  licence and the Minecraft EULA. The 1.12-era conventions borrowed (16×16
  chunk columns, 256-block height, ~1 m voxels) are design reference points.
- **Motherload is a *vibe*, not a source.** The digger rig deliberately evokes
  the classic dig-and-sell flash games. Every pixel is procedurally generated
  here; no sprites, names or trade dress.
- **The handheld drill has no brand.** The tunnel-boring company whose
  silhouette it evokes is a live trademark. In game it is "a compact boring
  drill" and it stays that way.
- **The pocket arcade will never be a Doom port.** WAD assets are not
  redistributable and a GPL engine would encumber the binary. When that stage
  arrives it is an original mini-FPS.
- **RuneScape lent the *shape* of the XP curve**, not its numbers. Curves are
  not copyrightable; names are, and ours are generic.

---

## Shipped

| Stage | Commit | What landed |
|---|---|---|
| 1 | `9deecc5` | Core scaffold |
| 2 | `bc9a8ef` | Renderer and a playable binary |
| 2.x | `e3e4ad7` | Block editing, physics, persistence |
| 2.5 | `4a88348` | Spline terrain, parallel meshing, frustum culling |
| 3 | `c2c8c6a` | Ore geology |
| 4 | `def13db` | Instanced objects, drone swarm, three mining methods |
| 4.5 | `39e9cc7` | Eight review fixes |
| 5 | `6834e8a` | Flying drone, sector scanning, ferry loop, minimap |
| 6 | `2d263c2` | Rigs, handheld drill, skills, bitmap-font HUD |
| 6.5 | `229a299` | Starting village, villagers, trading shop, trees |
| 7 | `42fb8ab` | Third person, drone piloting, NPC senses |
| 8 | `f050c16` | Day/night, container towns on a lattice, the beacon network |
| 9a | `b77aedc` | Residency pinning, content hashes, the command journal, seed tree, body ids, a real crew, 8-byte packed quads |
| 9b | `dbfe091` | Town books, moving prices, inter-town freight, player trade runs, copper bars |
| 10a | `63da370` | Machines cost credits, trade map on the console, handheld map page |
| 10b | `b12b1e2` | Player movement on a fixed clock: stance, sprint, slide, vault, mantle, stamina, carried mass |
| 10c | `67aa3a6` | Pregenerated spawn, streaming off the frame thread, the player's house, mail-order, the welcome panel |
| 11 | `289a143` | Permits: ranked claims, three grades of lockbox, witnesses and sneaking, bounty, breaching and lock-picking |
| 12 | `bbe6756` | The gold panel: journaled admin orders, live tuning, the operator's console compiled out of shipped builds |
| 13 | `4f22cd4` | The arsenal: the slug launcher, synthesized sound, recoil and shake, town warnings, witnessed property bounty, panic, caravan interception |
| 14 | `6b56c46` | The kestrel and the roost: a pack scout with standing orders and decaying contact marks, and the town's own watcher on the security office roof |
| 15 | `7e8d4f1` | Hacking through machines: spoofer coils, drone-borne intrusion on a leash, the watch box blinded, silenced or tapped, the impound, and a watch box for your own roof |
| 16 | `75bbb1b` | The fabricator: every block is stock, and a printer that turns raw material into ammunition, cells, building goods, modules and whole machines |
| 17 | `1643f59` | Caves: the first true 3D carve — tunnel galleries and deep chambers, pure in the seed, mouths in hillsides, ore showing in the cut faces |
| 18 | `8bbc730` | Lights in the dark: baked skylight makes the world below genuinely black, the suit's hand lamp cuts a warm cone through it, and the fabricator prints a high beam, night vision and thermal |
| 19 | `94f1ed1` | Bunkers: sacred-geometry layouts on their own lattice — three proportion systems, golden BSP on a Fibonacci vocabulary, golden-angle bearings, a 400-hardness shell, and supply caches to strip |
| 20 | `60b53e8` | The fuel loop: the fleet burns oxyhydrogen, an electrolyser on a shore splits water into it, and HHO joins the trade network as the first shortage that can stop you |
| 21 | `38024dc` | Star forts and banks: bastioned traces per town tier with gates, ditches and deterministic breaches, a strongroom in every town behind the first Tier Three lock ever stamped, and foundations under everything |
| 22 | `9148a77` | The terminal: the font's third user — typed commands, a caret and history, and four hundred lines of scrollback the toasts also land in |
| 23 | `58bc6ac` | The townsfolk: names, trades and temperaments per person, pure schedules with market days, a friendship ledger with gift tables and tier unlocks, and speech templated over the live simulation |
| 24 | `89aa6b3` | The controller: native gamepad play synthesized into the keyboard and mouse seams, a SELECT-key control scheme overlay, and a one-command Steam Deck installer |
| 25 | `c235ac7` | The workshop: upgrades printable in materials as well as bought in credits, three new lines (pack, press, lamp), and the rule that an upgrade may not change arithmetic the journal re-runs |
| 26 | `23d8bea` | Wear and recovery: machines age by the tick they work, the worst one sets the crew's pace, and spare parts printed at the workshop mend them — the first machine state that had to live inside the replayed simulation |
| 27 | `6a2eeef` | Micro-on-damage: a block gains a 4³ interior only when violence touches it, one `u64` a wound, so walls chew where they are shot, rays pass through the holes and feet never do |
| 28 | `4665d04` | Hostiles and health: a warrant sends deputies who believe rather than know, search an occupancy map, take cover, never fire through each other, and break or surrender on nerve |
| 29 | `12425b4` | Bunkers occupied: every shelter holds a squad derived from its own seed, hostiles route on the drones' flow fields, noise reaches them as 32-metre zones, pairs move under overwatch, and a surrendered holder can be taken in for the board's pay |
| 30 | `a4248b3` | Factions: the Compact and the Holdouts remember what the bounty board forgets — standing shades prices, buys a challenge before a volley, and puts the spoofers in the other side's hands |
| 31 | `02ac7c9` | Uranium, oil and gas, and the thing that hunts by ear: a deep ore that pays for itself in dose, fluid reservoirs worked by a wellhead that keeps paying until it stops, and a stalker that is told a thirty-two metre cell and has to find you inside it |
| 32 | `8c6d21a` | Faces, walls and a ward: the townsfolk get eyes that follow what they are watching and a grunt for anyone who crowds them, every town on the frontier walls itself with at least a mini star, and a free cot in every clinic mends you and scrubs the dose |
| 33 | `3b29a4d` | The handheld you hold: the fleet uplink becomes a cased unit that swings up into your hands with its screen coming on, the readout is projected onto the model's own glass through the frame's camera matrix, and your drill is away while it is up |
| 34 | `7cd3c26` | The pocket arcade: a cartridge printed at the fabricator turns the handheld into a games machine — an original corridor shooter, every wall and every pixel of it computed, floors that loop meaner, and a record the cabinet keeps |
| 35 | `b3ae393` | Three forests: every column belongs to a peat bog, a mixed hardwood cove or subalpine conifer, decided by how high and how wet the ground is — with emergent giants over the cove, krummholz mats at the treeline, bare rock above them and sphagnum underfoot in the lows |
| 36 | `98a0a7c` | Felling: cut low into a trunk and you are cutting a notch on its own cross-section, not chipping a block — past the notch and down to the hinge it goes over, toward the side you cut from, on a kinematic arc that flattens what it lands on, takes its neighbours with it and lies down as logs |
| 37 | `2b61043` | Water that moves: a wet block carries its fill in the same sixty-four cells a wounded block carries its damage, so cutting into a lake floods the gallery, an inland pool drains by exactly what ran out of it, and a printed pump lifts water over its own head |
| 38 | `14acc37` | Weather, fire and what comes back: the sky is a pure function of the seed and the tick, so a storm crosses the country the same way twice — it rains, the hollows fill, lightning takes the tall and the lonely, and about one strike in fifty lights a fire that runs uphill and downwind through anything wooden, your house included, until it meets an ancient grove or wet ground; then the burnt cell remembers, and comes back through meadow, thicket and mixed stand to the forest it was |
| 39 | `ef06f2b` | Towns that work: every town has a mayor and a sheriff who are people off its own roster, its residents put goods on its books and credits in their pockets on the hours the schedule already gave them, trading with somebody buys enough trust to be handed a key to their door — and a bounty is no longer a posse, because the sheriff has to get a warrant from a mayor with an opinion of you, with a fine and a closed counter on the way there |
| 40 | `4cdda7c` | The ballot box: a town's seats stop being permanent — it votes on its own market day, once a term, and the residents cast on the business you have put across their counters, so you can put your name in at the beacon console and be elected sheriff or mayor; a badge now belongs to the town that gave it rather than to the whole frontier, and a mayor cannot sign his own warrant, so paper on a town you run goes up the road to the next mayor along |
| 41 | `4856472` | Seasons: the twenty-eight day year the roster already counted for birthdays now runs the sky and the woods — a week each of spring, summer, autumn and winter, one season to an election term. The leaves turn gold and go bare and come back, the sward goes to straw, the spruce barely notice and the sky goes with them; **summer is a fire season** where the ground stays tinder days after a shower and lightning actually bites, winter is sodden and nothing will light; and the woods **stop growing over the winter**, so a stand you burn in October is still black in February and only starts coming back in the spring |
| 42 | `7beb30b` | The floating origin: the renderer draws relative to the chunk the camera stands in — the camera's corner and every chunk's corner are exact integers, subtracted in the vertex shader before anything reaches a float multiply — and the player's body, its physics, the camera and the aim ray move to `f64`, so the millimetre skin the collision sweep runs on means what it says everywhere. Proved the only way it can be: the same scene renders byte-identical sixteen hundred kilometres out, and the same journal walks the same walk at spawn and at three thousand |
| 43 | `2c2df23` | Founding a town, and taking one — the civic note's other two routes into office. Print a charter, stand on flat dry ground a lattice cell clear of anybody, and `FOUND IRON REACH`: the whole town is raised out of the ground by the same generator that draws every other one — plateau, tower, shed, clinic, bank, dwellings, lockboxes, the mini star, three settlers with the market already open — with you in both chairs and a founder's due that keeps you there through the quiet first terms. Or get a warrant, break the posse it sends, and `TAKE` at the console: both chairs are yours, the Compact hates it, every neighbouring mayor signs paper on you and the residents trust you nothing, so the town votes on you at its next poll and a town you took is a town you have to keep |
| 44 | `c90ecea` | Snow that settles, and ice — the half of a winter stage 41 deliberately left out, because it is the half that edits blocks. Four cold blocks: snowed grass, sand and sphagnum, each the bare block's twin at the same height, and ice, a solid translucent block. The cold is named in one place (`FREEZING`, a threshold on a temperature the place mostly decides and the year leans on), and a frost automaton shaped like the rain's source term — sixteen hashed columns within twenty-four blocks of the player every sixty-four ticks — snows open ground when it is snowing, freezes full still water at or below sea level when it is freezing, and gives both back exactly when it is not. It runs inside `Command::Advance` on both sides of the journal, so a winter replays to the same snow and the same ice. Everything else is an existing system reading a new block: you and the drones and the deputies walk across the lake, a pump on it lifts nothing, an electrolyser beside it finds no water, and fire finds no fuel |
| 45 | `0130222` | Entities in `f64` — the line stage 42 drew at the things that travel, moved out to everything that carries a position. Villagers, deputies, holders, the stalker, shots, sweeps, falling stems, marks, sightings, the belief board, caravans and crashes all run at the body's width; directions, headings, ranges and rig geometry stay `f32` and the difference is taken wide first. Every body is drawn in the camera's frame and marked so, the per-object lighting pass adds the origin back before reading a column, the muzzle crosses the journal's wire at `f64` (VERSION 29) and `arsenal.dat` widens its crash columns (VERSION 2, v1 read). Proved by the stage-42 tests' siblings: the same posse walks the same walk, the same townsfolk stroll the same stroll, the same rounds leave the same craters, and the same rig draws the same bytes, at spawn and three thousand kilometres out |
| 46a | `63e69ce` | The pad is the game — every control reachable from a controller and every panel escapable from one. Stage 24's single bit of context becomes five layers: the world, a held `LB` for the block palette, a held `SELECT` for the second layer, a panel, and — the one that did not exist — looking through a machine, where the pad had been in *world* context with no button that hung up. About twenty bindings that no button could reach get homes, the sticks drive panel cursors and the arcade, the triggers stop digging through an open panel, and a table-driven test walks every `KeyCode` the game binds and fails the build if the pad cannot produce it |
| 46b | `a1a0292` | The keyboard you never need, and the stick that creeps. Typing arrives on a window's text event, which `poll_pad` cannot raise, so no binding of any button to any key could ever produce a character — the pad gets a grid and a cursor instead, and what it picks goes into the same `type_char` the window's text lands in. And `MoveCommand` gains a quantised throttle byte, so the stick's *lean* reaches the simulation and is replayed with it: a gentle push is a gentle walk. Journal VERSION 30 |
| 47 | `2e114eb` | The body does not pass through the world. Every collision test in the game is a *binary* predicate — inside a block or not — and none of them ever said how much room was left, so the millimetre skin the sweep promises was a promise nothing checked. Three places were not keeping it: a mantle walked the hull along a fixed arc with **no collision query at all** and only its destination validated, the follow camera's minimum-orbit floor overruled a wall and put the lens a quarter block inside it, and standing up under a ceiling asked only whether the taller hull collides — which, thanks to the inset in the block query, a head resting exactly on the plane does not. All three fixed, the block query made symmetric at both ends, and the margin family of tests that never existed written: the hull rests *exactly* `SKIN` off a wall and a floor, a mantle is outside the rock on every tick of the climb, and the orbit keeps its whole skin at every angle inside a three-block room |
| 48 | `ee5249a` | The loop you can actually play. Asked to play a round — leave, collect, trade — and found that nothing could: every verb in the loop is a method on `App`, which owns a window, and `main.rs` has no tests and `vx-app` no library target, so the walk, the drill and the counter had never once been joined up. The drill's arithmetic comes out into `drill.rs` and a `Session` plays the game headlessly through the game's own functions — and the join turns out to be where the bug was: a block mined with no base container declared **evaporated in silence**, the only system in the game that produced goods and never said where they went. Played end to end on the shipped seed: out the door, 170 blocks to the copper outcrop, sixteen blocks cut at two seconds each, home slower than you left because the pile is the weight, and 224 credits over the counter — with four captures, and three walking bugs found by walking |
| 49a | _this_ | The machine you look through. Driving a machine by hand was never written down — not the wheel, not the controls — while `Operation::pilot_tick` calls `break_block`, so a hand-dug hole replayed as untouched ground. Demonstrated at two different hashes over the same orders, then closed: `Wheel` and `Pilot` on the wire, journal VERSION 31, and `MachineTag` gaining the kestrel its own doc comment said would be "a version bump, loudly". And the camera comes off the hull it was sitting inside — a gimbal under the nose, measured off each rig's real parts, with the subject machine culled from its own feed and machine *heading* interpolated between ticks for the first time, so a nose-mounted camera glides instead of snapping |

**1 — Core scaffold.** Block registry, palette-compressed chunk storage,
worldgen, greedy meshing. A chunk is 65 536 blocks; storing a `BlockId` each
would cost 128 KiB, so blocks index a per-chunk palette packed at the narrowest
bit width that fits.

**2 — Renderer and binary.** wgpu pipeline, camera, chunk streaming, and the
`--screenshot` path that renders offscreen with no display — which is how the
whole stack gets smoke-tested against a software Vulkan driver in CI.

**2.x — Editing, physics, persistence.** Voxel raycast (DDA), block break and
place through *cancellable events* (nothing listens yet; the point is that mods
will be able to veto an edit without any call site changing), AABB player
physics resolving one axis at a time, and region saves.

**2.5 — Terrain that means something.** Height comes from three independent
fields through splines rather than summed octaves — summing drives everything
toward the mean and produces flat terraces. Continentalness, erosion and
peaks/valleys are sampled separately and each mapped through its own curve, so
"high but flat" and "low but jagged" are both expressible. Relief went from ~22
blocks to ~100. Plus parallel meshing and frustum culling, the latter with a
test that renders with culling on and off and asserts the pixels are identical.

**3 — Ore geology.** Copper in irregular buried blobs from a jittered lattice.
Outcrops are a *consequence*, not a feature: a body sitting high enough pushes
through the soil. Every visible outcrop has ore continuing beneath it, so
prospecting means something.

**4 — Machines that dig.** The `vx-agent` crate: job board, BFS flow fields,
and three mining methods — adit, decline, open pit — ranked on *cost* rather
than dug volume, because the excavation is dug once and the haul is made
forever. Bodies are mined in benches, not as boxes, so a drone can always climb
back out of its own hole.

**4.5 — The review round.** Eight verified findings fixed, mostly swarm stalls:
asymmetric flow-field edges under overhangs, an unbreakable-block wedge, a
haul livelock, and a tick backlog that fast-forwarded a drone across the map
after a stall.

**5 — The air, and the map.** A flying drone that sweeps sectors and pings ore
up to 24 blocks down, ferries piles home once you place a base container, and a
fog-of-war minimap that stores nothing but the set of explored chunks —
unloaded terrain is recomputed from the height field on demand.

**6 — The game in your hands.** Composite cuboid rigs (the machines finally
look like machines), a handheld drill with hold-to-dig, name-keyed skills on a
RuneScape-shaped curve, and a 5×7 bitmap font with a HUD — the font that every
later panel is built on.

**6.5 — The hometown.** A static village at the origin, identical in every
seed, blended into the terrain over a smoothstep skirt; villagers; a supply
shop that really trades. Plus trees and grass, which needed the engine's first
non-cube geometry (crossed quads).

**7 — Eyes and hands.** Third-person camera with a player body and terrain
pull-in; the handheld fleet uplink that lets you look through any machine's
eyes and take the master override, driving and cutting yourself; and NPC
perception with line of sight and memory. The piloting layer is deliberately
the foundation for drone-assisted combat later.

**8 — The frontier.** A day/night cycle pushed to the renderer as a uniform
(never read from a clock in a render path, so captures stay byte-identical);
towns rebuilt in shipping containers and corrugated metal with a radio mast at
the centre; **many** towns on a jittered 512-block lattice, varied by size and
speciality; and a beacon network whose derived job postings can name a town the
player has never seen, pinning it on a map that is still black around it. The
structural half of the round was splitting `height_at` into `natural_height_at`
plus a blend, so town siting could stop reading its own output.

**9a — Determinism, and cashing it in.** An external design report prompted this
round; the audit of it is in the commit history. It began by finding a real bug:
`World::block` reports unloaded chunks as air and agents read through it, so a
drone's decisions depended on which chunks the camera had streamed in. Fixed by
pinning an operation's working span, and gated by a test that digs the same body
at two residencies and compares content hashes — verified sensitive by removing
the pins and watching it fail.

That made the round's real idea safe: agents author almost every block change in
this game, and they do it deterministically, so the save should record *orders*
rather than outcomes. The command journal does, and `--replay` rebuilds a world
from it and checks the result — a determinism oracle covering worldgen, agents
and editing at once. Also: `SeedPath`, `BodyId` on every chunk key, a crew of
drones finally exercising the job board's contention paths, and geometry packed
from 168 bytes a quad down to 8 — chunk-local, with the vertices synthesised in
the shader, verified by a byte-identical capture.

**9b — The trade network.** What stage 8's masts were for. Towns keep books,
derived from their site until something touches them; they produce, consume and
price by speciality, and they ship surpluses to neighbours that are short. A
counter now asks its own town what it pays, so ore is cheap at a mine and dear
at a refinery, and a big sale moves the price. `engine:copper_bar` makes it a
chain rather than a gradient. One shipment record serves town freight and the
player's alike, and a load in the air is a lerp rather than a simulation — free
by the hundred, and drawable as a real machine when one passes close.

Both the books and the network catch up in fixed quantised windows, which is the
round's central care: these flows clamp, and once a clamp binds, one big step
and many small ones disagree — so the world would otherwise depend on when the
player happened to look. Two tests hold that line, and both were verified by
breaking the code and watching them fail.

**10a — The opening loop, and maps worth reading.** Machines were free: a flier
arrived the moment the world opened and a crew of drones appeared unpaid on
every dispatch, while the shop sold two capped upgrade lines and nothing else.
So the economy had no sink and there was nothing to mine *for*. `garage.rs` is
that sink — machines are name-keyed, cost credits, and each one costs half again
what the last did. You keep one free starter flier so an empty garage is still
playable.

The maps turned out to be half built: the minimap has always drawn unexplored
ground as a dark pane and always stamped markers over it with no visibility
test. `render_map_sized` makes that picture drawable at panel size, so the
beacon console can inset a trade map — this town, the destination pinned, live
caravans, black where nobody has walked — with a bearing under it, and the
handheld gains a map page on `Tab`.

---

## Shipped — Stage 10b: player movement

The player was the last actor in the world still integrated off the frame clock.
Everything else took ticks; the player took `dt`, which meant where you ended up
depended on how fast your machine drew. House rule 2 says agents are
bit-identical given the same inputs, and the player is an agent.

**Movement is a command now.** Held keys become a `MoveCommand` once per frame;
the simulation consumes one per tick at 64 Hz. Same idiom as `PilotCommand`,
same reason. Look angles are quantised into the command as `i16` and resolved
through a direction table, so nothing in the integration path calls `sin` every
tick on the value the whole simulation keys off.

**Sixty-four hertz, not sixty.** The journal speaks in mining ticks and mining
runs at 8 Hz. Sixty over eight is seven and a half; sixty-four over eight is
exactly eight. The integer ratio means `Advance { ticks }` keeps the meaning it
already had and replay runs eight movement sub-ticks per journal tick.

**The verbs.** Sprint, crouch, prone, slide, vault, mantle, coyote time, jump
buffering. `STEP_HEIGHT` dropped from 1.05 to 0.6, so a full block can no longer
be strolled up — but vaulting is **automatic on contact**, which is what keeps
the benched pits the mine planner cuts pleasant to walk through rather than four
hundred keypresses an hour. Above 2.2 m there is nothing to do but go around.

**Sliding is an impulse and a decay curve.** Entering one kicks horizontal speed
to 1.4× and installs low friction; a slide-jump keeps every bit of the speed it
built. The design note wanted gravity projected onto the surface normal so a
slide would carry downhill — but there are no ramps in a voxel world. A decline
is a staircase and every normal is exactly `+Y`, so the projection yields
nothing. What actually differs downhill is that the run is spent falling off
one-block steps, so a slide converts a slice of each drop into forward speed.
Same behaviour, honest mechanism.

**Carried mass is the movement stat.** Speed scales from 1.0 empty down to a
floor of 0.55 fully laden, and mass raises the sprint drain. This wires three
systems that did not touch: `Stockpile`, the `Logistics` skill and the wallet's
cargo upgrades all became movement upgrades without a single new item — every
upgrade that lets you carry more is also a tax on using it. The load rides in
the command as a quantised byte, so a replay reproduces the laden player rather
than a lighter, faster one.

**The journal covers the player now.** `Command::Move` is recorded on *change* —
`Advance` already folds, so holding W for a minute is two entries — and
`--replay` reports where the walk ended. The regression test is the determinism
oracle that already existed rather than a fixture built beside it.

Three bugs worth recording, because each was found by a test rather than by
looking:

- **Ground speed was capped below sprint.** With exponential drag and capped
  acceleration the fastest a body holds is `accel / friction`. The note's 60 and
  10 put that ceiling at 6.0 m/s, making `SPRINT` at 6.5 a number you could
  never reach. `ACCEL_GROUND` is 100.
- **The inset made penetration worse, not better.** Shrinking the sweep box
  guarantees only the *shrunken* hull is clear, which lets the real hull
  penetrate by exactly the amount you shrank it — and the error compounded across
  sub-steps until the body was embedded in a wall. It is a tolerance in the block
  query now, ordered `INSET < SKIN` on purpose.
- **A slide died on the first step it fell off.** Leaving the ground ended the
  stance, and going downhill *is* leaving the ground, so the one case the verb
  exists for was the one case it could not do.

### Deferred from this round

The drone-traversability warning. A 2.2 m mantle means a hole the player can
climb out of is not necessarily one a drone can drive out of; breaking that
invariant is fine, breaking it *quietly* is what strands machines. Surfacing it
on the handheld is a mine-planner change and waits.

---

## Shipped — Stage 10c: home

The player woke up on a road while the world assembled itself around them, and
every walk across it hitched. Both had the same shape of cause: work done at
the wrong time, on the wrong thread, or over and over.

**The lag had three causes, and the biggest was the dumbest.** Every chunk
loaded from disk re-read and re-decoded its *entire* 32×32-chunk region file —
up to eight whole-file decodes per frame while walking through saved ground.
`WorldSave` now holds decoded regions in a small cache, invalidated by its own
writes; sixteen chunks from one region cost one read, by test. Second:
generation ran synchronously on the main thread. It now runs on a background
worker holding a *clone* of the generator — terrain is a pure function of
`(seed, position)`, so the clone is bit-identical by contract and by test, and
anything the simulation needs immediately still comes through the synchronous
path, so the stage-9a pinning story is untouched. A result arriving for a chunk
already resident is discarded: the resident copy may carry edits. Third: the
wanted-list sort and the dirty scan ran every frame; both are now gated.
Meshing stays a budgeted fork-join on purpose — it was never the hitch.

**The spawn area pregenerates before the first playable frame** — the whole
render distance, nearest-first, with progress in the title bar — and then the
hometown footprint is **pinned resident forever** with `pin_span`, the
"spawn chunks" idea Minecraft ships and stage 9a happened to have already
built. Coming home never hitches on regeneration. `--view-distance` picks the
radius (4–16, default 8).

**You wake up in your own house.** STONEHAVEN gained one building on its
south-west quarter — hometown only, by plan split on `is_home()`, so your house
is singular as a property of the data. Inside: a **chest**, your home
warehouse. Breaking it packs its contents into thin air and carrying the block
somewhere else unpacks them — the contents were never in the world, the block
is a marker and the side table is the truth, the pattern the fleet's base
container established. One chest may stand at a time; the place hook enforces
it. Outside, beside the door: a **mailbox**, unbreakable town furniture like
the counter, which deletes every "what if the mailbox is gone when the mail
lands" edge case in one line of registry data.

**Mail-order.** The shop counter now lists, for each good, a parcel offer from
the cheapest other town in radio range with stock to spare. Pay the source
price plus freight — freight priced by the same `travel_ticks` the caravan
flies by — and a real `Shipment` crosses the map with `Owner::Mail`, visible on
the trade map like any caravan, landing in your mailbox whichever counter took
the order. Mail never touches the destination's books: it was bought at the
source, and it lands in a mailbox, not a market. The order *does* move the
source town's books, honestly. Machines stay instant at the counter — the
opening loop's first-drone moment is not getting a shipping delay.

**The welcome panel.** First boot opens an in-world panel: where your stuff
is, and the whole story — parsed out of *this file* at compile time via
`include_str!`, so the changelog cannot drift from the truth. A test insists
the stage that built the panel appears in it. Dismissing it writes a zero-byte
marker in the config directory (player metadata, not world state) and it never
shows again. `--changelog` prints the same lines headless; `--welcome` draws
the panel over a capture.

**The honest cost: old journals restart.** Adding the house changes generated
ground, so a v3 journal would replay against terrain that no longer generates
and report divergence that is nobody's fault. The journal is now VERSION 4; an
old one is refused with a warning and the fresh log declines genesis coverage
(`keyframe_tick` nonzero), so `--replay` says "nothing to compare against"
instead of lying either way. Old worlds stay fully playable — region snapshots
load as ever, and player edits win over regeneration, which is correct.

---

## Shipped — Stage 11: permits

You could walk up to anybody's house and pull the walls off, and not one soul
said a word. This round the town grew a spine — and three ways past a locked
door, priced so the honest one is the best one.

**The bus finally has a subscriber.** `break_block` and `place_block` have
emitted cancellable events since stage 2.x with the module doc promising "mods
will, without any of these call sites changing". Eight stages later, that is
exactly what happened: the permission gate hooks the events, so the player's
drill, a drone's cutter and anything added later are all covered, and not one
caller changed.

**No actor identity was needed, and that was the finding.** The obvious design
threads an `Actor` through `break_block`'s seven call sites. It turned out the
only distinction that matters is live-versus-replay, and that already existed
structurally: `journal::replay` is handed a fresh `EventBus` by every caller, so
the oracle never sees the gate. `the_replay_oracle_runs_without_the_gate` is the
tripwire for anyone who later "helpfully" passes the live bus in.

**Claims are derived, never stored.** A town's buildings are a pure function of
its site, so `plan::buildings` gives every building a role and an AABB — one
below the floor and one above the roof, because a claim you can tunnel under is
not a claim. What is *stored* is only what cannot be derived: who has been let
in, who holds office, what you have been caught doing, and how far through a
lock you have drilled.

The ranking is **Sheriff > Mayor > owner > guest > everyone**. The sheriff opens
anything, because a lock the law cannot pass is a lock that makes crime safe.
The mayor is not an override — they own the streets, the paving, the tower and
every scrap of open ground inside the town line, which is status, not a skeleton
key. Outside the town line nothing is claimed and you build where you like.

**Stealth was already in the engine; nothing was reading it.**
`awareness::PLAYER_EYE` was a flat 1.62 and the villagers' sight ray aimed there
no matter what the player was doing. It now carries the player's *live stance*
eye height — 0.35 m prone — so going flat behind a one-block wall is occluded by
the raycast that was already running. No visibility stat, no detection meter:
stealth is geometry. `Stance::Prone`, which the stage 10b notes flagged as dead
weight, now has a job. Villager sight also gained a cone about the heading they
already track, because before this there was no *behind* to get to.

**Seen is the rule.** Prying at a claimed wall, picking a lock, smashing one —
all of it costs bounty only if somebody can actually see you, checked with a
fresh cast at the moment of the crime rather than the round-robin cache, which
is several frames stale. The HUD grew an eye, because without it the stealth
rules are invisible mechanics and the only teacher is being arrested.

**Three ways through a door.** Locks come in three grades and are deliberately
*breakable* — a lock you cannot attack is a wall with extra steps.
- **Authenticate**: free, permanent, no risk. Trust through trade is stage 12;
  this round only the owner qualifies.
- **Pick it**: needs the new `SECURITY` skill, leaves the lock standing so
  nobody need ever know. Deterministic — no roll — because what levelling buys
  is *speed*, and the real cost is standing there exposed while it runs.
- **Drill it**: each grade carries a hard `min_power` floor as well as a
  hardness, because "impossible for a new player" cannot be said in seconds
  (drilling is `hardness / power`, so any hardness eventually gives). Breach
  progress *persists* where ordinary drilling resets on a wobble — a breach is a
  project you come back to; a pick is not, and that asymmetry is what makes
  choosing between loud and quiet a decision.

Breaking a lock leaves the building open until the town puts a new box up. The
rebuild is recorded as an ordinary `Command::Place`, so a replay puts the lock
back at the same tick — a world edit the journal cannot see is a world edit that
makes the oracle lie.

**Stated plainly rather than hidden:** the player holds no office in ordinary
play, so the sheriff override is built and tested but exercised only through the
`--sheriff` development flag until stage 13's ballot box. And with town land
claimed, your chest and your base container can no longer go anywhere in town
except inside your own house — the toast says so rather than leaving you jabbing
at the dirt.

Journal VERSION 5: the lockboxes and the security office changed generated
ground, so an older log would replay against terrain that no longer generates.

---

## Shipped — Stage 12: the gold panel

The operator's console: spawn, tweak, inspect — a Garry's-Mod-shaped surface
built without breaking the one thing this codebase cannot afford to break.

**A cheat is an order like any other.** The panel owns no mutation path. Every
button resolves to a `Command::Admin(..)` in the same journal as movement and
dispatches — Give, SpawnMachine, Teleport, SetStat, SetStock, SetTuning — so a
session full of cheats still replays to a hash. Set up a situation through the
panel and the journal *is* that scenario: committed, a regression fixture;
handed over, a perfect reproduction case. Replay applies what `Rebuilt`
carries (the player, the tuning, the base pile); machines and town books are
no-ops on replay, the same honest line `run_replay` has always drawn for the
economy.

**Tuning constants are orders too — that was the subtle one.** Dragging
`friction_slide` feels like editing a file, but it changes how every later
command is *interpreted*; replayed under different constants, the same log
diverges. So the movement tranche moved into a name-keyed `Tuning` struct
riding on `Movement` (the shipped constants are its defaults), every slider
commit records `SetTuning { key, value }` — the f32 crossing the wire as raw
bits, exact — and a journal now carries its physics with it. Proven by test:
a mid-log tuning change lands the same sprint somewhere else, reproducibly.
Physics constants (`GRAVITY`, `STEP_HEIGHT`…) stay `const`: they cross the
crate boundary, and threading them is a bigger cut than tuning has yet earned.

**Fiction never sees gold — with one honest retrofit.** The gold *hue* was
already the house accent in all seven diegetic panels, so what marks the
console is chrome none of them have: the double gold border. If a capture
shows the border, the session was touched. The panel is the repository's
first cargo feature (`gold`, default-on for dev; the shipped Deck build
compiles it out with `--no-default-features`, verified by the marker string
being absent from the binary), and opens only behind `--gold` + F10.

**Deck-shaped, keyboard-driven.** The focus model is a controller's — tab
cycling, directional focus, one activate button, slider fields, X-to-reset —
but no gamepad backend exists anywhere yet, so the keys stand in (arrows,
Tab, Enter, X). When a gamepad crate lands, with hardware to test it on, it
maps onto this model without the panel changing. Journal VERSION 6.

Deliberately not built: spawning villagers (the roster is hardcoded at three
and rigs index by variant — a real refactor, waiting for a round that needs
it), search in the spawn grid, undo (every order is journalled, so replay-to-
tick-N is sitting right there when someone builds the button).

---

## Shipped — Stage 13: the arsenal, and what robbing costs

The brief was three words — heavy, loud, scary — and a fourth that goes
without saying in this repository: *replayable*.

**Heavy.** The launcher rides the same carried-mass system as ore: owning it
folds a fixed heft into the movement load byte *before* the command is
journalled, so the weight slows the same sprint on both sides of the oracle
without the journal ever learning what a weapon is. Firing queues a recoil
impulse through the exact one-shot slot the slide-entry kick uses — applied
before friction runs, or the stagger would be partly thrown away — and the
aim physically climbs, because the pitch is real input state and rides the
next `Move` command for free.

**Loud.** The repository grew its first audio: `rodio`, playback features
only, because every sound is *synthesized* — a sub-bass sweep under a hard
noise burst, soft-clipped so it reads overdriven rather than polite. There
are no audio assets and there will be none for the base game: nothing to
license, nothing to load, nothing for a mod to be missing. A machine with no
output device gets a silent player and the game does not care. The camera
shakes on a decaying trauma curve, applied to the *pivot* only — in third
person the orbit's wall raycast runs from the shaken anchor, so the kick can
never punch the camera into rock — and the whole felt layer is tunable from
the gold panel (`shake_power`, `slug_kick`, and friends joined the tuning
table, which is exactly why the panel shipped first).

**Scary.** Villagers understand the muzzle. Pointing it at somebody who can
see you — same fresh line-of-sight cast as witnessing — panics them, and each
of them settles it with their own deterministic coin (the same salted hash
streams the wander runs on): *de-escalate*, running home to hide, or
*escalate*, running for the security office. An alarm that reaches the office
is a signed report and costs bounty with no further witnesses needed. Blasts
and impacts panic close bystanders outright — hearing needs no line of sight
— and turn every head in earshot.

**The projectile steps on the journal clock.** The design note said "a sum,
not a simulation"; what shipped is stepped — one integration per journal
tick, fixed step, gravity sag and all — because slugs break blocks, blocks
are in the world hash, and per-tick segment sweeps through `raycast_solid`
were needed for hit detection anyway. Eight visible steps a second is also
what makes *leading* a caravan a skill. The `Fire` order (VERSION 6 → 7)
records the quantised aim **and the muzzle position** — the one deliberate
exception to "intent, not outcome": live movement runs on a real-time 64 Hz
clock and journal ticks on the drones' 8 Hz clock, so a replay-derived muzzle
would stand a few subticks from where the trigger was actually pulled, and a
hash must not depend on which clock you asked. `the firefight replays
crater-for-crater` is a test, not a hope.

**A slug does not ask permission.** Ballistic damage bypasses the cancellable
break event (the gate exists to refuse an edit; a fired slug is past
refusing) and the bill arrives by consequence instead: the first harmless
shot inside a town's line gets one warning, once, per town, and property
broken on somebody's claim costs `damage × (1 + 0.5 × (witnesses − 1))` —
seen is the rule for gunfire exactly as for lockpicks. What a slug can break
is capped between sheet metal and mast steel: buildings are vulnerable, ore
bodies, masts and every grade of lockbox are not, and at shop prices per
round it is the worst drill money can buy.

**Interception.** A slug sweep is tested against every caravan's hull —
`position_at` is pure in the tick, so the test is arithmetic. A downed load
falls where it was hit, waits as a crash site, and is salvaged onto the base
pile by walking up to it. The network bills half the cargo against your name
whether or not anyone watched: the manifest is its own witness, the one
crime in the game that needs no eyes. The destination town simply never
receives the goods, and the shortage working through its price curve is the
town noticing. Shooting down your own delivery is legal, free, and
its own punishment.

### What is deliberately not in 13

Player health, hostile escorts, death, and the rest of the weapon table
(scattergun, mining charge, beam cutter, rail lance, EMP burst, guided
missile — named for what they do, never for anybody's trademark). Ammunition
as a *trade good* with its own market line also waits; the counter sells
slugs for credits meanwhile. Nothing shoots back yet — that is stage 17's
health model — and the bounty contracts the mast should post wait for
somebody to take them.

---

## Shipped — Stage 14: the kestrel and the roost

One machine, two owners — the decision everything else followed from. The
player's scout and the town's watcher are the same airframe with the same
endurance and the same battery problem, so the player learns exactly what the
law can see by owning the thing that sees it, and surveillance stays a
mechanic instead of a punishment.

**The kestrel** rides the pack: bought once at the counter (one per person —
the fleet is for swarms, the pack is for one), tethered by recharge rather
than fuel. Flight and cooldown are one budget — the recharge is proportional
to what the flight spent, and the cell upgrade line shortens it. Its standing
orders are journalled commands (`Command::Scout`, VERSION 7 → 8) set from the
handheld's third page: orbit overhead, sortie where you look, perch as a
sentry at a quarter drain, fly vanguard ahead, dock. Piloting it is the
existing take-control path — `MachineRef::Kestrel` — and its movement is the
flier's, reused whole: same terrain-safe stepping, same never-inside-terrain
rule. It is deliberately **not** in the fleet roster, so `dispatch_scan` can
never grab it and the survey layer stays the paid flier's trade by
construction — the scout reveals *contacts*, never terrain, which is the line
that keeps it from quietly obsoleting the machine you pay for.

**A mark is a report, not a tracking beacon.** Contacts scanned within range
and line of sight get a hovering cube over where they *were* seen and a pin
on both maps, dimming as the report ages and gone after thirty seconds
unsighted. The occlusion raycast that already ran everywhere does the work,
which is what makes a roof — or tree canopy — real cover from above with no
new stealth system: cover now splits into cover from people and cover from
the sky, and the sky kind is geometry.

**The roost** is the same machine in a box on the security office roof (a
fifth blueprint layer — the worldgen half of the version bump). It hears
loud *reports* — a lock breached, a shot fired in town — pops out over three
readable seconds, flies to the noise, and watches. The first time it gets
eyes on you, you are *observed*: the drone overhead, visibly, is the
warning. Crimes committed while observed count it as one more witness in
the arithmetic stage 11 built. It never attacks. And every counter to it is
honest mechanics: break sky line of sight until the mark decays, or bait it
aloft and outlast its endurance — while it recharges in the box the town is
blind, and `the heist window exists` is a test, not a promise.

### What is deliberately not in 14

The homestead's own roost (stage 15, where hardening it against intrusion
has stakes); hacking anything (stage 15); contact classification beyond
person/machine; a second kestrel; per-kind mark decay; kestrel numbers in
the tuning table. Known rough edges, expected: the kestrel's cell resets on
a reload (endurance is not persisted); a perched kestrel under an overhang
sees nothing and says nothing about why; the roost watches but nothing yet
*acts* on what it witnesses beyond the ledger — the sheriff who serves a
warrant is a later tenant.

---

## Shipped — Stage 15: hacking through machines

The Security line's work, done at arm's length — and the town's own machines
made into targets worth doing it to.

**Not a minigame, on purpose.** Stage 11 set the rule that locks are gated by
hard floors, not dice. Moving the work onto a drone changes *who is exposed
and where the operator stands*; it does not change the odds, because there
are none. `refuse` is one pure function taking one `Attempt` — frame, coil,
Security, reach, link, target — so the whole rule set is testable in a line
and identical whether the machine flew there under orders or under your
thumb. Position is the only input, which is the strongest possible form of
"a piloted machine can do nothing an autonomous one could not."

**The tool is a module, the ceiling is the skill.** The counter starts
stocking spoofer gear at Security 10 — well before any of it is useful,
which is the right way round. A light coil rides the kestrel and opens
houses and shops; a heavy coil rides a real airframe and opens anything. The
grade of lock is still gated by the floors stage 11 set, so the shop cannot
sell what practice has not earned. The **hardened link** is on the shelf too:
nothing hacks *you* until factions land, so it is bought early and needed
late — which is exactly how the player's own conduct teaches the rules they
will later be on the wrong end of.

**The machine is exposed; the owner is billed.** A machine at a lock is
watched by the same eyes as a body: `Villagers::watchers_of` is the witness
query pointed at a point in the world rather than at the player, and the
roost counts too. Caught, the bill lands on the name in the garage papers —
and an *unattended* machine is seized where one you are personally flying can
be flown off, which is the honest difference between leaving a tool
somewhere and holding it. The pound wants a flat fee plus a tenth of the
machine's worth, payable at the counter, and it outranks every other row on
the shelf while it is owed.

**Range is the link, and the link is the leash.** The operator must stay
within 120 m of the machine and the machine within reach of the target, and
every condition is re-judged every tick — walk away and the job stops where
it stands. An intrusion is a planned act you stay committed to, not
fire-and-forget, and that number is what stage 25's jammers will exist to
shrink.

**The watch box, three ways.** Blind it (Security 15) and it stands down —
loud in its own way, because the town notices a dark box soonest. Silence it
(35) and it flies its patrols, sees everything, and files nothing; nobody
notices until an offence it should have witnessed goes strangely unpunished.
Tap it (70, heavy coil only) and its sightings mirror onto your handheld
through the same `Marks` the kestrel fills — the same eye, the same radius,
the same occlusion. The tap grants the sheriff's eyes, never better ones, and
it lasts longest because nothing about a tapped box looks wrong. The grades
were re-scaled onto the ladder the locks already use (1 / 20 / 60) rather
than the note's own 3 / 5 / 7, which was written against a scale this game
does not have.

**And the loud way still works.** The box is its own block now, so you can
read it across the plaza; it joins the lockboxes' exemption from the permits
gate for the same reason they have it — it is the attack surface, never the
thing defended — and drilling it out blinds the town until it is re-boxed, at
the breach price if anybody saw. **Your own roof** takes the same box for the
same price the sheriff paid: bought at the counter, mounted with a journalled
`Place`, and it watches your yard without ever flying, because it has nothing
to respond to.

### What is deliberately not in 15

Anything hacking *you* — the hardened link has no adversary until factions.
Targets beyond locks and the watch box: a rival's hauler, a faction digger.
The impound as a *place* — it is a counter and a fee, not a fortified
jailhouse holding your best machine, which is self-writing content
deliberately left unwritten. Unmarked chassis: every machine traces to its
owner, and stolen serials wait for a market to sell them in. And the Security
milestones still gate cleanly while teaching nothing; the skill line needs
its manuals.

---

## Shipped — Stage 16: the fabricator

**Why a printer and not a crafting table.** A grid of shaped recipes is a
memory game about where to put the sticks. This codebase already has a better
vocabulary for the same idea: everything it owns is a name-keyed row — blocks,
skills, upgrade lines, machines, goods. So the fabricator takes named goods
off a pile and puts named things back, and "it can make anything" means
exactly "adding a thing is adding a row". That is a promise the code can keep,
and the round is the proof: ammunition, a charged cell, planks, wall panels, a
spoofer coil and two whole machines are all one table.

**Every block is stock.** The player's drill used to be the only tool in the
game that produced nothing — you cut a block and it vanished. Now every block
yields *itself*, by name, onto the fleet's base pile: the same pile the flier
ferries into and the shop sells out of. One pile, three doors, and no
transfer minigame, because there is still no player-carried inventory and this
round deliberately did not invent one. It also means a town is feedstock if
you are willing to be that sort of person, and the permits gate is what has an
opinion about it.

**Materials and time, never credits.** Buying a drone with money and printing
one out of copper are two routes to the same machine, which is the point: the
counter is for people with money, the fabricator is for people with a mine.
Materials leave the pile *at the start* — queue three drones on one bar and
the pile would be lying — so a cancelled print costs you, and starting one is
a decision. The Fabrication skill buys speed and unlocks the harder patterns
through the same hard floors every other gate in this game uses; it never buys
certainty, because there are no rolls anywhere in this game.

**The ladder.** Slugs and copper bars at the bottom, so a fabricator pays for
itself the day you place it — and the ammunition loop finally closes, since
slugs were credits-only. Planks and wall panels next, which is the first time
the frontier's own building materials have been makeable. A charged cell in
the middle: swap it in and the kestrel flies *now* rather than after its
recharge. Then a spoofer coil, then the kestrel and the ground drone
themselves.

**It is a machine, so you buy one and place it.** Bought once at the counter,
placed from the belt wherever you want it, moved by breaking it — the chest's
own rule. Placing one you have not bought is refused, or the palette would
quietly hand out free drone factories. And while fixing the belt, slot six
turned out to have been unreachable since the chest was added, which had
quietly made the chest unplaceable; the belt now runs blocks on one to six,
the launcher on seven and the fabricator on eight.

**The oracle keeps up.** `Command::Print` is the first order since `Advance`
that is *not* a replay no-op: a print eats the base pile, and the pile is
something `Rebuilt` carries, so replay spends the same materials at the same
tick and finishes holding the same stock. Only the outputs that live outside
the pile — slugs, cells, machines, modules — are the honest no-ops, the same
line `Give` and `SpawnMachine` draw. VERSION 9 → 10.

### What is deliberately not in 16

Fuel: printing costs time and materials, and when the fuel loop lands it
plugs in as a cost on the *time* rather than a change to what a recipe means
(logs stand in for smelting fuel meanwhile). A print queue longer than one.
Recipes as moddable content — the catalogue is code, which is why the journal
records a print by index rather than by name, and the version bump that makes
it content will say so. And still no player inventory: the pile remains the
only place goods live.

---

## Shipped — Stage 17: caves

**The first hole in a height field.** `fill_column` fills one column from
bedrock to a surface height, and until this round nothing anywhere carved a
hole in the middle of it. Caves are the first genuinely **3D** thing worldgen
has ever done, and the cost was exactly what the plan predicted: not the caves
but the carve — a volumetric field that stays pure in `(seed, x, y, z)` so
chunks keep generating in parallel with no cross-chunk context, and a saved
world keeps regenerating identically. `noise.rs` grew the trilinear sibling of
its 2D value noise; `caves.rs` is the field.

**Tunnels are an intersection, not a threshold.** One noise field thresholded
into air gives bubbles. Two independent signed fields, carved where *both* run
near zero at once, give the intersection of two surfaces — a long winding tube.
That is the whole tunnel algorithm: `a² + b² < r²`, with the y-frequency
squashed so galleries run wide and low rather than as chimneys. A third,
slower field opens chambers where it peaks, gated `CHAMBER_COVER` below the
surface where a room cannot crater a hillside. Girth tapers toward the surface
(`MOUTH_FACTOR`), so mouths exist and stay scarce: at depth the rock is about
a tenth hollow, at the skin far less.

**What is never carved:** town footprints (`fill_column` masks them — a plaza
must not open into a void); the bedrock floor plus a margin (`CAVE_FLOOR`);
and the top `SEA_BED_COVER` blocks of any column ending at or below the
waterline, because there is no fluid simulation and a mouth under the sea
would be a hole the ocean visibly fails to pour into. Where a tunnel meets an
ore body it carves the vein with it — the carve wins — which both guarantees
no copper ever floats in the middle of a gallery and leaves the rest of the
body showing in the cut faces. That is the stage's opening-loop payoff: ore
at the surface of a wall, where hand-mining is pleasant.

**The agents already coped.** The plan flagged `flow::settle`, the mine
planners and span pinning for a pass. Settle was built for "the ground under
me vanished" the day drones learned to dig, so a machine over a void drops to
the gallery floor and carries on; a mine plan that meets a cavity finds part
of its digging already done and the whole agent suite runs green over the new
ground. Flora needed the one real fix: a tree whose base column got carved
into a mouth would have floated, so trees filter on the same pure field the
carve uses — every chunk a canopy reaches agrees, and the stamping stays
seamless.

**Journal VERSION 10 → 11**, a world change alone: every hash a version-ten
journal recorded was taken over terrain that no longer exists, and pretending
the old hashes still bound would make every old session replay as a divergence
that is really this bump. `--cave` joined the capture flags: it hunts the
roomiest pocket of underground air near `--at` and stands the camera in it,
facing down the longest gallery — the capture reads the generated world, not
the field, so what it frames is what generation actually built.

**Deliberately not in 17:** darkness — caves are lit by the same sun as the
surface, and an underground lighting model is its own round; bunkers (next,
now the 3D carve is paid for); anything living down there (hostiles wait for
health); water in caves; per-biome cave styles.

---

## Shipped — Stage 18: lights in the dark

**Darkness first, then the tools that cut it.** Stage 17 left one honest gap:
caves were lit by the same sun as the surface. This round closes it without a
light-transport system. Every face the mesher emits now bakes a 4-bit **sky
exposure** — how deep the air it faces sits beneath the topmost opaque block
of its column. The curve is gentle then steep: two blocks of roof reads as
shade, twenty reads as night. It is an approximation of enclosure, not
radiosity — cheap, pure, and rebaked for free exactly when digging changes
what the sky reaches, because that is when a chunk remeshes anyway. The
packed quad had seven spare bits; light took four of them, and it joined the
greedy merge key so a rectangle never smears one value across a light change.
Machines and people darken by the same column rule, per instance.

**The lamp is a light; the visors are ways of seeing.** The suit's hand lamp
(`L`) is a warm spot cone in the fragment shader, thrown from the active eye —
strength, reach and aim ride the sun uniform, which had reserved its `zw`
lanes for exactly this since the day/night round. Everything better is
printed, not bought: a **high beam** with nearly twice the throw, a **night
vision visor** (an intensifier: it amplifies received light plus a floor read
off the surface, shaped by facing and fading with range, so black galleries
resolve into green geometry rather than one flat wash), and a **thermal
visor** that ignores light entirely — terrain graded cold, objects rendered
as warm bodies, which is what the terrain/object flag in the shared fragment
shader is for. The sky is a clear colour, not fragments, so the visors tint
it on the CPU or a green cave would open onto a blue day.

**Optics are possessions, not upgrades.** `optics.rs` keeps a name-keyed
owned set and the dial, saved as `VXOL` (a dial restored onto gear that is
not owned falls back to Off). One key cycles Off → lamp → night vision →
thermal, skipping what you do not own; the HUD names what you are looking
through. The rows sit in the fabricator's ladder in floor order — beam at 6,
night vision at 12, thermal at 18 — which renumbered the recipes after them,
and `Print` records recipes by index: **journal VERSION 11 → 12**, the
"recipe indices are content" bump stage 16 promised by name. One-per-person
is enforced before the journal ever hears about a duplicate print. Rendering
touches no world state, so nothing else in the log changed.

**Deliberately not in 18:** placeable lights — a torch or floodlight block
needs flood-fill light propagation, which is the real lighting round;
batteries or wear on the visors; light bleed around corners (a lit shaft
does not illuminate the gallery beside it); villagers reacting to a lamp
beam sweeping over them, which wants the perception work hostiles will need
anyway.

---

## Shipped — Stage 19: bunkers, and the geometry that makes each one unique

**The claim, and why it is not mysticism.** The golden ratio is the *most
irrational* number — its continued fraction is all ones, so no fraction
approximates it well — and by the equidistribution theorem `{n·φ}` never
repeats, never clusters and spreads as evenly as a sequence can. That is the
property uniqueness actually needs. Uniform randomness makes every dungeon
feel like every other dungeon because noise has no grammar; quasirandomness is
deterministic, non-repeating and evenly varied. It is also plain arithmetic on
a site hash — no rejection loops, no stored state — which is the only kind of
mathematics the worldgen rules here admit.

**Variation between sites, invariance within one.** A bunker where every room
rolled its own dice is unique the way static is unique. Instead the site hash
picks a handful of numbers — a proportion system, a tier, a footprint, a
bearing — and everything inside derives from those. Each bunker is governed by
one geometry the way each town is governed by one plan.

| System | Ratio | Splits | Reads as |
|---|---|---|---|
| φ | 1.618 | two at `1/φ`, recursing deeper into the *smaller* child | a coil: rooms shrinking inward |
| √2 | 1.414 | two at `1/√2`, both children equally | the barracks grid |
| √3 | 1.732 | three at a time on the long axis | the industrial hive |

Tier weights the draw: shelters coil, works branch. **Golden BSP** places the
rooms — splits at `1/P` of the parent, jittered ±4%, on the longer axis, which
for a P-proportioned rectangle alternates by itself and never yields the
slivers a uniform split does. **Fibonacci is how an irrational proportion
lives on a block grid**: every room dimension snaps to `{3, 5, 8, 13, 21, 34}`,
adjacent pairs approximate φ, and they land exactly on voxels. Orientation is
the **golden angle**: bunker `k` faces `k · 137.507…°`, so no two hatches on a
ridge point the same way and the sequence never repeats world-wide, with not
one byte stored.

**Where the note's plan bent to the block grid** — two deviations, both
documented in the module and both because voxels are not paper. Orientation
is the *entry bearing* rather than a rotation of the plan, because rotating a
voxel BSP by an irrational angle aliases every wall into a staircase; and the
pool *furnishes* generated rooms rather than replacing them, because the
vocabulary cross-product is over thirty room shapes per system and the shell
is identical in all of them. The guarantee the note asked for survives whole:
every legal room size has at least one piece that fits, checked at authoring
time by test.

**The shell is one number, not a new mechanic.** Bunker concrete has hardness
400 against stone's 1 — cutting in is possible and almost never worth it — and
it is `Some(400)`, never `None`, because the point is a choice you can make
and usually regret rather than a wall. Caves are masked over the works so a
tunnel cannot open a sealed shell, and bunkers are refused under towns for the
same reason a plaza may not open into a void.

**Three bugs the tests caught, all of the same family.** Connectivity is the
one failure invisible from the surface — a sealed room is loot the ledger
promises and nothing can collect — so it is asserted, not argued: every room,
every level, every tier, reachable on foot from the hatch. Getting there found
(1) furnishings that ringed their own footprint and sealed an air pocket, now
a flood-fill test over every piece; (2) a stairwell plated per level, which
sealed the very column the levels are threaded on, now one open shaft with
landings beside the run; and (3) the entry corridor paving its floor straight
over the staircase's air — fixed by a rule worth stating plainly, that the way
in is cut *into* finished works and never paves over open space it finds. A
fourth, subtler one: the golden angle computed in `f32` collapsed distinct
bearings onto each other, because `k · 137.5°` runs to tens of millions of
degrees before the modulo and an `f32` that size has an ulp of about four
degrees. The whole irrational-rotation argument was quietly dead until it was
computed in `f64`.

**Loot is the first source to match the sinks.** Supply caches hold goods
derived from where they stand — so two visits agree and nothing is rolled
twice — and *that a crate was opened* is remembered by the crate not being
there any more, which is the one thing this engine already stores. Prising one
open with the drill pays out the same haul as opening it by hand, through one
shared path, so neither can start paying something the other does not.
Prospecting scales the haul rather than re-rolling it. **Journal VERSION 12 →
13**, a world change: the ground now holds works that were not there.

**Deliberately not in 19:** occupants — mobs and military both attack you, and
that needs the health model stage 25 brings; ruin and partial collapse; new
goods (rations, spirits, oil) which are a trade round wearing a bunker's
clothes; pressure, damp or anything else that makes a room at level three
different from the same room at level one; and the geodesic dome and
phyllotaxis silo columns of the large tier's concept art, which want a
rasterisation pass of their own.

---

## Shipped — Stage 20: the fuel loop, and what it is made of

**Machines stop being perpetual.** Every cost in this game had been a one-off:
buy the machine and it works free forever. A running cost is what turns a
stockpile into a supply line — the difference between "can I afford this" and
"can I keep this going" — and this round adds the first one.

**The fuel is oxyhydrogen**, water split back into the two gases it is made of
and burned back into water in the cutting gear. Which means the feedstock is
something the world has an ocean of, so the cost had to move somewhere honest:
to the **electrodes** (copper, dissolved into the bath), the **time** (real
minutes of a machine running), and the **place** — an electrolyser only works
within two blocks of water. That last one is the first machine in this game
whose *position* decides whether it works at all, and it is the reason a lake
shore is now somewhere worth building. The refusal is at placement, not at the
panel, because a machine that looks built and quietly does nothing is the
worse lie.

**How the burn stays inside the oracle.** Fuel decides how much ground gets
dug, and ground is what the world hash covers, so a tank that behaved
differently on replay would be a divergence with the fleet's name on it. The
tank therefore lives on `Mining` — which replay carries — and is drawn and
burned inside `Mining::advance`, which is the very call `Command::Advance`
replays. That is the whole trick: **no fuelling order exists at all**, because
both sides run the same code over the same pile for the same number of ticks
and arrive at the same tank. The only new order is `Electrolyse`, which moves
goods exactly the way `Print` does. The tank is persisted, and has to be:
replay re-derives it from tick zero, so a session that reloaded with an empty
one would run dry at a different tick than its own journal says it did, and
the ground would differ by exactly the digging that bought.

**Measured in machine-ticks, not canisters.** A crew of six burns six times as
fast as a lone drone, and counting whole cells would make that unrepresentable
without fractions — which are precisely what a determinism argument does not
want. One canister is 2,400 machine-ticks: five minutes of one machine, or
seventy-five seconds of a crew of four. The kestrel is not among the burners;
it runs on a charged cell and a cooldown, which was the point of that design,
and charging for the same wing twice would be a tax rather than a mechanic.

**HHO is a traded good, not a private resource.** It joins the goods table
with a price, and the specialities take sides: a depot banks and sells it, a
mine and a refinery burn through it. So the network hauls fuel the way it
hauls ore, prices move when somebody floods a market with it, and a mining
camp is now a town that stops when the fuel does. Adding a fifth good is a
format change to a market's books — an old file's four numbers cannot be read
as five without inventing the missing one — so the economy save bumps to
version three and re-derives a town's books from its site instead of guessing.

**Journal VERSION 13 → 14.** `Electrolyse` reaches the hash by a longer road
than most orders: it only moves goods, but the fleet burns those goods to dig,
so a log replayed without it would run the crew dry at a different tick and
leave a different hole.

**Deliberately not in 20:** a tank per machine, so a drone cannot yet be
stranded far from base with the rest still working; tanker runs, since
machines refuel from the pile at any distance; fuel for the player or the
kestrel; and the oxygen half doing anything of its own — the mix burns as one
good, and splitting it would be a chemistry round rather than a fuel one.

---

## Shipped — Stage 21: star forts, and somewhere to put your things

**A bastioned trace is an argument, not a decoration.** Tall thin walls exist
to stop ladders; once cannon exist they fall down, so the answer is low, thick
and *angled* — every face of the wall covered by the guns of another face, and
no dead ground where an attacker can stand unseen. The star is what falls out
of "no re-entrant angle may go unwatched", and it belongs here because stage 13
gave the frontier something to shoot with.

**The trace is a polar radius, which makes the wall a signed distance.**
`r(θ) = base + bastion · cos(points · θ + phase)`, so "how far is this column
from the wall" is `hypot(dx, dz) - r(θ)`: one cheap expression, pure in
`(seed, position)`, needing no cross-chunk context. Wall, walkway, parapet and
ditch are bands of that one number — exactly the shape the design note asked
for, and the reason a fort costs a chunk almost nothing.

**Tiered, gated, and sometimes ruined.** A hamlet stays open; middling towns
get a palisade or a four-point trace; only the largest earn six points with
re-entrant angles deep enough to read as ravelins. A refinery is likelier to
be walled than a depot, for the obvious reason. Gates sit on the four cardinal
axes because that is where the roads already ran, and each carries a lockbox —
the permits system needed nothing new. A quarter of walled towns have let
theirs go, and a deterministic segment pass drops the wall in places, because
a breach is the thing a player actually remembers.

**One geometry bug worth naming.** The first cut pinned the wall to the
plaza's level and put the trace ten blocks past the core. Both were wrong for
the same reason: a fort's curtain runs a long way out, and out there the
town's plateau has already blended most of the way back to natural ground —
so the wall hung in the air downhill and buried itself uphill. And at ten
blocks the *re-entrant* angles of a six-point trace cut back inside the market
square. The wall now rides each column's own ground, and the standoff is
wider than a bastion plus the curtain's half-thickness. A test pins both:
nothing is ever placed below the plateau, nothing is ever cut above it, and
the inner face never crosses the core.

**And the banks.** Every town now has one, carrying the **Tier Three lockbox**
— the grade stage 11 built and left with a comment saying "endgame; nothing
stamps one yet". It took a building worth robbing to give it a subject.
Inside is a vault: a per-town strongroom, keyed by town centre exactly as the
economy keys its markets, so a vault and a market can never disagree about
which town they belong to.

It answers two things at once. Staging a trade run stopped meaning "carry
everything and sell it in one lump" — leave a load where you mean to sell it
and come back when the price moves. And there is finally somewhere to put what
you cannot afford to lose. Unlike nearly everything else in this world a
vault's contents are **not derived**: they are exactly what somebody put
there, which is what a bank is.

**Robbing one is the loudest thing in the game.** A breach bills by the grade
of what was breached, and Tier Three is stamped on exactly one building in a
town — so a vault door costs `BOUNTY_VAULT`, several times any other crime and
past the warrant threshold on its own. Picking the same lock quietly still
costs the quiet price: that asymmetry *is* the permits round, and Security 60
is the toll on the quiet road.

`Command::Bank` records **the amount that actually moved**, not the amount
asked for — a vault's capacity can bite mid-deposit, and a log saying "all of
it" while the world took two thirds is a divergence waiting for the next
replay. **Journal VERSION 14 → 15**: both kinds of change at once, since the
ground now carries forts and bank buildings as well.

**And everything got foundations.** Buildings had been sitting on dirt, which
made every lock in the game decoration: the way past a Tier Three vault door
was a hole in the floor. They are founded now the way real ones are — a deep
strip under each load-bearing wall, a shallow slab under the floor between,
depth set by what the building protects rather than how tall it is (a shed
runs two courses, the tower four, the bank's strongroom five). Paving runs
none: a plaza is a surface, not a structure, and four hundred hardness under
the market square would wall the town's own ground off from anybody who ever
wanted a cellar.

The footing shares one `FORTIFIED_HARDNESS` constant with the bunker shell
and the heaviest lockbox, because all three are making the same promise —
`Some(400.0)`, never `None`, so it is a cost rather than a wall. A test holds
them to it. Building claims now reach the bottom of their own footing, so
undermining is a permit crime as well as a long afternoon. Fort curtains are
founded along their whole circuit including the gateways *and* the fallen
segments, so a ruin leaves its foundation in the ground where the wall used to
be — which reads right and also means a breach is a gap you walk through
rather than a hole you dig under. **Journal VERSION 15 → 16**, since all of
that is generated ground.

**Deliberately not in 21:** anybody manning the walls, which waits for
hostiles; rubble where a wall fell, or any state between whole and dropped;
fees or interest on a deposit, so banking is convenience rather than a priced
decision; and towns reacting to their own gates being locked or breached.

---

## Shipped — Stage 22: the terminal

**The font's third user, and the game's first typed input.** Every readout so
far answers one question in one place: the HUD says how you are, the handheld
says where your machines are, a shop panel says what is on the shelf. None of
them answer *"what just happened"* — because the answer has been a toast that
fades in three seconds. That is fine for "you levelled up" and useless for
"the crew ran dry while you were down a cave". The terminal keeps four hundred
lines of scrollback, and every toast the game raises is written into it, so
the message you missed is still there when you come up.

**Typing was its own round's worth of work.** A caret that edits *behind*
itself, a command history the arrows walk in both directions, page-up
scrollback, and word wrapping at the panel's width. Characters come from the
platform's own text for the key rather than from the scancode — a scancode is
a *position*, and reading letters off positions is how a console ends up
unusable on half the world's keyboards. Anything the bitmap font cannot draw
is refused at the point of typing rather than stored, so no line can ever
render as a hole; a test caught the first casualty of that rule, which was the
`|` in the help text for `scout`.

**Orders go the long way round on purpose.** The parser is pure — it returns
data, not effects — and an order typed at the prompt is handed to the very
same call the keys use. The journal therefore only ever sees one kind of
dispatch. A console that recorded its own orders would be a second
implementation of every rule in this game and the first one to drift, which is
the same argument that keeps the replay oracle honest everywhere else.

It answers `status`, `fleet`, `where`, `pile` and `bank` from live state, and
takes `dig`, `cancel`, `survey`, `lights`, `save` and the scout's whole order
set. `help` lists them, and a test holds the help and the parser to each
other: the help may not list a verb the parser has never heard of.

**Deliberately not in 22:** scripting, aliases or anything that would let the
terminal replay itself; a command that does something no key can already do,
which would make the console mandatory rather than convenient; and completion,
which wants a wider panel than the font is comfortable in.

---

## Shipped — Stage 23: the townsfolk

**One agent, two policies — this round builds the first.** The people design
note describes a population whose civic half lives in the Stardew Valley and
Animal Crossing tradition and whose combat half (nerve, cover, surrender)
lands with the hostiles rounds. Both halves spend the same derivation: a
`Temperament` — archetype, nerve, warmth, voice — rolled once per person from
the site hash. The archetype that makes a clerk chatty at the counter is the
archetype that will make her slow to panic in a fight; nerve is derived now
and read two stages from now, so nothing about a person ever re-rolls.

**Identity is derived; the hometown is authored.** Every town seats three
named people — one per dwelling, the same beds the permits round mapped — with
trades in the town's speciality (a mine keeps a foreman, a powderman and an
assayer), two loved goods biased by trade, one hated good never loved, and a
birthday in a 28-day year. The hometown trio are people, not arithmetic: THE
MAYOR (proud, loves bars), THE SHERIFF (steady, loves fuel), and OLD PRAT
(chatty, loves logs, hates being handed bars).

**A schedule is worldgen for people.** `where_is(site, person, day, hour)` is
a pure rule stack — alarm, night, market day, work hours, an evening shaped
by archetype, a dawn stroll — with every boundary jittered ±20 minutes by the
person's own hash so the town never moves in lockstep. The villagers' wander
machinery is untouched: a schedule swaps the *patch* it wanders and drives
the same home routes the day/night cycle used to. Each town holds market one
weekday of its own; at noon that day the square fills, and a stakeout, the
kestrel and the roost all observe the same consistent lives.

**Friendship is a ledger, like loot.** What was given and said is remembered
as entries; the number is derived. Loved +60, liked +30, neutral +8, hated
−50; two gifts a week count and a third is taken politely and scored zero;
birthday gifts triple; a first conversation each day is +2; witnessed crimes
against a town push a negative entry to everyone who lives there, scaled off
the same bill the bounty board charged — law and disposition linked but
distinct. Tiers open things that already exist: prices talk at Acquainted,
bunker intel at Trusted (a bearing into stage 19's loot loop, earned by
talking), and at Close a key — an authenticate-rung grant to that person's
own door through the permit system, not around it.

**Gossip is telemetry wearing a coat.** Speech pools key on archetype and
context, and templates fill from live systems: the ore price is the market's
real price, the bounty line quotes the board, "the fleet is dry" is the
tank's own flag. The terminal (stage 22, and this is why it shipped first)
gives it all a surface: `who` lists the roster with tier and current
whereabouts, `talk` has a word with whoever is nearest, `gift <good>` hands
over one good off the base pile — and E with nothing solid in reach talks to
the neighbour in front of you.

**The oracle's line, drawn again.** `Talk` is journalled and replays as a
no-op — disposition lives in its own file (`friends.dat`, VXDS), like permits
grants. `Gift` is *not* a no-op: the good comes off the base pile, which is
state replay carries, so both sides run the same take. Journal VERSION 17.

**Deliberately not in 23:** the rain rule from the note's schedule stack
(this world has no weather to look up); person-policies emitting
`MoveCommand` through the player integrator, composure, cover scoring and
surrender — the note's combat half, recorded whole below for the hostiles
rounds; roster sizes that grow with the books (three per town until the
plans grow more dwellings).

---

## Shipped — Stage 24: the controller, and the Deck package

**Synthesis, not a second input system.** The game has exactly one
implementation of every rule input can reach: keys route through
`handle_press` and `InputState`, the mouse feeds a look accumulator and two
buttons. The pad plugs into those seams and nothing else — buttons resolve
to the `KeyCode` the same action is already bound to and go down the very
same dispatch, the left stick merges into the same movement axes the keys
drive (summed, then capped, so pad and keyboard cannot fight), the right
stick fills the same accumulator the mouse fills in the mouse's own units,
and the triggers mirror the mouse buttons. Every panel, the shop, the map,
the handheld and the terminal's scrollback gained pad support without one
of them changing, which was the entire design argument.

**Context is one bit.** A pad has fewer buttons than a keyboard has keys,
so the face buttons follow the console convention when a panel owns the
screen: south confirms, east backs out, the d-pad becomes the arrows. The
mapping asks a single question — is any panel open — and everything finer
stays downstream, where the keyboard already routes it. Releases look up
what the button meant *at press time*, so a panel closing mid-hold can
never leak a stuck key.

**Honest analog.** A radial deadzone with rescale: rest drift dies at the
threshold, and the live range re-spans from zero so a barely-tilted stick
is a genuine creep rather than a dead spot followed by a lurch. Tested
monotonic. Disconnecting a pad mid-stride releases everything it held.

*Scoped later.* The rescaled magnitude reaches flying and piloting, which
read the axes directly. Walking consumes a `MoveCommand`, which carries
direction bits and no magnitude, so on foot the stick gives eight
directions at one speed — see the arc's analog round, which is the journal
change that would lift it. (The stick did not walk **at all** until the pad
fix after stage 45: the walk sampler read the keys and never the axes.)

**SELECT is the manual.** The control scheme renders on the bitmap font as
a panel in the game, from the same table the tests hold drawable — not a
wiki page. A pad connecting says so in a toast and points at it.

**The Deck package.** `dist/install-steamdeck.sh` verifies the binary
against `SHA256SUMS`, installs to `~/.local/share/gamingg/` (no root,
nothing system-wide — SteamOS's root filesystem is read-only and stays
untouched), writes a desktop launcher, and prints the *Add a Non-Steam
Game* steps that put it in Game Mode's library. `--uninstall` reverses it
and keeps saves. The pad backend links `libudev.so.1` at runtime (shipped
everywhere, SteamOS included); building from source now wants `libudev-dev`.

**Deliberately not in 24:** rumble; gyro aim (Steam Input can lend it as
mouse input, which the game already reads); remappable bindings (one good
layout first, a settings file when someone actually wants a second); pad
text entry for the terminal (an on-screen keyboard is its own round);
journal changes of any kind — input synthesis happens above the seams the
journal records, so a pad session records the same orders a keyboard one
does.

---

## Shipped — Stage 25: the workshop

**Two doors onto one upgrade.** The fabricator has always argued that "the
counter is for people with money, the printer is for people with a mine".
Upgrades were the last thing that ignored it: three lines, credits only,
bought at a shelf. Now every line worth fitting has a *part* — DRILL HEAD,
CARGO RACK, PACK FRAME, LAMP REFLECTOR — printable out of ore and time, and
raising the same line the counter raises. Not a second upgrade system: the
same `wallet` entry, the same retroactive effect on machines already in the
field, one number.

**The rule this round discovered.** Adding printable upgrades turned up a
constraint worth writing on the wall: **a recipe's inputs are oracle state;
its refusal is not.** Replay re-runs `Command::Print` by taking
`recipe.inputs` off the pile, so a price that scaled with what you already
own — a wallet level, live-only state a replay does not carry — would have
the two sides take different amounts and the pile would drift apart within
one print. So the parts cost a flat price forever, and the *gate* stiffens
instead: each mark on a line demands `FLOOR_STEP` more Fabrication than the
last, and `refuse()` is only ever asked live. Two tests pin it — one proves
the charge does not move when the wallet does, one walks a line from zero
to five and checks the floor rises every time and that a sixth is refused
at any skill.

**Three new lines, chosen by the same rule.** Every effect had to be
something the journal does not re-derive:

- **PACK** — carry more before the weight tells on you. Legal because the
  player's load reaches the log as a *byte recorded in the `MoveCommand`*,
  not as something replay recomputes.
- **PRESS** — every print finishes sooner. Legal because print *timing* is
  live-only; the journal records the order and its replay arm moves the pile
  in one go.
- **LAMP** — a longer, stronger throw on whichever lamp you carry. Legal
  because it is a shader uniform and reaches nothing else.

A fourth candidate was cut for failing the same test: fuel efficiency.
Burning happens *inside* `Mining::advance`, which is exactly the call
`Command::Advance` replays, so a wallet-dependent burn rate would run the
crew dry at a different tick and leave a different hole. The rule earned its
first refusal before it earned its first feature.

**The press is the one line the counter will not sell.** Rollers for the
fabricator come out of the fabricator, which gives the machine a reason to
be stood in front of rather than ordered from. The panel now shows marks
per row (`3/5`), and the terminal's new `kit` verb prints the character
sheet the game never had: every line, what is fitted, what the next mark
costs in credits, and what it actually does.

Journal **VERSION 18** — no command changed, but five rows joined the
catalogue in ladder order and that renumbers the indices `Print` records.
The same reason twelve exists: recipe indices are content.

**Deliberately not in 25:** per-machine fitments (a drone that is *this*
drone, with its own parts) — that wants machines to be distinguishable
first, which is the wear round's job; recycling a printed thing back into
materials; upgrade lines that touch replayed arithmetic, now formally out of
bounds rather than merely absent; and a sixth mark on any line.

---

## Shipped — Stage 26: wear, and what mends it

**The first machine state that had to be oracle state.** Every other number
a player accumulates in this game — credits, upgrade marks, optics,
friendships — is live-only by design, deliberately outside the replay hash.
Wear cannot be, and the reason is the whole round: a worn crew digs slower,
a seized crew stops, and *how long a crew turned* is exactly what decides
where the hole ends up. So the ledger lives inside `Mining`, beside the fuel
tank, which is the struct `Rebuilt` carries — replay re-runs the same ticks,
re-derives the same wear, and the two sides cannot disagree about how much
ground got cut. Stage 25 said an upgrade may not touch arithmetic the
journal re-runs; this round is the other half of that rule, a system that
*must*, and so had to be built where the oracle can see it.

**A seized tick is a tick nobody works** — the same sentence the fuel loop
wrote, deliberately. Wear plugs into `Mining::advance` beside `fuelled()`,
one gate below the other, and reads as its sibling: fuel decides whether the
crew *can* turn, wear decides whether it *does*.

**Ticks worked, never seconds elapsed.** A machine parked in the garage ages
not at all; one that dug all night is ruined. A session recorded at nine
frames a second wears identically to one at three hundred, for the same
reason the journal counts ticks rather than time.

**Fresh → Worn → Failing → Seized**, at 4 400 / 8 800 / 13 200 worked ticks,
losing one tick in five, then one in two, then all of them. A pleasing
consequence falls out of charging wear only on ticks that *turn*: a failing
machine wears more slowly than a fresh one, because it is working less. The
last stretch of a machine's life is the longest, decay curves rather than
falls off a cliff, and a test pins the ordering.

**The worst machine sets the pace.** A crew works as a crew, and the duty
cycle comes off the worst condition in it rather than an average — an
average is something a player must be *told*, a worst is something they can
*see*. The roster names the bad machine, the HUD says so out loud when one
starts failing, and mending that one machine visibly hands the dig back.

**Recovery closes the loop the workshop opened.** Spare parts are a printed
good, low on the fabricator's ladder on purpose: a fleet that cannot be
mended is a fleet that dies of old age, and nobody should meet that wall
before they can print their way past it. `repair` at the terminal mends the
worst machine, or a named one. `Command::Repair` is journalled — the most
oracle-entangled order in the log — and both the live game and the replay
arm run one `Wear::repair`, so there is no second implementation to drift.
Journal **VERSION 19**.

**Deliberately not in 26:** per-machine fitments and parts (a drone that is
*this* drone); wear on the kestrel, which runs on a cell and a cooldown and
would be charged twice; machines that break *catastrophically* rather than
seizing; and repair as a thing you must physically travel to — you already
can pilot to a machine, and making it mandatory is a fetch quest wearing a
maintenance costume.

---

## Shipped — Stage 27: micro-on-damage

**Detail allocated by damage.** The world is still one metre. Worldgen, the
lattice, blueprints, forts, bunkers, flow fields, footings and every
block-denominated constant are untouched — a *composite* is a wound on the
existing grid, not a resolution increase. A block gains a 4³ interior the
first time something hits it and loses exactly the cells the hit removed, so
detail costs nothing anywhere the player has not shot and is spent precisely
where they are looking hardest.

**A damaged block is one `u64`.** Sixty-four cells, one bit each, at bit
`x + 4z + 16y` — each vertical layer a sixteen-bit slice, the whole wound one
register. Single-material by decree: chewed stone is stone with pieces
missing, because that is what damage *is*. Every operation is register
arithmetic — a carve is one `AND`, the death check is one `popcnt`, the
mesher's face set is six shifts and six ANDs, and even connectivity runs in
registers (dilate-and-mask to a fixpoint, ten iterations being a proof
rather than a guess, because the longest path in a 4³ is nine steps).

**SWAR, not `PEXT` — the Deck is Zen 2.** The obvious instructions for
bit-plane surgery are microcoded on Van Gogh, tens to hundreds of cycles,
fixed only in Zen 3. Everything here is plain shifts, ANDs and `popcnt`,
single-cycle everywhere this game will run — and, being integer arithmetic,
identical on every machine, which is what lets wounds ride the replay oracle
for free.

**Rays read micro; feet never do.** The DDA descends into a composite and
walks its cells, so `sight::obstruction`, projectiles and every cover query
see the hole: chip a wall long enough and a firing hole opens. Movement
collision stays coarse — a composite is a full box until it dies. You can
shoot through a peephole; you can never fall through one. That single
asymmetry is what leaves physics, `supported`, the flow fields and every
footing byte-for-byte untouched, and it is the reason this was one round
rather than four. Both halves are pinned by tests.

**The arsenal grew the mechanic by itself.** A slug bites instead of
deleting, and which cell it bites comes from where the round actually
stopped. Nobody wrote "make a hole": fire twice down one line and the second
round flies through the cells the first one took. That behaviour is now an
arsenal test, and so is its complement — spread fire still demolishes, so
the launcher stays a weapon rather than a chisel. Drilling carves the layer
nearest the bit each quarter of the way through: the same total work, the
same finishing tick, visible at last.

**No second vertex stream was needed.** The note budgeted for one, at
quarter-metre scale with its own uniform. It turned out `PackedQuad`'s
`kind` field had six unused values and micro faces need only three of `w`
and `h`'s nine bits, so cell faces ride the terrain buffer with their
sub-cell offsets in the freed bits — no new pipeline, no new buffer, and
every quad that existed before this round packs to the identical eight
bytes. A test mirrors the shader's own arithmetic in Rust and checks each
face lands on the cell it belongs to, which is a class of bug no test on the
mesher alone can see.

**Wounds converge; battlefields do not accumulate.** Below `DEATH_CELLS` the
block becomes air by the same path every other break takes; a mask above
`HEAL_CELLS` is nearly whole. Carving drops anything it knocks loose, so a
wound is always one connected component — asserted exhaustively, every shape
at every cell and face. Region saves carry the masks (**format version 2**),
raw rather than interned, because the note is explicit that saved masks are
a cache of what replay would recompute and a cache gets pragmatic
compression, not optimal compression.

**Found and fixed on the way:** the gold panel indexed a four-entry stock
array with five goods and had done since HHO landed in stage 20 — it only
crashes under `--features gold`, and recent rounds ran the no-default-feature
suite. The array is now sized by the catalogue, so the next good cannot
repeat it.

**Deliberately not in 27:** the intern table and its quad cache (the note's
own advice is measure first); consolidation's seasonal repair; the distant
LOD majority vote, which is written and tested (`lod_solid`) but not yet
wired to a range; multi-material wounds at a two-block seam; and gravel that
tumbles, which is physics this round refuses to buy.

---

## Shipped — Stage 28: hostiles, health, and the warrant

**Combat had a reason to exist before it had any code.** Crime has raised
bounty since stage 11, and bounty has crossed a warrant threshold that
nothing ever answered. Now it does: cross it and the town sends deputies.
That closes a loop five stages in the making — crime, bounty, warrant,
posse, arrest — and it needed no new worldgen, which is why the occupied
bunkers can still be their own round.

**They believe rather than know.** Each squad holds a last-known position
whose confidence decays, and an occupancy map that spreads probability
across walkable ground from there. Searchers walk to the likeliest cell they
can reach; everything they can see is zeroed. Sweeping a room, covering
ground and doubling back all fall out of the arithmetic — the map cannot
send anybody where the player provably is not, and when the mass runs out
they say so and stand down. It is the same model the kestrel's marks give
the *player*, which is the point: their intelligence about you ages exactly
as fast as yours about them.

**Nerve, not aim.** Composure is the variable the player reads and plays
against. Hits wound it, near misses suppress it, and watching a partner go
down costs more than either — so pinning a deputy is a verb with no new
system behind it. Thresholds are shifted by the `nerve` byte derived back in
stage 23, and archetype overrides the floor: a `Proud` deputy will not
surrender and a `Craven` one never bothers with cover. Character was rolled
once, at creation; this is the second thing to read it, exactly as the
people note promised, and two tests watch temperament show up in *outcome*
rather than in data.

**Cover is a query, not an annotation layer.** Occlusion sampled at three
eye heights — the same `sight::obstruction` call the roost uses to witness a
crime. Blocked standing is a wall to fight from, blocked crouched is
waist-high cover, blocked only prone is a last resort. It is re-scored every
frame, so when the belief moves the old score is stale and they scramble:
that *is* the flanking behaviour, with no flanking code in it.

**Nobody fires through a friend.** One check at the shot, allies counted as
blockers, and a deputy who finds themselves in a partner's lane sidesteps
instead. It is an invariant rather than a tuning goal — a test fires from
three hundred and sixty angles with an ally at the midpoint and never once
gets a round through. Its absence is the thing that reads as contempt in
other games.

**Health is quiet-then-mend, and down is arrested.** Six hits, no medkit
economy: break contact for eight seconds and the count climbs back, which
makes *disengaging* the heal. Fall to zero in front of the law and the
bounty is settled out of credits, the rest written off, and you wake at the
homestead. Dying to something that is not the law waits for a round with
something else in it.

**Live-only, and the law does not shoot the scenery.** None of this reaches
the replay oracle — deputies react to where the player is, and reactions are
not orders, the same line villagers, the roost and contact marks have always
drawn. Their rounds damage you and never the world, which is also the honest
fiction, since property damage is precisely what they are billing you for.
Combat therefore needed no journal version of its own.

**Found on the way:** the composure constants were still unused after the
first pass, which was clippy pointing out that the player could not shoot
*back*. Wiring the return fire is what turned a chase into a fight. The
deputies also walked in x and z alone and hovered off every bank until they
were snapped to the ground.

**Deliberately not in 28:** flow-field pathing, so deputies walk straight
lines and the watchdog abandons blocked approaches rather than going around
— visible the moment a callout happens inside a town, and the first thing
stage 29 fixes; the note's paired movement (one moves while one watches);
the arrest verb applied to *them*, since a surrendered deputy currently just
stops; and the hunt note's stronger search claim, which is kept as a named
ignored test because with one searcher it is not true, and its not being
true is what makes running and hiding a real option.

---

## Shipped — Stage 29: the shelters are held

**The loot loop grows teeth.** Bunkers have been free salvage since stage
19: find the hatch, walk down, strip the caches. Now every shelter is
*held* — a squad derived from the bunker's own seed, raised at its hatch
the moment the player comes near, running the very machinery the warrant's
deputies run: composure, cover scored at three eye heights, fire discipline
at the shot. Same bunker, same holders, every session; the roster is
worldgen like everything else about a shelter. A cleared shelter stays
cleared for the session — no watch respawning behind your back.

**Hostiles path now.** The debt stage 28 documented is paid the way the
hunt note predicted: not with a navmesh but with the flow fields the drones
have walked since the first swarm, pointed at people. A shared `Pathing`
rebuilds a bounded field on a beat toward the squad's *belief* — never the
truth — so a squad that has lost you routes toward where it thinks you are,
which is exactly as lost as it deserves to be. A test builds a wall taller
than the mantle and pins that the routed deputy arrives without ever
standing inside rock; its first draft caught two things instead — that
unrouted walkers glide through walls, and that the old ground-snap politely
mantled them over anything three blocks tall. The snap now applies only
near the surface, so an underground holder no longer teleports to the
meadow overhead.

**The director never tells the truth.** The stalker's discipline from the
hunt note, landed early: what a garrison learns from *noise* is a
32-metre zone centre, never a position, asserted at the interface. A shot
landing rings every shelter in earshot; a running drill rings them on a
beat — the dinner bell, attached to the core mining loop exactly as the
note wanted. Run the swarm loud and rich, or slow and quiet, or dig decoy
noise a valley over and work in the shadow of your own diversion. A hint
never overwrites fresh eyes, and noise past the leash is nobody's business.

**Pairs, bounded on purpose.** Squad tactics are exactly two rules: pairs
alternate — one moves while one watches, on a two-second clock — and a
pair that loses its partner takes the ally-down hit `rake` already
charges. Suppression arcs and command layers stay out of scope; the morale
system generates enough behaviour to read without them.

**The quiet track pays out.** Break a holder's nerve and their hands go
up; walk over and press E and they are *taken in* — the board pays by the
shelter's tier, 80/120/180 credits a head. A shelter can be cleared
without a single body if you can break everyone's nerve, which is the
payoff the people note promised the surrender machinery. Holders flee
*into* the shelter — the hatch is the bolt-hole — and a downed player is
not theirs to arrest: the shelter got what it wanted, which is you not
coming down the stairs.

**Found on the way:** neither stage 28 capture fixture ever uploaded its
camera — the posse's overhead framing had silently rendered from the
default camera all along, and the held-shelter fixture is what exposed it.
Both upload now.

**Deliberately not in 29:** garrisons *inside* the works — holders fight
around the hatch and bolt down it, but room-to-room interior fighting
wants the mini-DDA's cover queries against bunker geometry proven first;
the stalker itself, which waits for the deep resources round; capture
contracts as board postings with names and prices (the pay is flat by
tier this round); and any journal change at all — held shelters are
live-only, like every reaction to the player.

---

## Shipped — Stage 30: factions, and the memory the board does not have

**Reputation is the ledger arrest cannot settle.** Bounty is a *bill*: it
accrues per crime, a warrant answers it, and the arrest wipes it. Standing
is what everybody remembers anyway. Pay every fine you like — a county
that has watched you breach four vaults trades with you accordingly, and a
shelter whose neighbours you dragged to the board does not care that your
paperwork is in order. Bounty and standing stay linked but distinct,
exactly the split disposition and law made at the townsfolk round, one
level up.

**Two factions, because two exist.** The **Compact** is the settled towns —
already one people in every mechanical sense: one lattice, one beacon
network, one economy hauling between them, one warrant that follows you
town to town. The **Holdouts** are whoever holds the shelters. Nothing else
in this world flies a flag, and a faction system with more factions than
peoples would be a menu, not a world. The file grows by adding a field.

**One deed, two opinions.** A capture pleases the towns (+25) and damns
you to the shelters (−40); a kill is worth less to the law (+8) and costs
more to the shelters (−60), because the board parades captures and graves
are quiet — an ordering pinned in `const` blocks so a rebalance is a
decision, not a drift. Witnessed crimes cost Compact standing at the
bounty bill over five, the disposition ledger's own scaling. Honest trade
is a trickle (+1 a sale) and takes a season to matter, which is the point.

**What standing buys.** Bands are seasons, not moods — Enemy, Cold,
Neutral, Warm, Friend at ±100/±400. The Compact's counters shade their
prices a few percent either way, never a cliff. Neutral-or-better with the
Holdouts buys the **truce**: a shelter's sentries challenge a stranger —
"WALK ON - THIS GROUND IS HELD" — and grant a grace period instead of a
volley. The truce ends at the inner ring, at the end of an ignored grace,
at any noise on their ground (a drill inside the leash is not a stranger
passing through), or at the first round fired.

**The spoofers arrive in their hands.** The arc has promised it since the
intrusion round: a shelter with a grudge runs a coil of its own, and
inside its leash the kestrel's contact marks simply do not take — "THE
SHELTER IS JAMMING YOUR SCOUT". Only grudged shelters jam, because running
a jammer is itself a declaration and a truce-holding squad would not tip
its hand. The tool the player learned to fear the roost with is finally
pointed back.

**Surface.** `standing` at the terminal prints both names, the points, and
the effects in force; band crossings land as toasts ("THE SHELTERS NOW
CALL YOU COLD"); `reputation.dat` (VXRP) persists it all, live-only —
the replay oracle never learns anyone had a name.

**Deliberately not in 30:** positive Holdouts standing — nothing can raise
it yet, because there is no fence or black market to be their side of the
counter; that door opens when their economy exists. Standing decay toward
neutral over seasons. Per-town grudges under the Compact umbrella (the
county shares one opinion; per-person disposition already covers the
local texture). And elections — trade goodwill as a vote stays with the
civic layer.

---

## Shipped — Stage 31: uranium, oil and gas, and what came up with them

**Three resources, and only one of them is an ore.** Uranium joins the ore
lattice as data — a band constant and a kind in the table, exactly what the
first ore module promised the second one would cost. Oil and gas are not ore
at all: they are bodies hundreds of blocks across on their own lattice eight
times coarser, stamped into the deep stone as saturated rock, and worthless
one block at a time. That difference is the round. A vein is a thing you cut;
a field is a thing you *stand a machine over and leave*.

**The well is the first machine that keeps paying.** Printed at the
fabricator, carried out, spudded in for casing and cement charged up front,
drilling on the journal's clock at fourteen ticks a block of depth, then
lifting a unit every four seconds into the base pile until the body is empty.
Everything about it is constant, because it lives inside `Mining::advance` —
the wear lesson from stage 26 and the tank lesson from stage 20, restated: a
machine whose arithmetic decides what is on the pile decides how long the
fleet turns, and how long the fleet turns is what the world hash covers. So
`Spud` is an order the log carries, the world is consulted exactly once at
the spud, and both sides sink the same hole.

**A dry hole is a place, not a roll.** `reservoir_under` is pure in the seed
and the column, so the same ground is dry in every session of a world. The
panel shows whether the mud log has a trace before the casing is spent; what
fluid, how much, and how deep are what the drilling buys. A duster costs
exactly what a strike costs, which is the whole weight behind the word
*commitment*.

**Gas is the second fuel, and honestly worse.** A canister of well gas is
worth three fifths of a canister of oxyhydrogen. The tank reaches for the
good stuff first and falls back on gas by itself, so a player who went to the
trouble of splitting water never has it quietly burned. What gas buys is
independence from lakes — the first answer this game has offered to "I am two
kilometres from water and out of fuel" that is not "walk back".

**Uranium costs you something to be near.** Exposure is a sum over the bare
uranium within five blocks, falling with the square of the distance, so every
lever is physical: back off, wall the face up, send a machine, or print the
lead lining — which is multiplicative and floored at a third, because
shielding that reaches zero is not shielding, it is permission. Dose spends
health, health has been live-only since stage 28, and none of it reaches the
oracle: two sessions with identical journals can end with different dose and
the same world hash.

**The stalker lands, and it is two brains.** The hunt note's last part,
built as the note specified. A director that knows the truth and may only
emit `HINT_GRADE` cells — the same thirty-two metre quantiser the shelters'
director uses, so the rule about lying to your own monsters is written once —
and a creature that closes the rest with `belief.rs`, the same occupancy
search a posse runs. Hints are weighted by noise at the note's `HINT_NOISE_W`,
and machines are the loudest thing in the world, so the tension attaches to
the mining loop rather than to a corridor. Heat fades, and four times faster
in daylight. Nothing arrives inside `NO_SPAWN_R`. Ninety seconds of contact
force a sixty-second break, which is the difference between a monster and a
rhythm. Every mode transition lands a line, because intelligence a player
cannot perceive is intelligence wasted.

**Journal VERSION 20, and it is two changes at once** — the ground moved
(uranium below the overburden, fluid bodies in the deep stone) and `Spud`
joined the commands. The same shape as eight and fifteen.

**Surface.** `wells` at the terminal lists every hole and what it is doing;
the wellhead panel is one row and one key, because a hole is a decision
rather than a menu; the HUD carries the dose and what is out there in the
dark; F3 gained the deep's heat, the dose and the well count. Three new
capture fixtures — and a third variation on the fixture trap, recorded here
so the next round does not find it again: `set_objects` culls against the
*last uploaded frustum*, so a scene built before the camera is pointed at it
is a scene the culler quietly throws away.

**Deliberately not in 31:** any fluid simulation — nothing flows, and a
reservoir is rock with something in it; refining oil into anything (it is a
trade good and the towns want it); wells that can be worked by the fleet
rather than by hand; more than one stalker, and anything it does to the
world (it damages the player, never the rock); uranium in the fabricator's
catalogue, which waits for something worth enriching it *for*.

---

## Shipped — Stage 32: faces, walls and a ward

**A face is five cubes and it is worth more than any five cubes in this
game.** The townsfolk have turned toward what they notice since stage 7 — the
work was all there — but a bare skin block for a head reads as a mannequin, so
none of it showed. Two eyes, two pupils and a flat mouth, and the pupils take
their gaze from *the same sighting the body turns toward*: one perception,
two tells, no second look at the world. The gaze **clamps** rather than
tracking all the way round, which is the whole of "roughly" — past about
thirty-five degrees the eyes saturate and the body has to do the turning,
which is exactly what a person does and what a googly toy does not.

Structurally it is one flag on a rig part. `Part::follow` moves with a gaze
and nothing else does, so every other rig in the game keeps calling
`objects()` unchanged and the deputies — who wear the villagers' rig —
inherit a face for nothing.

**A grunt is the other half of being crowded.** Under one and three quarter
blocks somebody grunts and shifts aside: three synthesized voices rather than
one recording, no line of sight required (being trodden on is not something
you watch coming), once per approach on the greeting's own hysteresis. It
lands as a toast as well as a sound, because a machine with no audio device is
a supported machine here and a tell that only exists in the speakers is a tell
half this game's players never get.

**Every town is walled now.** The star forts of stage 21 tiered a hamlet down
to `Open` — nothing at all — which left most of the map reading as sheds in a
field. The floor is now a **mini star**: four short bastions, a wall four high
and three thick, standing off nine blocks instead of fourteen, with the same
ditch, the same four gates on the four roads and the same lockbox on each. Its
own `standoff`, `height`, `half` and `ditch` come off the trace, so it is the
same geometry built smaller rather than a special case. `Trace::Open` was
deleted rather than left unreachable — a variant nothing can produce is a lie
in a table, and this file has held that line since the schedule dropped its
rain rule.

Four high rather than three, and the reason is the gate: a gateway is a
four-block opening with wall carrying on over it, so a three-high trace would
have to choose between a door you cannot walk through and an arch that is not
there.

**And there is somewhere to go when you are hurt.** A clinic in every town
plan — a nine-by-seven ward with two cots and a Tier Two lock, claimed like
any other building. The cot is free and it also **scrubs the dose**, which is
the door stage 31 deliberately left open: standing in a uranium face is a
bargain you take knowing there is somewhere to walk to afterwards.

Free is the right price because the walk is already the cost. A bed in town is
worth nothing at the bottom of a shaft, which is what makes the **medkit**
worth its forty-five credits — bought at the same counter, carried, spent by
the terminal's `patch` wherever you are, two hits back and never wasted on
somebody already whole.

**No journal change belongs to the ward.** Medkits are a count in a pocket
rather than a good on a pile, credits have always been live-only and health
has been live-only since stage 28, so the oracle never hears about a hospital.
**VERSION 21** is the *ground* moving twice: a wall around every town that had
none, and a building on plot that used to be open.

**Surface.** `patch` at the terminal; the ward panel on the electrolyser's
shape; F3 gained the medkit count. Three capture fixtures — and the ward's
taught its own lesson: a two-block-high shed photographs as a sheet of
corrugated metal from the inside, so the picture is the door somebody walks
through.

**Deliberately not in 32:** expressions — a mouth that moves is animation this
rig has not got, and a flat line at least never smiles at the wrong moment;
blinking; gaze for the deputies and holders, who wear the face but have not
been asked where they are looking; villagers who *staff* the clinic; illness
or injury that needs more than a bed; and a bill for the cot.

---

## Shipped — Stage 33: the handheld you hold

**The most-used object in the game had no object.** The fleet roster, the
country map and the scout's orders all live on the handheld, and it was a
rectangle that appeared in the middle of the screen. Now it is a cased unit —
bezel, glass, two dials, a stub aerial, a strap over the forearm — that swings
up into view on a third of a second and drops back out when you put it down.
The information did not change at all. What changed is that consulting it is a
*gesture*, with a cost in the moment it takes.

**The screen is on the model, and the maths is the frame's own.** Every frame
the four corners of the glass are put through the same
`Camera::view_projection` the world was drawn with, and the readout is placed
in the rectangle that comes back — so looking around while it is up carries
the screen with the thing holding it, because it is *on* it. One set of
numbers describes the glass (`rig::screen`), used both by the rig that builds
the plate and by the projection that lands the readout on it, so the two
cannot drift apart.

**Fitted, not stretched.** The case is tipped toward the face, so the glass
projects as a trapezoid and the box around it is wider than the face. Filling
that box would squash the text by however far the unit happens to be tilted,
so the readout is letterboxed inside it at its own aspect instead. A real
screen does the same thing with the wrong shape of picture.

**Both hands are busy.** The drill and the launcher are not drawn while the
unit is up. That is the whole reason the round is worth doing: checking where
your drones are with something coming down the gallery at you is now a
decision, where a panel that appears was a free pause.

**Live-only, and cheap.** A raise fraction on `Device`, four pure functions
over numbers, and nine cuboids on the existing instanced path. No renderer
change, no new pipeline, no journal anywhere near it — the alternative was
rendering the readout to a texture and sampling it on the quad, which is a new
render target, a second bind group, and blurrier text at every angle.

**Surface.** `V` raises and lowers it; third person, which draws no
viewmodel, falls back to the centred panel rather than floating a screen where
no unit is; the `--device` capture gained `--raising` for the mid-swing frame.

**Deliberately not in 33:** the unit in the player's hands in third person; a
world-space screen texture; the unit visible while stowed; buttons on the case
that do anything; and any change at all to what the readout says.

---

## Shipped — Stage 34: the pocket arcade

**The oldest item on the list, and it needed stage 33 first.** A game inside
the game is a joke when it opens as a window over the world. It is a different
thing entirely when it runs on the screen of a unit you raise into your hands:
a machine you are holding, in a world that carries on around it. So the toy
waited until the handheld was an object, and this round slots a cartridge into
it.

**It is not, and will never be, a Doom port.** The licensing section has
carried that rule since long before the stage arrived. WADs are not
redistributable and a GPL engine would encumber the binary, so every wall,
every corridor, every enemy and every pixel of the status strip is computed
here — the same answer the terrain, the tiles and the audio already give.
What it borrows is the *technique*, which is nobody's property: one ray per
screen column, a grid walked cell by cell, and the reciprocal of the distance
for the height of the strip.

**A floor is a number.** `Level::of(seed, floor)` carves a 24×24 grid with the
same splitmix hashing the world uses, in one continuous two-cell-step walk —
so the map is connected *by construction* rather than by generating and
checking, and the flood fill that proves it exists only in the test. Pure in
`(seed, floor)`: the same cartridge deals the same floors in the same order,
which is the whole reason a high score means anything.

**The loop tightens.** Reach the exit and the next floor comes with more of
them, moving faster, and less ammunition on the ground — floored so that no
floor is unwinnable before it starts. A kill pays two rounds back, so pressing
forward is what keeps you loaded and hiding is what runs you dry. Die and the
run ends; the cabinet keeps the best score and the deepest floor, and nothing
else.

**Earned, not given.** The cartridge is a fabricator print at a high floor of
the ladder, in bars, ore and a plank. Nothing about the world unlocks it and
nothing about it touches the world: no journal command, no clock, no pile, no
economy. `arcade.dat` (magic `VXGM`) holds three facts — cartridge owned, best
score, deepest floor — and a save with none of it plays exactly like a save
with all of it, minus the toy.

**Keys only, which is the Deck for free.** The buttons come from
`InputState::is_down`, so the game plays with the mouse released — the way a
panel behaves — and the pad reaches it through the key codes
`gamepad::key_for` already synthesises in panel context. While the arcade page
is up the rest of the keyboard is deliberately deaf: only `Tab` to leave the
page and `V` or `Escape` to put the unit away.

**Surface.** Print `POCKET ARCADE` at a fabricator, raise the handheld with
`V`, `Tab` round to the arcade page, and play: `W`/`S` walk, `A`/`D` strafe,
the arrow keys or `Q`/`E` turn, `Space` shoots, `Enter` starts. Without a
cartridge the page says where to get one, exactly as the kestrel page does
without a scout. `--arcade` captures it: the unit raised, a corridor, and
something with two eyes coming down it.

**Deliberately not in 34:** sound of its own; a second weapon; anything the
arcade does to the world outside it (no credits, no skill, no standing); saved
runs — the cabinet remembers the record, not the game in progress; and any
asset that is not computed.

---

## Shipped — Stage 35: three forests

**Every tree in the world was the same tree.** One lattice, one shape, one
green. The country underneath it had rivers, coasts, ridges and a hundred and
eighty blocks of relief, and none of it changed what grew. This round the
forest reads the ground.

**Two fields decide it, and both come off the terrain that already exists.**
How high a column is, and how wet. Wetness is the hydrologist's index with the
non-local half taken out: the real one is `ln(a / tan β)`, where `a` is the
upslope area draining through a point — and area accumulated over a catchment
cannot be a pure function of a column, which the house rules do not allow.
What survives is the shape of the idea: water gathers where the ground is
*flat* and *convergent*, so curvature over slope says most of what the index
says and says it from five height samples. A slow water-table field is added
on top, because some flat country is wet for reasons its shape does not show.

**The three forests, and what each one is.** A **peat bog** in the flat
convergent lows: thin crooked black spruce over a sphagnum carpet, crowded
with stems you can see straight through. A **mixed hardwood cove** through the
middle: broad crowns, an open floor, and one cell in thirteen carrying an
**emergent giant** standing a head above the closed canopy — the tulip poplar
of a real cove forest, and the only tree in the game you can pick out from a
ridge away. **Subalpine conifer** up high: tapered spires that get narrower
and darker with height, thinning at the **treeline** into knee-high
**krummholz** mats, and stopping altogether above the tree limit, which is
what finally makes a summit read as a summit.

**The bands are not contour lines.** A low-frequency field wobbles the
elevation thresholds by a few blocks, so the treeline wanders the way a real
one does instead of tracing a level curve around a hill. And cold air drains
downhill: a deep hollow counts as higher ground than it is, which is why
spruce fingers down the drainages into hardwood country. That is one term in
one function, and it is the single change that makes the map look like it was
grown rather than banded.

**The ground says it too.** A bog carpets itself in sphagnum instead of grass,
and grass tufts are a hardwood-country thing — the moss has the lows and the
high country is too cold and too thin for a lawn. You can tell which forest
you are standing in without looking up.

**Purity held, and it had to.** Which forest a column grows is read off the
*natural* height field, never the town-blended one: a market square's plateau
must not decide what grows on the hill behind it. Nothing is stored, both
sides of a chunk border derive the same answer, and the canopy has no seams.
The journal moves to twenty-two, because ground that changed is ground a
replay would drive a crew through.

**Surface.** `--forest bog`, `--forest cove`, `--forest high` and
`--forest treeline` each go and *find* a stand — walking out from `--at`
looking for a column of the right forest with more of the same around it, out
of town, and framing it from downhill so the trees have sky behind them.

**Deliberately not in 35:** felling anything; the succession clock and the
disturbance ledger the note wants (nothing yet disturbs a stand, so a ledger
would have nothing to hold); riparian corridors; wave-regeneration bands; and
pit-and-mound microtopography, which needs treefall first.

---

## Shipped — Stage 36: felling

**Stage 35 grew three forests and nothing could touch them.** A tree was
scenery. This round it is a thing with consequences: a cut that has to be
aimed, a fall that goes where the cut says, and a mass that flattens whatever
is under it.

**The thirty per cent rule turned out to be real forestry.** Directional
felling puts the face notch at 15–33% of trunk diameter and the hinge at about
a tenth: the notch *aims* the tree, the hinge *steers* it, and when the cut
takes the holding wood past the hinge the stem cannot carry its own lean any
more. All of that landed on machinery that already existed — stage 27's
sixty-four-cell mask **is** the trunk's cross-section, so "a third of the way
through" is a popcount and `Shape::Notch` is the cut. The notch is a wedge
that starts low, in the middle, against the struck face, and drives a slot
through the middle of the stem before it takes the corners — which is why the
wood left holding a felled tree is at the edges, exactly where a feller finds
it.

**One honest compromise, stated rather than hidden.** Four cells across a
block makes a hinge of a tenth of the diameter *a quarter of one cell*, which
the mask cannot say. So the rule is written as two conditions that measure
different things — how much of the whole section has gone, and how much of the
far layer is still holding — and calibrated so the stem drops at about two
thirds cut. The note's own advice about impact energy applies to its own
felling numbers: compress the span, keep the ordering.

**It falls toward you.** The notch faces the way the tree goes, and the notch
is the face you are standing at. Lean biases it, and a hard leaner cut against
its lean **barber chairs** — the trunk splits and goes where it is heavy
rather than where it was aimed. Cut low: only the bottom two blocks of a trunk
are a stump, and drilling higher up still just takes a block off a tree.

**The arc is kinematic, and that is the determinism decision.** Float
rigid-body integration diverges across runs and machines and the replay oracle
would not survive it. A trunk rotating about its hinge under the honest
pendulum — `3g·sinθ / 2L`, one line, and it is the whole of the physics — is a
pure function of the tree, the direction and the tick. Tall stems go over
slowly and saplings snap down, for the reason they do in the world.

**Shaped exactly like the arsenal, because the arsenal already solved this.**
The world edits happen *inside* `advance_falls`; the sweeps it returns carry
the live-only half. Live and replay call the same function on the same 64 Hz
clock, so the same blocks change in the same order — and the sweeps the replay
gets are dropped, precisely as the slugs' are. `Command::Fell` records the cut
and nothing else: the stump and the face. Where the stem swept, what it
flattened, which neighbour came down with it and where the logs came to rest
are all re-derived.

**What it does on the way down.** Foliage, soil and planks give way; rock and
steel stop it dead and the stem is **hung up**, which is a real morning in the
woods. Another trunk in the arc comes down too if the incoming energy is
enough — the domino, resolved inside the same call so a replay sees it the
same way. Impact energy is the note's own arithmetic, `E = m·g·h/2` with
`m = π·r²·h·ρ`: about 1.6 kJ for a sapling, 150 kJ for a mature stem,
1.9 MJ for an old-growth giant, and you are hit for the compressed version of
that span. A tree coming down is also the loudest thing in the woods —
everything that listens hears it, including whatever is out in the dark.

**Ancient trees.** One emergent in nine is older than anything built near it:
hash-selected on the same lattice, at a hardness tier that wants the good
drill rather than the starter one, and yielding **prime timber** — heartwood
the fabricator mills into three times the planks. Their promised fire immunity
waits for fire, in 38.

**Surface.** Hold the drill low on a trunk. `--fell swing` catches a stem
mid-arc; `--fell down` catches the stump and the line of logs afterwards.
Journal **VERSION 23**.

**Deliberately not in 36:** a felling saw of its own (the drill does it, and a
faster tool is a later round's reward); stumps that regrow; timber grades
beyond ordinary and prime; the fall crushing a *building* differently from a
hillside; and any change to what a standing tree looks like.

---

## Shipped — Stage 37: water that moves

**Water was painted on.** You could drive a gallery into the bottom of a lake
and the lake would sit there. This round it comes in after you.

**The fill level is the damage mask, and that is the whole design.** Stage 27
cut every block into sixty-four cells so a wounded block could say what was
missing. A wet block now uses the same sixty-four to say how full it is: one
`u64` in the sparse map the chunk already keeps, `popcount` for the volume,
and the mesher already draws masked blocks — so a half-full block comes out as
a slab with a surface on it, with no new pipeline and no second vertex stream.
Sixty-four steps of fill against the seven or eight a level-per-block system
gives, for nothing.

**The layout is canonical, which is the trick that makes it cheap.** The mask
fills from the bottom layer up, so a part-filled block is always flat-topped
and the *only* state is the count. The automaton moves integers and rebuilds
masks; it never reasons about which sub-cell went where, and two blocks
holding the same volume are the same block. That is the note's own
compromise — store the fill as a popcount, treat the layout as settled flat —
and taking it is what kept the round to one module.

**The sea is a source; everything else is conserved.** A full water block at
or below sea level supplies without draining, because the ocean is larger than
anything this automaton is allowed to touch and one player with a drill should
not be able to empty a coastline. Above the line every drop is honestly
conserved: cut a channel out of a pond and the pond goes down by exactly what
ran out, which is what makes draining a cave pool to get at its floor a real
thing to do.

**Determinism comes from the update order, not from luck.** The wake set is
kept sorted, so the sweep visits cells in one canonical order however they
were woken; the step is split into two passes by cell parity so a cell never
levels against another of its own colour mid-pass; nothing asks the clock or a
thread. Shuffle the wake list and the water ends up in exactly the same
place — there is a test that does precisely that.

**And it is bounded on purpose.** A body flows no further than twenty-eight
blocks from where it woke and retires after eight quiet steps. An unbounded
automaton would wander out of the ground a replay has loaded and the two sides
would disagree, so this is bucket-scale water: flooding a gallery, draining a
pool, filling a cistern. Redistributing an ocean is not a thing that happens
here, and the source rule is the other half of that same decision.

**Nothing new to record.** Cutting into water wakes it, and the break is
*already* an order — so both sides wake the same cells on the same tick and
the flood that follows is re-derived rather than replayed. The tick runs
beside the slugs and the falling trees, in the same shape for the third time.

**The pump.** A fabricator print at floor three, placed out of the pile, and
`E` switches it: one state, so no panel — a machine with a single switch does
not need a screen to say what it is doing, it says it in the water coming out
of the top. It takes from whatever it can reach and puts it out of a spout
above itself, and the automaton carries it from there. No facing and no
plumbing, and it does the one thing gravity will not: move water *uphill*, to
fill a cistern on a rise or get a gallery back that you flooded on yourself.

**Surface.** `0` selects the pump on the belt, `E` runs it. `--flood cut`
catches the moment a gallery is opened into the sea; `--flood level` is the
same gallery once the water has found its level. Journal **VERSION 24**.

**Deliberately not in 37:** pumps that survive a reload (a body of water in
motion is not saved either, and both settle in seconds); pressure of any kind,
so no siphons and no fountains; water that pushes you or slows you down;
anything that freezes; and the height-field surface mesh the note wants —
partial blocks ride the micro path stage 27 already built.

---

## Shipped — Stage 38: weather, fire and what comes back

**The forest note's last item, and the one it argues everything else was
for.** 35 grew the forests, 36 gave them a way to come down, 37 set the water
moving. What was missing was the thing that disturbs a stand with nobody
standing there — and the thing that heals it afterwards, without which a burn
is a scar rather than a cycle.

**The sky is a pure function of `(seed, tick, region)`.** No stored front, no
side generator, nothing to save. Coarse regions about 512 blocks across are
sampled off the same signed noise the water table uses, at
`(region, tick / PERIOD)`, and lerped between steps so a front crosses the
country rather than snapping into place. The sample point itself is advected
by a slow drift, which is what makes a storm *travel*. Minecraft's weather is
famously not reproducible from its seed; here it costs nothing to make it so,
and everything downstream — the rain, the strikes, the fire — inherits that
for free.

`Conditions` carries temperature, humidity, a wind vector, a rain fraction and
one of four states — clear, cloud, rain, storm. `fuel_moisture` back-samples
six weather steps with a decay, so the woods stay wet for a while after the
rain stops and dry out slowly, which is the single number the whole fire half
reads.

**What you see and hear is the uniform that was already there.** A storm greys
the sky colour, takes the sun off the hills and lifts the ambient floor — the
same three values `clock::sun_uniform` has always written, so an overcast noon
reads as overcast instead of as dusk, with no shader change. The rain rides
the instanced object path the falling trunk rides: a fixed sheet of thin
streaks around the eye, each one's position derived from its index and the
clock rather than spawned, slanted along the drop's own velocity so a gale
visibly leans the weather over. Thunder is a cue beside the launcher's, and a
strike is *loud* — the garrisons hear it, the stalker hears it, and the
townsfolk startle, the same three ears felling already fed.

**Rain is a source term on the water that already moves.** No second system:
while it rains, an exposed column near you takes a few cells through
`fluid::set_level` and wakes a body exactly the way a broken block does.
Hollows fill, the surplus runs downhill, and it drains away after. The note
offers Priority-Flood for unbounded pooling; this fluid is bounded by design,
so it is not needed.

**Lightning goes for the tall and the lonely, and mostly does nothing.** A
strike is hashed off `(seed, tick)` inside a storm, on its own slow clock; the
column it takes is the highest thing in a small neighbourhood, so an emergent
giant or a ridgeline spruce takes it and the stand below does not. Whether it
lights anything is the real statistic: roughly one in fifty, scaled by how dry
the fuel is and by what it hit.

**Spread is Rothermel's shape, not his parameter set.** A neighbour's ignition
chance is `p0 · fuel · (1 + φ_w + φ_s) · dryness`, wind and slope as
intensification factors, so **fire runs uphill and downwind** and the
direction of maximum spread is the vector sum. Per species, faithful to the
note: bog needles and spruce foliage go up like kindling, hardwood leaves and
logs resist, planks and roofs burn at a middling rate — and **ancient wood
does not burn at all**, which is the promise stage 36 made when it put
ancients at the top of the hardness ladder. Wood burns *wherever it stands*:
your house, a town's plank walls, the fabricator you left in a clearing. A
firebreak is a thing you cut, and an ancient grove is the safest ground on the
map.

**The ledger and the clock, which the note wrongly assumed already existed.**
`succession.rs` stores only *disturbed* stands, keyed by lattice cell, holding
when they were disturbed and how many stages have been stamped — untouched
forest stays a pure function of the seed and costs nothing, the same sparse
trick the damage mask uses for wounds. The clock is the note's four stages —
meadow, thicket, mixed, old growth — advanced on the day clock and *written
into the world as blocks*, so worldgen stays pure and regrowth is an edit like
every other. Each stage is the same tree the seed describes at a fraction of
its height, so a stand comes back as itself and the last stage is bit-for-bit
what worldgen would have grown. Black spruce comes back fastest, hardwood next
and subalpine slowest, which is the note's ordering rather than its absolute
durations. Felling writes the ledger too: stage 36's stumps are the first
thing that grows back.

**The oracle, for the fourth round running.** None of this records an order.
The weather is pure in the tick, the strike is hashed off the same tick, and
the player's position — the only other input — is already replayed. So both
sides call one shared function per tick, `journal::burn_and_grow`, the world
edits happen inside it and the reports carry the live-only half. The test that
matters is the one that has mattered since 36: let a strike land, let it burn,
replay the log on a fresh world, and the region hash matches.

**Surface.** No new key. `WEATHER` at the terminal reads the sky, the wind
bearing, how dry the fuel is, what is alight and whether the ground you are
standing on is still coming back. `--storm` catches rain over the country with
the sky down; `--fire burning` catches a stand alight running upslope, and
`--fire after` the ash and snags it left. Journal **VERSION 25**.

**Deliberately not in 38:** crown fire as a separate state, ember spotting
downwind, smoke of any kind, wind that pushes you or the drones, snow or
freezing, and lightning that hurts you directly. The note's absolute
succession durations are compressed by two orders of magnitude, on purpose —
the sequence is faithful, the calendar is not.

---

## Shipped — Stage 39: towns that work, and the warrant chain

**The civic layer's B2, outstanding since stage 11.** Everything it needed was
already here and none of it was joined up: offices existed but only the player
could hold one, residents had trades and schedules that meant nothing to the
economy, and the law was a single comparison — `bounty >= WARRANT_THRESHOLD`
and four armed men appeared over the hill.

**Every town has a mayor and a sheriff, and they are people.** The seat is a
roster index hashed off the town's own seed, so two machines agree on who runs
a town four kilometres away without either having been there, and nothing is
stored. The two are drawn without replacement — a man cannot be the sheriff who
asks himself for the warrant — and the seat is its own draw rather than the job
title, so a town where the powderman is mayor and the foreman wears the badge
is a town with a story in it. The hometown keeps its authored pair, because the
names `THE MAYOR` and `THE SHERIFF` were already on them.

**The residents work, on the hours the schedule already gave them.** The market
integrates in whole 240-tick steps and lands on a boundary, which is what makes
"integrate to 5 000 then to 10 000" identical to "integrate to 10 000"; the
people ride that clock rather than inventing one. Each step samples
`schedule::where_is`, and a resident at work puts their trade's own good on the
books while everyone burns their board awake or asleep. Per trade rather than
per town, so a town's three residents are three contributions instead of one
rate tripled — and a tallyman counts the pile without adding to it, because a
town whose people all produce is a town with no services in it. Their purses
are derived the same way the forest and the weather are: shifts worked times
what their hands are worth, pure in `(seed, index, tick)`, nothing written
down. Turn up on a market day and the shelves genuinely differ from four in the
morning.

**Trust is what you buy with business, and it is not friendship.** A fourth
entry on the ledger that already exists — a second file would have been this
one again, the same sparse map, the same key, the same save — counting toward
the friendship total but read separately. So a stranger you have made rich can
be handed a key to their door without ever having been liked, and somebody's
oldest friend is not automatically their supplier. Version two of `friends.dat`
loads version one rather than discarding it: throwing away friendships somebody
earned would have been the lazy reading of "tolerant".

**And the warrant chain, which is the point.** The sheriff files when the
bounty crosses the threshold; the mayor decides on the spot from their own
archetype, your tier with them, your trust and how far past the line you are.
A proud mayor signs because the law is the law; a craven one stalls; a mayor
you have been feeding gifts to for a season leaves it in a drawer. It buys
**time, not impunity**: past the price the permits round put on a vault, nobody
signs any differently. A refusal carries an expiry, so the sheriff may ask
again — a reprieve rather than a pardon. Deputies now muster on the
*signature*, never on the bounty.

**Short of force**, exactly what the note asked for: a fine debited once when
the paperwork is filed — and what the wallet cannot cover goes back onto the
bounty, so being broke and in trouble is worse than being solvent and in
trouble — and the town's counter closed to you while anything stands. The
embargo is a refusal rather than a worse price, because a player who simply
pays a bit more has not been punished.

**The oracle, inverted.** Every round since 36 has proved that a new system
edits the ground *identically* on both sides. This one proves the opposite and
it is the claim the whole stage rests on: offices, wages, trust and warrants
are bookkeeping, and a session that files a warrant, pays a fine, trades a town
into an embargo and watches its people work a fortnight hashes the same ground
as one where none of it happened. The day any of it reaches for a block — a
resident who really digs — that test goes red.

**Surface.** No new key. `TOWN` at the terminal names the offices, what each
resident is worth and where the paperwork stands, including which other towns
hold paper on you; `WHO` gains an office column and a trust column; the beacon
console gains a civic block under the price list. `--town` catches the console
with its seats named and `--warrant` the same console with a signature on it
and the market shut. Journal **VERSION 25** is unchanged — nothing here is an
order, because nothing here touches the ground.

**Deliberately not in 39:** elections, which are B3; residents who walk to a
job you can watch them do; a resident's purse that anybody can steal; deputies
who arrest on a warrant from a town you are not standing in; and any way to
hold office yourself — the ballot box is the next round's.

---

## Shipped — Stage 40: the ballot box

**The civic note's last item, and the close of an arc that opened in stage
11.** 39 gave every town a mayor and a sheriff who were real people; what it
could not do was let anybody lose the job. `office::holder` is a pure function
of the town's seed, so the mayor of a town was its mayor for ever — and a
warrant chain whose decider can never be replaced has one fixed link in it.

**A town votes on its own market day.** It already picks a weekday to hold a
market and already sends everybody to the square on it, so polling costs no new
clock, puts the whole town in one room for it, and stays pure in `(site, day)`
— which is what lets the live game and a reload agree on when an election
happened without anybody recording it. A term is a week; a town polls once a
term.

**A poll is a referendum on you, not a contest among neighbours.** In a town
of three, the man in the chair is not swapped for the man beside him — they
have lived next door to each other for years and nothing about that changed
this week. What *can* change is that somebody turned up from outside who the
town would rather have. So with nobody standing, the incumbent is returned; and
with your name in, every resident weighs what you are worth to them against
what they think of him.

That is also what keeps the register honest in the way this project means it:
a town you have never campaigned in re-elects its derived seat for ever and is
**never written down at all**. Only a seat that is not what the seed said costs
the save anything — and losing one *forgets* the entry rather than recording a
defeat.

**They vote on the ledger stage 39 built.** The note asked for goodwill "earned
through trade interactions — the resident who profits from trading with you
votes your way", and `disposition::trust` is exactly that: the credits you have
put across that person's own counter, kept separate from friendship. Trust
dominates the sum, friendship helps, and a bounty in that town is steep enough
to lose you a room you had bought — the frontier elects somebody it likes, not
somebody it is frightened of. Paper standing against the incumbent costs him
votes, which is the join straight back to the warrant chain.

**A badge belongs to a town now, and that is a bug fixed on the way past.**
`Permits.offices` was a bare list with no town on it, so `standing()` handed
the sheriff's override to every claim on the frontier. Harmless while only a
dev flag could set it; the moment a badge could be *won* in the smallest town
on the map it was a key to the world. It is now a set keyed by
`(town, office)`, `standing()` reads the town off `claim.key.town`, and
`permits.dat` bumps to version two — reading an old file's global badge as the
hometown's, which is strictly less reach than the file was claiming and the
only town the `--sheriff` override could meaningfully have meant.

**And a mayor cannot sign his own warrant.** Hold the seat and the sheriff
takes the petition up the road to the nearest other town's mayor, who has his
own archetype and his own opinion of you — `warrant::decides` reused unchanged
with a different man in the chair. A town you run is a haven. It is not a
sanctuary, and the vault price still signs regardless.

**Every effect is an existing system reading a new answer.** The sheriff's
badge is a permits entry, the mayor's claims are the ones permits already gave
the office, the standing with the Compact and the Holdouts is a reputation
entry, and the seat itself is one lookup with a derived fallback. Nothing was
invented to be a reward.

**The oracle, again inverted.** `Command::Stand` records the decision — it is a
thing the player did — and replay drops it, because polling day and the count
are both pure functions of ledgers that never touch a block. Stage 39's test
that the civic layer leaves the ground exactly as it found it now runs an
election and a term through it and still hashes identical.

**Surface.** The beacon console gains a **second page** rather than a longer
column — `TAB` turns it, the way the handheld's pages already work. It shows
the seats and who holds them, when the town next votes, how each resident is
leaning in words rather than points, and a row per seat to stand, withdraw or
resign. `TOWN` at the terminal grows the same in text. `--ballot` catches the
page with a poll due and `--elected` a town that has already made you sheriff.
Journal **VERSION 26**.

**Deliberately not in 40:** founding a town of your own and holding its offices
by default, and taking a town by force — the note's other two routes into
office, both of which want a round each. Deputies who answer to *you*; a seat
that pays a salary; residents who campaign against each other; and any way to
lose a seat other than at a poll.

---

## Shipped — Stage 41: seasons

**The first stage that is the board's own work rather than anybody's note.**
Every design note this project was handed shipped by 40. This one exists
because two clocks were already running and neither of them knew what month it
was: `weather::at` was pure in the tick and read beautifully, but August and
February were the same sky — so "it has been dry lately" was a coincidence
rather than a season, and lightning was as likely to bite in the wet half of
the year as the dry one. Meanwhile a burn scar healed at a rate the calendar
had no opinion about, and the country never changed colour.

**The year was already there.** `people::YEAR_DAYS` has counted twenty-eight
days since stage 23, purely so that villagers have birthdays. Four seasons of
seven days each fall straight out of it — and seven days is also
`ballot::TERM_DAYS`, so one season is one term of office and an election and a
season turn together. Nothing new was invented to hold a calendar; a test in
`succession.rs` holds `vx-world`'s day and year against the app's clock,
roster and ballot so the two sides cannot drift.

**A season edits no blocks, and that is the claim the stage rests on.** It is a
term inside functions that were already pure in the tick, plus a texture
upload. No save file changed, no wire tag was added, and the journal is still
**VERSION 26**. So this round has the *inverted* oracle test stages 39 and 40
introduced rather than the ordinary one: a year of seasons read over the same
journal — every seasonal term, at every day of the year, at three places —
must hash the same ground as a replay where nobody asked what month it was.

**Nothing snaps.** `season::phase` is continuous through the year and every
term downstream reads it, because a season that switched on a boundary would
put a hard edge in the sky and a hard edge in the tile atlas on one particular
midnight, and a player would read the edge as a bug. The continuity test walks
every boundary *including the wrap* from winter back into spring, which is the
one everybody forgets: the leaves have to hand over to the spring flush at
exactly the value they were at, so `leaf_turn` breaks its buds in the last tenth
of winter rather than resetting.

**The sky takes the month on the bar, not the field.** `weather::at` shifts the
*thresholds* a front has to cross to rain, rather than scaling the rain it
rolls. So a summer is not "fewer storms happened" — it is the same weather with
a higher bar, which is what a dry season is, and a summer storm that does
arrive is still a proper storm. The whole fire model inherits a fire season
without one line of `fire.rs` changing: `fuel_moisture` gets the season through
`wetness` for free, plus one direct term so a hot August evaporates what did
fall faster than an April one. It is checked as a redistribution rather than a
nerf: summer is measurably drier than winter over a country's worth of samples,
and the year's own mean sits between the two extremes it created.

**The growing season is a cost on the age, not a branch that skips a step.**
The obvious way to stop the woods over winter is to not call `advance` while it
is cold, and it is quietly wrong: `Ledger::due` has to stay a pure function of
`(disturbed_at, tick, species)` or a reload lands on a different stage than the
session that saved it, and "how often did we happen to call advance while it
was warm" is not a function of anything. So `season::grown_between` is the
integral of `growth`, quantised to whole days so the arithmetic stays in `u64`
and two machines cannot disagree in the last bit of a float — with
`when_grown` as its inverse for the panels and the tests. **Normalised**, on
the same principle as the weather: a calendar year banks exactly a calendar
year of growing, so a stand takes the same number of days to come back as it
did before there were seasons. It just does none of it in January and twice as
much in June. The oracle for that is its own test: the same clearing set in
spring and set on the last day before winter takes wildly different numbers of
calendar days to come back and arrives at **byte-identical ground**.

**The country changes colour with a repaint, not a shader.** The atlas has
always been sixty-three procedural tiles generated on the CPU and uploaded
once, which makes a season nearly free: `generate_tile` gains a point in the
year, and `TileTextures::repaint` rewrites only the seven living slots. No new
uniform, no branch in the fragment shader, no second atlas taking a second bind
group, no per-vertex anything — seven kilobytes on the days the year moves on.
The hardwoods do the work and the conifers barely move, which is what makes a
spruce a spruce and what makes the cove beside it legible when it turns; and
the tufts and the sward share one curve, because they are the same grass. The
sky and the light take a smaller lean of their own, applied *under* stage 38's
weather tint — a January overcast and a July one are not the same afternoon,
and doing those the other way round would make them one.

**Surface.** `WEATHER` at the terminal opens with the season, the day of the
year and whether the woods are in their fire season. `--season <name>` paints a
capture directly, which is how the three coves in this round's pictures are the
same seed and the same camera with nothing but the tick between them.

**Deliberately not in 41: snow.** Snow that settles is the one half of a winter
that *edits blocks*, which puts it on the other side of the line this whole
stage is built on — it would need the ordinary replay oracle, a mask to store
what is under it and a rule for what melting does to the ground. It is stage 44
on the arc, and it is a stage rather than a paragraph. Also absent: crops and
anything to farm; migrating or hibernating wildlife; ice on the water; a
seasonal shift in what a town's market wants; and any way for the player to
change or skip the season, which would make every test above a lie.

---

## Shipped — Stage 42: the floating origin

**The oldest item on the outstanding list, and the wall was not where the
roadmap said.** Stage 9a made chunk geometry chunk-local, and the note beside
it has said ever since that "the camera still meets the wall eventually." It
does — but exploration found two nearer ones first. The vertex shader added
each chunk's corner back in absolute space and the view matrix subtracted an
absolute camera straight after, two large numbers rounded before their
difference was taken; and the player's collision sweep ran in `f32` with a
millimetre skin, which at sixteen kilometres — fifty minutes' walk, one
drone ferry — is smaller than a float step, so a body snapped flush to a
block face rounded *into* it and stuck. The picture would have jittered a
long way past where the player could no longer walk.

**Nothing in the world moves.** Block coordinates stay `i32`; no save file,
wire format or generator changed. The renderer now draws relative to the
chunk corner the camera stands in: `Camera::origin` is exact integers, the
camera uniform carries it, and the vertex shader computes
`local + (chunk_origin - camera.origin)` — both corners are multiples of
sixteen below 2^28, so the subtraction is exact and nothing large ever
reaches a float multiply. No mesh is re-uploaded when the camera crosses a
chunk line; the chunk uniforms stay absolute and the subtraction happens per
vertex, on the GPU, for free. Objects arrive as they always did and the
renderer rebases them at upload; the lamp goes through one public seam,
`Renderer::relative`; and the tool in your hand and your own body are built
in the renderer's frame and marked `already_relative`, because they are the
two things that sit right in front of the eye where a quarter-block
quantisation would show most.

**The one thing that travels is `f64`.** `PlayerBody`, `Aabb`, the sweep,
`movement.rs`, the camera and the third-person placement. Villagers,
deputies, shots and marks stay `f32` — they never leave their town and never
run on the skin physics — and every place the player meets them takes the
*difference* in `f64` and narrows that, never the position. The aim ray
splits its `f64` origin into an exact starting cell and a fraction inside it
before the first DDA step, so a shot from three thousand kilometres out is as
precise as one at spawn; `sight::sees` widened with it.

**Proved the only way it can be.** The render test uploads the same chunk
meshes at the origin and a hundred thousand chunks out with the camera, an
object and the lamp moved by exactly the same offset, and demands the two
frames be **byte-identical** — no rounding to be close about. The physics
test walks one body down a corridor at spawn and another down the same
corridor at fifty thousand kilometres with the same inputs, and demands they
agree tick for tick to a micrometre and never overlap a block; in the old
sweep the far body stuck within a dozen ticks. The journal replays its
recorded walk from spawn and from three thousand kilometres out over the same
floor and lands the same displacement. And `--at 3000000,3000000` is a
capture flag now, not a crash.

**Journal VERSION 27.** Nothing on the wire changed, but a log recorded under
26 replays its movement through a different integrator and may land a block
off where it did. The one float that crosses the wire — the muzzle of a shot
— stays `f32`, and its rule stays: blocks are exact, floats are not.

**Known limits, documented rather than fixed.** `f32` objects are exact to a
sixteenth of a block out to a thousand kilometres and a quarter of one at
three thousand; in a still, invisible. Entities in `f64` is stage 45 if it is
ever wanted — and it was: stage 45 moved every body to the same width. The `i32` block lattice is the world's boundary and every test
above runs well inside the 2^28 where chunk corners are exact.

**Deliberately not in 42:** entities in `f64`; a world fence; persisting the
player's position (stage 10c's spawn rule stands — every session opens at the
homestead); the single dynamic-offset origin buffer the roadmap mentions
beside this item.

---

## Shipped — Stage 43: founding a town, and taking one

**The civic note's last two routes into office.** Stage 40 shipped the ballot
box; the note also said a player could *found a new town and hold its offices
by default*, or *take a town over — the hostile path, priced by the bounty
system itself*. Both are in.

**The gap, and the idiom that crosses it.** Every civic system — the roster,
the offices, the market, the ballot, the permits, the schedule, the wall —
takes a `TownSite` and keys on its centre or its seed, so a town that is not
on the lattice slots into all of them unchanged. The one thing that could not
see such a town was worldgen, which is pure in `(seed, cell)` and stays so. The
project's answer is the one every round since 36 has used: a stored ledger
whose effect on the ground is a journaled edit. `charter.rs` stores the sites
founded and the charters printed; `World` gains a list of charters and answers
`towns_near` with the lattice's towns and those together; and
`TerrainGenerator::generate` is split so `generate_among(pos, sites)` can be
asked for a chunk *as if the charter had always been on the map*.
`World::raise(site)` regenerates every chunk the town reaches that way. So a
founded town is built by exactly the code that builds every other one —
blended plateau, paths, tower, shed, clinic, bank, dwellings, lockboxes, mini
star, flora and cave masks — deterministic in `(seed, site)`, and the plot is
levelled, which means anything dug there is filled.

**Derived, not authored.** A charter's site is built as the lattice builds its
own: ground clamped the same way, the hometown's core width, a seed hashed off
the world's and the centre. Three settlers, their trades, the market's opening
books and the fort's trace all fall out of that seed. It is named from the
same sixteen-by-sixteen book every town is named from — `FOUND IRON REACH` —
and nothing downstream can tell which kind of town it is looking at.

**Founding is the ordinary oracle.** `Command::Found` (tag 26) carries the
centre and the name; both sides derive the same site and raise it with the
same generator, and the test replays a founding plus a broken plaza block to
the live session's exact hash — founding alone first, then with the break, and
again off the wire. The bookkeeping beside it — the charter consumed, the
chairs, the badges, the beacon ledger — is live-only, as `Stand`'s is. Journal
**VERSION 28**.

**The founder's due.** A player in a chair now faces the man the seed named
at every poll, without the incumbency bonus on the challenger's side — which is
what makes a taken town a town you have to keep, and what would have thrown a
founder out on the first quiet market day. So `standing_with` carries a
founder's term, sized to carry a seat against anybody's regard for the native
with nothing else on the sheet, and no more: the test holds a quiet founder in
across every town on the frontier and shows a mere holder with the same empty
books thrown out somewhere along it.

**Taking is the inverted oracle.** A warrant calls out a posse as before; the
posse now knows whose it is and can say when it is *broken* — every deputy down
or surrendered. `TAKE` at that town's console records `Command::Take` (tag 27),
which replays as nothing because a seat is a ledger entry, and the test says
so with a hash. The price is every ledger that already exists reading the same
fact: `TAKEN_COMPACT` and `TAKEN_HOLDOUTS` on the factions, a vault's worth on
the bounty sheet so the neighbours' warrant chains fire on the next civic tick,
and `Disposition::forfeit` tearing up every book in the town. With that sheet
unpaid the next poll throws you out; paid down and bought back, it does not —
the test runs both.

**Found on the way past.** The register kept "polled this term" per town, so
the first office polled on a market day quietly closed the ballot for the
second: since stage 40 no sheriff's chair has ever come up for a vote. It is
per seat now, `elections.dat` is version two (the old shape loads and closes
both seats, which is what it meant), and the same bump carries the taken map.

**Surface.** `FOUND <HEAD> <TAIL>` and `TAKE` at the terminal, with refusals in
words — `TOO CLOSE TO STONEHAVEN`, `THE DEPUTIES ARE STILL STANDING - 2 OF
THEM`, the name book when a word is not in it. `TOWN` and the console's civic
block open with `FOUNDED BY YOU - DAY 12` or `TAKEN - DAY 30`. A `TOWN CHARTER`
row at the top of the fabricator's ladder, one at a time. `--founded` founds a
town at `--at` and frames it from outside its wall; `--taken` photographs the
console of a town whose chairs you seized.

**Deliberately not in 43:** choosing a founded town's speciality; settlers who
migrate in over days rather than arriving with the charter; a town's own
standing garrison (the posse is the defence); a salary for a seat; deputies
who answer to you; abandoning or renaming a town.

---

## Shipped — Stage 44: snow that settles, and ice

**The other half of a winter.** Stage 41 shipped the year as readings and
drew the line at the ground: `a_year_of_seasons_never_touches_the_ground`
hashed a fortnight of seasons against an unread one and named *snow that
settles* as the thing that would turn it red. Snow and ice are the half of a
winter that *is* ground, and ground belongs to the journal — so they wanted a
round of their own, with the ordinary oracle to satisfy rather than the
inverted one.

**Four cold blocks, same shape.** `engine:snowy_grass`, `engine:snowy_sand`
and `engine:snowy_sphagnum` are each the bare block's twin: a white top, the
bare block's side with a white band draped over it, the same hardness, the
same height. A snow *layer* on top of the ground would have raised the floor
a block every snowfall, because `collides` treats every solid block as a full
cube on purpose; and snow depth in a mask would have been invisible to the
hash, which reads names only. A snowed block is a name, and names are what
the region files, the hash and the minimap already read. `engine:ice` is
`uniform(ICE).translucent()` and solid: the mesher's roofs are opaque, so an
opaque ice block would have darkened the lake bed under it.

**The cold, named once.** `weather::FREEZING = 0.30` on a temperature the
place mostly decides and the year leans on by `±0.22`. The place's own share
runs `0.15..0.95` rather than `0..1` so the *year* is what crosses the line:
no summit freezes at the height of the fire season and no cove stays open
through midwinter. `Conditions::freezing()` is the ground's question and
`snowing()` — freezing and wet — is the sky's; `sky_word()` says `SNOW` for a
status line. The test walks a summer and a winter week over five places and
wants zero frosts in the one, some in the other, and a country that is not
frozen solid.

**The frost automaton.** `frost::settle(world, seed, tick, standing)` is the
rain's source term moved to the oracle's side and given a wider hand: every
`EVERY = 64` ticks it reads the sky over the player once and touches
`SAMPLES = 16` columns hashed off `(seed, tick, n)` within `REACH = 24`. The
top of a column is its highest non-air block, and a tuft on it is looked
through. Snowing and the top is bare grass, sand or sphagnum: swap for the
twin. Freezing — wet or dry — and the top is full still water at or below
sea level: ice. Not freezing: a snowed block goes back to the bare one it
came from, and ice goes back to a full block of water. `Report` carries the
counts and where the ice melted; replay drops it, the live side wakes the
water there and says `SNOW IS SETTLING` / `THE LAKE HAS FROZEN` / `THE THAW`
on the fire line's beat. `journal::burn_and_grow` gained the one call, so
the replay's `Advance` loop and `Game::advance_weather` got it from the same
place. No new save file, no new wire tag: the ground is the state.

**Everything else was already true.** Ice is solid, so `collides`,
`supported` and `flow::is_standable` say a body and a drone stand on it
where they sank in water; `fluid::level_at` is zero for anything that is not
water, so a pump on ice lifts nothing and `electrolysis::water_near` refuses;
`fire::fuel` returns `None` for any name it does not know, so a burn in
winter leaves ash on snow. Each of those is a test now, and none of them is
a line of new code outside the test.

**The oracle, both ways.** `fire_tests::a_winter_replays_to_the_same_snow_and_ice`
finds the first snowing pass over the origin from the start of winter,
journals an `Advance` to two hours past it, and wants the live and replayed
hashes equal, different from the untouched country, and at least one snowed
or frozen block within reach to say why. Stage 41's test is not deleted but
*succeeded*: renamed `reading_the_year_never_touches_the_ground`, it still
says that calling every function in `season` and `weather` for every day of
the year moves nothing.

**Two bugs on the way past.** The rain's second hash draw was `unit(hash ^
0x51)`, and `unit` reads a hash's top 24 bits, so every shower since stage 38
fell on the diagonal through the player. The frost's first draft had the
same bug, caught by its own reach test — forty-nine columns whitened, and
forty-nine is seven squared. Both draws are finalised now. And the same test
found `load_around` loads a disc, not a square: the corners of a radius-two
load are not there.

**`--frost` and `--thaw`.** The fixture finds a shore near `--at` with the
most lake within the frost's reach, walks the clock to the first snowing sky
of the winter over it, and runs the automaton through a day's worth of
freezing passes. Over the shore at `64,130`: 1,461 columns snowed, 924
frozen, under a snowing sky at `0.30`. `--thaw` runs on until three quarters
of what the frost took is given back — two days later, `CLEAR` at `0.39`,
1,087 thawed and 701 melted — and the picture is the last of the snow on the
sand with the ice breaking up into open water.

**Deliberately not in 44:** snow depth or drifts; snow on roofs, logs and
leaves; snow drawn falling; icicles; ice you can fall through; skating; snow
that changes what a market wants; a shovel, or any way to clear a path other
than breaking the block; frost outside the player's reach.

## Shipped — Stage 45: entities in `f64`

**The line, moved.** Stage 42 made the renderer camera-relative and moved
*the one thing that travels* — the player's body, its physics, the camera and
the aim ray — to `f64`, and wrote the limit down for everything else: `f32`
objects exact to a sixteenth of a block at a thousand kilometres and a
quarter of one at three thousand, "in a still, invisible". Three things made
it worth moving. Every body but the held tool and the player's own was built
in absolute `f32` and rebased by the renderer *after* the rounding, so a
deputy's feet far out were a quarter block from where the simulation had
them. The muzzle of a shot was narrowed to `f32` before it was recorded, so a
round far out began a quarter block from the eye. And twenty sites in
`main.rs` handed the `f64` player position to entity systems narrowed, with a
matching widening at every `sight` call on the way back — ninety-nine
conversions, each a place the two widths could disagree.

**The rule.** A world position is `DVec3` wherever it lives. A direction, a
heading, a range, a speed and a rig's local geometry stay `f32` — they are
scale-free — and the difference is taken wide before it is narrowed:
`Gaze::towards` and `in_cone` narrow `(at - from)`, `yaw_towards` takes the
level difference as `f32`, the raycast takes an exact `f64` origin and a
`Vec3` direction. `vx_agent` was never in this: drones, fliers, the kestrel
and the base are `BlockPos`, and the oracle already covered them at that
width.

**The bodies.** `Villager`, `Deputy`, `Garrison.hatch`, `Stalker`, `Shot`,
`Sweep`, `Impact`, `Falling`'s hinge and tip, `Mark`, `Sighting`,
`Surroundings.player`, `Belief.last_known`, `Shipment::position_at` and
`Crash` — every place named is wide, every `x as f32 + 0.5` that built one is
`f64::from(x) + 0.5`, every `dt` is widened once at the top of the walk, and
every `.as_dvec3()` at a `sight` call is gone because the position already
was. `dose::exposure`, `roost::sees`, `mining::roster` and the intruder's
reach take `DVec3`.

**The draw path.** `Rig::objects` and its siblings still take a `Vec3`, but
it is the *local* one: every body block in `App::render` — machines,
villagers, deputies, the stalker, falling stems, holders, caravans, shots,
the roost kestrel, marks, crashes — builds from `renderer.relative(position)`
and maps `Object::already_relative`, exactly as the held tool and the
player's body did since 42. `Villagers::objects`, `Mining::objects` and
`kestrel_objects` take the `relative` closure so the modules never see the
renderer. The fixtures build their bodies after the camera is placed, through
`Camera::relative`, which is a pure function of the camera's position. The
per-object lighting pass adds `render_origin()` back before it reads
`surface_y`, so it reads the right column for every object for the first
time — the player's own body had been lit from the origin chunk's column
since stage 42. Two duplicated blocks (the roost kestrel and the marks, each
drawn twice) collapsed to one.

**The wire.** `Command::Fire { muzzle: [f64; 3] }`, tag 12 unchanged,
**journal VERSION 29**: the loader is exact-match only, so a version-28 log
restarts the oracle like every older one. `muzzle_of` returns the eye
unnarrowed. `arsenal.dat` is **VERSION 2** with `f64` crash columns and an
`OLDEST = 1` reader that widens the `f32` ones — the first save file in the
project whose range check widens a float.

**Proved the way 42 was.** `the_posse_walks_the_same_walk_three_thousand_kilometres_out`:
the same callout on the same flat floor at the origin and at
`(3_000_000, 3_000_000)`, two hundred and forty frames, every deputy's
displacement within a micrometre, and the squad actually walked.
`the_townsfolk_stroll_the_same_stroll_three_thousand_kilometres_out`: the
home town's plan with its centre moved, four thousand frames, every villager
within a micrometre of the town centre's frame.
`a_shot_lands_the_same_crater_three_thousand_kilometres_out`: the same two
rounds over the same floor through `replay_from`, the wounds compared block
for block *and mask for mask* relative to the floor's corner.
`the_same_body_draws_the_same_bytes_sixteen_hundred_kilometres_out`: the
player's rig through `Camera::relative` at the origin and a hundred thousand
chunks out, every matrix and bound compared as bits. And
`a_crash_column_survives_the_disk_at_both_widths`: a hand-written version-1
file widens, and a version-2 column no `f32` could hold round-trips.

**Three bugs on the way past.** A stem that landed across the *stump* of a
tree already felled found the tree in the worldgen's list and felled it
again — twice the logs — and the chain count was read off the landing tick
alone, so a neighbour taken early in the arc was forgotten by the time the
stem came down. Stage 45's rounding exposed both: `cos` of a single-precision
right angle is not zero, and in `f64` the level tip sat a millionth of a
block under the hinge, one block lower, which is how the old test had been
passing. `tip()` says down is level, `is_standing` reads the first trunk
block above the stump band, and `Falling.took` counts across the fall. And
the handheld's roster pushed the kestrel's row twice.

**`--posse --close --at 3000000,3000000`.** The same callout at spawn and
three thousand kilometres out, three deputies and the player, framed
overhead: every body on its feet in both.

**Deliberately not in 45:** drones in `f64` (they are cells, and the note
that says why stands); a world fence; persisting any entity that is not
persisted now; the single dynamic-offset origin buffer; widening headings,
ranges, speeds or rig geometry.

## Shipped — Stage 46a: the pad is the game

**Stage 24 shipped a pad you could hold. This is a pad you can play with.**
The mapping asked one question — is a panel open — which was enough to reach
the things a hand finds without looking, and not enough to finish a session
with. About twenty bound keys had no button on any context: the terminal,
the block palette entire, taking a machine's wheel, *withdrawing from a
vault* (you could put money in a strongroom and never take it out), map
zoom, prone, the debug readout, saving, and every control the arcade has.

**The trap.** `device.feed()` was not in `a_panel_is_open`, so while
looking through a drone the pad was in **world** context: east was crouch,
not back out, and **no button on the pad ended the feed**. You could take a
machine up and never get your own eyes back. That is what the context enum
exists for — a feed is neither a panel nor the world, and now it is its own
layer where `B` hangs up and `Y` takes or hands back the wheel.

**Thirteen buttons, twenty actions, two layers.** `LB` held is the block
palette — ten slots on the d-pad, the face buttons, `RB` and the right
stick, no cursor, because a palette is muscle memory or it is nothing.
`SELECT` held is the second layer: the terminal, prone, vault withdrawal,
scan, walk-or-fly, the debug readout, the save. Neither acts on the way
down, because whether a hold was a hold is only knowable when it comes back
up; released unused, `LB` swaps first and third person and `SELECT` raises
the scheme, exactly as they did before. What a tap means is declared once in
`gamepad::TAPS` and read by `main`, so the mapping and the game cannot
drift.

**The sticks reach the menus.** Before this the left stick was invisible to
every panel — the d-pad's four arrows were the only cursor — and the arcade,
whose controls are `KeyW`, `KeyS` and `KeyQ`, was unplayable on a pad
despite stage 34 claiming otherwise. A held tilt now walks a cursor on the
keyboard's own repeat shape, and both sticks drive the toy. The analog
triggers stopped firing through open panels, which is what a mouse button
would never do.

**`every_binding_the_game_has_is_reachable_from_the_pad`.** The round's
claim as a test: every `KeyCode` `handle_press` and the movement sampler act
on, checked against every button on every layer. A binding with no pad path
fails the build unless somebody writes it into `NOT_ON_THE_PAD` with a
reason — there are four, and three of them are the terminal caret keys the
on-screen keyboard carries in 46b.

**Two documents were lying.** `README.md`'s own rough-edge list still said
"There are no gamepad bindings at all" and "no gamepad support", twenty-two
stages after the pad shipped.

**Deliberately not in 46a:** text entry, which is 46b — typing arrives on
`event.text` from a `WindowEvent` that `poll_pad` never synthesises, so a
pad cannot enter a character however it is bound.

## Shipped — Stage 46b: the keyboard you never need

**A pad could not type, and no binding could have fixed it.** Every other
control the pad reaches is a `KeyCode` synthesized into the same
`handle_press` the keyboard drives — that is stage 24's design and it is why
the pad gained every panel for free. Text is the one thing that does not work
that way: typing arrives on `event.text` of a `WindowEvent`, deliberately,
because a scancode is a *position* and reading letters off positions is how a
game becomes unusable on half the world's keyboards. `poll_pad` never raises
a `WindowEvent`. So the pad gets a grid and a cursor, and what it picks goes
into `Terminal::type_char` — the same function the window's text lands in.

**`osk.rs` is a pure render function over a cursor**, like every panel in this
game: a row, a column, and a drawing that is a function of them. Every key on
it is one the font draws *and* the terminal keeps, checked by a test that
types each one and reads it back — a key that types nothing is worse than a
key that is not there. The cursor wraps at every edge, because a thumbstick
has no edges and clamping would make the walk from `A` to `.` a thirteen-step
crawl. `Y` in the terminal raises and stows it; the board goes away with the
line it types into.

**The one action with no keyboard twin.** Every button on the typing layer
resolves to a `KeyCode` except the one that types: on a keyboard you press
the letter itself, so there is no key for "the character I am pointing at".
`poll_pad` does that one directly, and `Context::Typing` exists to mark where
the exception lives rather than letting it hide.

**The stick that creeps.** `MoveCommand` carries a quantised `throttle` byte
beside the two quantised angles, for the same reason they are quantised: it
crosses the journal's wire and the simulation replays it. The walk sampler
fills it from the stick's rescaled magnitude; a key writes `FULL_THROTTLE`
and wins any tie, so keyboard play is unchanged to the bit. The scalar is
kept out of `wish_dir` on purpose — the ledge probe, the landing and the
slide launch all read the wish for its *direction*, and a shortened vector
would have quietly changed all three.

**Journal VERSION 30.** Tag 5 is one byte wider. A log recorded under 29 is
a byte short per `Move` and restarts the oracle, like every older log.

**The arc's stage 46 is closed.** It was named after the pad fix that
followed stage 45, and it shipped here as half of a round about the pad.

## Shipped — Stage 47: the body does not pass through the world

**The bug report said the player clips into things, and the obvious cause was
not it.** The near plane is nowhere near the wall: eye-to-wall when the hull
is flush is 0.301 m and the furthest corner of the near plane reaches 0.174 m,
so there is 0.127 m of margin at 16:9. What was actually happening was three
separate paths that put the hull, or the lens, inside geometry — and the
reason all three survived twenty stages is the same reason: **every collision
test in this engine is a binary predicate.** `collides` answers *is this box
inside a block*. Nothing anywhere asked *how much room is left*. `SKIN` is a
millimetre the sweep leaves against every surface it stops against, and it was
a promise no test made the code keep.

**The mantle ran with no collision query at all.** It is the one movement that
is not ordinary physics — the integrator is suspended and the box is walked
along a fixed arc, hand over the edge and up — and that exception was meant to
be about *momentum*. It had quietly become an exception about *geometry*:
`classify_ledge` checks where a mantle lands, and nothing checked where it
went. For about six tenths of a second per ledge the hull, and the eye inside
it, passed through the block being climbed. Every step of the arc is checked
now, and a blocked one aborts to `Airborne` — a hand slipping, which is what
the sweep would have done. Writing that check immediately failed a *legitimate*
mantle, which turned out to be a second bug hiding behind the first: the lift
and the reach phases overlapped between 0.4 and 0.6, rising while already
moving in, and that overlap is precisely the window where the hull cuts the
corner of the block. Separating them — up first, then across, which is what the
comment always claimed — costs nothing and is what makes the arc survive its
own check.

**The camera had a floor that outranked the wall.** `clear_orbit_distance`
used to `clamp` a blocked distance back *up* to `MIN_ORBIT_DISTANCE`. That
floor exists so the orbit cannot collapse onto the back of your own head, and
it was also, silently, overruling geometry: a wall closer than
`MIN_ORBIT_DISTANCE + CAMERA_SKIN` put the lens up to a quarter of a block
inside the rock. It returns `Option` now and refuses; the caller falls back to
the eye for that frame. Backing into a corner briefly becomes first person,
which is what the rough-edge note about filling the frame with your own back
wanted anyway — and first person is never inside anything.

**Standing up was the third, and the smallest.** `headroom` built the taller
hull and asked whether it collides. A head resting *exactly* on the ceiling
plane does not collide — the inset in the block query sees to that — so the one
gesture in the game that grows the hull was the one place in the system whose
margin was zero rather than `SKIN`. It adds the skin now, which is why `SKIN`
became `pub` and is exported from `vx-world`.

**The block query is symmetric.** `overlapping_blocks` nudged its upper bound
inward by `INSET` and its lower bound not at all, so the box's two ends
answered the same question differently: a face resting on a line was outside
the far block going up and inside the near block going down. Nothing depended
on the difference while every caller only asked the binary question. The moment
a test asserts *how much* room the sweep leaves, an asymmetric predicate makes
the margin `SKIN` on one face of the hull and `SKIN` minus a grid line's worth
of luck on the other.

**The margin family, which did not exist.** Walk into a wall and the hull's
face rests exactly `SKIN` from the plane, to a part in a billion — not merely
"not colliding". Fall onto a floor and it rests exactly `SKIN` above it, because
a floor is a wall you fell onto. Climb a two-block bench and assert on *every
tick* that the hull is outside the rock, rather than on where it ended up.
Sweep the orbit all the way round inside a closed three-block room and assert
there is a whole `CAMERA_SKIN` of clear air past the lens at every angle, or
that the frame honestly refused. And the block query claims one block for a box
that pokes less than `INSET` past a line at either end.

**No capture.** This round is a margin, not a picture: the fix is a millimetre,
and a screenshot of a millimetre is a screenshot of a wall. The tests are the
evidence.

## Shipped — Stage 48: the loop you can actually play

**The ask was to play a round of it. The finding was that nothing could.**
Every verb in the loop — walking, aiming, drilling, opening the counter,
selling — is a method on `App`, which owns an `Arc<Window>`; `main.rs` has no
test module and cannot usefully have one, and `vx-app` is a binary crate, so no
integration test can reach `movement`, `mining`, `shop` or `economy` either.
The two headless paths do not help: `run_screenshot` builds a `World` and a
`Camera` and nothing else — no player, no pile, no wallet — and `run_replay`
can only re-run a journal a *windowed* session already wrote.

So the loop's three verbs had wildly different coverage. The walk is proved to
a micrometre three thousand kilometres out. The drone's mining is proved end to
end on real terrain. Selling is proved against a hand-built `Stockpile` and a
bare `Market`. **Nothing joined them**, and the join is where the bug was.

**The bug.** `App::update_drilling` banks a broken block on the fleet's pile
with a bare `if let Some(base)` and no `else`. There is no base when a world
opens — the pile only exists once a container has been placed to declare one —
so a new player who walked out and mined got **nothing, silently**: the block
broke, the ore evaporated, and no line was printed anywhere. Every sibling
system says so out loud ("NO BASE PILE. PLACE A CONTAINER.", "NO BASE PILE TO
DRAW ON", "NO BASE CONTAINER SET", "NO BASE PILE"); the one that *produces* the
goods was the only one that stayed quiet. And the welcome panel said "THE CHEST
KEEPS YOUR GOODS", which is not true of anything you mine.

**`drill.rs` — the drill's laws, extracted.** The power, the bite, the four
layers a worked face shows and the deposit rule come out of `App` as free
functions; everything that is *policy* — the launcher, who hears the drill
running, what a lockbox does differently, when a trunk is a tree, who saw you,
what a refusal costs — stays exactly where it was. The point of the split is
that the played session drills with **the same four functions the window
drills with**. A playthrough that re-implemented the drill would be a test of
itself. `deposit` returns what became of the block rather than a `bool`, which
is what lets the caller say the line.

**`session.rs` — a session with nobody at the keyboard.** The headless twin of
the player's half of `Active`, in the shape `journal::Rebuilt` established for
replay. `advance` is the journal's own `Command::Advance` arm — `Mining::advance`
then `movement::advance_journal_tick` per tick — recording `Move` and `Advance`
exactly as the live game does, so a played session writes a journal `--replay`
can check. `walk_to` is a steering behaviour and *not* a pathfinder: hold
forward, let stage 47's vault do the climbing, and return `Arrival::Stuck` with
a position when it cannot get there, because a wall is a real answer.

**Three walking bugs, found by walking.** The first run left the house and
walked into the wall beside its own front door: it set off on the bearing of
something 170 blocks away while still stood in the kitchen. The house's
doorway and the shop's are named places now (`plan::door_offset`,
`shop_door_offset`, `counter_stand_offset`), with the geometry tests to keep
them true. The second run got over a three-block cliff by holding jump the
whole way — which also meant it climbed the *shop*, and finished stood on the
roof trying to sell ore down through the ceiling. So the walk reaches for the
jump only once it is genuinely stuck and never on a final approach, and
`at_the_counter` is a **raycast at the till** — the question `App::interact`
asks — rather than a radius, which a lens four blocks above the counter passes
happily. The third could not pass that cliff at all until the walker learned to
commit to a side and work along a face instead of bouncing off it.

**What a played loop actually costs**, on the shipped seed: spawn at
`(-14, 73, 9)`; 170 blocks east to copper breaking the surface at
`(154, 95, 37)`; two seconds a block on a drill nobody has spent anything on;
sixteen blocks cut; home again in **more** ticks than the walk out took,
because the pile is the weight on your back and `mass_from_byte` floors at
0.55; 224 credits over the counter at the price that town was paying, and the
price down for the next seller.

**`--play`** is the first fixture that is a playthrough rather than a tableau,
and the first to write more than one image: four beats, framed by the game's
*own* follow camera, which means the shot inside the shop is first person
because stage 47 taught that camera to refuse a wall rather than clamp through
it — and `ViewMode::draws_body`'s rule means it draws no body there, for the
good reason that all it would show is the inside of your own head.

**Named, not smuggled in: selling is not on the journal.** There is no
`Command::Sell`; the wallet, the pile's contents and the market sit outside the
replayed hash, so `--replay` cannot see wealth. That is a round of its own.

## Shipped — Stage 49a: the machine you look through

**The hole, demonstrated before it was closed.** `journal.rs` contained no
pilot, wheel or override concept at all, while `Operation::pilot_tick` — the
one pilot path that takes `&mut World` — calls `break_block` and can fell a
tree. Live, `KeyR` and the per-frame pilot command reached `journal.record`
nowhere; only `Advance { ticks }` was written. On replay that ran
`mining.advance` on a `Mining` whose `piloted` is `None`, `pilot_sub_tick`
matched `None => {}`, and every hand-dug block failed to reappear.

It was proved rather than argued: a test that drove a drone exactly as the live
game does, recording only what the live game recorded, played to
`0xc217b149207d88fd` and replayed to `0x4f3c9afb4075ea33`. Same orders, two
different worlds. The fix is the same test with the two new orders in it,
passing.

**`Wheel` and `Pilot`, tags 28 and 29, journal VERSION 31.** `Wheel` carries
`Option<MachineTag>` — `None` is hands off — and is recorded only once the
simulation has *granted* control, so the log never claims a wheel that was
refused. Hanging up records a release too, or replay would keep driving a
machine nobody is holding. `Pilot` is one byte (heading in bits 0-2, the cutter
in bit 3, climb in bits 4-5), folded on change exactly as `Move` is and for the
same reason: it is a held input, and `Advance` counts the ticks it covers. It
deliberately does not name a machine — `Wheel` already said who is driving,
the same way `Move` does not name the player.

Replay needed **no new stepping code**: `Rebuilt.mining` already owned
`piloted` and `pilot_command`, and `Mining::advance` already called
`pilot_sub_tick`. The arms are three lines each, running the very mutators the
key press runs.

**`MachineTag` gains the kestrel**, which its own doc comment predicted: "the
kestrel is absent here because it takes no wear, and if that ever changes it is
a version bump, loudly." It still takes no wear — `MachineTag::of` refuses it
and `Repair` still answers "THE KESTREL TAKES NO WEAR" — but it can be *flown*,
so control needs a total constructor beside the upkeep one.

**What is not on the wire**: where the pilot is looking. `set_pilot_look` feeds
only the drawn nose and never reaches a world edit, so it is presentation. The
code says so, because it is the obvious thing to add by mistake.

**The gimbal.** `eye_height` — a bare lift up the Y axis — becomes
`gimbal(machine)`, an offset in the rig's own frame, turned by the machine's
heading and added to the same interpolated ground point the rig is drawn at.
The old lift was wrong three different ways: the digger's eye sat *inside* its
cab box (`y 0.66..0.92`), the flier's floated in the gap between hull roof
(0.58) and rotor (0.83), and the kestrel's hovered above its own blades. Each
new offset is measured off that rig's real parts, and a test asserts the
property rather than the numbers: **the camera is outside every box its own rig
draws**.

**Two things fell out of moving it.** The machine you look through was never
culled from its own feed — invisible only because the camera sat in an air
pocket — so `Mining::objects` takes the feed and skips it. And machine
*heading* was tick-snapped while position had always been interpolated, which
nothing could notice while the camera ignored heading; a nose-mounted camera
would have jolted on every turn. Headings now glide on the same clock, by the
**short way round** (a nose crossing the wrap point moves a hair, not very
nearly a full revolution), so the rigs you are merely watching stop snapping
as well.

**`--gimbal`** photographs both halves: the feed itself, which is now a clean
downward view with no hull in it, and the same instant from outside.

## Planned — the hunt: how hostiles will search, shoot and stalk

A design note arrived extending the combat half of the people note, and it
is recorded whole here — as every note before it has been, because this file
is the one record and a second copy is a second thing to drift. It is the
*engine* the next two rounds assemble rather than invent. Its thesis: the famous failures of game
AI — enemies that do not search, do not survive, do not check their fire,
cannot walk, and never hunt — are pipeline rot, not intelligence problems.
Baked navmeshes desync from streamed worlds; hand-annotated cover goes
stale; the AI reads a copy of a world that moved on. **This engine has none
of those failure modes available to it**: the world *is* the grid, cover and
navigation are derived from live voxels on demand, and derived data cannot
rot. Most of the work is the discipline of not reintroducing by habit the
problems the architecture already solved.

**Belief, not position.** Each squad holds a last-known position with a
confidence that decays — exactly the model the kestrel's marks already give
the player, and the symmetry is the point. Their intelligence about you goes
stale the way yours goes stale about them, breaking line of sight means the
same thing on both sides, and stealth becomes a system rather than a stat.

**Search is an occupancy map, and the map is the behaviour.** Probability
diffuses from the last-known position across walkable cells each tick;
searchers move to the highest-probability cell they can reach; every cell
they see is zeroed. Sweeping rooms, covering exits and doubling back all
fall out of the arithmetic, with no scripted search points and no cheating —
the map literally cannot send a searcher where the player provably is not.
Below a mass threshold they give up, visibly: weapon lowered, patrol
resumed, a line landed.

**Cover is only cover against a belief**, so re-scoring when the belief
moves *is* the flanking mechanic; suppression is composure spent faster than
cover restores it, which hands the player pinning as a verb without a new
system. **Nobody fires through a friend**: two `sight::obstruction` checks —
allies count as blockers at the shot, and the cover scorer charges for
standing in an ally's lane — enforced as an invariant rather than tuned.
Allies extend the same courtesy to the player, because the player's yaw is
already in the command stream.

**A progress watchdog bounds the one failure players never forgive.** An
agent that closes no distance in `STUCK_TICKS` replans; twice stuck, it
abandons the approach. Flow fields already derive from the live grid, so a
dug wall is in the next field rather than in a mesh three patches stale;
people-shaped traversal costs reuse the player's own movement
classification, so a route a hostile takes is a route a player could.

**The stalker is two brains, and it hunts by ear.** A director that knows
the truth but feeds the creature only *zone-grade hints* — a 32-metre cell,
never a position — and a creature that must close the rest with the same
occupancy search as anyone else, so it genuinely hunts and can genuinely be
evaded. What makes it belong to *this* game: hints are weighted by noise,
**and machines are loud**. Drills, crews, the fuel loop's burners — every
report the roost can hear, the deep thing hears better. A dig site in
stalker country is a dinner bell, so the tension attaches to the core mining
loop rather than to a haunted corridor: run the swarm loud and rich, run it
slow and quiet, or dig decoy noise a valley over and work in the shadow of
your own diversion. Pacing is the director's other job — a pressure budget,
forced back-off, a floor distance it never spawns inside.

**Intelligence the player cannot perceive is intelligence wasted.** Every
mode transition lands a tell through the toast-and-terminal channel the
townsfolk round built. The search that sweeps a room in silence may as well
be random; the same search behind "check the far side" is the smartest enemy
the player has ever met.

| Constant | Value |
|---|---|
| `LKP_DECAY` | confidence 100 → 0 over 45 s unseen (matches the kestrel's marks, on purpose) |
| `DIFFUSE_RATE` | 0.18 per tick to open neighbours |
| `GIVE_UP` | total mass < 0.15 |
| `LANE_W` | 1.5 m ally firing lane, charged in the cover score |
| `SUPPRESS_NEAR` | impact within 2 m, composure −8 |
| `STUCK_TICKS` | 40 (~0.6 s) |
| `HINT_GRADE` | 32 m zone — the director never says more |
| `HINT_NOISE_W` | report loudness × 3 |
| `BACK_OFF` | forced ≥ 60 s after 90 s pressure |
| `NO_SPAWN_R` | 48 m |

**How it lands against this roadmap.** The note sequences itself by what a
part has to attach to, which maps onto the arc as it now stands:

| Part | Stage | Why there |
|---|---|---|
| `belief.rs`, the occupancy search, the watchdog | 28 — shipped | lands with the first hostile — there is nothing to hold a belief until then |
| Fire discipline and lane costs | 28 — shipped | needs a *pair* of hostiles before "do not shoot your friend" can be violated |
| The director and its pacing budget | 29 — the zone-hint half shipped; interior pacing waits for room fighting | arrives with the occupied bunkers it paces |
| The stalker, and noise-weighted hints | 31 — shipped | landed with the deep resources, and hunts by the noise the mine working them makes |

Its tests come with it, and one is a house rule restated: **all of it
replays** — a firefight's journal reproduces every mode transition at the
same tick, hunting AI covered by the oracle like everything else. Nothing in
the note blocks the civic half, which shipped in stage 23.

---

## Planned — the forest note: felling, fluid and weather

A design note arrived covering the forest, the water and the weather, and it
is recorded whole here as every note before it has been. **It is finished**:
the three forests shipped as stage 35, felling as 36, the fluid as 37 and the
weather, fire and succession as 38. What follows is the note's own reasoning,
kept because the ordering and the arithmetic in it are the most useful things
it contains, and because the shipped sections above assume it.

**Two of its premises were wrong about this engine and are worth stating
plainly**: it assumed a succession clock and a flora ledger already existed
here. They did not. The forest was a pure function of the ground with nothing
stored, so the ledger arrived with the first things that can *disturb* a stand
— felling in 36, and fire in 38, which is also where the clock landed.

### Felling, and the thirty per cent rule

The rule the note proposes — cut past about a third of the trunk and it goes
over — turns out to be real forestry rather than a guess. Directional felling
practice (OSHA's eTool, Husqvarna's academy, ANSI Z133) puts the **face notch
at 15–33% of trunk diameter**, traditionally a third and lower in modern
guidance; the **hinge at about 10% of diameter, at least 80% of the diameter
in length**; and the back cut slightly above the notch. The notch *aims* the
tree, the hinge *steers* it, and when the back cut takes the holding wood down
toward the hinge the stem can no longer support its own lean and tips. Cut a
leaner too fast and the trunk splits upward — a **barber chair** — and goes
where it is heavy rather than where it was aimed.

That maps onto machinery this engine already has. The trunk's cross-section
lives in the **micro-mask** (stage 27's 4×4×4 occupancy `u64` at 0.25 m); a
chop clears micro-cells along a cut plane; the fraction severed is a popcount.
Past the notch depth, with the holding wood down to the hinge, fall
probability spikes. Fall direction is the notch, biased by lean and wind.

**The fall must be kinematic, not a rigid body.** Float rigid-body
integration diverges across runs and machines, and the replay oracle would
not survive it. A scripted arc — rotation about the hinge with a fixed
angular-acceleration profile — is a pure function of `(tree, direction, start
tick)`, cheap, and collidable at every fixed tick against people, saplings,
micro-masks and other trunks. Vintage Story's mods take this shape; Valheim's
rigid-body logs are the fun the note wants and exactly the determinism this
project cannot have.

**Impact energy is honest arithmetic.** A trunk pivoting about its base drops
its centre of mass by half its height, so `E ≈ m·g·(h/2)` with
`m = π·r²·h·ρ` and wood at 600–900 kg/m³. A 15 cm sapling is ~1.6 kJ, a
mature 15 m stem ~152 kJ, an old-growth metre-thick giant ~1.9 MJ — a
thousandfold span that has to be compressed into a playable damage curve but
whose *ordering* is the whole point: a sapling taps you, a giant kills you and
takes its neighbours with it. A struck trunk above its own threshold starts
its own fall, which is the domino, deterministically.

**And the log is a block, not an entity.** When the arc finishes, the trunk
becomes horizontal log blocks along the fall line, choppable into graded
timber. No persistent physics object, nothing to desync.

**Ancient trees.** Rare, hash-selected on the existing lattice inside
old-growth stands. Their wood sits at the bunker-shell hardness tier, which is
a smaller extrapolation than it sounds: real wood spans ~67 lbf Janka (balsa)
to ~5,060 (Australian buloke), a 75-fold range, against red oak's ~1,220. They
want high-tier tools, they yield the most prime timber, and — the good hook —
**ancient wood is the only wood that does not burn**, which makes an ancient
grove a firebreak and a prize.

### Fluid: the micro-mask is already the answer

The note's decision, and it is the right one: **reuse the 64-cell micro-mask
as a 0–64 fill level** rather than building a parallel 0–7 level system. One
extra `u64` per wet block, stored sparsely; `popcount` is the volume; settling
is shifts, ANDs, ORs and popcounts — the SWAR vocabulary stage 27 already
speaks. A damaged block and a partly-filled block become one representation,
and rays that already read micro detail get sub-voxel water for free. Fully
wet blocks collapse to a "full" flag so only the shell of a body costs
anything.

Conservation comes from a falling-sand rule — fall first, then spread to
lower neighbours — with a small compressibility allowance so stacked water
equalises in communicating vessels without a pressure field. Stability comes
from a minimum-flow threshold, hysteresis, and **settle-to-sleep**: a body
that has not changed stops ticking.

**Determinism comes from the update scheme, and this is the part that cannot
be skipped.** A naive sequential sweep depends on iteration order. A **two-pass
checkerboard** or a **Margolus block automaton** makes the new state a pure
function of the old one, independent of traversal order, and every tie-break
comes from `hash(cell, tick)`. Rendering is a per-column height-field mesh,
not marching cubes.

### Weather, last, because fire closes the loop

Weather is a **pure function of (seed, tick, region)**: low-frequency noise
over coarse regions for temperature, humidity, wind and pressure, plus a
seeded Markov chain for the discrete states, advected by a slow wind vector so
fronts move across the map. No stored state, no side RNG — Minecraft's weather
is famously *not* reproducible from seed, and that is the mistake to avoid.

**Rain fills basins.** Priority-Flood (Barnes, Lehman & Mulla 2014) is the
right offline primitive: flood inward from the edges with a priority queue,
label the watersheds, and precompute each basin's spill elevation. Then
rainfall routes runoff into basins, the micro-cell fluid fills them to the
spill, the overflow runs downhill, and only the active front ticks. The bogs
flood first and hold longest, which is what makes them bogs.

**Lightning ignites, rarely and unevenly.** Strikes are hash-selected within
a storm, biased toward tall and isolated stems. Ignition is gated by fuel
moisture: roughly one fire per fifty discharges in the wetter half of the
literature and one in fourteen hundred in the drier neighbouring region, dry
strikes 30–50% likelier to ignite than wet ones, and the whole thing rising
sharply once dead-fuel moisture drops below about 20%.

**Fire spreads as a cellular automaton with Rothermel's shape.** Per-cell
states, and a neighbour ignition probability built from fuel, wind and slope
as the standard multiplicative `(1 + φ_w + φ_s)` — fire runs uphill and
downwind, and the direction of maximum spread is the vector sum of the two.
Crown fire where ladder fuels allow it, and low-probability ember spotting
downwind. Per biome: **hardwood coves resist** and act as refugia, **black
spruce explodes** — ground-to-crown ladder fuels, semi-serotinous cones, an
aerial seed bank that opens *because* of the fire — and **subalpine burns
rarely but totally**.

**And burnout writes the ledger.** A cell that finishes burning records
`disturbed_at = tick`, and a succession clock regrows the stand through
meadow, pioneer thicket, mixed and old growth. That is the payoff and the
reason weather is last: fire plus succession is the ecological cycle made
mechanical, and by then almost every part of it already exists.

### The build order, and what not to build

Biomes first (done), then felling, then fluid, then weather. Explicitly not
worth building: fixed-point rigid-body physics for trees; a true
Navier–Stokes or shallow-water solver; the full Rothermel parameter set; a
global flow-accumulation index recomputed per tick; and marching-cubes or
screen-space water.

**The compression is deliberate.** Real stands take 80–400 years to mature,
a hardwood canopy turns over in ~128, subalpine regeneration runs 300–400,
krummholz individuals reach a thousand, and black spruce burns on a 50–150
year cycle. The game compresses all of it into tens of seasons. Keep the
*sequence* and the *relative* rates faithful — pioneers before climax, black
spruce resetting hardest — and compress the absolute durations by one to two
orders of magnitude.

---

## Planned — the civic layer: permits, offices, elections — **finished**

The town-law arc, in three rounds, and **all three have shipped**: B1 as stage
11, B2 as 39 and B3 as 40 — with B3's other two routes into office, founding a
town and taking one, following in 43. The user's design is kept whole below,
because the shipped sections above are written against it.

### B1 — Build permissions and the bounty ledger — **shipped as stage 11**

See the stage 11 section above.

### B2 — Towns that work: offices and the warrant chain — **shipped as stage 39**

See the stage 39 section above. What follows is the design as it was written.

Named town offices: **Mayor, Sheriff (plus deputies), Shop clerk, Residents**.
Every individual in town runs the same loop the player does — mine or gather,
trade at the market, accumulate credits — because credits are the end goal of
every individual, NPC and player alike. Villagers become deterministic economic
agents trading against the town books on the same tick clock; the economy's
quantised catch-up already makes that replay-safe.

**Trust through trade** is the per-resident stat that unlocks guest
authorization on their building.

**The warrant chain**: when an individual's bounty crosses a threshold, the
sheriff cannot act alone — they must obtain a **warrant from the mayor**, and
only then may dispatch the offender. Enforcement-by-force lands with stage 17
(hostiles and health); until then the chain carries consequences short of force
— fines, revoked market access.

### B3 — Elections and goodwill — **shipped as stage 40**

See the stage 40 section above. What follows is the design as it was written.

The town's main computer — the beacon console — gains a **voting page**:
residents elect the mayor and sheriff. Votes are cast on **goodwill points**,
earned through trade interactions — the resident who profits from trading with
you votes your way, which reuses the trade ledger rather than inventing a
reputation system from nothing.

**The player can hold office**: win the sheriff's badge at the ballot box,
found a new town and hold its offices by default, or take a town over — the
hostile path, priced by the bounty system itself. Offices held across towns tie
into stage 19 (factions).

---

## Planned — the world below: caves and bunkers

From concept art supplied for the project: a small civilian bunker interior
(one room, reinforced doors onto a stairwell, generator and exhaust, breaker
box, radio bench, a supply shelf and a cot — lived in, untidy) and a large
industrial complex (silo hall, dome, conveyor ramps, catwalks, a surface
canopy over a multi-level base). Those are the small and large tiers.

Everything below stays inside the house rules: sited as a pure function of the
seed, stamped rather than stored, and original in every name and shape.

### Caves

Shipped as stage 17 — see the shipped section above. What remains below is
the bunkers half, which rests on the carve the caves round paid for.

### Bunkers

The built half shipped as stage 19 — see the shipped section above. What
remains is the *occupied* half: mobs and military, held until the health
model exists, because both of them attack you.

### The three tiers

| Tier | Shape | Reads as |
|---|---|---|
| Small | One room off a stairwell: reinforced doors, generator and exhaust, breaker box, radio bench, supply shelf, a cot. Somebody's private shelter, lived in and untidy | A find. Modest loot, a story told by what is on the shelf |
| Medium | Several rooms off a corridor, its own stairwell and generator room. Military or civilian | A dungeon. Worth clearing, worth coming back for |
| Large | Multiple levels, a silo hall, a dome, conveyor ramps and catwalks, part of it breaking the surface under a canopy | A landmark you can see from a ridge and plan an expedition to |

---

## Also outstanding

**Letting the journal shrink saves.** Region files are still written every save,
so the journal is currently an oracle rather than a disk win. The keyframe
machinery is in place; turning it on is worth doing after the oracle has run
against real sessions for a while.

**A real min-cost flow.** Trade routing is greedy nearest-deficit matching. At a
few dozen towns it picks the runs a proper solver would; worth revisiting only
if the traffic it produces reads as dull.

## The arc beyond

**Every design note this project has been given is now shipped.** The hunt
note's engine landed in 28, its director half in 29 and its stalker in 31. The
toy shipped in 34. The forest note finished in 38 — three forests in 35,
felling in 36, the fluid in 37, and the weather, the fire and the succession
clock in 38. And the civic layer, open since stage 11, closed in 40: B1 in 11,
B2 in 39, B3 in 40.

What follows is the board's own work rather than anybody's note. Seasons
shipped in 41; the floating origin in 42; the civic note's last two routes into
office — founding a town, and taking one — in 43; the ground half of a winter
— snow that settles, and ice — in 44; and in 45 the line stage 42 drew at the
things that travel moved out to every body in the game.

The arc's one named entry — analog movement on foot, put there by a bug
found on a Deck — shipped inside 46b, which is where it belonged: a round
whose thesis is *the pad is the game* cannot leave the stick walking in
eight directions at one speed. And 47 closed the reported clipping: the
mantle arc, the orbit floor and the headroom check all stopped bypassing the
sweep, and the margin the sweep promises is asserted rather than assumed.

And 48 answered "play a round of it" by finding that nothing could, building
the thing that can, and fixing the silent loss it turned up on the way.

And 49a closed the replay hole under it: driving by hand is on the log now,
and the camera has come off the hull it was sitting inside.

One round is named now:

| Stage | What | Why here |
|---|---|---|
| 49b | The drone you can lose | A machine cannot collide with anything, so it cannot crash. Integrity beside `wear.rs` and on the oracle for the same reason wear is; the flier's auto-climb off under manual control so it can be flown into a cliff, while the digger keeps the standability rule that stops a hand-driven drone stranding itself; gunfire through `segment_hits_box`; a persistent, mapped wreck you walk out to and salvage or rebuild; and `garage.rs`'s first `lose` mutator, since `grant` only ever added |

Beyond those the board holds the outstanding engineering below, and whatever
the next note says.

## The feature map

The whole game at a glance, as of stage 49a.

**Shipped:** core scaffold; wgpu renderer + headless capture; block editing
through cancellable events; AABB physics; region saves (name-keyed, cached);
spline terrain; parallel meshing; frustum culling; ore geology with honest
outcrops; drone swarm and three mining methods (adit / decline / pit); flying
drone, sector scanning and the ferry loop; fog-of-war minimap; composite
rigs; handheld drill; RS-curve skills (mining / prospecting / logistics /
security); bitmap font + HUD; starting village, villagers and perception;
day/night; container towns on a lattice; beacon network and contracts; third
person and drone piloting; world hashes and the command journal (the replay
oracle); seed tree; body ids; packed quads; town economies with moving
prices, inter-town freight and player trade runs; machines cost credits;
trade and handheld maps with bearings; tick-based player movement (stance /
sprint / slide / vault / mantle / stamina / carried mass); pregenerated
pinned spawn, background chunk generation, region cache; the player's house,
chest, mailbox and mail-order; the welcome panel with its self-updating
changelog; permits (ranked claims, three lock grades, witnesses and sneaking
by stance eye height, bounty, breaching, lock-picking); the gold panel
(journaled admin orders, live tuning, compiled out of shipped builds); the
arsenal (slug launcher, synthesized audio, recoil and screenshake, town
warnings, witnessed property bounty, panic with flee-or-alarm, caravan
interception and salvage); the kestrel (pack scout, standing orders from
the handheld, decaying contact marks, cell upgrade line) and the roost (the
town's watcher on the office roof, observed-then-witnessed, the heist
window); intrusion through machines (spoofer coils, the leash, machine
witnesses and the impound, the watch box blinded/silenced/tapped, a watch
box for your own roof); the fabricator (every broken block is stock on the
one pile, and a printer that turns named goods into ammunition, bars, planks,
wall panels, charged cells, spoofer coils and whole machines, with the
Fabrication skill buying speed); caves (the first true 3D carve: tunnel
galleries, deep chambers, hillside mouths, ore in the cut faces, nothing
under towns or into the sea); lights in the dark (baked per-face skylight,
a genuinely black underground, the suit's hand lamp, and printed optics —
high beam, night vision, thermal); bunkers (a lattice of their own, three
proportion systems, golden BSP on a Fibonacci vocabulary, golden-angle
bearings, a 400-hardness shell, an authored furnishing pool, and supply
caches whose contents are derived from where they stand); the fuel loop (the
fleet burns oxyhydrogen and stops when it runs out, an electrolyser splits
water into it on any shore, and HHO trades on the network like any other
good); star forts (bastioned traces by town tier, gates on the road axes,
ditches, and deterministic breaches) and town banks (a strongroom per town
behind the first Tier Three lock, for staging trade or keeping what you
cannot lose); foundations under every building and fort wall; the terminal
(typed commands, a caret and history, and a scrollback the toasts land in);
the townsfolk (names, trades and temperaments derived once per person, pure
schedules with per-town market days and ±20-minute personal jitter, a
friendship ledger with gift tables, weekly caps, birthdays and crime
spillover, tier unlocks up to bunker intel and a door key, and speech
templated over live prices, bounty and fuel state);
native gamepad play (buttons synthesized into the keyboard seams,
context-sensitive face buttons, analog sticks with a rescaled deadzone, a
SELECT control-scheme overlay) and a one-command Steam Deck installer;
the workshop (upgrade parts printed from ore as a second route to the same
lines the counter sells, the pack, press and lamp lines, and the rule that
an upgrade may not touch arithmetic the journal re-runs);
wear and recovery (machines age by the tick they work, the worst one sets
the crew's pace, printed spare parts mend them, and the ledger lives inside
the replayed simulation because it decides how much ground gets cut);
micro-on-damage (blocks gain a 4³ interior only where violence touches them,
one `u64` a wound, carved with register arithmetic, drawn as quarter-metre
faces riding the existing quad stream, with rays reading the cells and feet
never doing); hostiles and health (a warrant musters a posse who hold a
decaying belief and search an occupancy map for you, score cover as
occlusion at three eye heights, never fire through each other, and fight,
hide, run or surrender on nerve derived back at the townsfolk round — with
six hits, quiet-then-mend recovery, and an arrest that settles the bounty);
held shelters (every bunker garrisoned from its own seed, hostiles routing
on the drones' flow fields toward their belief, noise heard as 32-metre
zones with the drill as dinner bell, paired overwatch, and surrendered
holders taken in for the board's tier-scaled pay); factions (the Compact
and the Holdouts as standings the bounty board cannot settle — captures,
kills, crimes, trade and gifts each moving two opinions, prices shaded a
few percent, a challenge-and-grace truce at Neutral, and grudged shelters
jamming the scout with the spoofers the intrusion round taught);
uranium, oil and gas (a deep ore banded below the overburden and hot enough
that a face is a place you visit rather than live in, shielding printed
against it; fluid reservoirs on their own coarse lattice, worked by a
printed wellhead that spuds in for casing, drills on the journal's clock and
then lifts into the pile until the body is empty; a dry hole that is a place
rather than a roll; and well gas as a second, honestly worse fuel that needs
no lake);
the stalker (a director that knows the truth and may say only a 32-metre
cell, a creature that closes the rest on the same occupancy search the law
uses, hints weighted by the noise a working mine makes, a floor distance it
never arrives inside, and a pressure budget that makes it break off);
a handheld you actually hold (a cased unit that swings up into your hands
with the readout projected onto its own glass, and both hands busy while it
is up);
faces on the townsfolk (eyes that follow whatever has their attention and
saturate rather than swivelling, and a grunt in three voices for anybody who
crowds them);
a mini star around every town that had no wall at all, gates and ditch
included;
a clinic in every town (a free cot that mends you and scrubs your dose, and
medkits bought there and spent in the field);
an F3 debug readout in every build (engine vitals, the journal clock, the
fleet, the hostiles' belief, your dose, your medkits, your wells, how roused
the deep is and both standings — diagnostics that change nothing they report on, unlike
the gold panel, which is why one ships and the other is compiled out);
a pocket arcade on the handheld's own glass (a printed cartridge, an
original corridor shooter computed pixel by pixel, floors that loop meaner and
a cabinet that keeps the record);
three forests decided by height and wetness (peat bog under sphagnum, mixed
hardwood cove with emergent giants, subalpine conifer thinning to krummholz at
a wandering treeline and bare rock above it);
water that moves (a fill level carried in the same sixty-four cells a wound
is, an endless sea and finite inland pools, a bounded automaton that settles
flat and goes to sleep, and a printed pump that lifts over its own head);
felling on the forestry numbers (a notch cut into the trunk's own
cross-section, a hinge that lets go, a kinematic arc that falls toward the
side you cut from, flattens what it lands on, chain-fells its neighbours and
lies down as logs — with ancient trees at the top of the hardness ladder
yielding prime timber);
weather that is a pure function of the seed and the tick (fronts that drift
across the country rather than snapping, a greyed sky and a lifted ambient on
the uniform that was already there, rain drawn as a derived sheet of streaks
and fed into the water automaton as a source term, fuel moisture that lags the
rain both ways, and thunder every ear in the game already listens with);
fire that closes the loop (lightning biased to the tall and the lonely, about
one strike in fifty lighting anything, spread on Rothermel's shape so it runs
uphill and downwind, every wooden thing burnable wherever it stands and
ancient wood burnable nowhere) and the succession clock behind it (only
disturbed cells stored, cut or burnt alike, coming back through meadow,
thicket and mixed stand as the tree the seed always described);
towns that are run by somebody (a mayor and a sheriff drawn without
replacement off the town's own seed, residents who put goods on the books and
credits in their pockets on the hours the schedule already gave them, trust
bought with business rather than kindness and spent on somebody's front door,
and a warrant chain where the sheriff has to ask a mayor with an opinion of
you — a fine when the paperwork is filed, a closed counter while it stands,
and deputies only on a signature);
a ballot box in every town (polling on its own market day once a term, a poll
that is a referendum on the outsider rather than a contest among neighbours,
residents casting on the business you have put across their counters, seats
that are stored only when they are not what the seed said, badges that belong
to the town that issued them, and a mayor who cannot sign his own warrant); a year
that turns (four seven-day seasons on the twenty-eight day calendar the roster
already kept, one season to a term, the whole of it derived from the tick and
stored nowhere — hardwoods that go gold then bare then green while the spruce
barely move, a sward that goes to straw, a sky and a light that go with them,
wet thresholds that make high summer a fire season and midwinter safe, and a
growing clock that stops dead over the winter and still banks a full year in a
year); the floating origin (the renderer draws relative to the camera's own
chunk with the subtraction done on exact integers in the vertex shader, the
player's body, physics, camera and aim ray in `f64`, and the oracle that the
same scene renders byte-identical sixteen hundred kilometres out); towns of
your own (a printed charter filed on open ground raises a whole town by the
same generator that draws every other one, journaled as ground; three settlers
and a market on day one; both chairs by default with a founder's due to keep
them; and the hostile route — break a town's posse and seize its chairs at the
console, priced by the factions, the bounty sheet, the neighbours' warrants and
the residents' torn-up trust, so the town votes on you next term); snow that
settles and ice (four cold blocks the same shape as the bare ones; the cold
named once as a threshold the place mostly decides and the year leans on; a
frost automaton on the journal's side of the line that snows open ground,
freezes still low water and gives both back, so a winter replays to the same
snow; a lake you and the deputies walk across, a pump that lifts nothing and
an electrolyser that finds no water until the thaw); every body in `f64`
(villagers, deputies, holders, the stalker, shots, stems, marks, caravans and
crashes at the body's width, drawn in the camera's frame, the muzzle on the
wire at `f64`, and the same walk, stroll, crater and bytes at spawn and three
thousand kilometres out); a game you can play with a controller and nothing
else (six mapping layers including two held modifiers and a machine feed that
can finally be hung up, an on-screen keyboard for the terminal, an analog
stick on foot, and a test that fails the build if a binding has no pad path);
a body that does not pass through the world (the mantle arc validated at every
tick instead of only at its destination, an orbit camera that refuses a wall
rather than clamping through it, the skin added to the headroom check, and the
margin family of tests that assert *how much* room the sweep leaves rather than
merely that it leaves some);
a loop that can be played with nobody at the keyboard (the drill's arithmetic
lifted out of the window into laws the headless session and the live game
share, a session that walks by holding forward and reports where it got stuck
rather than pretending, the house's and the shop's doorways named so a body can
aim at the gap, and a four-beat `--play` capture framed by the game's own
follow camera) — and the silent loss it found, where a block mined with no base
container declared evaporated without a word;
the machine you look through (driving by hand on the log at last — the wheel
and the held controls, journal 31, after the same orders were shown playing to
one world and replaying to another — and the FPV camera moved off the hull it
was sitting inside onto a gimbal under the nose, with the subject machine culled
from its own feed and machine heading interpolated between ticks for the first
time, so noses glide the short way round instead of snapping);
a Steam Deck dist build every round.

**Planned, in arc order:** the drone you can lose — and, first inside that
round, the replay hole where a hand-dug hole is not written down.

**Outstanding engineering:** journal-shrunk saves;
real min-cost flow for freight; ammunition as a trade good; the rest of the
weapon table; the kestrel's
cell state surviving a reload; anything that hacks *you* (the hardened link
has no adversary until factions); **selling on the journal** — there is no
`Command::Sell`, so the wallet, the pile's contents and the market sit outside
the replayed hash and `--replay` cannot see wealth (found while building the
played session in 48, and named there rather than smuggled into it).

## Known rough edges

Tracked in `README.md` under "Known rough edges" — currently ~28 entries, the
notable ones being: saves store a whole chunk snapshot per modified chunk;
water is alpha-blended without depth sorting; a running excavation is not
persisted; only one drone and one flier are ever created; selling is not on
the journal, so a session replays to the same ground but not to the same
books; the played session steers rather than paths, so a route with a doorway
in it needs the doorway named; and there is no player-carried inventory, so
everything routes through the fleet's base pile — which is also what the
movement system now weighs you down by, for want of a real backpack.
