# Changelog

Everything in this fork that is not upstream, against `859afab`.

29 commits, 89 files, 7,333 insertions, 156 deletions.

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

`godot/addons/manifest_headers/` adds an editor dock with three actions: check
what would change and write nothing, clean up and fix it, and declare the custom
fields a project wants tracked. It injects missing headers, standardizes existing
ones, and expands a bare `HEADER` marker into a full divider.

## Documentation

Sixteen documents were touched. Line numbers are ranges in the current file,
against the upstream base 859afaba.

Five are new, and the whole file is the change:

| File | Lines | Why it exists |
|---|---|---|
| `docs/narrative.md` | 1-300 | A search of the existing docs for theme, setting, story, or tone returned no design statement |
| `docs/services.md` | 1-283 | The whole written record of services was one line naming `always_on_service` |
| `docs/region.md` | 1-240 | Founding, expansion, the density law, incorporation, and national parks had no home |
| `docs/simulation_layers.md` | 1-109 | The physical and living layers being ported, and the ordering |
| `docs/transit.md` | 1-51 | Records that tram, subway, and train are one rail network before three systems get built |
| `docs/README2-ONBOARDING.md` | whole file | A landing page: the Rust and Godot split and which side owns what, how to run it and the debug flags, the directory layout, which document owns which subject, where to start by task, and the in-engine documentation convention |

Eleven already existed and were edited in place:

| File | Lines touched | What changed |
|---|---|---|
| `docs/zoning.md` | 18, 53, 206, 314-567 | Alleys entire, build granularity, gridless parcels, districts, water frontage, parking supply |
| `docs/roads.md` | 23-26, 114-271, 356, 372, 389, 526, 767-773 | The lane model, intersection control, and three forbidden regressions naming the formulas that must not return |
| `docs/terrain.md` | 97, 160, 167, 181, 518, 933, 948, 952, 982, 989, 1253, 1266-1360, 1367, 1460 | World generation and mineral deposits |
| `docs/economy.md` | 10, 2229-2318, 3216 | The industry scaffold, the tycoon layers, the two money pools, foreign capital |
| `docs/ui.md` | 5-6, 45-46, 82-83, 109-110, 161, 166, 287, 381, 552, 609-612, 618, 623, 627-670 | The road builder, post-placement editing, the camera |
| `docs/asset_editor.md` | 1004, 1293, 1738-1739, 1779, 2096-2151, 2232 | The editor as a first class system, interiors generated once |
| `docs/building_allocator.md` | 328-393 | Frontage roles |
| `docs/README.md` | 22-75 | What 2.5D_engine is, how the project was found, what this contribution changed |
| `docs/traffic.md` | 48, 328-372, 405 | Routing feedback |
| `docs/entrance_and_exit.md` | 653, 726, 734-741, 767 | Frontage role and service way consistency |
| `docs/project.md` | 9, 20, 522 | Scale target |

Two design changes landed after the documents were first written. The funding
model is now two pools that arrive in sequence rather than three that coexist:
the city budget pays upward to a region the player cannot see into, the region
unlock turns those taxes into income sized to found the second city, and the
second region unlock creates the national pool and moves power, border patrol,
and national parks onto it. `economy.md` owns the pools; the three-pool table in
`services.md` is gone. Immigration grants are a new lever in `services.md`,
priced per arrival and bounded above by the border policy in `narrative.md`.

The scale target moved from 1,000,000 population to 20,000,000 agents across
cities, regions, and simulation tiers, in `CLAUDE.md`, `docs/project.md`,
`docs/economy.md`, and `docs/asset_editor.md`. Three sites keep 1,000,000
deliberately: a pathfinding argument that holds at any population, a memory
budget whose arithmetic is only correct at that number, and an F12 cheat that
adds money.

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

That run proves the types marshal and the signal math is right. It does not
prove a vehicle obeys a signal, because no vehicle is involved, which is how a
gate that never fired passed 27 assertions.

A vehicle has still not been observed stopping at a red light. The gate bug
above is fixed and the code compiles, but the windowed run that would demonstrate
it has not produced a clean result, because the junction conflict problem below
dominates whatever the signal does.

Nothing has been rendered, played, or profiled. No benchmark has been run against
the Criterion suite. The 20 km benchmark map does not finish generating on the
development machine in a debug build, so no populated world has been loaded.

Traffic does run: `spawn_test_traffic` puts looping cars on an authored grid,
and 400 cars were observed alive across a junction at 4x speed for 600 frames.
What has not been observed is a car stopping at a red light, because the
shutdown crash below cut every run that combined signals with moving traffic.

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

Two earlier verification claims in this file were wrong and are withdrawn. A
windowed run reporting that traffic stopped under an all-red junction was
measuring cars leaving a fixed radius, not cars halting, and it ran against a
gate that never fired.

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

- Both directions of one street go together, which is what a signal phase does.
  A street is identified by approach bearing, because a cross junction splits one
  street into two edges and an edge id cannot name it.
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
needs checking after the next change. They build cross junctions, spawn traffic,
install two-phase signals, group junction arms into streets by geometry, cluster
car positions, and count overlapping vehicles.

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
