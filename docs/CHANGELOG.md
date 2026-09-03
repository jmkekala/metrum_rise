# Changelog - 27/08/2026

Everything in this fork that is not upstream, against `cd85e11`.

1 commit, 131 files, 13,105 insertions, 439 deletions.

## Roads

### An edge is a cross-section, not two numbers

`rust/src/simulation/network/graph/lane_spec.rs` is new and holds the model.

An edge described its lanes as `fwd_lanes` and `bkw_lanes`, every lane
implicitly one standard width and carrying cars. That cannot express a median, a
bus lane, a cycle track, curbside parking, a planted verge, a turn pocket, or a
lane wider than its neighbors.

A road is now an ordered list of bands across the carriageway, stored leftmost
backward lane through rightmost forward lane. Each band carries its kind,
direction, real width, permitted modes, marking, turn set, longitudinal range,
and parking angle.

Lane kinds: travel, median, parking, verge, shoulder, cycle track, reversible.

### The count fields are gone

`fwd_lanes` and `bkw_lanes` first survived as stored fields kept in step with the
layout. Two sources of truth for one fact is a bug waiting on a writer that
updates one and not the other, and that writer already existed.

Both fields are removed from `Edge`. What replaces them is
`fwd_lane_count()`, `bkw_lane_count()`, and `car_lane_count()`, computed from the
layout on call. The 47 files that read the fields were converted one at a time,
not by a pattern: the same text appeared in struct literals, function
parameters, and struct definitions, and a regex that matched all three broke 264
call sites.

Three latent defects fell out of the conversion, each one a place where the
counts and the layout could disagree:

- `network/topology.rs` rebuilt lanes from counts when splitting a road, so
  every median, bus lane, verge, and parking band was silently dropped the moment
  a road was split. It now clones the layout.
- `network/lanes/geometry.rs` computed `road_half_width` as
  `(fwd + bkw) * LANE_WIDTH * 0.5`, one of the three formulas `roads.md` names as
  forbidden. It now reads `asphalt_width()`.
- `economy/agents/remap.rs` built an `Edge` claiming one lane each way while
  leaving the layout empty, which is the case the test suite caught.

### The old geometry path is deleted

Two lane builders coexisted. The full rebuild walked the layout; the incremental
rebuild computed offsets as `(index + 0.5) * LANE_WIDTH` from the counts. A road
drawn from scratch and the same road after an edit produced different geometry,
and every band the counts cannot express was silently dropped the moment
anything touched the edge.

Both paths now walk the layout. Offsets accumulate real widths, so a wide truck
lane or a median pushes its neighbors outward correctly. Edge width is the sum of
the bands rather than a lane count times a constant.

### Asymmetric and reversible roads

A three-lane road has a center belonging to neither direction. A two-way
left-turn lane is entered from both sides, so `fwd_count` and `bkw_count`
deliberately do not sum to the lane total. A tidal lane is the same band with a
direction that changes on a schedule instead of per vehicle, so both are one
`LaneKind::Reversible`. Flipping the tide changes which count the band joins and
moves no geometry.

A road's lane count also differs at its two ends. `with_turn_pockets` inserts
left pockets inboard and right pockets outboard, each a lane with a partial range
so it takes width only where it exists. `asphalt_width` is the widest the road
ever gets, which the roadbed reserves; `asphalt_width_at` is what it is at a
point, and the two differ exactly where a pocket opens.

### Curbside parking carries its angle

The angle sets both how deep the band is and how many cars fit along a meter of
curb, so it is stored on the lane rather than inferred from width.

| Angle | Depth | Curb per car |
|---|---|---|
| Parallel | 2.5 m | 6.0 m |
| 45 degrees | 4.8 m | 3.5 m |
| 90 degrees | 5.5 m | 2.7 m |

Angling trades roadway depth for curb frontage, which is the whole reason a
street is marked one way rather than another. `parking_spaces_along` is what a
supply model reads: the same width of street holds very different numbers of cars
depending on how the bays are marked.

### Verges and sidewalk width

`LaneKind::Verge` is the planted strip or the run of planters between carriageway
and sidewalk. It carries nothing, takes width like a median, and sits at curb
height. It is what makes a street tree-lined, and what separates a wide sidewalk
from a wide road with a narrow sidewalk on it.

Sidewalk width is authored per layout, falling back to `config::SIDEWALK_WIDTH`.
The save records whether a layout authored one, so changing the project default
later moves every street that never overrode it and leaves the rest alone.

### Road roles from mode bits

`TransitFlags` gains `BUS` and `BIKE`, and `ROAD_TRAFFIC` for what an ordinary
lane admits. A bus lane is defined by the bit it withholds, and a cycle track is
not a footway.

Bus-only lanes, part-time bus lanes, cycle tracks, pedestrian streets, and truck
routes are all sets of mode bits on a band rather than new road types. The
pedestrian case proves the model: it is a street with private vehicles withheld,
so deliveries still arrive, the bin lorry still runs, and an ambulance is never
blocked.

`count_for_mode` answers the lane question per mode, so a road with a bus lane
returns a larger number to a bus than to a car.

### The band emitter fills what was declared

`RoadSurfaceBandKind` already had `Median`, `Parking`, and `CycleTrack` declared
and never emitted. The carriageway is now subdivided into the layout's ordered
bands rather than emitted as one slab split at the centerline, so those kinds are
real. `Verge` is added. A built median is emitted at curb height and a painted one
flush, which is the only difference between a line on the road and an island a
vehicle may not cross.

### The cross-section reaches the road tool

`add_road_with_cross_section` takes seven integers per band across the Godot
boundary: kind, direction, width in millimeters, mode bits, marking, turn set,
parking angle. `LaneLayout::from_flat` and `to_flat` round-trip it. A malformed
payload falls back to an ordinary road rather than to nothing, and lane counts
are derived from the section when one is present.

This is what makes it a road builder rather than a lane-count menu.

## Intersection control

`rust/src/simulation/network/graph/control.rs` is new.

Lane connectors already existed: `Node.lane_connections` holds the table and
`vehicle_junctions.rs` enforces it, blocking every unlisted turn at a node that
carries any explicit connection. What a junction had no way to express was
*when* a permitted turn may be taken.

