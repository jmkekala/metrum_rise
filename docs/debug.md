# Metrum Rise — Debug System

This document owns the runtime debug contract (`DEBUG-01`, `DEBUG-02`): the debug channels, the
runtime visual modes, the log format, the shared log, the always-on `events` channel, the Godot
bridge, the in-game console and the `run.sh` launch flags. Crash diagnostics (`METRUM_CRASH_DIAGNOSTICS`, the hang watchdog and the
flight recorder) are separate; this document lists only their launch flag.

## Overview

The debug system has one set of channels, shared by Rust and GDScript:

- Rust owns the channel state, the visual modes and the log
  ([`rust/src/debug.rs`](../rust/src/debug.rs) and its `channel`, `visual` and `log`
  submodules).
- GDScript reads and switches channels and visual modes, and logs lines, through the stateless
  `MetrumDebug` class. It never parses `METRUM_DEBUG*` variables itself.
- The environment sets each channel and visual mode at startup. The in-game console switches
  them at any time.
- Every enabled line prints to stdout as `[DEBUG:<tag>] message`. It is also kept in a bounded
  log that the console reads, copies and dumps.

## Channels

Each channel is one bit in a single `AtomicU32`. The tag is the exact text inside `[DEBUG:<tag>]`
and the name used by filters, the bridge and the console.

| Tag | Group | Content |
|-----|-------|---------|
| `road` | general | Road placement, preview, commit and road-surface geometry. |
| `traffic` | opt-in | Junctions, lane changes, replans, access attach/detach, network connectivity. |
| `economy` | general | Economy hour and day summaries. |
| `economy_phases` | general | Per-phase economy timing detail. |
| `economy_workplace` | general | Workplace assignment timing detail. |
| `spawn` | general | Household admission and building spawn. |
| `sim` | opt-in | Hourly simulation summaries. |
| `perf` | opt-in | Renderer frame CPU summaries (`PerfDebug`), preview and simulation frame timing. |
| `terrain` | opt-in | Terrain and water patch residency, meshing and timing. |
| `buildings` | opt-in | Building-site mesh, material and grading samples. |
| `save` | general | Save and load. |
| `world-editor` | general | World editor activity. |
| `asset-editor` | general | Asset editor export and validation. |
| `isect` | general | Road intersection candidate timing. |
| `init` | general | Simulation node start-up. |
| `lighting` | opt-in | Scene lighting and shadow-policy summary at scene start. |
| `events` | always | Warnings, errors and rejections the game already reports. |

The groups decide the startup state:

- **General** channels turn on with a bare `METRUM_DEBUG=1`.
- **Opt-in** channels need their own variable or a filter entry.
- **`events`** is always on and cannot be switched off.

`debug_log!("<tag>", ...)` checks its tag at compile time, so an unknown tag fails the build.
`traffic_log!(...)` logs on `traffic`.

### Startup state

[`init`](../rust/src/debug.rs) applies these rules once, from the GDExtension core init:

| Variable | Channels turned on |
|----------|--------------------|
| `METRUM_DEBUG=1` without `METRUM_DEBUG_FILTER` | Every general channel. |
| `METRUM_DEBUG=1` with `METRUM_DEBUG_FILTER=a,b,...` | Each named channel. Entries are trimmed and case-insensitive; unknown entries are ignored. |
| `METRUM_DEBUG_TRAFFIC`, `METRUM_DEBUG_SIM`, `METRUM_DEBUG_PERF`, `METRUM_DEBUG_TERRAIN`, `METRUM_DEBUG_BUILDINGS` | `traffic`, `sim`, `perf`, `terrain`, `buildings`. |
| `METRUM_DEBUG_ROAD_PROBE` set, or `road-probe` in the filter | `road`, which is where probe dumps print. |
| `METRUM_DEBUG_TERRAIN_VISUAL=lighting` (`light`, `sun`) | `lighting`. |

These rules also apply:

- A variable counts as set when it is non-empty and not `0`.
- The filter accepts these aliases:
  - `terrain-verbose`, `terrain-full`, `terrain-lod1` and `terrain-full-lod1` select `terrain`.
  - `building-sites` selects `buildings`.
