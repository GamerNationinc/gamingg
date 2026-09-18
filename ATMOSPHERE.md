# ATMOSPHERE.md — sealed rooms, gas as a number, leaks measured in cells

Where pressurised air lives in `gamingg`, why it is **not** the particle system
the Starship EVO talk describes, and what it costs.

Reference: Starship EVO devlog on moving oxygen from a voxel-grid BFS to a
particle emulation — <https://youtu.be/V5XkSNuDl5c>.

---

## 0. Read the video as a constraint, not a recipe

The talk is honest about why it went where it went, and the reason is the whole
point:

> the grid went away.

EVO moved from voxel construction to non-grid brick construction. Once your
walls are arbitrary oriented boxes, "which cells are inside" stops being a
graph walk and becomes a solid-geometry query. A stochastic particle emitter is
a *reasonable* escape hatch from that: fire five particles, let them bounce,
grow a bounding volume around where they get to, and call it a room.

Look at what the escape hatch then costs, in the video's own order:

| Symptom | Their fix | Root cause |
|---|---|---|
| Rooms that should be sealed report pressure | ray-cast emitter → player | five particles don't sample a room |
| Unpressurised rooms receive oxygen anyway | occlusion test on the ray | the bounding volume is a hull, not the room |
| Oxygen reading jitters | exponential decay filter | the signal is a sparse random hit count |

Every one of those is a repair to a sampling error that only exists because the
room is being *estimated*. None of them is a feature.

**We still have the grid.** `gamingg` is voxels on a lattice with a block
registry that already knows `is_solid` and `is_opaque`, a chunk store that
already keeps a sparse map of per-block damage masks, and a `micro` module that
already does connected components on a 64-bit mask. Estimating a room here
would be paying EVO's tax without owning EVO's problem.

And there is a harder reason. A five-particle emitter with a random walk is a
clock-adjacent, order-dependent process. The house rule is that the journal
records orders and the replay oracle proves the live game and the replay reach
the same world hash. `fluid.rs` earns that by keeping the awake set **sorted**
and splitting the tick by cell parity; `fire.rs` earns it by making lightning a
pure function of `(seed, tick)`. A particle sim that decides whether your base
is holding air would either need the same discipline — at which point it is
just a slow, noisy flood fill — or it would put an asterisk on the oracle. That
trade is not available.

---

## 1. The four ideas from the video that do survive

Take these, leave the emitter:

1. **A room is an object, not a per-voxel field.** EVO is right that you
   simulate a *volume*, not billions of molecules. One scalar per room.
2. **Leak detection is the primitive.** "Did anything escape the sealed set" is
   the question that matters, not "what is the pressure at this coordinate".
3. **Smoothing belongs on the output.** Exponential decay on what the player
   *reads* is correct even when the underlying number is clean — it makes an
   airlock cycle feel like a cycle instead of a step function.
4. **A bounding volume per room is worth keeping** — not to define the room,
   but as a cheap AABB for the fog volume, the audio bus, and broad-phase
   queries. We get it free while labelling.

---

## 2. Architecture: the room graph

New module `vx-world/src/atmos/`, sitting beside `fluid` and `micro` and
leaning on both.

```
atmos/
  label.rs   per-section connected components of open space
  graph.rs   union-find across section faces; RoomId; incremental rebuild
  gas.rs     integer gas amounts, pressure, mixing, venting
  mod.rs     the tick, the wake set, the public queries
```

### 2.1 Sealing is a block property, not a block list

Add one `bool` to the block definition beside `solid` and `opaque`:

```rust
/// True when the block holds pressure. Independent of `solid`: glass and a
/// closed hatch are sealing, a grate and a ladder are not, and a wounded
/// block is sealing only while its damage mask still covers the face.
pub sealed: bool,
```

Why a third flag and not a reuse of `solid`: the mesher's notion of solid is
about collision, and the two genuinely disagree in both directions. A
force-field pane you can walk through should hold air; a fence you cannot walk
through should not.

### 2.2 Label per 16³ section, not per column

Chunks are `16 × 16 × 256`. Relabelling 65,536 voxels because someone drilled
one block is wasteful, so `atmos` labels in **16³ sections** — sixteen per
column. This is an `atmos`-local subdivision. It changes nothing in
`chunk.rs`; it is an index, not a storage format.

Per section, `label.rs` produces:

- `labels: [u8; 4096]` — local component id per voxel, `0` = not open. In
  practice a section has under a dozen components, so `u8` is roomy.