`Node.control` now carries a `JunctionControl`. Priority signs assign main,
yield, or stop per approach arm, with an arm nobody assigned yielding rather
than assuming priority. A signal program is an ordered phase list with green,
amber, and a cycle offset, so junctions along a corridor can be progressed into
a green wave. A program that cannot cycle shows green rather than deadlocking
the junction it controls.

Enforcement sits in `movement/network/junction.rs`, at the point an agent
commits to entering a junction. A held agent is pinned at its stop line by the
same mechanism a queued connector uses, so a signal and a jam are
indistinguishable to everything downstream. A stop sign stamps a release time on
arrival, so it pays its halt once instead of on every tick it spends waiting.

Not built: switching driven by measured flow against wait time, junction
restrictions per arm, per-lane speed limits, and per-lane vehicle restrictions.

## Routing feedback

`rust/src/simulation/economy/agents/tick/planning/reroute.rs` is new.

Half of this loop already existed and was not recorded anywhere: congestion is
aggregated per tick from observed agent speed against the limit
(`lane_buckets/congestion.rs`) and the contraction hierarchy prices it into its
metric as `base_cost * (1.0 + current_congestion)` (`pathing/cch.rs`). The half
that was missing was the vehicle's. Every replan in the tree fired only after a
path was exhausted, so a car holding a valid route never reconsidered it no
matter what happened ahead of it, which is the Cities: Skylines failure
`traffic.md` describes.

A car now reconsiders at each junction. It prices the remainder of its route
with the live congested metric, asks the router for a fresh one, and switches
only when the candidate costs less than 85% of the remainder. Pricing the
remainder at free-flow cost instead would make every congested route look cheap
and the comparison meaningless. The margin is what stops two near-equal routes
trading vehicles back and forth every time they are compared, and attempts are
rate limited to one per vehicle per 15 seconds.

Pedestrians and border-bound freight are excluded: neither can take a different
road, and freight holds a plan this must not overwrite.

## Regional funding scaffold

`rust/src/simulation/region/mod.rs` is new, and it is a scaffold: nothing ticks
it, nothing saves it, and no system reads it.

It records the funding model as types so the shape is settled before anything
depends on it. `FundingScope` names which pool owns a service, `FundingStage`
names how far the two-pool sequence has progressed, and `stage.payer(scope)`
resolves them, so a national service falls back to the region until a second
region unlocks. `RegionLedger` holds one balance and a `CityLedger` line per
city, because a city is budgeted individually and the money is regional.

Per-city statistics are deferred until there is a second city to test against.
What this waits on: a City entity the simulation recognizes, and an owner for
the tiles a city holds. Neither exists.

## Borders

`rust/src/simulation/network/border.rs` is new.

The migration term was binary: a border either existed or it did not.

```
external_connection_available = if connected_border_count > 0 { 1.0 } else { 0.0 }
```

It is now continuous, driven by a `border_openness` policy dial that persists in
the fiscal policy. Four states derive from the dial rather than being stored
separately: sealed, restricted, controlled, open. Barrier kinds, far-side strain,
and development reach come with it.

A sealed border is 0.0, which the existing model already handles, so the change
is a widening rather than a new mechanism.

## Frontage roles, and the rule that makes alleys work

`rust/src/simulation/buildings/frontage.rs` is new.

A building held exactly one `edge_idx`, so it had exactly one street, and every
consequence the genre gets wrong about alleys followed from that one field.

A frontage says what it is for rather than how wide the road is: `Primary` is the
address, `Service` is deliveries and waste and utility access, `Water` is a
navigable edge. An edge declares which roles it accepts through
`EdgeFrontageClass`, now a field on `Edge`.

The load-bearing rule: a service way never accepts an address. Without it an
alley is a thin street, the allocator fills it with houses facing the wrong way,
the result looks like a mistake, and that is why the genre cuts alleys rather
than fixing them.

`can_address()` is checked in all seven paths that assign an address: candidate
scoring and frontage repair in the allocator lifecycle, candidate collection and
final placement validation in allocator placement, and parcel projection,
placement, and restore in zoning. All seven matter. A filter in placement alone
would still let zoning cut parcels against an alley, and the buildings would
arrive later by another path.

Defaults reproduce the old behavior exactly: `FrontageRole::Primary`,
`EdgeFrontageClass::Street`, and an unknown ordinal degrades to the working case
rather than dropping a building off the network.

## Persistence

Save version 58 added `network_edge_lanes`, one row per band.

Save version 59 adds the edge frontage class, an authored sidewalk width, and the
parking angle.

`fwd_lanes` and `bkw_lanes` are still written so an older build can open the
save, and a save without the lane table loads with the layout its counts imply,
which is exactly how it rendered when it was written.

## Windows

`run.ps1` mirrors `run.sh` flag for flag, with three platform differences: it
builds `metrum_rise.dll` rather than the `.so`, user assets live under
`%APPDATA%`, and Godot is found via `-Godot`, `$env:GODOT`, or `PATH`.

The crate builds for `x86_64-pc-windows-msvc`, the resulting DLL loads in Godot
4.7.1, and a headless run exits 0.

`UPSTREAM_ISSUE_crash_rs.md` reported that `rust/src/debug/crash.rs` was absent
from the repository, so `main` did not build for anyone. Upstream supplied the
real file in `87059cb` and that report is now answered.

## Bug fixes

A cycle track admitted nothing at all. `carries()` gated on `is_travel()`, which
is true only for `LaneKind::Travel`, so every cycle track would have compiled,
rendered its band correctly, and silently refused all routing. `is_moving()` is
now the predicate for whether a band carries moving traffic, true for travel
lanes, cycle tracks, and reversible lanes, false for medians, parking, verges,
and shoulders.

Two upstream `Edge` literals had no lane layout after the merge. Git reported no
conflict because neither side touched the same lines, but the tree did not
compile.

## Godot addon

