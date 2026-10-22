# Metrum Rise — Debug System

This document owns the runtime debug contract (`DEBUG-01` to `DEBUG-09`): the debug channels,
the runtime visual modes, the junction connector overlay and stall detector, road rejection
reports and their replay, the agent inspector, trip failure lines, the log format, the shared log, the always-on `events` channel, the Godot
bridge, the in-game console, crash and hang diagnostics, how the extension's threads stop before
Godot unloads it, and the `run.sh` launch flags.

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
| `junction_connectors` | `off`, `arcs`, `speed`, `occupancy`, `lanes` | `METRUM_DEBUG_JUNCTION_CONNECTORS` | The connector arcs cars drive through junctions near the camera, and the stuck-at-junction detector ([details](#junction-connectors)). Aliases: `on` and `connectors` for `arcs`, `turn-speed` for `speed`, `cars` for `occupancy`. |

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
  - `junction_connectors`: the junction overlay rebuilds its mesh for the new view, or clears
    it and drops the stall detector when the view goes off.
- Selecting the current value changes nothing and emits nothing.

While no mode changes, the switch path costs nothing per frame. Nothing on the simulation tick
reads visual modes. The cost of a mode itself is unchanged: full-world residency costs what it
did when it could only be set at launch.

`MetrumDebug.set_visual_mode` on its own changes the stored value, but renderers do not see the
change; script code switches modes through `DebugConsole`.

### Junction connectors

Every allowed turn at a junction is a connector lane: a sampled cubic Bezier in `Lane::geometry`
(see [Junction Connectors](traffic.md#junction-connectors)). Cars drive that exact polyline, and
the `junction_connectors` overlay draws it unchanged, 0.25 m above the road, with a chevron
three quarters along each lane pointing the travel direction.

| View | Draws |
|------|-------|
| `arcs` | Every vehicle connector. Orange: the node whitelists its turns (it has a user lane connection). Blue: the default open turns. |
| `speed` | Connectors colored by `connector_turn_speed()`: red at `CAR_JUNCTION_MIN_SPEED_MS` (2 m/s), yellow midway, green at `CAR_JUNCTION_SPEED_MS` (6 m/s). |
| `occupancy` | Grey: empty. Yellow, orange, red: one, two, three or more cars. Magenta: a car stopped (below 0.1 m/s) at the connector end, waiting for its exit lane. |
| `lanes` | The `arcs` colors plus the vehicle lane center lines of every edge at the drawn junctions, to check how lanes line up at junction mouths. |

A strict degree-two split links road lanes directly and has no connector; the overlay marks it
with a violet diamond, so "no connector" is distinguishable from "missing". Every view draws
the markers.

Coverage and cost:

- The overlay ([`junction_debug_overlay.gd`](../godot/scripts/renderers/junction_debug_overlay.gd))
  covers the camera's 512 m chunk and its eight neighbours, a 1,536 m square around the camera's
  focus point. Rust finds the junctions through the 16 m node lookup grid, never a scan of all
  nodes or lanes. A build costs O(grid cells in the square + nodes found + vertices drawn); its
  output is one line-list mesh for all junctions.
- `arcs`, `speed` and `lanes` rebuild only when the network render generation, the view, the
  focus or the camera's chunk changes. `occupancy` also rebuilds four times a second.
- If the simulation core is busy, a rebuild keeps the last mesh and retries on the next frame.
- While the view is off the overlay does no work at all, and nothing here runs on the
  simulation tick.

Focus: with no tool active, a click within 12 m of a drawn junction focuses it. The overlay then
draws only that junction, and the click is not passed on to the building inspector. Focusing logs
the junction on `events`, one line per connector, as below. A click elsewhere clears the focus
and then inspects as usual; Esc also clears it. While a tool is active, clicks go to the tool.

```text
junction 12: 6 vehicle connectors, turns open
  connector 41: from edge 3 lane 0 to edge 5 lane 0, 17.8 m, cap 6.0 m/s, open
```

Stuck-at-junction detector: while the view is on, the overlay asks Rust to scan four times a
second of real time. Each scan does nothing until one simulated second has passed since the last.
A scan reads the lane occupancy snapshot the agent tick leaves behind. It visits only occupied
lanes and the cars on vehicle connectors, so it costs O(occupied lanes + cars on connectors).
A car that has stayed within 0.5 m on one connector for 10 simulated seconds is logged once on
`events`:

```text
junction stall: car 5120 (agent 233) stopped 10 s at node 12 on connector 41 (17.8/17.8 m): exit lane 77 entry occupied by car 4410 at 1.2 m
```

The cause is one of:

- `exit lane N entry occupied by car C at D m`: the car waits at the connector end, and the exit
  lane's entry slot is taken. This is the exit check the agent tick runs.
- `exit lane N entry is clear now; merging connectors keep winning its claim`: the car waits at
  the end, but the entry is clear at scan time.
- `queued behind car C at D m on the same connector`.
- `connector has no exit lane`.
- `no blocker found (speed S m/s)`.

The detector keys cars by their runtime-stable render id, so swap-remove does not confuse it.
It forgets a car that leaves its connector, starts over if the simulated clock goes back (as
on a load), and is dropped with all its state when the view goes off. It adds no per-agent
field and no per-tick work.

`SimulationNode` methods behind the overlay:

| Method | Contract |
|--------|----------|
| `get_junction_debug_lines(view, center, half_extent_m, focus_node)` | Line-list `points` and `colors` plus the counts `junctions`, `connectors`, `pass_throughs`, `occupied` and `blocked`. Covers the square around `center`, or only `focus_node` when it is not negative. Empty while the core is busy or `view` is `0`. |
| `pick_debug_junction(pos, radius_m)` | Nearest drawn junction within `radius_m` (XZ), or `-1`. |
| `log_junction_connectors(node_id)` | Logs the focus lines on `events`; returns how many. |
| `scan_junction_stalls()` | One detector scan, rate-limited to once per simulated second; returns the number of new reports. |
| `clear_junction_stalls()` | Drops the detector. |

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

The `events` channel collects messages the game already reports. It never adds per-tick
messages of its own. Per-agent messages come only from the stall detector, once per stalled car
while the `junction_connectors` view is on, and from [trip failures](#trip-failures), with flood
control. Per-stroke messages come only from refused roads.

- The debug console installs a Godot `Logger`
  ([`debug_event_logger.gd`](../godot/scripts/ui/debug_console/debug_event_logger.gd)). It
  copies every `push_warning`/`push_error`, `godot_warn!`/`godot_error!`, script and shader
  error, and `printerr` into `events`. A script call site replaces the engine's
  `push_warning` location when one is available.
- Every refused road stroke, whether refused by the road tool's click check or by the simulation's
  commit, logs a [road rejection report](#road-rejections).
- The console's dump logs `dump save=<path> road_generation=<n>` before it writes the text dump.
- The [junction overlay](#junction-connectors) logs a focused junction's connectors and, while
  on, each stalled car.
- Every [trip failure](#trip-failures) logs why and what happens next, folded into a summary per
  minute past 20 per place or 200 in total.

Godot calls loggers from any thread. The logger keeps no state and forwards to the thread-safe
Rust log.

## Road Rejections

A refused road stroke logs two lines on `events`. The first says why, with the numbers the check
used, where, and what was drawn:

```text
road rejected by commit: parcel_overlap: the road corridor overlaps zoned parcels; 3 parcels, first 812. Stroke (412.0, -96.5) -> (530.2, -40.1), 2 points, 131.0 m, lanes 1/1, snap on; start node 57, road y 12.40, terrain y 12.40; end open terrain, road y 14.05, terrain y 13.90. Attempt 4.
road_attempt {"attempt":4,"by":"commit","reason":"parcel_overlap","generation":31,"fwd_lanes":1,"bkw_lanes":1,"snap":true,"points":[[412.0,12.4,-96.5],[530.25,14.05,-40.125]]}
```

- `by` is `tool` when the road tool's click check refused the stroke, before anything reached the
  simulation, and `commit` when the simulation's authoritative commit refused it.
- The reason is the stable key the checks use: `bridge_clearance`, `tunnel_clearance`,
  `same_node_connection`, `too_short`, `water_requires_bridge`, `surface_geometry_invalid` and
  the `terrain_clip_*` compile failures, `field_overlap`, `parcel_overlap`,
  `zoning_cell_overlap`, `render_geometry_invalid` (the committed surface or its terrain failed
  to build and the edit was rolled back), `validation_unavailable`, and the plan states
  `road_plan_stale`, `road_plan_pending`, `road_plan_provisional`, `road_plan_consumed`,
  `road_plan_mismatch` and `road_plan_invalid` (terrain earthwork failure). A plan refused
  because its stroke failed a check reports that check instead of the plan state.
- `at (x, z)` follows the detail when the check measured one place: clearance is measured at the
  middle of the prepared road. The stroke's ends give the snapped node or open terrain, with
  the road and terrain heights there.
- `road_attempt` carries the stroke exactly as submitted: every point at full `f32` precision,
  the lane counts, snapping and the road-tool generation (`get_road_tool_surface_generation`),
  which changes whenever the road network is rebuilt: a built or bulldozed road, an undo, a
  loaded world. A refused stroke leaves it unchanged. Both lines end with the same attempt
  number.

The road tool sends its click refusal through `SimulationNode.log_road_tool_rejection(points,
fwd_lanes, bkw_lanes, snap, validation)`, with the validation it showed. The commit report comes
from `SimCore::commit_road` on the simulation thread. `get_road_commit_result` also returns the
reason as `reason`. Reports are built only for a refused stroke, so placing roads costs the same
as before, apart from one copy of the stroke's points per commit.

### Replaying refusals

The console's **Dump** writes the save first and logs `dump save=<path> road_generation=<n>`, so
the text dump names its save and the road network it holds. To check a refusal against it:

```bash
./run.sh --replay-road-attempts <dump>.txt
```

This builds, then runs `godot/tests/replay_road_attempts.gd` headless in a throwaway profile. It
loads every installed asset pack read-only, loads the save, and sends each `road_attempt` along
the player's path: the click check, then, if that passes, the commit. Each attempt prints one
line:

```text
attempt 4 (commit, parcel_overlap): reproduced, refused by commit
attempt 2 (tool, same_node_connection): reproduced, refused by tool [made on road generation 29, save holds 31: the network changed since]
```

- An attempt made at the save's road generation was made on the saved network, so it should be
  refused again for the same reason. The command exits `1` if one is not, `0` otherwise, and
  `2` for a missing dump, save or attempt.
- An older attempt was made on an earlier network; its result only hints. Zoning, fields and
  building sites do not change the road generation, so a parcel or field refusal can also stop
  reproducing after zoning edits made on the same network.
- A replayed stroke that is built stays in the loaded copy, so later attempts see it. The save
  file is never written.
- `METRUM_ROAD_REPLAY_SAVE` replays against another save; `METRUM_ROAD_REPLAY_PACKS` replaces
  the asset pack folder.
- The replay also reads a captured stdout log, since `events` lines print there too; pass the
  save with `METRUM_ROAD_REPLAY_SAVE` when the log has no `dump save=` line.

## Agent Inspector

`DEBUG-05`. With no tool active, a click on a car, bike or pedestrian that is on a road or in a
junction inspects its agent; a click on another agent switches to it. Rust keeps the selection,
so it survives other agents being removed. While an agent is inspected:

- A window opens beside the cursor, like the Building Inspector, and the player can drag it by
  its title bar and resize it from its edges; its size is kept between sessions. It shows the
  agent's render id (the `agent` of `events` lines), mode, state, speed, activity, home, work,
  destination and the building it is in (id, zone kind and economy profile, or `none`), its
  lane with the distance along it, the route (path node *i* of *n*, lanes ahead, metres
  left and how the route ends) and its latest [trip failure](#trip-failures) line. Clicking another agent keeps the window where it is.
- Home, work and destination are links. A link stops the follow, frames that building and
  opens its Building Inspector beside the agent window (an inspector already open for it stays
  open). The inspection and its route stay; a Follow button brings the camera back to the
  agent and follows it again.
- The window takes the keyboard focus only while a mouse button is held inside it (so a resize
  drag that leaves the window keeps working), then gives it back: zoom, orbit and Esc keep
  reaching the game while it is open.
- The route is drawn from the agent to its destination door, above buildings and terrain, as a
  6 px line inside a dark 2 px outline. The width stays the same on screen at any zoom, so the
  route can be traced from far away: Rust turns each line segment into two triangles, and the
  `route_ribbon` shader moves their corners apart across the segment's screen direction.
- The camera pivot sits on the agent's drawn position every frame. Zoom and orbit work as
  usual. Inside a building the pivot stays at its door, and follows again when the agent comes
  out.
- Panning (WASD) takes the camera back: the follow stops, but the inspection, its window and
  its route stay, so the player can look along the route. Follow in the window returns to the
  agent. Esc, or a click that hits no agent, still ends the inspection.

| Route color | Part |
|-------------|------|
| Blue | The lane or connector the agent is on. |
| Green | The next road lane. |
| Cyan | Road lanes after that. |
| Amber | Junction connectors. |
| Pale yellow | Driveways between a door and the road. |
| Magenta cross with a post | The destination door. |
| Red cross with a post | Where the route stops without reaching a destination. |

The route is what the agent will drive, not a straight line between nodes. Rust walks the
remaining node path with the movement tick's own rules: the path index steps as at a lane end,
lanes continue straight through degree-two splits, and the zero-hop connector leads into the
planned detach lane. Among equally short connectors it takes the stable pick, which is the one a
car takes when the junction is free. Overtaking, returning to the outer lane, or waiting for a
busy connector can put the car on another lane; the drawing then follows from that lane. The
`Route end` row says how the route ends:

- `destination building N`: the agent leaves the road at its planned detach point and drives to
  the door.
- `path ends at node N`: the path stops at node N without a detach, as a trip to the world border
  does; a car that still has a destination replans there.
- `no lane leads on to node N; the agent replans there`: the movement tick will drop the path at
  this point.
- `no route on the roads`: the agent is in a building or moving in from the world border.

Starting logs the agent on `events`. The inspection ends on Esc, the window's close button, a
click that hits no agent (the click then reaches the building inspector), a world load, or the
agent's removal; the camera stays where it is. A removal is explained on `events`:

```text
agent inspection: agent 5120, car, on a road, home 12 residential (house_small), work 40 office (office_tower), destination 40 office (office_tower)
agent inspection ended: agent 5120 left the city with its household
```

The other removal is `finished its freight trip`; anything else says `is no longer in the
simulation`.

Limits:

- An agent on a driveway (leaving or entering a building) or moving in from the world border is
  not on a lane and cannot be picked until it reaches a road. An agent already inspected stays
  inspected through those states.
- The window has no source building. Agents do not store where a trip began, and `DEBUG-05`
  adds no per-agent field for it.

Cost:

- A pick visits the roads and junction nodes within the pick radius plus 20 m (the edge R-tree
  and the 16 m node grid), their lanes, and the agents on those lanes from the lane occupancy
  snapshot. An entry made stale since the last tick fails the agent's current-lane check. The
  pick radius is 1.5% of the camera distance, from 3 m to 15 m.
- The selection stores the agent's row and render id. The single swap-remove site repairs the
  row in O(1), so each frame's `get_agent_inspection` is O(1) and never searches the agents.
  The camera follow adds one lookup in the car smoother's drawn poses.
- A route build is O(path nodes + connectors at each junction + geometry points drawn). It runs
  ten times a second and when the agent changes lane or path.
- With nothing inspected the overlay does no work. The agent system gains two optional fields,
  no per-agent data, and no tick work beyond one check per agent removal.

Code: [`agent_inspector.gd`](../godot/scripts/renderers/agent_inspector.gd) (controller),
[`agent_inspector_panel.gd`](../godot/scripts/ui/agent_inspector_panel.gd) (view),
[`agent_inspection.rs`](../rust/src/nodes/sim/render/agent_inspection.rs) (picking, details,
lines), [`route_preview.rs`](../rust/src/simulation/economy/agents/tick/route_preview.rs) (the
route walk) and [`inspection.rs`](../rust/src/simulation/economy/agents/inspection.rs) (the
selection).

| Method | Contract |
|--------|----------|
| `SimulationNode.pick_debug_agent(pos, radius_m)` | Inspects the agent on a road or junction lane nearest `pos` (XZ) within `radius_m`; returns its render id, or `-1` and leaves the inspection as it was. |
| `SimulationNode.clear_agent_inspection()` | Ends the inspection. |
| `SimulationNode.get_agent_inspection(render_id)` | The window's fields, `position`, `last_failure` (the latest trip failure line, or empty), and `drawn_position` for a car drawn this frame. While the core is busy, only `drawn_position` (or nothing), so the follow keeps up during long ticks; `active: false` with a `reason` once `render_id` is no longer inspected. |
| `SimulationNode.get_agent_route_lines(render_id)` | Line-list `points` and `colors`; the same lines as a ribbon (`ribbon_vertices`, `ribbon_uv`, `ribbon_uv2`, `ribbon_colors`: six corners per segment, each with the segment's other end in `uv` and `uv2.x` and its side in `uv2.y`); plus `lanes`, `remaining_m` and `end`. Empty while the core is busy or when `render_id` is not inspected. |
| `SimulationNode.spawn_debug_car(from, to)` | Debugging and tests: a car at the start of the shortest car route between the road nodes nearest `from` and `to`, with no home or destination; returns its render id or `-1`. It changes the simulation outside the tick, so the session is no longer reproducible. |
| `CameraNode.follow(target)`, `stop_follow()`, `is_following()` | `follow` puts the pivot on `target` and starts following; `pan` and `pan_screen` end the follow, so `is_following()` turns false when the player pans. |

## Trip Failures

`DEBUG-06`. Every trip failure in the movement tick logs a line on `events`, so a stranded car or
a resident who never leaves home can be explained without `--debug traffic`. A line names the
agent by its render id (the agent inspector's id) with its row in brackets. It then gives its
kind (resident, freight carrier or household moving in), its mode, where it was (node, lane,
edge and distance, or the building it is in), its destination building with its zone kind, or
its border node, the reason, and what happens next:

```text
trip failed: agent 5120 [row 33], resident, car, at node 12, lane 77 on edge 5 at 93.5 m, to building 418 (commercial): no turn from lane 77 (edge 5) onto edge 30 at node 12; drops its route and replans
trip failed: agent 5120 [row 33], resident, car, at node 12, lane 77 on edge 5 at 30.0 m, to building 418 (commercial): planned exit from the road no longer legal (building 418, lane 80 at 40.0 m), then no route from node 13 to building 418: no road path from node 13 to node 40; no road path from node 13 to node 41; retry in 5 s (failure 1 of 3)
trip failed: agent 5120 [row 33], resident, car, at node 12, lane 77 on edge 5 at 30.0 m, to building 418 (commercial): planned exit from the road no longer legal (building 418, lane 80 at 40.0 m), then no route from node 13 to building 418: no road path from node 13 to node 40; no road path from node 13 to node 41; sent home to building 12 at (120.0, 44.0) after 3 failures
trip failed: agent 7 [row 2], resident, in building 12, to building 418 (commercial): no route from building 12 to building 418 by car or on foot; tries to leave again in 30 s
```

The reason comes from the failure site's own data. For a failed replan, that is the planner's
rejection of each exit candidate:

| Reason | Where it comes from |
|--------|---------------------|
| `no road from node A to node B, the next node on its route` | The route's next node has no road from the junction. |
| `no turn from lane L (edge E) onto edge F at node N` | No connector onto the route's next edge. A car drops its route; a pedestrian waits at the lane end. |
| `no turn from lane L into its exit lane X at node N` | No connector into the planned exit lane (zero-hop access). |
| `no route from node N to building B: ...` | The replan found no exit candidate. Each candidate's rejection follows: `no road path from node N to node M`, `no lane by the door from node M`, `no turn into exit lane X at node M`, `no way from exit lane X to the door`, `no walkable path ...`, and so on. |
| `building B no longer exists`, `the road beside building B (edge E) is gone` | The destination itself is gone. |
| `planned exit from the road no longer legal (building B, lane X at d m), then ...` | The planned exit lane no longer serves the door, and the replan failed for the reason that follows. |
| `planned way onto the road no longer legal (building B, lane X at d m)` | The planned entry onto the road no longer serves the door, so the agent goes back inside. |
| `no road path from node N to border node M` | A freight carrier's route to its border. |
| `no route from border node M to building B` | A household moving in. |
| `no route from building A to building B by car or on foot` | A departure that could not be planned (also `on foot (no car)`). |
| `no position on the road to plan from`, `travelling with no plan to reach its destination` | The agent's own state is missing. |

What happens next is `drops its route and replans`, `waits at the end of its lane`,
`retry in 5 s (failure k of 3)`, `tries to leave again in 30 s`, or the watchdog's recovery
after the third failure: `sent home to building B at (x, y)`, `sent to border node N at (x, y)`
(with `to drive in to building B again` for a household moving in), or
`no border node to send it to`. Road edits clear the routes they touch, so junction lines point
at lane or connector data that disagrees with the road graph.

Flood control keeps the 4096-line log readable when a road edit strands many agents:

- An agent that repeats the same failure at the same place within a simulated minute of the
  last one is not logged again. The watchdog's quiet retries therefore log once, and its
  recovery logs as a failure of its own.
- Each simulated minute logs at most 20 lines per reason and place (the node, or the building
  for departures and entries) and 200 in total.
- The rest are counted per reason and logged once each when the minute ends, with the place
  that had the most:

```text
trip failures: 212 more in the last simulated minute: no turn at a junction, 180 at node 12 (first agent 5120)
```

The inspected agent's failures are never folded into a summary. The agent inspector shows its
latest line in the `Last failure` row, which stays after a watchdog recovery moves the agent.

Cost:

- Failure sites record into one buffer behind a mutex that is only locked when something fails,
  so a pass without failures pays one branch at each failure site; the records are built in
  cold, out-of-line functions. The buffer keeps its capacity between ticks. The agent tick is no
  slower ([measurement](#trip-failure-measurement-2026-10-13)).
- After the movement pass the records are sorted by agent row (O(f log f) for f failures), so
  the log does not depend on the worker schedule. Only logged lines are formatted.
- The streak and per-minute counts live in hash maps keyed by render id and place, are only
  touched when something fails, and are pruned once a simulated minute. Each tick adds one
  minute check. There is no new per-agent field; the watchdog's existing
  `network_replan_failures` counter is unchanged.
- The `traffic` channel lines (`[NETWORK_REPLAN_FAILED]`, `[NETWORK_REPLAN_WATCHDOG]`,
  `[JUNCTION_MISSING_*]`, `[REPLAN_DIAG]`) are unchanged.

Code: [`trip_failures.rs`](../rust/src/simulation/economy/agents/tick/trip_failures.rs)
(records), [`trip_failures/report.rs`](../rust/src/simulation/economy/agents/tick/trip_failures/report.rs)
(flood control) and [`trip_failures/text.rs`](../rust/src/simulation/economy/agents/tick/trip_failures/text.rs)
(the words).

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

## Crash and Hang Diagnostics

`METRUM_CRASH_DIAGNOSTICS=1` installs the panic hook, the ten-second hang watchdog and a
fixed-capacity flight recorder. An interactive `./run.sh --release` launch enables diagnostics
and `METRUM_HANG_ABORT=1` by default (`DEBUG-09`). After the first hang report is flushed, the
watchdog raises `SIGABRT` so an enabled operating-system core-dump service can retain every
thread's stack. `METRUM_HANG_ABORT=0` explicitly keeps that launch dump-only. Headless runs,
tests, benchmarks, profilers and replay automation never receive the abort default; a packaged
game or a debugger launch outside `run.sh` also keeps its own environment policy.

An `add_road` command record carries its point count, start and end XYZ, lane counts, snapping
mode and a 16-digit `points_hash`. The hash is allocation-free FNV-1a over the point count and
the exact little-endian `f32` bits of every XYZ component. It distinguishes strokes with equal
endpoints but different interior points without retaining a variable-size payload.

Road commit markers bracket every operation before authoritative topology mutation:

| Last marker | The stalled operation |
|-------------|-----------------------|
| `add road shared pool wait` | Dispatch into the joined shared Rayon pool. |
| `add road shared pool entered` or `add road preview plan validation` | Preview-plan identity and live-state validation. |
| `add road cold plan compile` | Cold road geometry/topology compilation after no reusable preview plan was available. |
| `add road terrain plan completion` | Road earthworks, local terrain dependencies and refined terrain products. |
| `add road transaction start` | Opening the road-edit transaction. |

Each stage has a corresponding `entered`, `validated`, `compiled`, `completed` or `started`
marker. Therefore the last unmatched start marker names the stalled stage; a later marker proves
the preceding stage returned. `add road transaction started` is emitted before the existing
`add road internal` mutation marker.

## Shutdown

Godot unloads `libmetrum_rise.so` soon after it deletes the scene tree. A thread still running
extension code at that point executes unmapped memory and the process dies with `SIGSEGV`
(`CODE-17`). In the 2026-10-19 core dumps the thread was the road-preview worker: it was detached
when `SimulationNode` was created, and after the node was freed it returned and dropped the last
`SimCore` reference while the library unloaded. The `__backtrace` frames in those dumps belong to
Godot's own crash handler, which faulted again while unwinding through the unmapped code. The
extension installs no signal handlers; its only hooks are Rust panic hooks, which only the
library's own code calls.

Every thread the extension starts now ends before the library unloads:

| Thread | Stopped by | Joined by |
|--------|------------|-----------|
| Simulation thread | `SimCommand::Stop`, sent when the node is freed; it runs the commands queued before it | `SimulationNode`'s drop |
| Road-preview worker | Its request channel disconnecting when the node is freed; it finishes the running request | `SimulationNode`'s drop |
| Preview Rayon pool (`road-preview-N`) and reclaim thread (`road-preview-reclaim`) | The worker returning; the reclaim thread frees what is queued first | The worker, before it returns |
| Terrain and water payload jobs and water-mesh jobs | They run to completion | `SimulationNode`'s drop waits for them |
| Shared Rayon pool (`metrum-N`) | The Scene-level deinit hook (`thread_pools::stop`), once its running and spawned work is done | The same hook |
| Hang watchdog (`metrum-hang-watchdog`) | The Scene-level deinit hook (`debug::shutdown`) | The same hook |

After that wait the node holds the only `SimCore` reference, so the core is freed on Godot's main
thread, while the node is freed and before Godot deinitializes the extension.

Rayon's global pool cannot be stopped or joined: after one full `run.sh --test` with the joins
above, `growth_meter_test` still crashed on a global-pool worker caught in Rayon's idle spin
(`yield_now`, round 13 of 32) when the library unmapped. The extension therefore never uses it.
All of its Rayon work runs in one shared pool
([`thread_pools.rs`](../rust/src/thread_pools.rs)), sized like the global pool
(`RAYON_NUM_THREADS`, else one thread per logical CPU) and built on first use, in the game and in
test binaries alike:

- A thread outside every Rayon pool (Godot's main thread and `WorkerThreadPool`, the simulation
  thread, a test thread) enters it through `thread_pools::install`, which blocks the caller as a
  global-pool call does, or queues a job with `thread_pools::spawn`. Code already on a worker of
  any pool runs inline, so nested entries cost nothing.
- The functions that do the parallel work enter the pool themselves, not the `#[func]` bodies:
  the frame step, the render snapshot, cell-chunk preparation, the road commit, undo and
  bulldoze, loads and world resets, the routing rebuild, and leaf rebuilds such as the entrance
  cache, site index, admission order, compile work, overlay images and car smoothing. Every caller,
  tests and benchmarks included, takes the same path. The simulation thread keeps the core lock
  while the work runs on a worker; a pool worker never waits for the core (`SIM-01`).
- The road-preview worker keeps its own joined pool (`ROAD-38`).
- When the library loads (the Core init stage), `thread_pools::disable_global_pool` spends
  Rayon's one-time global initialization on a pool that cannot start. Any Rayon call outside the
  shared pool then panics at its call site with "The global thread pool has not been
  initialized" instead of starting threads that outlive the library.
- At Scene deinit, after Godot has deleted the scene tree and every node has joined its own
  threads, `thread_pools::stop` waits for entries in flight and spawned jobs, then joins the
  workers. A later install runs on the caller.

Two checks find a call that bypasses the pool. A panicked `#[func]` returns a default value and
GDScript errors exit 0, so grep the Godot test logs for `global thread pool has not been
initialized`. `bridge_entry_points_never_use_the_global_pool` re-runs the library test binary as a
child process that disables its global pool and then drives a town's tools, a day of growth,
saves and loads, world resets, a bulk-loaded city, the simulation thread, the preview worker and
spawned jobs.

## Console

`DebugConsole` is an autoload
([`debug_console.gd`](../godot/scripts/ui/debug_console/debug_console.gd), with the view in
[`debug_console_view.gd`](../godot/scripts/ui/debug_console/debug_console_view.gd)). It is
available in every build and scene.

### Opening and input

- The physical key left of `1` opens and closes the console: `§` on Nordic layouts and `` ` `` on
  US layouts. Most platforms report it as physical `KEY_QUOTELEFT`; macOS reports it as
  `KEY_SECTION` on ISO keyboards, where `KEY_QUOTELEFT` is the `<` key beside left shift, so a
  `KEY_QUOTELEFT` press labelled `<` keeps typing. The key is always consumed, so it
  never types into a text field.
- The console slides down from the top edge, anchored left. By default it is one third of the
  window wide and as tall as the HUD allows. It stays between the visible controls in the
  `debug_console_keep_clear` group that its width overlaps: those in the top half of the window
  push its top edge down to their bottom, and the others stop its bottom edge 8 px above their
  top. The top menu bar (gameplay and both editors) and the gameplay bottom-left strip with the
  clock, money and RCI boxes are in the group; where neither exists, it is full height. The rest
  of the world stays visible and clickable.
- Dragging the right edge, the bottom edge or the bottom-right grip resizes it, down to
  360 × 240 px and up to the window width and the free height. The size is saved on release in
  `user://settings.cfg` (layout `debug_console`) and restored at the next start; a smaller
  window or a taller HUD clamps what is shown without changing the saved size.
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
  - `.sqlite`, an ordinary save of the running game and its camera, written first through
    `SimulationNode.save_game`. Only gameplay scenes write it; the world and asset editors and
    the menus have no game to save. It can be loaded like any other save, and the dump's
    `dump save=` line names it for [replays](#replaying-refusals).
  - `.txt`, the text dump described below;
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
- the default one-third, full-height layout, the start below a top bar and the stop above
  keep-clear HUD strips, grip and edge drags, clamping, and the saved size restored by a new
  console;
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

`godot/tests/road_rejection_report_test.gd` runs in the bridge list. On a test world it covers:

- a commit refusal and a tool refusal, each with its explanation, stroke and attempt line;
- exact coordinates in the attempt records, their order and source;
- the console dump writing the save first and naming it with its road generation;
- a replay against a fresh load of that save: every refusal reproduces, the attempt made before
  a built road is flagged as made on an older network, and a changed reason is reported as not
  reproduced.

`network_tool_chunk_renderer_test` checks that a refused click reports once, with the clicked
stroke and its validation. Rust tests in `nodes::sim::core::road_rejection` cover the report
text, the attempt record's exact round trip and the reason texts; `road_commit` covers a refused
commit's report through `SimCore::commit_road`.

`godot/tests/junction_debug_overlay_test.gd` runs in the bridge list. On a test world, with
roads committed through the simulation thread, it covers:

- no work while off;
- an empty world, then a straight road (two dead ends), then a T junction: each network
  generation redraws, with more vertices;
- focusing the T: only its six connectors are drawn and logged on `events`; a miss and Esc clear
  the focus;
- `speed` and `occupancy` color the same arcs, and `lanes` adds the road lane lines;
- the stall detector reports nothing on a network without cars;
- switching off clears the mesh and stops the overlay.

Placing a car on a connector needs the agent tick, which no bridge call exposes, so the Godot
test does not produce a stall. The Rust unit tests in `simulation::economy::agents::junction_stalls`
cover the reports:

- a car held at a blocked exit is reported once, naming the blocking car;
- a moving car is never reported;
- a car that leaves its connector is forgotten;
- the queued-behind and claim-lost causes;
- a restart when the clock goes back.

`nodes::sim::render::junctions` tests cover the overlay geometry on a T junction:

- six open turns, the vertex layout and the lift;
- the pass-through markers;
- the node grid search and picking;
- whitelist colors;
- `lanes` vertex counts;
- speed colors;
- the focus log lines;
- occupancy counts and the held-car highlight.

`godot/tests/agent_inspector_test.gd` runs in the bridge list. On a test world with one road
and a car from `spawn_debug_car`, driving the real `input_manager` click and Esc handlers, it
covers:

- a click where the car is drawn inspects it and does not reach the building inspector;
- the inspection fields, the window's title and rows, and the `Route end` row; `Last failure`
  reads `none` for a car that has not failed and shows a failure line the view is given;
- the window opens beside the click without taking the keyboard, its title bar drags it and
  its corner resizes it outward, and it still has not taken the keyboard afterwards;
- a home link stops the follow, frames the building, opens its inspector beside the agent
  window without toggling it, and keeps the camera there; Follow brings it back to the car;
- the route: it starts at the car, reaches the far end of the road, measures the rest of it,
  and is drawn as a ribbon of six corners per segment;
- while the car drives at 4x, the pivot sits on its drawn position (at least 80% of frames, never
  more than 5 m off), a mouse wheel event over the world zooms with the window open, and orbit
  changes the view;
- holding D through the Input singleton pans the camera, ends the follow and offers Follow,
  while the inspection, window and route stay and the camera stays where it was panned; Follow
  brings it back to the car;
- an Esc key event through the viewport ends the inspection and the follow, clears the route
  and the window, drops the Rust
  selection, and leaves the camera where it was; pan works again;
- a click beside the road ends a new inspection and reaches the building inspector;
- a real Building Inspector window, open and then clicked, never keeps the keyboard, and WASD
  moves the camera while it is open;
- the start line on `events`.

Rust tests cover the rest:

- `simulation::economy::agents::tests::route_preview`: on a two-lane four-way junction, the
  planned lanes for both approach lanes and all three exits equal the lanes the agent tick then
  drives. The route starts at the car's lane distance and ends at the detach point, a replan
  changes it, a path without a road onwards ends at that node, and an agent in a building has
  none.
- `simulation::economy::agents::inspection`: the selection follows the agent a swap-remove
  moves, and the removal of the inspected agent ends it with its reason.
- `nodes::sim::render::agent_inspection`: the pick takes the nearest car on a nearby lane,
  misses beyond the radius, skips stale snapshot entries and finds cars on junction connectors;
  lane-span lines run between the requested distances; ribbon corners carry their segment's
  other end and matching sides.

Trip failure tests (`DEBUG-06`):

- `simulation::economy::agents::tests::trip_failures`: in the agent tick, a cut turn and a route
  to a node with no road each log one line naming the agent, the junction and the replan; a
  stale exit with no new route logs its first failure and the recovery home, with the retries
  in between quiet; 600 departure failures in the parallel pass log the same lines on two runs,
  the first 20 rows in order and one summary for the other 580.
- `simulation::economy::agents::tick::trip_failures::tests`: the full line for a missing turn;
  a repeat inside a minute is quiet, another place or a recovery is not, and the streak ends a
  minute after its last repeat; the place limit folds the rest into one summary at the end of
  the minute and the next minute starts fresh; the minute limit folds failures everywhere but
  never the inspected agent, whose line the inspector keeps.

The `route_ribbon` shader draws nothing in headless runs. A one-off rendered check on Xvfb
(`gl_compatibility`, 1280x720) measured the drawn route at 6 px of color inside a 10 px outline,
the same with the camera 50 m and 3,000 m away.

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

### Trip failure measurement (2026-10-13)

`DEBUG-06` adds failure branches and a buffer swap to the movement pass, so it was compared the
same way: baseline `700f694e` against the `DEBUG-06` working tree, three alternating pairs with
the `DEBUG-01` command and settings above.

| Case (100k agents) | Baseline runs | Current runs | Median change |
|--------------------|---------------|--------------|---------------|
| `tick/on_road` | 1.893, 1.890, 1.887 ms | 1.828, 1.824, 1.839 ms | −3.3% |
| `tick/idle_scaling` | 201.3, 200.8, 199.8 µs | 191.0, 194.7, 196.6 µs | −3.0% |
| `tick_access/access_egress_car` | 4.409, 4.418, 4.425 ms | 4.427, 4.418, 4.409 ms | +0.0% |
| `tick_access/access_ingress_car` | 911.2, 920.2, 926.4 µs | 916.9, 908.9, 918.8 µs | −0.4% |

The first comparison showed `idle_scaling` 5–6% slower in every pair, and still under the
forced-alignment control, so it was real work rather than placement. A profile found it outside
the new code: the serial scan for dynamic lane claims after the parallel pass reloaded
`movement_claim_modes.len()` on every agent once `MovementSlices` held the failure sink's
mutex. The scan now iterates a borrowed slice, which keeps the bound in a register. The
departure failure record also moved into a `#[cold]`, never-inlined helper, so
`handle_in_building`, which every indoor agent enters, keeps its old stack frame.

## Launch Reference

`run.sh` is the canonical launch wrapper for local debug sessions. These flags set environment
variables before building Rust, deploying `libmetrum_rise.so` on Linux or
`libmetrum_rise.dylib` on macOS, and launching Godot. Flags for [visual modes](#visual-modes)
set only the starting value; the console switches them later.

### Launch Flags

| Flag | Main environment | Output / effect |
|------|------------------|-----------------|
| `--release` | `METRUM_CRASH_DIAGNOSTICS=1`, `METRUM_CRASH_LOG_DIR=<repo>/logs` unless already set; interactive game launches also default `METRUM_HANG_ABORT=1` | Enables the release-safe panic hook, hang watchdog, and fixed-size flight recorder. Panic and hang dumps are written under `logs/`; an interactive developer game aborts after its first hang dump so the OS can retain an all-thread core. Tests, headless runs, benchmarks, profilers and replays remain dump-only. Explicit environment values win: `METRUM_CRASH_DIAGNOSTICS=0` disables diagnostics, `METRUM_HANG_WATCHDOG_MS=0` disables only the watchdog, and `METRUM_HANG_ABORT=0` keeps an interactive run dump-only. |
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
| `--replay-road-attempts <dump>` | `METRUM_ROAD_REPLAY_SAVE`, `METRUM_ROAD_REPLAY_PACKS` (optional) | Replays a dump's refused road strokes against its save in a throwaway profile and reports whether each refusal reproduces ([Replaying refusals](#replaying-refusals)). Exits `0`, `1` (a refusal on the saved network did not reproduce) or `2` (bad input). |
| `METRUM_DEBUG_JUNCTION_CONNECTORS=<view> ./run.sh` | `METRUM_DEBUG_JUNCTION_CONNECTORS=<view>` | Starts the [junction connector overlay](#junction-connectors): `arcs`, `speed`, `occupancy` or `lanes`. No `run.sh` flag; the variable passes through. |

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
