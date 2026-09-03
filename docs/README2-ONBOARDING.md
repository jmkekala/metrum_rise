# Onboarding

Where everything is, what owns what, and where to go for what you are
trying to do. If you are new, read this page top to bottom once.

## What this is

Metrum Rise is a city builder with a Rust simulation core and a Godot 4 shell.
The split is strict, and its shape dictates much of how you build:

**Rust owns the simulation.** The road graph, lanes, agents, economy, zoning,
terrain storage, pathfinding, and saves. It decides what is true.

**Godot owns presentation and input.** It uploads buffers Rust computed, binds
materials, draws, and collects clicks. It must not decide road topology, terrain
heights, or material ownership. Godot code making any simulation decisions is a
bug. Fix it; never build on it.

**The 2.5D engine owns derived reality.**
Since the transplant there is a third party: the engine evaluates ground,
weather, minerals, habitability, acoustics, tides, and minds as pure functions
of position and seed. It decides nothing about the city; Rust still owns what is
true, and the engine answers what the world is like. It reaches the game only
through adapters in `godot/scripts/core/engine_*.gd`, never by calling into the
sim, and its files are not in this repository at all.

The boundary is a GDExtension. `SimulationNode` is the Rust class Godot talks
to, and everything crossing between them goes through methods marked `#[func]`.

## Running it

`run.sh` on Linux, `run.ps1` on Windows. Both build the Rust crate, copy the
resulting library into `godot/bin/`, register the extension, and launch.

Add `--debug <category>` for logging. Useful categories: `road`, `traffic`,
`terrain`, `economy`, `demand`, `perf`. Note that `--debug traffic` also turns on
a visual overlay that replaces the cars with debug geometry, which is what you
are seeing if the vehicles suddenly look like wireframes.

Other entry points, all through the same launcher: `--asset-editor`,
`--world-editor`, `--economy-editor`, and `--benchmark`.

## The layout

```
demand/
docs/                       All project docs are found here
├── archive/                Legacy reference material
├── asset_editor.md
├── building_allocator.md
├── CHANGELOG.md
├── demand.md
├── earthworks.md
├── economy.md
├── entrance_and_exit.md
├── in_engine_docs.md
├── narrative.md
├── project.md
├── README.md
├── README2-ONBOARDING.md   You are here
├── reference.md
├── region.md
├── roadmap.md
├── roads.md
├── services.md
├── simulation_layers.md
├── traffic.md
├── transit.md
├── terrain.md
├── ui.md
└── zoning.md
economy/
godot/
└── scenes/                 Main, MainMenu, and the three editors
    ├── scripts/
    │   ├── core/           input, launch routing, world bootstrap
    │   ├── renderers/      terrain, roads, agents, buildings
    │   ├── ui/             panels and tools
    │   └── editors/        asset, world, and economy editors
    ├── addons/             four directory JUNCTIONS into the 2.5D_engine repo
    │   ├── 2.5D_engine/    the engine: evaluators, bus wrappers, tools
    │   ├── GOAT_bus/       the event bus
    │   ├── SPEECH_socket/  speech and sockets
    │   └── FILE_browser/   browse file manifests, inject and repair headers
    └── spike_*.gd          diagnostic scenes, see below
maps/
rust/src/
├── simulation/             the part you will spend the most time in
│   ├── network/            road graph, lanes, junctions, borders
│   ├── economy/            agents, households, freight, logistics
│   ├── zoning/             parcels, land use, occupancy
│   ├── buildings/          placement, allocator, frontage
│   ├── terrain/            heightfield storage and chunking
│   ├── pathing/            contraction hierarchy, flow fields
│   ├── region/             regional tier scaffold, funding model
│   └── save/               SQLite schema and load/store
└── nodes/                  the Godot boundary
    ├── simulation_node/    the #[func] API surface Godot calls
    └── sim/                the core the node wraps, and editing operations
screenshots/
tools/                      offline Python: DEM import, VAT baking, terrain chunking
zoning/
```

## Which document owns what

[`README.md`](README.md) is the full index. The short version:

| If you want | Read |
|---|---|
| What the game is, the setting, the arc | [`narrative.md`](narrative.md) |
| The tier above the city, founding, expansion | [`region.md`](region.md) |
| Roads, lanes, junction control | [`roads.md`](roads.md) |
| How cars move, follow, and turn | [`traffic.md`](traffic.md) |
| Parcels, districts, alleys | [`zoning.md`](zoning.md) |
| Money, industry, freight | [`economy.md`](economy.md) |
| Who builds what and where | [`building_allocator.md`](building_allocator.md) |
| Growth pressure and migration | [`demand.md`](demand.md) |
| Emergency response, utilities, schools | [`services.md`](services.md) |
| Terrain, minerals, world generation | [`terrain.md`](terrain.md) |
| Constants, buffer formats, vocabulary | [`reference.md`](reference.md) |
| What is being worked on now | [`project.md`](project.md), [`roadmap.md`](roadmap.md) |
| What changed and what is knowingly broken | [`CHANGELOG.md`](CHANGELOG.md) |
| The 2.5D engine: transplant, boundary, everything it feeds | [`ENGINE_CONVERSION.md`](ENGINE_CONVERSION.md) |

The directory stays flat on purpose. One file, one clear role.

## Starting points by task

**Reading the code for the first time.** Start at
`rust/src/simulation/network/graph/data.rs`. `Edge` and `Node` are the spine of
the road network, and most systems reach them eventually. Then
`rust/src/nodes/simulation_node/` to see what Godot is allowed to ask for.

**Changing something in the simulation.** Find the owning doc above, read it
first, then change the code and the doc together. A doc that disagrees with the
code is worse than no doc.

**Adding something Godot needs to see.** It goes in the render snapshot. Direct
reads are forbidden. Look at `rust/src/nodes/sim/core/snapshot.rs` for the pattern. 

**Debugging traffic.** `METRUM_DEBUG_TRAFFIC=1` prints per-agent junction
decisions with reason codes: which car was held, at which node, and why. This is
the fastest way to tell a signal hold from a blocked connector from a missing
turn.

**Testing a change.** `cargo test --lib` in `rust/`. The suite is about 1,500
tests and takes roughly 20 minutes to run plus 15 to link, on two cores.

## Documentation is in-engine

Written documentation goes stale once the work is in the engine, and updating two
things separately loses to updating one. Documentation belongs in the engine,
spatially next to the content it describes.