`godot/addons/file_browser/` adds an editor dock that regulates and browses files
by their manifest, with three actions: check what would change and write nothing,
clean up and fix it, and declare the custom fields a project wants tracked. It
injects missing headers, standardizes existing ones, and expands a bare `HEADER`
marker into a full divider. I have not dont much cleanup on this, is is ported using
Claude from an old internal 3.x build of 2.5D_engine. I meant to get it running again
already, but I am familiar enough with my own codebase that I've put it off for years,
and now when it would become especially useful it will need some more work, stay tuned.

It is in the addons folder because I meant to work on it more and then left it there.
It does work, the manifests and headers across the Rust files are all the files I touched
along the way. The tool earns its place on my own engine, where browsing hundreds of
thousands of lines across that many files is otherwise miserable. The next PR finishes
the pass and wires the add-on up properly.

## Documentation

Twenty documents were touched. Line numbers are ranges in the current file,
against the upstream base 859afaba.

Seven are new, and the whole file is the change:

| File | Lines | Why it exists |
|---|---|---|
| `docs/narrative.md` | 1-314 | A search of the existing docs for theme, setting, story, or tone returned no design statement |
| `docs/services.md` | 1-279 | The whole written record of services was one line naming `always_on_service` |
| `docs/region.md` | 1-262 | Founding, expansion, the density law, incorporation, and national parks had no home |
| `docs/simulation_layers.md` | 1-114 | The physical and living layers being ported, and the ordering |
| `docs/transit.md` | 1-51 | Records that tram, subway, and train are one rail network before three systems get built |
| `docs/README2-ONBOARDING.md` | whole file | A landing page, where to start by task, and the in-engine documentation convention |
| `docs/CHANGELOG.md` | whole file | This file: the full record of what changed |

Thirteen already existed and were edited in place:

| File | Lines touched | What changed |
|---|---|---|
| `docs/zoning.md` | 314-542 | Alleys entire, build granularity, gridless parcels, districts, water frontage, parking supply |
| `docs/roads.md` | 23-26, 114-268, 354, 370, 801-807 | The lane model, intersection control, and three forbidden regressions naming the formulas that must not return |
| `docs/terrain.md` | 181, 989, 1265-1439 | World generation, world extent, real-scale slice validation, and mineral deposits |
| `docs/economy.md` | 5-9, 14-17, 19-20, 26-27, 35-36, 51-52, 56-57, 61-62, 73-74, 76-77, 94-95, 97-99, 103-104, 125-126, 136-137, 146-149, 219-220, 225-228, 230-231, 254-255, 257-264, 268-270, 282-283, 285-286, 297-298, 300-303, 305-306, 319-320, 324-325, 334-335, 363-364, 366-367, 379-382, 391-400, 403-404, 2283-2397, 3295 | The industry scaffold, the tycoon layers, the two money pools, foreign capital |
| `docs/ui.md` | 5-6, 45-46, 82-83, 109-110, 161, 166, 287, 381, 552, 609, 612, 618, 623, 627, 629-664 | The road builder, post-placement editing, the camera |
| `docs/asset_editor.md` | 1004, 1738-1739, 2096-2134, 2158-2159, 2177-2178, 2181-2182, 2187-2188, 2190-2193, 2199-2200, 2214-2216, 2221-2231 | The editor as a first class system, interiors generated once |
| `docs/building_allocator.md` | 328-381 | Frontage roles |
| `docs/README.md` | 5-6, 24, 26-74 | What 2.5D_engine is, how the project was found, what this contribution changed |
| `docs/traffic.md` | 26-28, 38-40, 156-173, 349-393, 421-423, 427 | Routing feedback |
| `docs/entrance_and_exit.md` | 3-4, 6-13, 49-50, 82, 92-94, 96-97, 99-100, 737, 745-752, 778 | Frontage role and service way consistency |
| `docs/project.md` | 9, 20, 27-35, 72, 76, 84 | Scale target |
| `docs/reference.md` | 15 | The `20 km` fallback world demoted from target to leftover |
| `docs/roadmap.md` | 41-49, 76-79, 108 | `WORLD-04`, the real-scale slice and the shader proof |

The funding model is now two pools that arrive in sequence rather than three
that coexist: the city budget pays upward to a region the player cannot see
into, the region unlock turns those taxes into income sized to found the second
city, and the second region unlock creates the national pool and moves power,
border patrol, and national parks onto it. `economy.md` owns the pools; the
three-pool table in `services.md` is gone. Immigration grants are a new lever
in `services.md`, priced per arrival and bounded above by the border policy in
`narrative.md`.

The scale target moved from 1,000,000 population to 20,000,000 agents across
cities, regions, and simulation tiers, in `CLAUDE.md`, `docs/project.md`,
`docs/economy.md`, and `docs/asset_editor.md`. Three sites keep 1,000,000
deliberately: a pathfinding argument that holds at any population, a memory
budget whose arithmetic is only correct at that number, and an F12 cheat that
adds money.

The world extent moved with it. The country is `100,000 km2` of land, roughly
`125 km x 800 km` along the U, plus an inland sea of `50,000` to `75,000 km2`:
`150,000` to `175,000 km2` generated. Around 20 land regions average
`5,000 km2` each, and 5 or 6 ocean regions run `8,000` to `15,000 km2`. The
`20 km x 20 km` world stays in `reference.md` as what the code boots today.
Extent and the generator are in `terrain.md`, the region tier in `region.md`.

## Verification status

Compile-verified clean, library and tests, zero errors.

The Windows port is demonstrated and measured: the crate builds for
`x86_64-pc-windows-msvc`, the 21 MB DLL loads in Godot 4.7.1, and a headless run
exits 0.

The full library test suite passes: 1,548 tests, 0 failed, 0 ignored, in
1865.58s. That run covers the alley, parking, verge, and cross-section work, the
frontage filter in the address-assignment paths, and the junction control,
rerouting, and funding scaffold added since.

Checked at the API boundary in Godot 4.7.1: `godot/spike_junction_control.gd`
builds a cross junction and checks 27 assertions, all passing. It covers the
seven bound control methods, the uncontrolled default, priority signs, a
two-phase signal across green, amber, red, and the cycle wrapping, the green-wave
offset, clearing back to uncontrolled, and a four-band cross-section whose edge
width comes back as 12.00 m, the exact sum of its bands.