- `sizes: [u16; N]` — open-voxel count per component, for volume.
- `faces: [[u8; 256]; 6]` — the label touching each cell of each of the six
  section faces. **This is the only part the neighbours ever read.**

Cost: two scans of 4096 bytes. Sub-microsecond, `rayon`-parallel across
sections, and a pure function of the section's blocks — so it caches, and the
cache invalidates on edit exactly like the mesher's does.

### 2.3 Stitch with union-find, on faces only

A global room is a union-find over `(section, local_label)` pairs. Two locals
merge when their face arrays agree on any of the 256 cells of a shared face.

The merge scan reads **256 bytes per face**, never the 4096 voxels behind it.
Joining a section to its six neighbours is 1,536 byte comparisons.

Incremental rebuild on a block edit:

1. Relabel the one section that changed. (4096 voxels)
2. Re-merge it against its six neighbours. (1,536 bytes)
3. If step 1 *split* a component or step 2 *joined* two roots, run the gas
   fix-up in §3.3. Otherwise nothing else happens at all.

Splits are the awkward case for union-find — you cannot un-union. The cheap
answer, and the one to ship: when a split is detected, drop the affected root
and re-walk it from the section graph, bounded by the same budget as §2.4. A
room is at most a few thousand sections' worth of labels and the walk touches
face arrays only.

### 2.4 Outdoors is a budget, not a search to infinity

The classic failure of flood-fill pressurisation is that the moment someone
opens a door, the algorithm tries to enumerate the sky.

`fluid.rs` already solved the shape of this problem with `REACH` and
`PATIENCE`: the simulation is *bounded on purpose*, and the sea is a source
rather than a finite body. Atmosphere takes the same deal.

```rust
/// Open volume beyond which a region stops being a room and becomes weather.
/// A hangar is large. A valley is not a hangar.
pub const SEALED_VOLUME_MAX: u32 = 32_768;   // blocks — a 32×32×32 hall
```

A region is `Outdoors` — never pressurised, infinite sink, no state stored — if
any of these hold:

- its open-voxel count crosses `SEALED_VOLUME_MAX`, **or**
- it touches a section in an unloaded chunk, **or**
- it touches the top of the column.

The count check short-circuits the walk the instant it trips, so the expensive
case is the one that stops early. This is also the honest answer to EVO's
"particle exits the bounding volume": here, a room that reaches the sky *is*
the sky, by the same test that builds it.

### 2.5 A door is a conductance, not a topology change

If opening a door merges two rooms and closing it splits them, the graph churns
every time someone walks through a base, and gas has to be re-partitioned on
each swing. That is the single biggest cost trap in this design.

So: blocks marked `portal` (doors, hatches, valves, the airlock inner and outer
faces) are **never** sealing for labelling purposes. The rooms either side stay
separate rooms forever. What the door state changes is the **conductance of the
edge between them**, from zero to full. Topology is a function of construction;
door state is a function of play. Only the second one changes at sixty hertz,
and it changes one `u8`.

This falls straight out of an airlock: two rooms, two edges, never both open.

---

## 3. The gas

### 3.1 Integers, because the oracle reads the hash

```rust
/// Gas in one block of volume at one atmosphere. Mirrors `micro::CELLS`
/// in spirit — a unit small enough to divide, large enough to count.
pub const PER_BLOCK: u32 = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Gas {
    pub o2: u32,
    pub n2: u32,
    pub co2: u32,
}

pub struct Room {
    pub volume: u32,          // open blocks
    pub gas: Gas,
    pub bounds: Aabb,         // free from labelling; fog + audio use it
    pub anchor: BlockPos,     // §5
}
```

Pressure is `gas.total() / volume` in the same units — integer division, one
instruction, identical on every machine. No `f32`, no `exp()`, no transcendental
whose last bit differs between a Ryzen and an M-series, which is exactly the
hazard the byte-identical capture tests exist to catch.

Breathability is a **partial pressure** check, not a total-pressure check, which
is what makes the CO₂ scrubber a real machine rather than a decoration:

```rust
pub fn breathable(&self) -> bool {
    self.gas.o2 / self.volume >= O2_MIN && self.gas.co2 / self.volume <= CO2_MAX
}
```

### 3.2 One tick, over rooms

```rust
/// Player-clock ticks between one step of the air and the next. Air is
/// faster than water and slower than the frame.
pub const EVERY: u32 = 2;
```

Per step, over the **awake** room set only — a room with no edge flow, no leak
and no consumer sleeps exactly like a settled body of water does:

1. **Flow along open edges.** `moved = (p_hi - p_lo) * conductance / K`,
   capped by `MAX_FLOW`, moved in the composition of the donor room.
2. **Vent along leak edges.** §4.
3. **Sources and sinks.** `electrolysis` puts O₂ in, `fuel` takes it out,
   people and villagers breathe, `fire` needs it (§6).

Determinism comes from the same place `fluid.rs` gets it: the awake set is
iterated in sorted `RoomId` order, and a step reads only the previous step's
values. Shuffle the wake order, get the same air.

### 3.3 Merges and splits conserve mass

When an edit joins two rooms, the new room's gas is the sum and its volume is
the sum — pressure equalises instantly, which for a hole in a wall between two
pressurised rooms is right within a tick anyway. When an edit splits a room,
each half takes gas **in proportion to its volume**, with the remainder going to
the half containing the lower anchor so the total is exact to the unit. No
rounding leak. There is a test for this and it should be the first one written.

---

## 4. Leaks: the damage mask is already the orifice

This is the part the particle approach structurally cannot do, and the part
where the existing engine hands us the answer for free.

`fluid.rs`'s opening line is that **the fill level is the damage mask** —
sixty-four cells per block, `popcount` is the volume. The same sixty-four cells
say how big a hole is.

A wounded wall block does not flip from sealed to open. Its leak area is

```rust
let open = micro::face_layer(face) & !mask;   // cells missing on that face
let area = open.count_ones();                 // 0..=16 on a face layer
```

and the vent rate scales with it. Which means, with no new representation and
no new tuning knob:

- A pinhole from a stray ricochet bleeds a hab down over minutes. You hear the
  hiss, you have time to find it.
- A hull breach from a charge dumps it in seconds.
- Welding a plate over *most* of the hole helps *proportionally*. Partial
  repair is a real thing a player can do under time pressure.
- Drilling a wall with the tool the game is built around produces a leak whose
  size is the size of the drill.

Orifice flow, in the integer world:

```rust
/// Units of gas across one open cell per step, per unit of pressure
/// difference. Choked flow's shape without choked flow's arithmetic.
const VENT_PER_CELL: u32 = 3;

let drop = pressure - outside_pressure;
let vented = (drop * area * VENT_PER_CELL).min(gas.total().min(MAX_VENT));
```

Outside pressure is not zero everywhere: vacuum is zero, but a flooded
compartment is `reservoir`'s water head, and a deep shaft is the column
weight. That is the seam that lets this system pay for the submarine and the
mine gallery later without a rewrite.

---

## 5. Persistence, under the house rules

The rule is one concern per save file, `MAGIC` + `VERSION` (u32 LE), tolerant
loader, data name- or centre-keyed and never index-keyed. `RoomId` is a
union-find index — the worst possible save key, since it changes on any edit
anywhere and on load order.

So rooms are **anchor-keyed**: the canonical anchor is the lexicographically
lowest `BlockPos` of any open block in the room. Deterministic, recomputable
from the blocks alone, stable across sessions as long as that corner of the
room exists.

`atmos.sav` stores `(anchor, Gas, volume)` triples and nothing else. On load,
the labelling rebuilds every room from the blocks — it is a pure function of
them — and each rebuilt room claims the saved gas whose anchor it contains.
Rooms with no saved entry come up at whatever the ambient is; saved entries
whose anchor no longer sits in a room are dropped. A tolerant loader falls back
to ambient everywhere, which is a playable world, not a crash.

---

## 6. What else the room graph pays for

The graph is worth building even if pressure were never a mechanic, because
five existing modules are currently doing without it:

- **`fire.rs`** — a fire in a sealed room should consume the O₂ and go out.
  That is two lines against a room's gas and it is the most satisfying
  emergent consequence in the whole system. Backdraft when you open the door
  is then free.
- **`frost.rs`** — heat leaks through the same edges gas does, at a different
  conductance. One graph, two fluids.
- **`audio.rs`** — sound attenuates per room hop instead of per metre. A closed
  door should muffle.
- **`stalker.rs` / `awareness.rs`** — "is the player in a room that connects to
  mine" is a room-graph reachability query, and a far better AI predicate than
  a distance check.
- **`vx-render`** — rooms are portals. This is the classic portal-culling
  structure; indoor scenes stop drawing the rest of the base.