- A filter has no effect without `METRUM_DEBUG=1`. `run.sh` always sets `METRUM_DEBUG=1` together
  with a filter.
- `--asset-editor` turns on the channels `METRUM_DEBUG=1` would select.
- The camera's under-terrain debug orbit follows whether the session was launched with
  `METRUM_DEBUG=1`. Channel switches do not change it.

### Startup-only modes

Some debug variables switch a diagnostic that is not a channel and not a [visual mode](#visual-modes):

- Rust derives them once, at startup. Each is on or off.
- Renderers and tools read them through `MetrumDebug.startup_mode(name)` when they load.
- The console lists them read-only, and they do not change at runtime.

| Mode | Source |
|------|--------|
| `road_geometry_dump` | `METRUM_DEBUG_ROAD_GEOMETRY_DUMP` (`--debug road`). Road patch geometry dumps also need the `road` channel. |
| `road_surface` | `METRUM_DEBUG_SURFACE`, or `road-surface` / `surface-debug` in the filter. Compiled road-surface overlay. |
| `road_probe` | `METRUM_DEBUG_ROAD_PROBE`, or `road-probe` in the filter. Surface probe dumps under the cursor. |
| `terrain_verbose` | `METRUM_DEBUG_TERRAIN_VERBOSE`, or `terrain-verbose` in the filter. |
| `building_sites_dump` | `METRUM_DEBUG_BUILDING_SITES_DUMP` = `1`, `true`, `yes` or `full`. |

Rust road-commit code still reads its own road-geometry dump size and full-validation variables
(`METRUM_DEBUG_ROAD_GEOMETRY_DUMP_FULL`, `_MAX_BYTES`, `METRUM_DEBUG_ROAD_COMMIT_FULL_VALIDATION`).

## Visual Modes

Visual modes change what a renderer draws. [`debug::visual`](../rust/src/debug/visual.rs) holds
each mode's selected value as an index into its fixed value list; `off` is always index 0. A
launch variable sets only the starting value, and the console switches every mode at runtime.

| Mode | Values | Launch | Shows |
|------|--------|--------|-------|
| `terrain_visual` | `off`, `patch`, `lod`, `height`, `relief`, `shore`, `water-depth`, `water-lod`, `water-patch`, `water-material`, `lighting` | `METRUM_DEBUG_TERRAIN_VISUAL` | Terrain and water material views ([modes](#terrain--water-visual-modes)). |
| `terrain_grass` | `off`, `raw`, `macro`, `mid`, `micro`, `fades`, `material`, `height`, `mask`, `luminance`, `footprint` | `METRUM_DEBUG_TERRAIN_GRASS` | Terrain grass layers ([modes](#terrain-grass-visual-modes)). |
| `building_sites_visual` | `off`, `material` | `METRUM_DEBUG_BUILDING_SITES_VISUAL` | Site ground, asphalt and concrete tinted by material source ([modes](#building-site-visual-modes)). |
| `pedestrian_vat` | `off`, `rest`, `uv`, `offsets` | `METRUM_DEBUG_PEDESTRIAN_VAT`, overridden by `--pedestrian-vat-debug` | `rest`: no VAT animation, so rigid sliding is expected. `uv`: vertex-ID UVs. `offsets`: VAT offset magnitude while offsets apply. |
| `terrain_force_lod1` | `off`, `on` | `METRUM_DEBUG_TERRAIN_FORCE_LOD1`, or `terrain-lod1` / `terrain-full-lod1` in the filter | Every resident terrain and water patch at LOD1, the finest mesh. |
| `terrain_force_full_world` | `off`, `on` | `METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD`, or `terrain-full` / `terrain-full-lod1` in the filter | Every terrain patch resident, with water following it. |

These rules apply:

- Names are case-insensitive. Each value also accepts its index, and the aliases listed in the
  linked mode tables. `false` and an empty value are `off`; `on` modes also accept `true` and
  `yes`.
- An unknown launch value starts `off`. The exception is `building_sites_visual`: any other set
  value starts its only overlay, `material`.
- At runtime, an unknown mode or value is refused and the reason is logged on `events`.
- `pedestrian_vat` `off` now means no debug view. Before `DEBUG-02` it selected the offsets
  view, which is now `offsets` (alias `offset`).
- Switching `terrain_visual` to `lighting` does not turn on the `lighting` channel. Its launch
  variable still does, as before.

### Applying a switch

`DebugConsole.set_visual_mode(mode, value)` is the one switch path. It calls
`MetrumDebug.set_visual_mode` and applies the result:

- `terrain_visual` and `terrain_grass` are global shader uniforms, `terrain_visual_debug_mode`
  and `terrain_grass_visual_debug_mode` (`[shader_globals]` in `project.godot`). The terrain
  and water shaders read the first, and the terrain shader reads the second. A switch is one
  `RenderingServer.global_shader_parameter_set` call; no patch material is touched. The console
  also sets both globals from the launch values when it loads, before any scene is built.
- Every other mode reaches its renderer through the console's `visual_mode_changed(mode)`
  signal:
  - `pedestrian_vat`: the walker renderer sets `debug_mode` on its four walker materials.
  - `building_sites_visual`: the building renderer swaps the site ground override and asks for
    a fresh site frame. The next frame rebuilds the site surfaces with the new materials through
    the normal site-mesh path. This is one O(sites) site mesh rebuild per switch.
  - `terrain_force_lod1` and `terrain_force_full_world`: the terrain and water renderers update
    their cached flags. Residency already reads the full-world flag on every frame, so the next
    frames load or drop patches through the normal per-frame budget, without a teardown. LOD
    refresh normally waits for the camera to move, so the switch makes it due at once. Turning
    either mode off returns to the camera-based patch set and LODs.
- Selecting the current value changes nothing and emits nothing.

While no mode changes, the switch path costs nothing per frame. Nothing on the simulation tick
reads visual modes. The cost of a mode itself is unchanged: full-world residency costs what it
did when it could only be set at launch.

`MetrumDebug.set_visual_mode` on its own changes the stored value, but renderers do not see the
change; script code switches modes through `DebugConsole`.

## Output Contract

- Enabled lines print to stdout as `[DEBUG:<tag>] message`. Rust and GDScript lines use the same
  format, and the same text as before `DEBUG-01`. `tools/road_terrain_replay.py` and `run.sh`
  parse it.
- `traffic` lines print to stdout as `[DEBUG:traffic] [JUNCTION_WAIT] ...`. They used to go to
  stderr unprefixed.
- `events` lines never print. Each source has already printed its own warning or error, so
  `events` only keeps them in the log.
- A line prints and joins the log under one lock, so log sequence numbers follow stdout order
  across threads and across Rust and GDScript.

## Shared Log

[`debug::log`](../rust/src/debug/log.rs) keeps up to 4096 lines in a ring buffer and drops the
oldest line first. Each line stores:

- a sequence number, starting at 1 and never reused;
- milliseconds since debug init;
- the channel;
- the source, `rs` or `gd`;
- the message without its prefix.

Lines join the log only while their channel is on.

### Cost

- A disabled call costs one relaxed atomic load and a constant mask test. Its arguments are not
  evaluated or formatted, and it never allocates. A unit test proves this with a side-effecting
  argument.
- An enabled line costs one `String`, the stdout write and one short mutex hold.
- A push is O(1). `read_since` is O(log 4096 + k) for k returned lines.

Channel checks on the agent tick and movement paths cost the same as the four `AtomicBool`
flags they replaced. Treat enabled channels as diagnostic cost: an enabled `traffic` channel
logs per agent event, and `--debug traffic` performance numbers are not acceptance timings.

## Events

The `events` channel collects messages the game already reports. It never adds per-agent or
per-tick messages of its own.

- The debug console installs a Godot `Logger`
  ([`debug_event_logger.gd`](../godot/scripts/ui/debug_console/debug_event_logger.gd)). It
  copies every `push_warning`/`push_error`, `godot_warn!`/`godot_error!`, script and shader
  error, and `printerr` into `events`. A script call site replaces the engine's
  `push_warning` location when one is available.
- The road tool logs a click on an invalid preview as `road_tool rejected reason=...`, with the
  grade numbers it already computed.
- The road tool logs a rejection by the simulation as `road_commit_rejected request_id=...
  detail=...`.

Godot calls loggers from any thread. The logger keeps no state and forwards to the thread-safe
Rust log.

## Godot Bridge

`MetrumDebug` ([`debug_bridge.rs`](../rust/src/nodes/debug_bridge.rs)) is a stateless
`RefCounted` class with static methods only. The game, the world editor and the asset editor all
share it. It does not depend on `SimulationNode`.

| Method | Contract |
|--------|----------|
| `channel_names()` | Every tag, in registry order. |
| `is_enabled(tag)` / `set_enabled(tag, on)` | Live channel state. `set_enabled` returns `false` for unknown tags and for switching `events` off. |
| `is_locked(tag)` | `true` for `events`. |
| `log(tag, message)` | Prints and keeps a GDScript line if the channel is on. Returns whether it was kept. |
| `read_since(seq, max)` | Up to `max` newer lines as packed columns `seq`, `time_ms`, `channel`, `source`, `message`, plus `last_seq`. |
| `clear()` | Drops buffered lines. Sequence numbers keep counting. |
| `write_dump(path)` | Writes channel state, visual and startup modes and the log to a `user://` or absolute path. Returns the absolute path, or `""` after a warning. |
| `startup_modes()` / `startup_mode(name)` | The startup-only modes above, each a `bool`. Unknown names are `false`. |
| `visual_mode_names()` / `visual_mode_values(mode)` | The visual modes in console order, and one mode's value names with `off` first. |
| `visual_mode(mode)` / `visual_mode_index(mode)` | The selected value name, and its index, which is also the shader's mode number. Unknown modes are `off` / `0`. |
| `set_visual_mode(mode, value)` | Stores a value by name, alias or index. Returns `false` and logs the reason on `events` when refused. Renderers learn of the change only through `DebugConsole.set_visual_mode`. |
| `is_debug_session()` | Whether the game was launched with `METRUM_DEBUG=1`. |

GDScript debug flags are properties whose getter calls `MetrumDebug.is_enabled`. A channel
switched in the console therefore takes effect on the next frame or the next action. A summary
that runs once at load, such as the lighting summary, does not repeat after a switch.
`PerfDebug` keeps its 0.5 s summaries, logs them on `perf` and exposes the latest one as
`PerfDebug.last_summary`.

## Console

`DebugConsole` is an autoload
([`debug_console.gd`](../godot/scripts/ui/debug_console/debug_console.gd), with the view in
[`debug_console_view.gd`](../godot/scripts/ui/debug_console/debug_console_view.gd)). It is
available in every build and scene.

### Opening and input

- The physical key left of `1` opens and closes the console (`KEY_QUOTELEFT` by physical
  keycode): `§` on Nordic layouts and `` ` `` on US layouts. The key is always consumed, so it
  never types into a text field.
- The console slides down from the top edge, anchored left. It is full height and one third of
  the window wide, never wider. The rest of the world stays visible and clickable.
- The game keeps the keyboard while the console is open. Clicking the filter or the command line
  gives that field the keyboard, and Esc gives it back to the game. Buttons, chips, tabs and
  toggles never take keyboard focus.

### Tabs and commands

- **Log**:
  - Channel chips show or hide each channel's lines; they do not change capture.
  - A text filter matches the message or the tag, ignoring case.
  - The log scrolls automatically and keeps up to 2000 lines.
- **Perf**: frame FPS and frame time, the simulation tick time, agent and pathfind counts from
  `get_perf_stats()`, and the latest `PerfDebug` summary. It refreshes twice a second while
  visible. `get_perf_stats()` reads the published snapshot only and never locks the simulation
  core.
- **Channels**:
  - on/off toggles for every channel (`events` is locked);
  - a **Visual modes** section with one value picker per [visual mode](#visual-modes), synced
    when the console opens;
  - the startup-only modes, read-only.
- **Command line**:
  - `help`
  - `channel` lists the state of every channel.
  - `channel <tag> on|off`
  - `visual` lists every visual mode's value.
  - `visual <mode>` lists the mode's values.
  - `visual <mode> <value>` takes a value name, alias or index.
  - `visual off` turns every visual mode off, which also returns residency to camera-based.
  - `dump`
  - `clear`
- **Copy** puts the visible, filtered lines on the clipboard.
- **Dump** writes up to three files with the same `user://debug/dump-<date>_<time>` name:
  - `.txt`, the text dump described below;
  - `.sqlite`, an ordinary save of the running game and its camera, written through
    `SimulationNode.save_game`. Only gameplay scenes write it; the world and asset editors and
    the menus have no game to save. It can be loaded like any other save.
  - `.png`, a screenshot of the game view with the console hidden for that frame. Headless runs
    skip it.

  The text dump holds:
  - the uptime;
  - every channel's state;
  - every visual mode's value;
  - the startup modes;
  - every buffered line, as `seq time source [DEBUG:<tag>] message`.

While closed, the console does no work except watch for its key.

## Verification

`godot/tests/debug_console_test.gd` runs in the `./run.sh --test` bridge list. It covers:

- the registry and the `events` lock;
- runtime toggles that drop or keep GDScript lines;
- single capture of a warning, with its script call site;
- opening and closing with the key;
- the one-third, full-height layout;
- the channel command;
- the dump's channel state and lines.

`godot/tests/debug_visual_modes_test.gd` runs in the bridge list and again on a rendered display
(Xvfb or `$DISPLAY`, Linux only). It covers:

- the visual mode registry and starting values;
- switching every mode by command and by its picker, aliases, indices, refusals on `events`,
  `visual off`, and one notification per change;
- on the rendered run only, the values the shaders receive, read from a probe pixel (the
  headless renderer draws nothing, and global shader values cannot be read back at runtime);
- the walker VAT materials;
- the building-site ground override and rebuilt surface materials;
- live terrain and water renderers on an 8 km test world with a corner camera: 49 of 64 patches
  are resident, at LOD 1, 2 and 4. Full-world residency loads all 64; force-LOD1 refines every
  terrain and water patch; switching either off returns the same patch set and LODs.

Rust unit tests in `debug::channel`, `debug::log`, `debug::visual` and `debug` cover:

- the startup rules;
- the ring order;
- the dump line format;
- the startup modes;
- visual mode names, aliases and indices, launch values (variables, filter entries and
  `--pedestrian-vat-debug`), and runtime refusals;
- unevaluated arguments on disabled channels.

### Agent tick measurement (2026-10-12)

`DEBUG-01` changed the channel checks and `traffic_log!` calls on the agent movement path, so
it was compared with a matched release run of `agent_benchmark`:

- **Baseline:** `fd134ec2`, plus only the visibility fix that lets the benchmark compile again.
- **Current:** the `DEBUG-01` working tree.
- **Setup:** three alternating pairs on an i9-12900K, CPU affinity `0,2,...,14`,
  `RAYON_NUM_THREADS=8`, `METRUM_DEBUG=0`, unprofiled, with separate Criterion homes:

```bash
RAYON_NUM_THREADS=8 METRUM_DEBUG=0 CRITERION_HOME=$ARTIFACTS/criterion-<side> \
  taskset -c 0,2,4,6,8,10,12,14 $BENCH_BINARY --bench \
  'AgentSystem::tick/(on_road|idle_scaling)/100000$|AgentSystem::tick_access/access_(egress|ingress)_car/100000$' --noplot
```

| Case (100k agents) | Baseline runs | Current runs | Median change |
|--------------------|---------------|--------------|---------------|
| `tick/on_road` | 1.846, 1.814, 1.832 ms | 1.861, 1.836, 1.819 ms | +0.2% |
| `tick/idle_scaling` | 201.3, 205.8, 204.2 µs | 212.7, 208.7, 204.9 µs | +2.2% |
| `tick_access/access_egress_car` | 4.935, 4.594, 4.536 ms | 4.562, 4.473, 4.476 ms | −2.6% |
| `tick_access/access_ingress_car` | 924.4, 920.2, 915.9 µs | 968.9, 974.9, 935.7 µs | +5.3% |

`on_road`, `idle_scaling` and egress are within run-to-run spread.

Ingress is slower in all three pairs. Code placement causes this, not added work.
`objdump` of `handle_access_ingress`, which is not inlined, shows:

- **Instruction count:** 662 before and 663 after (3514 → 3488 bytes).
- **The disabled check:** a byte load with `test %al,%al; je` before; a 32-bit mask load with
  `test $bit,%al; jne` after.
- **Everything else:** register allocation inside the cold logging branches, which never run
  with `METRUM_DEBUG=0`.

A control rebuilt both sides with every function and branch target forced to 64-byte
alignment, which removes placement effects:

```bash
RUSTFLAGS='-C target-cpu=x86-64-v2 -C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=6'
```

It ran the access cases as three more alternating pairs with the same settings:

| Case (100k, aligned) | Baseline runs | Current runs | Median change |
|----------------------|---------------|--------------|---------------|
| `tick_access/access_egress_car` | 4.506, 4.519, 4.517 ms | 4.444, 4.434, 4.425 ms | −1.8% |
| `tick_access/access_ingress_car` | 948.2, 945.0, 939.1 µs | 939.0, 920.7, 974.4 µs | −0.6% |

The ingress gap disappears under forced alignment, so the default-build +5.3% comes from code
placement. `DEBUG-01` adds no per-agent cost.

`PERF-06` tracks a shorter agent-tick contract to replace this 30-minute matched comparison.

## Launch Reference

`run.sh` is the canonical launch wrapper for local debug sessions. These flags set environment
variables before building Rust, deploying `libmetrum_rise.so` on Linux or
`libmetrum_rise.dylib` on macOS, and launching Godot. Flags for [visual modes](#visual-modes)
set only the starting value; the console switches them later.

### Launch Flags

| Flag | Main environment | Output / effect |
|------|------------------|-----------------|
| `--release` | `METRUM_CRASH_DIAGNOSTICS=1`, `METRUM_CRASH_LOG_DIR=<repo>/logs` unless already set | Enables the release-safe panic hook, hang watchdog, and fixed-size flight recorder. Panic and hang dumps are written under `logs/`. Set `METRUM_CRASH_DIAGNOSTICS=0` to disable diagnostics, `METRUM_HANG_WATCHDOG_MS=0` to disable only the hang watchdog, or `METRUM_HANG_ABORT=1` to abort after the first hang dump. |
| `--debug` | `METRUM_DEBUG=1` | Every general channel (stdout). The opt-in channels `traffic`, `sim`, `perf`, `terrain`, `buildings` and `lighting` stay off. |
| `--debug <category>` | `METRUM_DEBUG=1`, `METRUM_DEBUG_FILTER=<category>` | Only the named channels, comma-separated. Channel names are listed under [Channels](#channels); the terrain and building-site aliases are listed under [Startup state](#startup-state). |
| `METRUM_DEBUG_FILTER=<category> ./run.sh ...` | `METRUM_DEBUG=1`, `METRUM_DEBUG_FILTER=<category>` | Environment shorthand for `--debug <category>`. |
| `--debug road` | `METRUM_DEBUG_FILTER=road`, `METRUM_DEBUG_ROAD_GEOMETRY_DUMP=1` | Road placement timings, geometry dump and terrain/water patch diagnostics. Add `METRUM_DEBUG_SURFACE=1` for the road-surface overlay. |
| `--debug terrain` | `METRUM_DEBUG_TERRAIN=1` | Terrain and water patch residency/perf summaries. |
| `--debug terrain-verbose` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_VERBOSE=1` | Terrain summaries plus residency-change logs. |
| `--debug terrain-full` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD=1` | Starts with full-world terrain/water residency, for cost comparison. |
| `--debug terrain-lod1` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_FORCE_LOD1=1` | Starts with every resident terrain/water patch mesh at LOD1. |
| `--debug terrain-full-lod1` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD=1`, `METRUM_DEBUG_TERRAIN_FORCE_LOD1=1` | Starts with both, for seam/material debugging. |
| `--debug perf` | `METRUM_DEBUG_PERF=1` | Frame CPU summaries by renderer every 0.5 s. Reports total renderer CPU per completed frame plus per-renderer/detail averages and maxes. |
| `--debug buildings` | `METRUM_DEBUG_BUILDINGS=1` | Building-site mesh, material, height, and site metadata logs only. |
| `--debug building-sites-visual [mode]` | `METRUM_DEBUG_BUILDING_SITES_VISUAL=<mode>` | Starts the building-site visual overlay. Current mode: `material`. |
| `--debug site-grading` | `METRUM_DEBUG_FILTER=road`, `METRUM_DEBUG_BUILDINGS=1`, `METRUM_DEBUG_BUILDING_SITES_VISUAL=material` | Combined road CDT, building-site edge/grading, and site material diagnostics for road/yard seam debugging. |
| `--debug traffic` / `--debug-traffic` | `METRUM_DEBUG_TRAFFIC=1` | Traffic/routing and road-network connectivity logging to stdout as `[DEBUG:traffic]` plus visual lane/connector debug labels. Emits road split/apply tags, road-edit agent remap tags, entrance-cache diffs, exact replan rejection diagnostics, and watchdog recovery lines. |
| `--pedestrian-vat-debug=<mode>` | `METRUM_DEBUG_PEDESTRIAN_VAT=<mode>` | Starts a pedestrian VAT material view: `rest`, `uv` or `offsets` (see [Visual Modes](#visual-modes)). `off`, or no flag, gives normal animated character colors. |
| `--debug-world-editor` | `METRUM_DEBUG=1`, `METRUM_DEBUG_FILTER=world-editor` | WorldEditor create/open/save/tool activity. |
| `--debug-sim` | `METRUM_DEBUG_SIM=1` | Hourly simulation summaries to stdout. |
| `--debug visuals [mode]` / `--debug visual [mode]` / `--visuals [mode]` | `METRUM_DEBUG_TERRAIN_GRASS=<mode>` | Starts a terrain grass material view. Defaults to `material`. |
| `--debug terrain-visual <mode>` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_VISUAL=<mode>` | Starts a terrain/water/lighting material view. Defaults to `patch`. |

Removed debug aliases:

- `--debug road-geometry` is intentionally rejected; use `--debug road`

### Terrain Grass Visual Modes

Values of the `terrain_grass` visual mode: `visual terrain_grass <mode>` in the console, or
`--debug visuals <mode>` / `--visuals <mode>` at launch.

| Mode | Aliases | Meaning |
|------|---------|---------|
| `raw` | `albedo` | Direct Grass002 albedo at terrain UV scale. |
| `macro` | | Large stochastic grass layer. |
| `mid` | | Mid-distance stochastic grass layer. |
| `micro` | | Close-up grass layer. |
| `fades` | `fade`, `visibility` | RGB visibility of macro / mid / micro grass layers. |
| `material` | `composite` | Grass material composite before hillshade and contour effects. |
| `height` | | Grass002 height map. |
| `mask` | `grass-mask` | Where grass detail is allowed. |
| `luminance` | `luma`, `brightness` | World-space bands for base, macro, mid, micro, and final brightness. |
| `footprint` | `footprints` | RGB = texture footprint, micro visibility, grass mask. |

`luminance` band colors:

- red = base brightness
- yellow = macro brightness
- green = mid brightness
- blue = micro brightness
- white = final brightness

### Terrain / Water Visual Modes

Values of the `terrain_visual` visual mode: `visual terrain_visual <mode>` in the console, or
`--debug terrain-visual <mode>` at launch.

| Mode | Aliases | Meaning |
|------|---------|---------|
| `patch` | `patches` | Terrain patch identity colors with patch borders. |
| `lod` | `lods` | Terrain mesh LOD colors with patch borders. |
| `height` | | Terrain height field. |
| `relief` | | Local terrain relief. |
| `shore` | `shoreline` | Terrain-side shore mask from visible water depth. |
| `water-depth` | `water`, `depth` | Water depth field on terrain and water. |
| `water-lod` | | Water mesh LOD colors. |
| `water-patch` | | Water patch identity colors. |
| `water-material` | `water-mat`, `material-water` | Water material bands: depth tint, alpha, Fresnel, foam, and normal strength. |
| `lighting` | `light`, `sun` | Sun-facing strength, shadow/cascade bands, and water specular mask. |

### Building-Site Visual Modes

Values of the `building_sites_visual` visual mode: `visual building_sites_visual <mode>` in the
console, or `--debug building-sites-visual <mode>` at launch.

| Mode | Aliases | Meaning |
|------|---------|---------|
| `material` | `materials`, `source`, `sources` | Tints site ground, asphalt, and concrete by material source. |