That run covers the types and the signal math. It says nothing about whether a
vehicle obeys a signal, because no vehicle is involved.

Vehicles obey lights: cars hold at the stop line through a red and pull away on
the change. Under load, `spawn_test_traffic` puts 400 looping cars across a
junction at 4x speed for 600 frames.

Nothing has been profiled, and no benchmark has been run against the Criterion
suite. The 20 km benchmark map does not finish generating on the development
machine in a debug build, so no populated world has been loaded.

## Junction control did not work, and the fix

The first version of the gate read `graph.node(node_id)` with a raw id. Building
a junction merges nodes, and a merged-away id stays resolvable only through
`node_aliases`; the id an agent carries can be one of those. So the gate looked
up a different node, found no control on it, reported uncontrolled, and every car
drove through every red light.

Every other reader in the tree resolves this first. `get_node_lanes_internal`
calls `get_valid_node` before touching a node, for exactly this reason. Seven
places did not: the movement gate and all six control functions. All seven now
resolve the id.

`get_sim_time()` is new. `get_junction_signal_aspect` takes a time, and the sim
clock was not exposed at all, so every caller was passing an invented value and
asking what a light would show at some other moment. A renderer drawing an aspect
needs the same clock the gate reads.

## Conflicting movements inside a junction

Watching the game surfaced cars turning left across oncoming traffic and cars
overlapping inside the junction box. Each permitted turn is its own connector
lane, and `claim_connector_entry` admitted a car when that car's own connector
was free. Cars were separated along a lane; nothing checked across two lanes
whose geometry crosses, so a left-turn connector and the oncoming through
connector never tested each other and both cars proceeded.

`rust/src/simulation/network/lanes/conflicts.rs` is new. It builds a per-node
table of which connector paths cross, once when lanes are rebuilt rather than per
tick. Two movements conflict when their paths pass within about two meters
somewhere other than a shared endpoint. Straight connectors meeting in an X have
every vertex far from the other's path, so the test is a segment intersection
rather than a vertex-to-segment distance.

Three groupings keep it from holding traffic that should run:

- Both directions of one street go together. A street is identified by approach
  bearing, because a cross junction splits one street into two edges and an edge
  id cannot name it.
- Movements out of one approach lane are a diverge, not a crossing. They are
  recorded separately as co-entrants, because they share the ground a waiting car
  stands on: three cars taking three different turns out of one lane would
  otherwise all park on the identical point.
- Movements into one exit are a merge, governed by gap acceptance on the exit
  lane.

Four defects surfaced while getting this to work in the running game, three of
them introduced here:

- `hold_at_junction_control` read the node by raw id. Building a junction merges
  nodes, and the id an agent carries can be one merged away, so the gate read a
  different node, found no control, and every car drove through every red light.
  Seven sites now resolve through `get_valid_node`.
- Requiring an exit lane to be empty before entering deadlocked the junction:
  the head car on each approach waited for an exit that ordinary moving traffic
  kept occupied, so no queue ever advanced. It now tests whether the exit is
  jammed, meaning a stopped car within one length of the mouth.
- Holding a car already inside a connector stranded it mid-box, where it blocked
  every crossing movement. Yielding happens before entry, never after.
- The lane-attach path set `lane_d = 0.0` with no occupancy check at all, so
  cars entering a lane landed on the same coordinate. `lane_attach_slot_clear`
  existed and was called only when a car left a building.

Verified in the running game: the two-phase signal holds and releases all four
approaches, and traffic clears the junction.

## Diagnostic spikes

`godot/spike_*.gd` holds five scenes, each written to answer a question about
a live failure and each kept afterward because the system it exercises still
needs checking after the next change, and they stand to be wired into the museum
eventually. They build cross junctions, spawn traffic, install two-phase signals,
group junction arms into streets by geometry, cluster car positions, and count
overlapping vehicles.

Two things they taught that are worth stating outright:

Edge ids are not stable between runs. The same geometry produced different ids
on consecutive runs, so a test that assigns signal phases by sorted edge id is
unreliable. Arms are grouped by bearing from the junction instead.

An uncontrolled junction gives every arm permanent green, so cross traffic runs
simultaneously by design. A junction test that does not install a signal first
is measuring nothing.

`godot/spike_record.gd` writes each run to `user://spike_runs/<name>.json` and
prints what moved since the previous run, calling out any check that passed last
time and fails now. Check labels carry no measured numbers, because a label is a
check's identity across runs and embedding this run's figures would make every
run a different check.

## Found while testing: the simulation thread is never shut down

`SimulationNode` spawns its simulation thread in `_ready` and has no `exit_tree`.
Nothing signals the thread to stop and nothing joins it, so on quit Godot tears
the node down while the thread is still ticking and reading state it no longer
owns.

Reproducer, `godot/spike_poll.gd`, three runs out of three: build a road, poll
`get_network_nodes()` every frame while the sim thread applies it, build a
crossing road, poll again, quit. The script prints its result and returns 0, then
the process dies with SIGSEGV during teardown.

The same script also reads 0 nodes where an identical build without per-frame
polling reads 2 and then 5, so the render snapshot is being read while it is
being replaced.

This is upstream code, unchanged by this contribution: `get_network_nodes` is
byte-identical to 859afaba, and no `exit_tree` or `join()` exists anywhere under
`rust/src/nodes/`. It is recorded rather than fixed because thread lifetime is
the owner's call, not a contributor's.

## Every file touched

The documents above carry their line ranges. This is every other file in the
commit, with insertions and deletions against `cd85e11`. The small ones are the
manifest and section-header pass; the rest are the work described above.

