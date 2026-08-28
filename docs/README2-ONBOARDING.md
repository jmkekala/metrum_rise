# Onboarding

Where everything is, what owns what, and where to go for the thing you are
trying to do. If you are new, read this page top to bottom once; it should
only take a few minutes and save you a week of exploring.

## What this is

Metrum Rise is a city builder with a Rust simulation core and a Godot 4 shell.
The split is strict and worth understanding before you read any code:

**Rust owns the simulation.** The road graph, lanes, agents, economy, zoning,
terrain storage, pathfinding, and saves. It decides what is true.

**Godot owns presentation and input.** It uploads buffers Rust computed, binds
materials, draws, and collects clicks. It must not decide road topology, terrain
heights, or material ownership. When you find Godot code making a simulation
decision, that is a bug, not a pattern to copy.

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
rust/src/
  simulation/        the simulation, and the part you will spend most time in
    network/         road graph, lanes, junctions, borders
    economy/         agents, households, freight, logistics
    zoning/          parcels, land use, occupancy
    buildings/       placement, allocator, frontage
    terrain/         heightfield storage and chunking
    pathing/         contraction hierarchy, flow fields
    region/          regional tier scaffold, funding model
    save/            SQLite schema and load/store
  nodes/             the Godot boundary
    simulation_node/ the #[func] API surface Godot calls
    sim/             the core the node wraps, and editing operations
godot/
  scenes/            Main, MainMenu, and the three editors
  scripts/
    core/            input, launch routing, world bootstrap
    renderers/       terrain, roads, agents, buildings
    ui/              panels and tools
    editors/         asset, world, and economy editors
  addons/
    file_browser/    editor dock: browse files by manifest, inject and repair headers
  spike_*.gd         diagnostic scenes, see below
tools/               offline Python: DEM import, VAT baking, terrain chunking
docs/                you are here
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

The directory stays flat on purpose. One file, one clear role.

## Starting points by task

**Reading the code for the first time.** Start at
`rust/src/simulation/network/graph/data.rs`. `Edge` and `Node` are the spine of
the road network, and most systems reach them eventually. Then
`rust/src/nodes/simulation_node/` to see what Godot is allowed to ask for.

**Changing something in the simulation.** Find the owning doc above, read it
first, then change the code and the doc together. A doc that disagrees with the
code is worse than no doc.

**Adding something Godot needs to see.** It goes in the render snapshot, not a
direct read. Look at `rust/src/nodes/sim/core/snapshot.rs` for the pattern.

**Debugging traffic.** `METRUM_DEBUG_TRAFFIC=1` prints per-agent junction
decisions with reason codes: which car was held, at which node, and why. This is
the fastest way to tell a signal hold from a blocked connector from a missing
turn.

**Testing a change.** `cargo test --lib` in `rust/`. The suite is about 1,500
tests and takes roughly 20 minutes to run plus 15 to link, on two cores.

## Documentation is in-engine

This is the project's documentation convention, and it is why the docs you are
reading stay short. Written documentation goes out of date because you ship the
game, not the document. Once the work is in the engine, updating two things
separately loses to updating one, so documentation belongs in the engine,
spatially next to the content it describes.

The reference is [Gyms, Zoos and Museums: Your Documentation Should Be In-Game](https://youtu.be/5PJRCz0t7yY)
by Robin-Yann Storm, tool designer on IO Interactive's Glacier editor and
Guerrilla's Decima editor. Twenty-six minutes, and it is required onboarding
here, as this is an open source project and the *only* way this doesn't turn into
one giant cluster-fuck is with in-game documentation.

Every system gets three scenes:

| Scene | What it holds | The question it answers |
|---|---|---|
| Gym | A system under load and against its edges | How far can a car travel before the junction refuses it? What is the tightest turn a connector will build? |
| Zoo | One of everything, side by side | What does every lane kind look like at real scale, in this lighting, next to each other? |
| Museum | The cases that were once broken | Does the defect that shipped in August still reproduce? |

A gym is not a level, a zoo is not an asset browser, and a museum is not a
changelog. Each is a scene you load and look at, and the point is that a
regression is visible rather than inferred from a number in a log.

Tracked as `GYM-01` in [`roadmap.md`](roadmap.md). Not built yet.

The three scenes cover systems you can watch. Everything else, the API surface
itself, uses Godot's class reference: right-click a node or a property in the
editor, choose "open documentation", and the entry opens inside the editor
rather than a browser. godot-rust exports Rust doc comments into that reference,
so a `///` line above a `#[func]` method becomes the answer a contributor gets
without leaving the editor. Entries link out to the owning document here when
the full design context matters. Tracked as `GYM-02`.

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

Run one with:

```
godot --path godot --script spike_left_turn.gd
```

Install a signal before measuring anything about a junction: an uncontrolled junction gives
every arm permanent green, so cross traffic runs simultaneously by design.

Each run writes to `user://spike_runs/<name>.json` and prints how it compares to
the previous run, so a change that halves junction throughput shows up instead
of scrolling past. `godot/spike_record.gd` is the recorder.

## Conventions

**`CLAUDE.md` and `AGENTS.md`** at the repository root carry contributor guidance and
architectural invariants for your agents if you are using any. Read them before making
**ANY** agentic changes, *especially* the simulation safety notes.

**File headers.** Many files open with a MANIFEST block naming what the file is and what
it depends on, with section headers dividing the body by concern. Both rules are 75
characters for maximum ninimap visibility without wrapping, and a section header carries
a title and nothing else because description belongs in the manifest. The addon can
quickly add manifests and standardize headers for you. The addon writes and repairs but is
not fully wired up yet, so treat this as the direction rather than a rule to enforce.