And for the drones, which is the point of the game: `vx-agent` already runs BFS
flow fields for the diggers. **A leak is a job.** The room graph hands the fleet
an exact target list — position, face, cell count — and patching is the mining
job with the sign flipped. A hull breach that dispatches the swarm while the
player runs for the airlock is the scene this system exists to produce.

---

## 7. What the player actually sees

Only here does the video's smoothing come back, and only here:

- **The HUD** runs the exponential filter EVO describes. `hud.rs` displays a
  decayed reading of the room's partial pressure, so a door cycle reads as a
  sweep. The filter lives in presentation, never in the sim — sim-side it would
  be state to journal and a chance to diverge.
- **Vent particles are decoration.** Spawn them in `vx-render` from the leak
  list the sim already computed: position, face, and a rate from the cell count.
  They are the *consequence* of the leak, drawn after the fact, authoritative
  over nothing. This is the inversion of EVO's design and it is the whole
  argument in one line — **particles as output, not as input.**
- **Fog volume** per room from the `bounds` AABB, tinted by composition. CO₂
  buildup you can see before the meter says so.
- **Audio** — the hiss is the best leak indicator in any game that has one, and
  its volume is `area`.

---

## 8. Determinism checklist

Before this stage ships, all of these hold:

- [ ] No `f32` anywhere in `atmos::gas`. Integers only.
- [ ] No clock read, no thread id, no `HashMap` iteration order in the tick.
- [ ] Awake set iterated in sorted order; a step reads only last step's values.
- [ ] Labelling is a pure function of a section's blocks — same bytes in, same
      labels out, whatever order sections were built in.
- [ ] Every order that changes door state has an apply arm in `journal.rs`.
      Gas state is *derived*, never journalled.
- [ ] The world hash includes the atmosphere hash, and the replay oracle test
      covers a run that breaches, vents, patches and repressurises.

---

## 9. Tests, as invariants

In house style — the name is the claim:

```
a_sealed_room_holds_its_pressure_for_a_thousand_ticks
breaking_one_block_between_two_rooms_conserves_total_gas
splitting_a_room_divides_gas_by_volume_with_no_rounding_loss
a_bigger_hole_vents_faster_in_proportion_to_its_cells
patching_half_a_hole_halves_the_vent_rate
opening_a_door_changes_no_labels
a_room_that_grows_past_the_budget_becomes_outdoors_and_vents
shuffling_the_wake_order_ends_with_the_same_air
a_fire_in_a_sealed_room_burns_out_when_the_oxygen_does
atmosphere_survives_a_save_and_load_by_anchor
a_room_whose_anchor_was_drilled_away_comes_back_at_ambient
the_replay_oracle_agrees_after_a_breach_and_a_repair
```

---

## 10. Staging

Ship whole, per the house rule — tests in both feature configs, clippy clean in
both, captures, README and ROADMAP, `intro.rs` bumped, `dist/` rebuilt.

| Stage | Ships | Proves |
|---|---|---|
| **A** | `label.rs` + `graph.rs` + a debug overlay that tints rooms | the graph is correct and the incremental rebuild is cheap |
| **B** | `gas.rs`, sealed rooms, leaks from damage masks, vent audio | pressure is a thing; holes have sizes |
| **C** | player breath in `health.rs`, HUD filter, fog, vent particles | it is a mechanic |
| **D** | `electrolysis` fills, `fuel` draws, `fire` consumes, `frost` shares edges | the room graph pays for itself |
| **E** | leak-patch job in `vx-agent` | the swarm has a reason to fly indoors |

Stage A alone is the honest experiment: build it, drill a wall, watch the tint
change, and measure the rebuild. If a section relabel plus a six-face merge is
not comfortably inside a frame, nothing after it matters — but it is 4KB of
scanning and it will be.

---

## 11. What not to build

Per the simplicity rule, these are named so they can stay unbuilt:

- **No per-voxel pressure field.** Rooms are the resolution. A gradient across
  a hab is not a mechanic anyone will notice at the rate air actually moves.
- **No temperature-coupled ideal gas law in stage B.** `PV = nRT` is one more
  variable and a lot more tuning for a effect the player cannot see yet.
  `frost.rs` can share the edges in stage D and the constant `T` holds until
  then.
- **No gas composition beyond three.** O₂, N₂, CO₂. Adding H₂ is tempting
  because `electrolysis` makes it, but an explosive atmosphere is its own
  stage and it needs fire coupling to mean anything.
- **No particle simulation.** Not as a fallback, not behind a feature flag.
  Two systems that can disagree about whether a room is sealed is worse than
  either one alone.