| File | Lines |
|---|---|
| `.gitignore` | +6 / -0 |
| `CLAUDE.md` | +3 / -3 |
| `godot/addons/file_browser/fields.cfg` | +17 / -0 |
| `godot/addons/file_browser/file_browser.gd` | +540 / -0 |
| `godot/addons/file_browser/plugin.cfg` | +7 / -0 |
| `godot/addons/file_browser/plugin.gd` | +208 / -0 |
| `godot/spike_conflict.gd` | +220 / -0 |
| `godot/spike_junction_control.gd` | +200 / -0 |
| `godot/spike_left_turn.gd` | +180 / -0 |
| `godot/spike_live_red_light.gd` | +156 / -0 |
| `godot/spike_poll.gd` | +57 / -0 |
| `godot/spike_record.gd` | +122 / -0 |
| `readme.md` | +13 / -5 |
| `run.ps1` | +322 / -0 |
| `rust/Cargo.lock` | +1 / -0 |
| `rust/Cargo.toml` | +25 / -0 |
| `rust/src/nodes/sim/benchmark.rs` | +96 / -12 |
| `rust/src/nodes/sim/core/snapshot.rs` | +38 / -0 |
| `rust/src/nodes/sim/core/tests.rs` | +29 / -4 |
| `rust/src/nodes/sim/core/thread.rs` | +50 / -1 |
| `rust/src/nodes/sim/editing.rs` | +263 / -13 |
| `rust/src/nodes/sim/query/lanes.rs` | +24 / -2 |
| `rust/src/nodes/sim/query/mod.rs` | +31 / -2 |
| `rust/src/nodes/simulation_node/economy_api.rs` | +34 / -4 |
| `rust/src/nodes/simulation_node/network_api.rs` | +112 / -0 |
| `rust/src/nodes/simulation_node/system_api.rs` | +46 / -0 |
| `rust/src/nodes/simulation_node/tests.rs` | +27 / -2 |
| `rust/src/nodes/simulation_node/tests/async_payload.rs` | +23 / -2 |
| `rust/src/simulation/buildings/allocator/lifecycle.rs` | +47 / -1 |
| `rust/src/simulation/buildings/allocator/placement.rs` | +44 / -1 |
| `rust/src/simulation/buildings/allocator/site/tests.rs` | +23 / -2 |
| `rust/src/simulation/buildings/frontage.rs` | +261 / -0 |
| `rust/src/simulation/buildings/mod.rs` | +24 / -0 |
| `rust/src/simulation/economy/agents/building_refs.rs` | +27 / -2 |
| `rust/src/simulation/economy/agents/data.rs` | +45 / -0 |
| `rust/src/simulation/economy/agents/lifecycle.rs` | +36 / -0 |
| `rust/src/simulation/economy/agents/remap.rs` | +55 / -8 |
| `rust/src/simulation/economy/agents/test_departure_side.rs` | +23 / -2 |
| `rust/src/simulation/economy/agents/tests/lane_dynamics.rs` | +23 / -2 |
| `rust/src/simulation/economy/agents/tests/support.rs` | +35 / -6 |
| `rust/src/simulation/economy/agents/tests/trips.rs` | +22 / -2 |
| `rust/src/simulation/economy/agents/tick/movement/network.rs` | +27 / -0 |
| `rust/src/simulation/economy/agents/tick/movement/network/junction.rs` | +298 / -2 |
| `rust/src/simulation/economy/agents/tick/movement/network/junction/enter.rs` | +39 / -0 |
| `rust/src/simulation/economy/agents/tick/movement/network/lane_entry.rs` | +64 / -4 |
| `rust/src/simulation/economy/agents/tick/movement/network/replan.rs` | +36 / -0 |
| `rust/src/simulation/economy/agents/tick/movement/replan_watchdog.rs` | +34 / -2 |
| `rust/src/simulation/economy/agents/tick/movement_pass.rs` | +23 / -0 |
| `rust/src/simulation/economy/agents/tick/planning.rs` | +28 / -0 |
| `rust/src/simulation/economy/agents/tick/planning/candidate.rs` | +43 / -2 |
| `rust/src/simulation/economy/agents/tick/planning/reroute.rs` | +155 / -0 |
| `rust/src/simulation/economy/agents/tick/slices.rs` | +28 / -0 |
| `rust/src/simulation/economy/agents/tick/speed.rs` | +45 / -3 |
| `rust/src/simulation/economy/agents/tick/traffic.rs` | +37 / -6 |
| `rust/src/simulation/economy/agents/tick/traffic/occupancy.rs` | +137 / -0 |
| `rust/src/simulation/economy/demand/snapshot.rs` | +45 / -1 |
| `rust/src/simulation/economy/demand/tests.rs` | +27 / -2 |
| `rust/src/simulation/economy/fiscal.rs` | +93 / -0 |
| `rust/src/simulation/economy/households/tests/commercial.rs` | +31 / -0 |
| `rust/src/simulation/economy/households/tests/support.rs` | +25 / -4 |
| `rust/src/simulation/economy/logistics/tests.rs` | +29 / -4 |
| `rust/src/simulation/grid/noise.rs` | +35 / -6 |
| `rust/src/simulation/mod.rs` | +27 / -0 |
| `rust/src/simulation/network/border.rs` | +346 / -0 |
| `rust/src/simulation/network/graph/control.rs` | +339 / -0 |
| `rust/src/simulation/network/graph/data.rs` | +101 / -4 |
| `rust/src/simulation/network/graph/lane_spec.rs` | +1817 / -0 |
| `rust/src/simulation/network/graph/mod.rs` | +36 / -0 |
| `rust/src/simulation/network/graph/rebuild/tests.rs` | +26 / -2 |
| `rust/src/simulation/network/interaction.rs` | +27 / -2 |
| `rust/src/simulation/network/lanes/conflicts.rs` | +474 / -0 |
| `rust/src/simulation/network/lanes/geometry.rs` | +35 / -2 |
| `rust/src/simulation/network/lanes/mod.rs` | +91 / -0 |
| `rust/src/simulation/network/lanes/rebuild.rs` | +109 / -42 |
| `rust/src/simulation/network/lanes/tests.rs` | +49 / -28 |
| `rust/src/simulation/network/lanes/vehicle_junctions.rs` | +36 / -4 |
| `rust/src/simulation/network/mod.rs` | +98 / -8 |
| `rust/src/simulation/network/render/road/standard_surface/markings.rs` | +34 / -2 |
| `rust/src/simulation/network/render/test_road_mesh.rs` | +24 / -2 |
| `rust/src/simulation/network/surface/band_semantics.rs` | +28 / -1 |
| `rust/src/simulation/network/surface/debug/geometry_dump/build.rs` | +27 / -2 |
| `rust/src/simulation/network/surface/edge/preview.rs` | +44 / -2 |
| `rust/src/simulation/network/surface/edge/profile.rs` | +116 / -15 |
| `rust/src/simulation/network/surface/edge/sections.rs` | +31 / -2 |
| `rust/src/simulation/network/surface/mod.rs` | +41 / -0 |
| `rust/src/simulation/network/surface/query/terrain_cdt/mapping.rs` | +26 / -0 |
| `rust/src/simulation/network/surface/tests/bend_terminal/logged_cases/terminal.rs` | +24 / -5 |
| `rust/src/simulation/network/surface/tests/bend_terminal/terminal.rs` | +22 / -2 |
| `rust/src/simulation/network/surface/tests/bend_terminal/vertical_steps.rs` | +23 / -4 |
| `rust/src/simulation/network/surface/tests/support/fixtures.rs` | +29 / -10 |
| `rust/src/simulation/network/test_clips.rs` | +29 / -8 |
| `rust/src/simulation/network/test_compaction.rs` | +25 / -2 |
| `rust/src/simulation/network/test_ped_junction.rs` | +23 / -2 |
| `rust/src/simulation/network/test_topology.rs` | +24 / -2 |
| `rust/src/simulation/network/test_uturn.rs` | +27 / -6 |
| `rust/src/simulation/network/topology.rs` | +37 / -4 |
| `rust/src/simulation/network/types.rs` | +50 / -0 |
| `rust/src/simulation/pathing/cch.rs` | +41 / -4 |
| `rust/src/simulation/pathing/flow_field.rs` | +41 / -4 |
| `rust/src/simulation/pathing/tests.rs` | +43 / -22 |
| `rust/src/simulation/region/mod.rs` | +213 / -0 |
| `rust/src/simulation/save/agents.rs` | +35 / -0 |
| `rust/src/simulation/save/mod.rs` | +38 / -3 |
| `rust/src/simulation/save/network.rs` | +134 / -8 |
| `rust/src/simulation/save/schema.rs` | +157 / -3 |
| `rust/src/simulation/save/tests.rs` | +29 / -4 |
| `rust/src/simulation/zoning/parcels/placement/projection.rs` | +27 / -0 |
| `rust/src/simulation/zoning/parcels/placement/run.rs` | +30 / -0 |
| `rust/src/simulation/zoning/system/restore.rs` | +23 / -0 |
| `rust/src/simulation/zoning/tests/helpers.rs` | +27 / -6 |
| `UPSTREAM_ISSUE_crash_rs.md` | +144 / -0 |