The reference is [Gyms, Zoos and Museums: Your Documentation Should Be In-Game](https://youtu.be/5PJRCz0t7yY)
by Robin-Yann Storm, tool designer on IO Interactive's Glacier editor and
Guerrilla's Decima editor. Twenty-six minutes, and it is required onboarding
here, as this is an open source project and the *only* way this doesn't turn into
one giant cluster-fuck is with in-game documentation.

<video width="640" height="360" controls>
  <source src="https://youtu.be/5PJRCz0t7yY" type="video/mp4">
  Your browser does not support the video tag.
</video>

Every system gets three scenes:

| Scene | What it holds | The question it answers |
|---|---|---|
| Gym | A system under load and against its edges | How far can a car travel before the junction refuses it? What is the tightest turn a connector will build? |
| Zoo | One of everything, side by side | What does every lane kind look like at real scale, in this lighting, next to each other? |
| Museum | The cases that were once broken | Does the defect that shipped in August still reproduce? |

Each is a scene you load and look at, which makes a regression visible on screen.

Tracked as `GYM-01` in [`roadmap.md`](roadmap.md). Not built yet.
[`in_engine_docs.md`](in_engine_docs.md) owns the design: a registry generates
the scenes, because anything baked at authoring time rots the moment the system
changes.

The three scenes cover systems you can watch. Everything else, the API surface itself
uses Godot's class reference: right-click a node or a property in the editor, choose
"open documentation", and the entry opens inside the editor, with no browser involved.
godot-rust exports Rust doc comments into that reference, so a `///` line above a
`#[func]` method becomes the answer a contributor gets without leaving the editor.
Entries link out to the owning document here when the full design context matters.
Tracked as `GYM-02`. 

## The engine

The game runs on the 2.5D engine, which lives in its own repo beside
this one and mounts through four directory junctions under
`godot/addons/`. A fresh clone needs both repos side by side and a
one-time junction step; the engine's files are untracked here by
design, which is also the license boundary (the game is GPL-2.0, the
engine is not).

### What the engine gives you

Every engine node is a pure function: same position, same seed, same
answer, on any machine, forever. Nothing is stored, nothing is
authored, and nothing needs a save slot. The ground under the city is
an fBm field; the minerals under a mine are strata chemistry read by
atomic number; the desirability of a parcel is habitability scored
against biome suitability; a struck material's sound is its own modal
profile rendered on the spot.

That is why the boundary is two arrays and a tick rather than a
library call: the economy hands over one batch of positions, gets one
batch of answers, and per-agent chatter never crosses.

### Working with it

The whole conversion, both directions of the boundary, every renderer
branch, and every finding the drills earned lives in
[`ENGINE_CONVERSION.md`](ENGINE_CONVERSION.md). Read it before
touching an adapter. The short orientation:

| Piece | Where |
|---|---|
| Adapters, one per system | `godot/scripts/core/engine_*.gd` |
| The harness that drives them | `godot/scripts/core/engine_tick.gd` (autoload) |
| The boundary itself | `engine_boundary.gd` down, deposit grids up |
| The Rust intake | `rust/src/simulation/engine_inputs.rs` |
| Bit-exact kernel twins | `rust/src/engine_twin/` |
| The workflow toggle | `user://engine_meshes.cfg` |

Four laws hold across all of it, and breaking one is how the hard
bugs got made:

**One ground function.** `EngineTerrainSource.ground_m()` is the
ground. Terrain, water, roads, and physics all read it, so the world
cannot fork between renderers.

**Every sampler passes its own spacing.** The footprint argument is a
band limit, so a sampler asking every ten metres says so; detail finer
than the grid reading it can only alias, and evaluating it anyway was
most of one boot's cost.

**Measured beats derived, and never the reverse.** A painted deposit
or a sculpted cell wins over the field every time; the derived value
is the fallback, computed live and never baked into a save.

**Stored heights are not metres.** They are metres over the render
exaggeration, the same convention the level tool and DEM imports obey.
Writing raw metres drew every hill twenty times too steep, and
flat-zero test worlds hid it completely.

### Three standing rules

Every spike appends a JSON entry to `godot/benchmarks.json` (verdict,
wall time, CPU, cores, RAM, Godot version), so any machine's numbers
stand beside any other's.

Every windowed launch wraps in a log-stall watchdog that kills the
process by name after five stalled minutes, because a frozen Godot
never sends a completion signal and one overnight freeze cost six
hours.

Screenshots go into a dated `screenshots/spikes_<range>/` folder named
`spikeNN_<subject>[_pixelart_filter]_<W/100>x<H/100>.png`: the
resolution collapses two zeros (1604x881 reads 16x9), and the
pixel-art filter is named only when it is actually on.

### Before any long launch

Run the parse gate. `godot --headless --path godot --script
res://gate_check.gd` loads every adapter and drill as a fresh
GDScript and reports each one; it takes seconds and has caught three
parse errors that would each have cost a twelve-minute boot.

## The spikes

`godot/spike_*.gd` are diagnostic scenes, and the raw material for the above.
Each was written to answer a question about a live failure, and each stays in
the tree afterward because the system it exercises still needs checking after
the next change. Between them they already build cross junctions, spawn traffic,
install two-phase signals, group junction arms into streets by geometry, cluster
car positions, and count overlapping vehicles.

One per concern:

| Spike | Covers |
|---|---|
| `spike_left_turn.gd` | Signaled junction: do crossing movements enter the box together? |
| `spike_conflict.gd` | The same junction with and without a signal, compared |
| `spike_live_red_light.gd` | Does a car stop at a red light, in the rendered game? |
| `spike_junction_control.gd` | Junction control and lane cross-sections across the GDExtension boundary |
| `spike_poll.gd` | Reproducer: the simulation thread is never shut down |
| `scripts/core/spike_*.gd` | The engine suite: twelve headless drills over the boundary, gateway, renderers, and every system consuming the engine |
| `spike_engine_live.gd` | Every engine insertion point in the rendered game: dig, coal loop, growth, sound, screenshots into `screenshots/` |
| `scripts/core/gate_check.gd` | Cache-free parse gate over every drill and adapter; run it before any long boot |

Run one with:

```
godot --path godot --script spike_left_turn.gd
```

Install a signal before measuring anything about a junction: an uncontrolled junction
gives every arm permanent green, so cross traffic runs simultaneously by design.

Each run writes to `user://spike_runs/<name>.json` and prints how it compares to
the previous run. `godot/spike_record.gd` is the recorder.

## Conventions

**`CLAUDE.md` and `AGENTS.md`** at the repository root carry contributor guidance and
architectural invariants for your agents if you are using any. Read them before making
**ANY** agentic changes, *especially* the simulation safety notes.

**File headers.** Many files open with a MANIFEST block naming what the file is and
what it depends on, with section headers dividing the body by concern. Both rules are
75 characters for maximum minimap visibility without wrapping, and a section header
carries a title alone, because description belongs in the manifest. The addon can
quickly add manifests and standardize headers for you. It writes and repairs, and
it is not fully wired up yet, so treat this as the direction and enforce nothing on
it. 