## Not built

Named here so the gap between what the documents specify and what the code does
is not left to a reader to discover.

A gym, a zoo, and a museum for every system and mechanic, plus an in-editor class
reference for everything they do not cover. Tracked as `GYM-01` and `GYM-02` in
[`roadmap.md`](roadmap.md), specified in
[`README2-ONBOARDING.md`](README2-ONBOARDING.md).

Survey teams. Deposits are placed by the generator and shown immediately, so
there is no prospecting decision. Tracked as `SURVEY-01`, specified in
[`terrain.md`](terrain.md) and [`economy.md`](economy.md).

Turning lanes and a turn hierarchy. This is the next thing to build. Right turns
still collide: the conflict table holds a movement out of a crossing street, and
groups both directions of one street together the way a signal phase does, but it
has no notion of one turn outranking another. A right turn on green and a
conflicting movement with the better claim are treated as peers, so they meet in
the box. What that needs is dedicated turn lanes, so a turning car queues out of
the through lane rather than in it, and a stated precedence among permitted
movements: through beats turning, and a protected turn beats a permissive one.
Until both exist, the yield rule can only say whether a path is occupied, never
who should have gone first.

Intersection control. Signal switching driven by measured flow against wait
time; the cycle is fixed. Junction restrictions per arm: U-turns, lane changing
inside the junction, entering a blocked junction, and pedestrian crossing.
Per-lane speed limits. Per-lane vehicle restrictions as a routing penalty. Super
nodes, so a large interchange can be signaled and read as one object. The control
that does exist is bound to GDScript but has no tool or panel behind it: a player
cannot yet place a sign or time a light.

Routing feedback. The observability half. `traffic.md` requires a view that
answers why a number is what it is, and a congestion heatmap is not that. No
traffic report exists.

The regional tier. Everything except the type scaffold. No City entity, no tile
owner, no pool that holds money, nothing ticks or saves. Per-city statistics are
deferred until there is a second city to test against.

Parking, verges, and sidewalk width are expressible in a lane layout and nothing
consumes them. `parking_spaces_along` is computed and no parking supply model
reads it. A verge takes width and grows nothing.

The physical layers are the largest gap and are a port, not a fix: water that
flows, fire with a fuel bed, wind, minerals from how the rock formed, flora and
fauna. `simulation_layers.md` owns the ordering. None of it is started.

A world at small-country scale. No parts of the 2.5D_engine have been implimented
into the codebase. No seed generator exists yet, and no region is drawn to its
environment. The largest world ever loaded is the imported `324 km2` Kuopio DEM.
Tracked as `WORLD-04`, specified in [`terrain.md`](terrain.md).

---

# Changelog - 02/09/2026

Everything below is new since the flattened fork commit and has not
shipped anywhere. The section above is frozen at that push.

42 commits, 2,045 files, the engine's own tree leaving the index as it
became a mount rather than a copy.

## The engine is mounted, not copied

`godot/addons/` holds four directory junctions into the 2.5D_engine
repo: `2.5D_engine`, `GOAT_bus`, `SPEECH_socket`, and `FILE_browser`.
They are not files in this repository. The junction targets are
gitignored, so this tree carries adapters, hooks, and renderer
branches and no engine source at all.

That shape is the license boundary as much as a convenience. The game
is GPL-2.0; the engine components each carry a LICENSE.md marking them
all rights reserved, outside that license, consumed across a call
boundary, with SPEECH_socket and FILE_browser naming an intended MIT
release once they are polished enough to merge cleanly. A clone needs
the engine repo beside this one and a one-time junction step.

Engine updates now land live. The previous arrangement was a copy that
had to be re-synced by hand, and the sync before the junctions took
three rounds against a source tree that was still being written.

## The simulation boundary

The Rust economy and the engine's evaluated fields keep their own
ontologies and meet at two arrays and a tick boundary.

### Down: batched sampling and a revisioned intake

`engine_boundary.gd` samples any engine field at a batch of positions
in one call. The array is `PackedFloat64Array`, not 32-bit: the
engine's kernels and goldens are f64, and a 32-bit boundary truncated
every sample, which the first drill caught by comparing bit patterns
rather than values.

`rust/src/simulation/engine_inputs.rs` is the intake, at the
simulation layer so grid systems read it without upward dependencies.
It holds the delivered arrays, the probe grid's world geometry, and a
revision. Sampling is bilinear and returns `None` outside coverage,
which is what keeps each consumer's own default the honest fallback
rather than a silent zero.

`EngineTick` delivers every 120 ticks on a probe grid aimed at the
city: `engine_parcel_bounds` hands up the parcels' bounding box and
the grid spans it plus a 200 m margin, with a 50 m spacing floor so a
one-parcel city cannot collapse the grid, and a listener-centred
layout before any city exists.

### Up: the city's actions as measured rows

`get_extractor_sites` exports every committed extraction site's world
position and depletion. The tick harness aggregates them into
hundred-metre deposit cells and writes the engine's own converted-grid
format, signed 16-bit values with a JSON sidecar, which the engine's
`heightmap_node` opens as measured ground truth.

The drill runs the loop end to end: a mine extracts under time, the
harness writes the grid, and the engine reads 15.0 units back where
15.0 were extracted. The engine reads rasters north-up, latitude
decreasing with row, which cost two wrong queries before the boundary
spike's own fixture check named the convention; it is now recorded
beside the writer.

## What the sim consumes

### Land value reads the engine

`DesirabilitySystem::tick` takes the delivered desirability as its
base wherever the probe covers, bilinear, with the flat 50 everywhere
else. The snapshot is taken once before the parallel loop so no lock
crosses it. No delivery means byte-identical old behaviour.

Desirability itself is the engine's habitability node read as
settlement suitability: the weighted mean of its per-biome scores
(grassland and temperate found cities, arid and rainforest resist)
normalised by their own mass, multiplied by buildability from the
field's slope. The first version read a scalar `score` key that never
existed, and every parcel scored 0.0 until a windowed run printed the
mean and said so.

### Extractor reserves read the engine

A painted deposit cell is a measured row and wins outright. An
unpainted cell falls back to the delivered coal channel through the
same bilinear geometry, clamped so a hot channel cannot mint reserve
past full richness, and outside coverage the reserve stays the zero it
always was. The fallback is computed live at polygon-commit time and
never baked into a save, so derived values cannot masquerade as
authored ones.

Iron and stone gained profiles (`iron_mine_basic`, `quarry_basic`),
channel fractions, and an engine-only reserve walker for resources
with no paint layer. Neither has a building asset in the pack yet, so
they wait on the asset editor for bodies.

### The director paces arrivals

`border_openness` becomes the twelfth fiscal control, which is what
lets the public policy setter accept it and gives the border the dial
its presentation layer was already built around. The director's
population multiplier drives it on the delivery cadence: build-up
admits everyone, the peak saturates, the fade closes the gate, relax
reopens it halfway. Frequency, never strength, which is the design law
the director node itself states.

## The world on the engine

### The stored-height convention

Stored terrain heights are real metres divided by the render
exaggeration; the level tool divides by it and world Y multiplies it
back. The ground fill and every byte the terrain source emits ignored
that convention and wrote raw metres, which drew every hill twenty
times too steep. Flat-zero worlds hid it because every convention
agrees at zero.

The three-world probe convicted it: flat zero placed a building, the
engine-filled world rejected every placement with tie-in failures, and
the same world placed first try on raw unleveled ground once the fill
divided. Slopes are now their authored gentleness and the game's own
budgets meet honest grades.

### World scale and the band limit

`WORLD_SCALE_M` is 1000 m per field unit, and `ground_m()` is THE
ground: terrain, water, network, and physics all read it, so the scale
cannot fork between them. The Rust `apply_engine_ground` takes the
same scale from the shell for the same reason.

The footprint is a band limit in field units, authored in metres, so
it rides the same division. Passed raw it faded out every octave finer
than half a kilometre and a 64 m patch measured flat. Every sampler
now passes its own spacing as its band limit: the sim its 10 m cell,
each render patch its texel step, because detail finer than the grid
that reads it can only alias.

Ore veins got the same treatment at 250 m per unit, authored game
taste like the settlement weights. At raw metres a whole vein lived in
every metre of rock, so the probe grid read noise and an extractor
polygon averaged the ore away. Ten metres apart now reads the same
body to four decimals; a kilometre of samples spreads 0.33.

### Ground truth reconciliation

`apply_engine_ground` derives every terrain sample still at the base
elevation from the promoted fBm twin at that sample's world position,
into both height buffers, running the same road-surface dirtying a
sculpt runs. Sculpted samples stay as measured overrides and nothing
pushes undo state, because derivation is not an edit. It runs once per
world, re-applies on both game load paths, and never runs in the
editors, where a filled cell would export as authored world data.

The fill parallelises by row through rayon and widens its footprint to
the terrain cell. Boot on the development box went 684 s to 353 s
across those two changes plus the band limits. The remaining floor is
per-texel GDScript patch evaluation, whose successor is the engine's
compute path.

### Deformation draws

The renderer's engine branch draws the fine field plus the sim's
deviation from its undeformed baseline. The baseline samples the
ground on the sim's own cell lattice and interpolates to texels
exactly as the sim's payload does, so the difference is deformation
and nothing else: a sculpt draws its full depth where the sim holds
it, untouched ground keeps octaves the sim grid cannot carry, and
detection costs arithmetic only.

Water carries the same law. `WaterPatchSnapshot` gained `ground_data`,
the sim terrain heights on the exact samples its own loop walks,
filled through a caller closure so the water system grows no terrain
dependency. The shell's depth source composites that testimony, so a
dug shore moves the shoreline.

### The pixel posture

`EngineTick` sets the viewport's 3D render scale to one art pixel per
`pixel_size` screen pixels, read from the pixelate node's own config.
It is SPEC 13.9's cheap end until the dial is built, and the largest
performance lever the development box has.

## Systems consuming the engine

Weather at the listener, fire danger from Rothermel timings, tides
breathing the water level through the orbital and sea-level nodes,
hydrology and flood basins, minerals from strata environments and
biomineral carbon accounting, physics contact from the field's own
gradient, gait phase per stride, contagion, snowpack, vehicle
rollover, cartography, and derived sound: acoustic modal profiles
rendered by damped modal synthesis, so a material's voice is its
composition and geometry rather than a sample.

Minds are a roster of living instances on the finished mind contract:
spawn, fixed-step tick, per-mind state, each carrying a certified
creature and its drives. The policy table this once cached was removed
upstream with the layer it certified, so the intake's policy channel
came out end to end rather than staying a dead wire.

## Proven in a rendered window

`spike_engine_live.gd` runs the real Main scene windowed, the way the
traffic spikes ran. Twenty-six checks, zero failures, in one boot: the
dig drawn (sim height and centre pixels moving together), the coal
loop banking 6,979.99 units from the delivered channel with the mine
placing first try on raw filled terrain, five parcels zoned, the city
exporting bounds and the re-aimed grid delivering whole, five family
houses grown on delivered land value, an engine-derived strike playing
through the game's audio, and 49 ms/frame sustained over 300 frames at
speed five on a two-core A9.

The growth precondition, learned by measurement: demand sits at
(0, -1, -1) forever without an outside world, and the gate counts a
Border node with any road edge attached. A sixty-metre disconnected
stub at the map edge, designated through `check_border_candidate` and
`set_border_connection`, lifted demand to 1.0 and grew five buildings.
A living economy needs more than pressure, though: households,
workers, and therefore production all waited on a connected border
road.

## Verification

Twelve headless spikes cover the boundary, gateway, mesh source,
terrain source, sound, systems, tides and minds and water, the
director, weather and fire, vehicles, and two integration waves; all
twelve green on the final tree. Headless probes cover placement
discrimination, the save round trip, deposits upward, and the director
consumer.

Every spike appends one JSON entry per run to `godot/benchmarks.json`:
verdict, wall time from process start, CPU model, cores, RAM, and
Godot version, so any machine's numbers stand beside any other's.

The parse gate (`gate_check.gd`) loads every adapter and drill as a
fresh GDScript before any long launch, and caught three parse errors
that would each have cost a twelve-minute boot.

Screenshots go to a dated `screenshots/spikes_<range>/` folder named
`spikeNN_<subject>[_pixelart_filter]_<W/100>x<H/100>.png`.

## Found and recorded, not fixed

A pre-existing frontage failure: on a flat world at a nonzero base
elevation, with no engine involvement, placement rejects with "no
nearby road frontage can fit this building". Flat-zero worlds mask it.
Probed three ways and left for the author's ruling.

A terrain process-loop deadlock against the state a stepped sim leaves
behind: six reproductions, a renderer lineup that convicted Terrain by
name, and a CPU sample reading 0.00 s over eight, a true lock rather
than a grind. Headless with the same city survives, so it is
render-path bound. The live spike fences it loudly and the diagnosis
is tasked.

Savegames saved from a filled world persist the derived cells, so that
world's ground freezes at the fill-time field rather than re-deriving.
Measured in the round-trip drill rather than assumed.

## Process rules earned the hard way

Every windowed launch embeds a log-stall watchdog that kills the
process by name after five stalled minutes, because a frozen Godot
never sends a completion signal and one overnight freeze cost six
hours.

Heavy compiles run at below-normal priority so the development box
stays usable, and no two Godot processes run at once.

## Every file touched

Adapters, all new in `godot/scripts/core/`: `engine_boundary.gd`,
`engine_terrain_source.gd`, `engine_water_source.gd`,
`engine_network_source.gd`, `engine_mesh_source.gd`,
`engine_physics_source.gd`, `engine_mineral_source.gd`,
`engine_social_source.gd`, `engine_weather_source.gd`,
`engine_fire_source.gd`, `engine_tide_source.gd`,
`engine_hydrology_source.gd`, `engine_mind_source.gd`,
`engine_director_source.gd`, `engine_sound_source.gd`,
`engine_sound_player.gd`, `engine_ambience_source.gd`,
`engine_gait_source.gd`, `engine_outbreak_source.gd`,
`engine_snow_source.gd`, `engine_map_source.gd`,
`engine_flora_source.gd`, `engine_vehicle_source.gd`,
`engine_tick.gd`, `rust_gateway.gd`, `spike_stats.gd`.

Renderer branches, each behind the workflow toggle with the Rust
payload as fallback: `terrain.gd`, `water.gd`, `buildings.gd`,
`agents.gd`, `network_tool.gd`, and `input_manager.gd` for the
re-apply hook.

Rust: `simulation/engine_inputs.rs` and `engine_twin/fbm.rs` are new;
`grid/desirability.rs`, `resources.rs`, `extraction.rs`,
`terrain/mod.rs`, `water/mod.rs`, `economy/fiscal.rs`,
`nodes/sim/editing.rs`, `nodes/simulation_node/engine_api.rs`,
`async_terrain/node_jobs.rs`, `variant_export/water.rs`, and
`save/mod.rs` changed.

Data: `economy/profiles.toml` gained two extractor profiles.

Docs: `ENGINE_CONVERSION.md` is new and owns the whole conversion;
`README2-ONBOARDING.md` gained the engine section and the standing
rules.

## Not built

The actor path (creature, anatomy, and limb on gait) waits on the
engine side. Raymarched terrain waits on the engine's compute path.
The GPU twin does not yet share render load. Iron and stone extractors
have no building assets. The in-game gym, zoo, and museum are queued
behind the wiring.
