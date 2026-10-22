# Metrum Rise - Traffic Movement

This document owns lane-bound vehicle movement behavior: car following, junction connector
traversal, lane changes, conservative overtaking, render smoothing, and traffic debug output.

It does not own:

- road surface / roadbed geometry; see [`roads.md`](roads.md)
- building entrance planning and exact attach / detach selection; see
  [`entrance_and_exit.md`](entrance_and_exit.md)
- stable constants tables and buffer formats; see [`reference.md`](reference.md)
- future public transport mode design; see `TRANSIT-*` rows in [`roadmap.md`](roadmap.md)

Implementation lives mainly in:

- `rust/src/simulation/economy/agents/tick.rs`
- `rust/src/simulation/economy/agents/data.rs`
- `rust/src/simulation/network/lanes/vehicle_junctions.rs`
- `rust/src/nodes/sim/render/lane_pose.rs`
- `rust/src/nodes/sim/core/snapshot.rs`
- `rust/src/nodes/sim/render/agents.rs`
- `godot/scripts/renderers/agents.gd`

## Current Scope

### Shared lane occupancy contract (TRAFFIC-02)

All live lane acquisitions use `agents/lane_state.rs`: building egress and ingress rollback,
network entry, connector entry/exit, direct continuations, lane changes, road-edit reattachment,
building-origin recovery, and debug spawning. The shared writer validates the lane ID and travel
mode, finite nonnegative distance, and connector junction ownership before publishing lane,
edge, node, distance, and transit state together. Routing, access legality, and traffic claims
remain the responsibility of the requesting subsystem. Leaving a lane and clearing/replanning
a route remain separate operations.

The contract costs O(1) per acquisition and allocates nothing on success. It uses safe disjoint
field references, including at the existing parallel movement boundary; it adds no unsafe blocks.
A violated runtime invariant panics at the caller with agent/mode, previous occupancy, requested
lane and network revision. It does not silently change travel mode or relocate the agent.
`./run.sh --release` enables the existing crash diagnostics by default, so a rejected transition
is captured with its caller and the simulation flight recorder under `logs/` (see
[`debug.md`](debug.md)). This is a fail-fast invariant, including in release builds.
Save and load share the mode/owner checks. A rejected save retains the previous file through the
existing transactional replacement mechanism.

The `zoned-city-26-days.sqlite` investigation found 14 walkers assigned to vehicle lanes; lane 61
held five walkers ahead of nine cars. Their pedestrian destination plans and saved positions
confirm inconsistent runtime occupancy. Nearby road additions are a possible trigger, but the
original corrupting transition has not been reproduced. This contract refactor does not repair
that already-invalid snapshot or establish the historical root cause.

Performance acceptance (2026-10-14): matched unprofiled release executables, baseline
`abb37aa5` (SHA-256 `eabe4dc799995762e66b84552deb5865133b31fc19d59b8f5dee797da221136d`)
and the contract working tree (`11e08f2a3a59097cca794527c1741edbcbfa2451b6d8d0d53ba0f2678167c3a3`),
each run as `RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" <test> --exact
--ignored --nocapture --test-threads=1` in alternating order with nothing else running.

- `benchmark_lane_transitions` (five pairs): transition counts match in every scenario. At
  65,536 groups the median deltas are clear +1.3%, merge +1.6%, blocked +1.4%, cruise +2.9% and
  lateral −4.0% (at most +0.08 ms); smaller tiers stay within pair-to-pair noise.
- `benchmark_agent_tick_contract` (`PERF-06`, 25 pairs): work counts are identical. Medians at
  10k/30k: network 0.228→0.237/0.494→0.494 ms, egress 0.478→0.471/1.415→1.381 ms; ingress
  (0.230→0.265/0.443→0.361 ms) has overlapping interquartile ranges in both directions.
- `python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS"` on both
  executables reports the same pre-existing `PERF-03`–`PERF-05` budget and scaling failures,
  with every non-timing field identical. Daily ticks are 6.98→6.62/34.45→34.56 ms
  (baseline) and 22.01→21.72/73.88→70.96 ms (fiscal stress) at 10k/30k.

The shared contract therefore adds no measurable cost beyond about 2% on transition-dominated
microbenchmarks and leaves simulation work unchanged.

Save format 58 stores one identity row per live lane: road edge, direction and local lane index,
or junction node plus incoming/outgoing road lanes. Current and planned agent references resolve
through these identities after graph compaction and lane rebuilds. Tombstoned lanes are excluded.
Loading rebuilds final visible-surface lane heights before restoring distances and references,
and validates each current lane against its owner and travel mode. Pedestrian lanes remain active
through load, including final frontage legs with an exhausted graph-node path.
An empty network path can also be a pending immediate replan after demolition. With valid
node, edge and destination references, loading preserves its planned target and path cursor
even when retry time and failure count are zero. An agent in network transit at a valid node
stays there and replans from the node whatever its mode, as the running game does: a road edit
that rebuilds the junction lane a walker is crossing leaves it at the node with no edge (found in
`SIM-02-I` part 3, when loading sent such walkers straight into their destination). Delayed
retries retain their backoff; agents without a usable destination or pending retry still
receive orphan repair.

Version 57 saves recover sidewalk direction from the saved road/node and connector identity from
the saved junction, outgoing route and authoritative position (including the pedestrian offset).
Recovery only inspects that edge or junction's indexed lanes and rejects unmatched connector
poses. Equivalent matching route prefixes use stable lane order. New saves use explicit identities.
This work occurs only during save/load; current-lane pose sampling remains allocation-free O(log P)
in the lane's point count, and agent restoration uses local owner lookups rather than city-wide scans.

The current traffic model is intentionally local and deterministic. Cars are independent agents
travelling along lane centerlines with per-lane occupancy buckets used for local gap checks.
There is no global microscopic traffic assignment, signal timing, parking search, multi-car
negotiation, or stochastic driver personality model yet.

Supported live behavior:

- car following with simplified IDM
- road speed limits
- junction connector lanes with smooth cubic geometry
- curve-based junction turn speed caps
- acceleration and braking limits
- connector entry spacing so multiple cars can use a junction when safely separated
- planned same-edge lane changes for reaching the correct destination frontage lane
- conservative same-edge overtaking and return-to-cruise-lane behavior
- render-side position and rotation smoothing by stable car render ID
- `./run.sh --debug traffic` logging and visual traffic debug overlays

## Core State

Traffic state is stored in the agent SoA. The traffic-owned continuation fields are:

- `current_lane_id`: the lane currently governing longitudinal movement
- `lane_distance`: metres travelled along `current_lane_id`
- `speed`: current car speed in m/s
- `lane_change_from_lane_id`: source lane for an active S-curve lane change, or `u32::MAX`
- `lane_change_start_d`: lane distance where the active lane change started
- `lane_change_length_m`: longitudinal length of the active lane-change S-curve
- `overtake_blocked_time_s`: time spent held below free-flow speed by lane traffic
- `overtake_cooldown_s`: cooldown before another discretionary overtake / return lane change

Save format 75 preserves speed, `lane_change_*` and `overtake_*`: they affect occupied lanes,
arrival times and later transactions. Active lane-change origins are remapped through saved lane
identities. New benchmark agents may still start with inactive defaults.

## Lane Buckets

The buckets hold `(lane_distance, agent_index)` per lane, sorted by distance with an index
tiebreak. The tick keeps them across ticks: the end of each tick fills them from every live lane
agent on the shared pool (agents grouped by lane range in one pass over 4,096-agent chunks, then
each lane range fills, sorts and overlap-corrects its own lanes in agent order), appended agents
join at the next tick start, and swap-removes patch them in place (`SIM-02-K` part 4,
[simulation.md](simulation.md#sim-02-k-part-4--agent-tick-2026-10-20-first-commit)). A bucket is
sorted again after an overlap clamp, so the snapshot is always the function of the agents' state
that a rebuild gives. These buckets are the hot-path structure for:

- nearest-car-ahead gap lookup
- connector entry slot checks
- lane-change target gap checks
- post-movement overlap correction

The bucket fill reuses retained scratch buffers and allocates nothing per agent once they have
grown. This is required for the 1M-agent scale target; traffic logic must not allocate per agent
in the tick hot path.

Active lane-changing cars are inserted into their target lane bucket and, before movement, also
into their source lane bucket while the S-curve is still active. This lets cars in both lanes react
to the crossing car during the IDM speed pass. Post-movement overlap correction clamps the current
authoritative lane.

Removing the last agent also retires dirty lane occupancy before the
empty-agent tick returns (`AUDIT-01-A5`). Cleanup reuses the existing dirty-lane list and
buffers. Once these are empty, later empty ticks skip the occupancy rebuild. The separate
low-frequency frontage-delay cache continues its existing decay from live agent speeds. Its
once-a-simulated-second update notes each agent's lane in parallel, then sums each lane range's
cars in agent order on the pool, so the delays are bit-identical for any worker count
(`SIM-02-K` part 5).
Save format 75 preserves that smoothed delay and its fractional update clock, restoring lane
values through saved lane identities; see [frontage delay persistence](entrance_and_exit.md#exact-frontage-congestion-penalty-cache).

Claim preparation classifies each agent independently into the retained byte per agent
(`AUDIT-01-A4`, `AUDIT-01-A12`). Large collections use contiguous 4,096-agent Rayon batches; smaller collections
and single-worker execution use the same direct loop. Lane-change selection is shared with
movement (`AUDIT-01-A12`): it inspects the current edge's lanes and sorted local occupancy, with
O(D + log K) work for D local lanes and K occupants. The byte distinguishes ordinary parallel
movement, fixed lateral candidates and dynamic handoffs, so ordinary movement skips a second
lane-change decision. There are no per-agent allocations or additional per-agent buffers.

A lateral move that cannot reach another lane end or frontage handoff this tick reserves its
target in the existing per-lane array. Atomic minimum selects the lowest agent index independently
of worker scheduling; these cars then move in parallel. Dynamic handoffs execute afterward in
stable agent-index order and use the remaining unreserved lanes. Fixed lateral reservations
therefore have priority over connector/frontage arrivals for that tick. Reusing one's own
reservation is allowed, including entry followed by a frontage detach.

Each lane stores one owner index (`usize::MAX` when free), replacing the former boolean. On a
64-bit target this adds seven bytes per retained lane; reset remains O(L) in retained lanes, in
parallel 4,096-lane chunks from 65,536 lanes.
Connector choices retain their existing `stable_index` algorithm and seeds.

## Car Following

Cars use a simplified Intelligent Driver Model in `idm_new_speed()`:

- free-flow speed is the current road edge speed limit, or the connector turn speed in junctions
- gap is bumper-to-bumper distance to the next car in the lane bucket
- the full approach-speed IDM term is not implemented yet because per-agent lead speed is not
  tracked

Current constants:

- `IDM_A_MAX = 6.0 m/s^2`
- `IDM_B = 6.0 m/s^2`
- `IDM_T_HEAD = 0.5 s`
- `IDM_S_MIN = 0.1 m`
- `CAR_LENGTH = 2.6 m`

After IDM proposes a target speed, `limit_speed_change()` clamps the change by acceleration and
comfortable braking. This prevents cars from snapping instantly between stopped and free-flow
speed.

The speed pass writes each result directly to that agent's SoA slot (`AUDIT-01-A7`). This is
independent because the shared lane snapshot contains distances and the current IDM model reads
no other agent's speed. The duplicate speed array and its final serial copy are removed. Non-car
and inactive agents retain their current speed while their existing traffic-timer rules run.

If a car is approaching a blocked connector or a required lane change whose target gap is blocked,
the speed pass computes a braking speed for the remaining distance. This makes the car slow before
the conflict instead of teleporting or stopping only at the exact handoff point.

## Road Speeds

The current urban road presets use `50 km/h`, stored as:

- `DEFAULT_URBAN_ROAD_SPEED_MS = 13.89 m/s`

The simulation stores speeds in m/s. `KMH_TO_MPS = 1 / 3.6`.

Normal road bends do not currently receive a curvature speed cap. A car on a curved road lane
keeps the road speed limit unless traffic or upcoming connector / lane-change constraints slow it.

## Junction Connectors

Vehicle junction movement uses explicit connector lanes generated in
`network/lanes/vehicle_junctions.rs`.

For every allowed inbound-to-outbound vehicle lane pair at a node, the lane system creates a
connection lane with:

- `edge_id = usize::MAX`
- `lane_type = LaneType::Vehicle`
- `node_id = junction node`
- `next_lanes = [target road lane]`
- cubic Bezier geometry from the inbound lane neck to the outbound lane neck
- at most `1 m` chord sampling, clamped to `8..64` steps

At true junctions (`degree >= 3`) and user-authored vehicle connection nodes, connector control
distance and sampling use the junction footprint span: the larger of the lane-mouth chord, the two
mouth distances through the node center, and the widest incident roadbed span. This prevents
two-lane to four-lane T-junctions and explicit connection nodes from degenerating into tiny lateral
connector shims that cars traverse almost instantly.

Allowed turns are derived from `Node::lane_connections`:

- if the node has no user vehicle connections, all non-U-turn outbound lanes are open
- if the node has any user vehicle connection, the node is in whitelist mode and unspecified turns
  are blocked
- terminal nodes may connect back to the same edge

Strict degree-two corridor splits may skip materializing a vehicle connector and directly link the
aligned incoming physical lane to the outgoing physical lane. This direct pass-through is allowed
only when the node has exactly two incident edges, no user-authored vehicle lane connections, the
lane mouths are coincident, and their XZ tangents are near-collinear. True junctions and all
user-authored vehicle connection nodes must keep explicit connector lanes so junction speed,
spacing, and whitelist semantics still apply.

Cars enter connector lanes through `TRANSIT_INTERSECTION`. Connector lanes are lane-bucketed like
road lanes, so multiple cars may occupy the same connector when they respect separation. Connector
entry also uses a per-tick claim to avoid two cars grabbing the same zero-distance entry slot.
Car exits check outgoing-lane occupancy and acquire the same reservation (`AUDIT-01-A11`). A
blocked car waits at the connector end with its route intact until the outgoing entry clears.

## Junction Speeds

Junction movement uses a cap separate from the parent road speed:

- `CAR_JUNCTION_SPEED_MS = 6.0 m/s`
- `CAR_JUNCTION_LATERAL_ACCEL_MS2 = 2.2 m/s^2`
- `CAR_JUNCTION_MIN_SPEED_MS = 2.0 m/s`

`connector_turn_speed()` estimates turn severity from the angle between the connector's first and
last non-degenerate tangents. For straight or very gentle connectors, the cap is
`CAR_JUNCTION_SPEED_MS`. For tighter connectors, approximate radius is:

```text
radius = connector_length / turn_angle_radians
```

The turn speed cap is:

```text
sqrt(CAR_JUNCTION_LATERAL_ACCEL_MS2 * radius)
```

clamped to `CAR_JUNCTION_MIN_SPEED_MS..CAR_JUNCTION_SPEED_MS`.

`CAR_JUNCTION_MIN_SPEED_MS` is the lower bound for the curvature-derived cap. It is not a rule that
boosts a stopped or slow car up to crawl speed when entering a connector.

Road-lane speed prediction looks ahead to the next connector and brakes early if the connector is
tight or blocked. Once in `TRANSIT_INTERSECTION`, the car remains capped by the connector's
curvature speed until it exits to the target road lane.

## Planned Lane Changes

Planned lane changes solve a different problem from overtaking: a car may need to move to a sibling
lane on the same edge to reach the exact `planned_detach_lane_id` for building access.

Ownership split:

- [`entrance_and_exit.md`](entrance_and_exit.md) owns choosing `planned_detach_lane_id` and
  `planned_detach_lane_d`
- this document owns how the car physically changes lanes once that plan requires it

Rules:

- only cars in `TRANSIT_NETWORK` may start a planned lane change
- source and target lanes must be same-edge, same-direction vehicle lanes
- movement advances one adjacent lane at a time toward the final planned detach lane
- the target lane must have a speed-scaled safe gap at the current distance
- the car must own the target lane's reservation before starting the maneuver
- clear target lane means no intentional speed penalty
- blocked target lane is traffic and may force braking before the detach point
- planned lane changes take priority over discretionary overtaking

Lane-change length is speed-scaled:

```text
length = speed * 3.5 s
```

clamped to:

- minimum `18 m`
- maximum `70 m`

The car's authoritative longitudinal lane becomes the target lane when the maneuver starts, while
rendering blends from the source lane to the target lane over the S-curve. This keeps traffic gap
checks and destination detach checks tied to the lane the car is entering, without creating a
temporary lane object per car.

## Conservative Overtaking

Overtaking is intentionally conservative and local. It exists to make multi-lane roads feel alive
without adding a full driver AI.

Rules:

- only cars in `TRANSIT_NETWORK`
- only same-edge, same-direction sibling vehicle lanes
- never while a planned lane change is pending
- never while already lane-changing
- never near the current edge end
- never near a planned building detach point
- target lane must have a safe speed-scaled gap at the current distance
- target lane must have meaningfully more space ahead
- one adjacent lane at a time
- cooldown after each discretionary overtake / return

Current constants:

- stuck time before passing: `2.0 s`
- cooldown: `8.0 s`
- minimum speed-gain condition: `2.0 m/s`
- minimum gap gain over current lane: `12 m`
- target ahead gap for passing: `30 m`
- target ahead gap for returning: `20 m`
- edge-end buffer: `12 m`
- detach buffer: `25 m`

Lane direction convention:

- forward lanes use non-negative lane indices; lane `0` is closest to the road center
- backward lanes use negative lane indices; lane `-1` is closest to the road center
- overtaking moves toward the center lane
- return-to-cruise moves outward

This rule intentionally avoids weaving:

- a car must be traffic-blocked before passing
- a car returns only when the outward lane is clear and the overtake cooldown has expired
- planned destination-lane changes override overtaking

## Render Movement

Rust produces car transforms from lane poses in `nodes/sim/core/snapshot.rs`; the former direct
car/pedestrian exporters are removed. `nodes/sim/render/agents.rs` owns path debug overlays only.
The snapshot samples an active car lane-change curve only when source and destination lanes have
matching road, direction and lane type. Invalid/stale sources use the current lane pose. This
reuses the existing allocation-free lane sampler; it adds one bounded source-lane lookup and, for
active changes, source-lane sampling, not a scan over agents or roads. The snapshot regression
`car_snapshot_keeps_lane_change_curve_in_both_directions` checks start/midpoint/end positions,
sloped lane heights, lateral orientation, ownership flags and mismatched-source rejection.

The final audit's `lane_change_snapshot_benchmark` measures 100/1,000/10,000 cars on two sloped
lanes, with fixture setup excluded, ten warmups and 100 recycled snapshots per workload. Release
p50 for inactive versus all-active lane changes is 0.0316/0.0363, 0.1189/0.1507 and 0.8272/0.9872
ms respectively (`RAYON_NUM_THREADS=24`). This isolates the snapshot sampling cost; it is not a
gameplay frame-time benchmark.

Lane pose sampling in `nodes/sim/render/lane_pose.rs`:

- samples position by distance along lane geometry
- samples tangent using a `2 m` look-behind / look-ahead window
- falls back to the local segment tangent when the look window degenerates
- samples active lane-change S-curves by smoothstep blending source-lane and target-lane positions
- derives S-curve tangent from blended lane tangents plus the lateral blend derivative

Render-side car interpolation runs in Rust (`nodes/sim/render/car_visual.rs`, `RENDER-14`).
Each frame `godot/scripts/renderers/agents.gd` passes `get_car_render_data()` to
`smooth_car_render_data(car_data, delta)` and uploads the returned per-bucket buffers:

- cars have stable render IDs from Rust; drawn poses are main-thread presentation state on
  `SimulationNode`, never saved or read by the simulation
- pedestrian and vehicle MultiMeshes use the shared dynamic shadow-caster policy
- origin interpolation smooths per-tick position updates
- basis interpolation smooths rotation (Godot's `Basis.slerp` formula, reimplemented on `glam`;
  godot-rust's `Quaternion::slerp` is an engine call per car)
- large jumps above `80 m` snap instead of interpolating across the map
- a change between lane and off-lane height owners snaps instead of blending
- off-lane cars are grounded by the shared support solve after interpolation; when the core is
  busy they show their validated snapshot pose, never an unsupported interpolated one
- a car absent from one frame is forgotten; degenerate or non-finite snapshot poses pass through
  unchanged and are not remembered
- transform uploads happen every render frame so fast sim multipliers do not quantize cars to the
  simulation tick rate

Current render interpolation constants:

- `INTERPOLATION_RATE = 24.0`
- `ROTATION_INTERPOLATION_RATE = 18.0`
- `SNAP_DISTANCE_M = 80.0`

**Cost (`RENDER-14`, 2026-09-30).** Smoothing is O(visible cars) with one hash lookup and insert
per car. Buckets of at least 2,048 cars interpolate in parallel on the global Rayon pool, the pool
the support solve already used from the main thread. A populated-city run should confirm that the
main thread does not wait on a pool the simulation tick is saturating.

Matched A/B, `godot/tests/vehicle_ground_support_test.gd -- --benchmark-vehicle-support`: 100 timed
`update_swarm` frames with cars advancing every frame. Baseline is `HEAD` `eedd0094` GDScript;
both sides load the same release library (SHA-256 `92a4c47b…94841fda`), whose only change is the
new function. Runs were interleaved, two per side, headless and windowed 640×360, with
`RAYON_NUM_THREADS=4` on an i9-12900K / RX 7900 XTX. Windowed p50 ranges in milliseconds:

| Cars | Lane cars | Off-lane cars |
| --- | --- | --- |
| 1,000 | 1.22 → 0.074 | 1.87–1.96 → 0.60–0.63 |
| 5,000 | 6.06–6.12 → 0.34–0.49 | 8.74–9.28 → 2.23–2.37 |
| 20,000 | 24.9–25.3 → 1.19–1.25 | 34.8–35.3 → 8.22–8.26 |

Headless numbers agree (lane cars at 20,000: 24.6–24.8 → 0.82–0.88 ms). Off-lane cars keep the
unchanged native support solve, about 0.4 µs per car, which is now their dominant cost. Real
off-lane counts are limited to cars in an access phase. The Rust-only kernel
(`car_smoothing_frame_cost`, ignored test) measures 0.045 / 0.17 / 0.66 ms for 1,000 / 5,000 /
20,000 lane cars. After the runs, a clippy cleanup (`as_chunks` in place of `chunks_exact`, with no
behaviour change) produced the deployed `godot/bin` library `7a09021d…bb31156e`. The native tests
and the headless and windowed vehicle grounding test were rerun on it.

**Correctness checks:**
- Native `car_visual` tests cover:
  - a first sighting shows the snapshot pose;
  - origin and heading close their gaps at 24/s and 18/s, and the next frame starts from the drawn
    pose;
  - long jumps and height-owner changes snap;
  - absent cars are forgotten;
  - degenerate and non-finite poses pass through unrecorded;
  - a busy support solve restores only the off-lane snapshot poses.
- The 156 related release vehicle, car and render tests pass. Rustdoc reports no warnings. Clippy
  reports nothing new in the touched files.
- `vehicle_ground_support_test.gd` drives the production `agents.gd` update through the native
  call with real vehicle meshes, and passes headless and windowed. It checks 4,840 poses: no buried
  contact, windowed MultiMesh buffers equal to the drawn pose, and the height-owner snap within the
  snap distance.

## Debugging

Run:

```bash
./run.sh --debug traffic
```

or:

```bash
./run.sh --debug-traffic
```

Traffic debug logging goes to stdout through `traffic_log!` on the `traffic` debug channel, each
line prefixed `[DEBUG:traffic]`. The channel can also be switched on in game from the debug
console; see [`debug.md`](debug.md).

Important log markers:

- `[JUNCTION_ENTER]`
- `[JUNCTION_BYPASS]`
- `[JUNCTION_EXIT]`
- `[JUNCTION_WAIT]`
- `[JUNCTION_MISSING_CONN]`
- `[JUNCTION_MISSING_EDGE]`
- `[JUNCTION_MISSING_EXIT]`
- `[LANE_CHANGE_START]`
- `[LANE_CHANGE_WAIT]`
- `[OVERTAKE_START]`
- `[OVERTAKE_RETURN]`
- `[ACCESS_EGRESS_ATTACH]`
- `[ACCESS_INGRESS_DETACH]`
- `[ACCESS_INGRESS_WAIT]`

When traffic debug is enabled, the agent path overlay (`P`) also shows each car's lane and
junction labels and the connector it will take. To see every connector at nearby junctions,
colored by whitelist, turn speed or occupancy, and to log cars stuck on a connector, use the
`junction_connectors` visual mode ([debug.md](debug.md#junction-connectors)).

## Performance Contract

Traffic code is a hot path. New traffic behavior must preserve these rules:

- no allocation inside per-agent tick loops
- reuse existing lane buckets and scratch buffers
- use `rayon` through existing dispatch helpers for independent agent work
- keep per-agent decisions O(1) or bounded by tiny per-edge lane counts
- do not add a new spatial structure when lane buckets, edge lanes, the road graph, or existing
  indices answer the query
- keep route planning and traffic movement separate; local traffic behavior must not trigger
  per-tick CCH pathfinding

### Short agent-tick contract (`PERF-06`)

Routine traffic changes use the ignored release test
`simulation::economy::agents::tick::contract::benchmark_agent_tick_contract`. It warms each isolated
lane fixture once, then records one setup-free timed tick for the 10k and 30k agent tiers. The
test emits one `PERF_AGENT` JSON line per mode and tier with deterministic row counts for lane
bucket preparation, speed updates, lane claims, movement, the end-of-tick lane fill and frontage-delay
aggregation. The fixture uses one long straight lane, fixed 16 m spacing and no route-planning
work; construction and buffer allocation stay outside the timed tick.

The bounded counts are the contract's first diagnostic. Every emitted phase count must grow by at
most `1.2 × (30k / 10k) = 3.6×`; a zero count at 10k must remain zero at 30k. A failed count is
investigated before any timing comparison.
The one timing sample per tier is a routine regression signal, not a speedup claim. Keep a
release baseline executable beside the current executable and report both build identities when
the contract runs. Matched alternating process pairs remain reserved for milestone comparisons
and claimed speedups. The short contract is run serially with no concurrent build or measurement;
the 30-minute Criterion `agent_benchmark` is no longer a routine gate.

Movement now owns route-query scratch by shards that each take every shards-th chunk of 256
agents (`SIM-02-K` part 4), retaining the existing
small-city serial threshold and separate ordered dynamic movement. Scoped TLS access restores
the previous scratch on return or unwind; this preserves planner entry points while making
memoization independent of which Rayon worker executes a range. Storage is prepared outside
agent loops and bounded by worker count and graph size. The 2026-10-14 owned-scratch build
(`c7ca3b0bf9514d00a0e89d5c7f02a16b47fc0489da9536418073c08de8a2d66a`) passes the short
agent contract: network 0.205/0.587 ms, egress 0.472/1.334 ms, ingress 0.404/0.427 ms at
10k/30k. Agent work is exactly 3× and lane work stays constant. These single samples are
routine validation, not speedup evidence. See `economy.md` for repeatable whole-day CCH counts.

Fresh infrastructure check (2026-10-13): the release test above passed with eight Rayon
workers and inherited CPU affinity, run serially as
`RAYON_NUM_THREADS=8 "$TEST_BINARY" simulation::economy::agents::tick::contract::benchmark_agent_tick_contract --exact --ignored --nocapture --test-threads=1`.
Executable SHA-256: `b660a61e02aa320874e141994a55a518dcdf78efd60569bdba60694dd111c84e`
(working tree based on `395b810f6575ac8c73eee0ff1cbc1d5c5b2b9ad3`).
Network ticks measured 0.417/1.077 ms at 10k/30k, access egress 0.556/1.368 ms,
and access ingress 0.211/0.316 ms. All exercised agent-row counts grew exactly 3×,
lane counts stayed at 16, and fresh-fixture counter repeats matched. The complete test
took 0.05 seconds including setup. These single samples validate the short runner and
work bounds; they establish no speedup against the retained baseline.

`PERF-06` acceptance (2026-10-14): the complete performance runner passes all six agent samples
and deterministic repeats with eight Rayon workers pinned to CPUs `0,2,4,6,8,10,12,14`.
Current executable SHA-256 is `06e8aa0a4a2899a89cff7ad78bcfea5a6c2eb32ee2bdabbd3f2c33449347074c`;
the retained baseline executable's verified SHA-256 is
`b34a0bedb12609c2b1fb8382d2e4f2cabc5e0102553fcd59fca46944fd88d546`.
Network ticks measure 0.242/0.507 ms, access egress 0.458/1.342 ms and ingress 0.143/0.313 ms
at 10k/30k. Every exercised agent count is exactly 3×; lane counts remain 16. The 36 targeted
agent-tick regression tests pass (eight manual tests ignored). These are routine contract
samples, not a measured speedup against the baseline.

Current known bounded scans:

- adjacent lane lookup scans the current edge's lane list, whose count is tiny for supported road
  presets
- connector selection scans `next_lanes` from the current lane, bounded by junction fan-out
- direct pass-through detection scans only the candidate `next_lanes` of the current lane, bounded
  by degree-two split fan-out
- lane bucket gap checks use sorted vectors and `partition_point`

### Claim Preparation Audit Measurements (2026-09-12)

Five alternating unprofiled release process pairs used eight physical cores (`taskset -c
0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`); three pairs used CPU 0 and one worker. The isolated
SoA fixture includes idle agents, walking/car egress, normal road travel, imminent lane endings,
stopped cars, invalid lanes and junction travel. One lane is sufficient for these classification
branches; placement, route planning, fixture allocation and checksum validation are outside timing.
Each process warms up three times, then records 21 samples of eight preparations. Every output
flag matches across versions and worker counts.

| Agents | Eight-core idle before → after, ms | Eight-core mixed before → after, ms |
| --- | --- | --- |
| 1,024 | 0.000631 → 0.000693 | 0.001533 → 0.001574 |
| 16,384 | 0.009895 → 0.010659 | 0.025071 → 0.024581 |
| 131,072 | 0.080190 → 0.014952 | 0.200441 → 0.030323 |
| 1,048,576 | 0.648757 → 0.094635 | 1.621333 → 0.225566 |

| Agents | Single-core idle before → after, ms | Single-core mixed before → after, ms |
| --- | --- | --- |
| 1,024 | 0.000631 → 0.000694 | 0.001604 → 0.001548 |
| 16,384 | 0.009894 → 0.010864 | 0.024039 → 0.024992 |
| 131,072 | 0.080474 → 0.089581 | 0.199360 → 0.200038 |
| 1,048,576 | 0.648518 → 0.687894 | 1.607550 → 1.636986 |

The accepted tradeoff is a small serial cost: the largest single-worker idle case adds about
0.039 ms and the mixed case adds 0.029 ms per preparation. Small-city differences remain below
0.001 ms in these runs. These are classification timings, not full-frame or full-movement timings.
The existing lane-claim order, actual movement, legal detach checks and lane-gap rules are unchanged.

Run `BINARY --exact simulation::economy::agents::tick::claims::tests::benchmark_claim_preparation
--ignored --nocapture --test-threads=1` with the affinity/worker settings above. Both hot predicates
explicitly retain their original in-loop execution in the accepted build.

### Speed Update Audit Measurements (2026-09-12)

The isolated speed-phase fixture uses one straight road lane and one straight connector, with
fixed 16 m spacing and the final agent at least 1,000 m from the lane end. It exercises road cars,
a mix of road cars/walkers/inactive agents/junction cars, and an all-inactive case. Placement,
route planning, occupancy construction, allocation and result hashing are outside timing. Each
process performs three warmups and 21 samples of eight updates; the same fixed sequence produces
identical speed, blocked-time and cooldown bits across builds and worker counts.

Five alternating release process pairs use eight physical cores (`taskset -c
0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`); three pairs use CPU 0 with one worker.

| Agents | Eight-core road before → after, ms | Eight-core mixed before → after, ms |
| --- | --- | --- |
| 1,024 | 0.013677 → 0.012956 | 0.012443 → 0.011602 |
| 16,384 | 0.081637 → 0.068452 | 0.055334 → 0.043820 |
| 131,072 | 0.613947 → 0.551550 | 0.335940 → 0.280316 |
| 1,048,576 | 5.971705 → 6.180697 | 3.360615 → 2.515615 |

At 1,048,576 agents, single-core road timing is 43.944670 → 46.577261 ms and mixed timing is
31.389801 → 24.839429 ms. The fully occupied single-lane stress case therefore has an accepted
3.5% eight-core / 6% single-core cost; the change is not a universal timing improvement. Idle
work retains comparable timings.

The change removes one retained `f32` per speed-buffer slot (4 MiB in the largest fixture) and an
O(A) serial copy. Existing per-agent gap lookup remains O(log K) in that lane's occupancy, with
bounded per-edge lane queries. This fixture intentionally concentrates traffic in very long
lanes; it measures the speed phase rather than gameplay frame time or ordinary city density.

Run `BINARY --exact simulation::economy::agents::tick::speed::tests::benchmark_speed_update
--ignored --nocapture --test-threads=1` with the settings above. The regression
independently reverses agent storage and runs with one/eight workers, comparing every speed
and timer bit after six updates.

### Lane Reservation Audit Measurements (2026-09-12)

`AUDIT-01-A11/A12` closes collisions at connector exits and lateral lane changes. The movement
fixture isolates dispatch, including claim preparation, with 512, 8,192 and 65,536 independent
lane groups. Each connector group has two 10 m connectors entering one 100 m road lane; cases
cover an available exit, competing exits and a stationary blocker. Lateral groups have three
100 m road lanes, competing return/overtake maneuvers into the middle lane and a stopped leader.
Cruising groups have three cars 20 m apart with no maneuver. Cars move at 4 m/s in 0.25 s steps.
Fixture creation, occupancy construction and resets are outside timing. Each process uses three
warmups and 21 single-step samples; no road-end route query occurs in this minimal lane fixture.

Five alternating unprofiled release pairs use eight physical cores (`taskset -c
0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`); three pairs use CPU 0 and one worker. Source and
binaries remain fixed, with no concurrent builds or other benchmark jobs. Median process medians:

| Workload | Agents or retained lanes | Eight-core before → after, ms | Single-core before → after, ms |
| --- | --- | --- | --- |
| Clear connector exits | 131,072 agents | 4.176683 → 4.231970 | 7.400009 → 7.498916 |
| Competing connector exits | 131,072 agents | 5.973477 → 5.260708 | 7.990074 → 7.279029 |
| Blocked connector exits | 196,608 agents | 6.598414 → 6.290669 | 10.158671 → 9.745280 |
| Competing lateral moves | 196,608 agents | 2.500647 → 3.442970 | 13.851123 → 19.403899 |
| Cruising | 1,536 agents | 0.026580 → 0.055582 | 0.051212 → 0.066385 |
| Cruising | 24,576 agents | 0.216818 → 0.237956 | 0.834715 → 1.090162 |
| Cruising | 196,608 agents | 2.064773 → 2.301867 | 11.618839 → 13.034764 |
| Idle classification | 1,048,576 agents | 0.093443 → 0.078447 | 0.633645 → 0.558090 |
| Mixed classification | 1,048,576 agents | 0.234366 → 0.224720 | 1.670352 → 1.637971 |
| Empty occupancy/claim reset | 1,048,576 lanes | 0.122143 → 0.188869 | 0.122626 → 0.202618 |

The corrected contested cases intentionally admit fewer cars: one winner instead of two, or
zero when blocked. Those timing differences are not equivalent-output throughput improvements.
The runner verifies the expected transition counts separately for each build. Classification
checksums match across builds and worker counts; that fixture has no eligible lateral moves.
Classification/reset timings use 21 samples of eight preparations after three warmups. The
empty-agent reset fixture warms all buffers and isolates the wider owner array; reset is serial
under both worker settings.

The accepted eight-core cost is about 0.24 ms for 196,608 cruising agents and 0.94 ms for the
extreme lateral case, which attempts 131,072 lane changes at once. Small cruising batches show
a 0.029 ms increase; the 24,576-agent single-core case costs 30.6% more. These are movement-phase
measurements, not full-city frame timings. The retained classification buffer stays one byte
per agent. On 64-bit targets, replacing a boolean claim with an owner adds seven bytes per
retained lane (7 MiB at 1,048,576 lanes), with O(L) reset work and no per-agent allocation.

The first correct implementation serialized all lane changes and increased the eight-core
lateral case from 2.601286 to 13.154782 ms; it was rejected. The accepted version reserves fixed
lateral targets in parallel using minimum agent ID, then executes those moves in parallel.
Three classification modes in the existing byte avoid repeating the decision for ordinary
movement. Dynamic handoffs retain stable serial order.

The measured ignored tests are
`tick::movement_pass::tests::benchmark_lane_transitions`,
`tick::claims::tests::benchmark_claim_preparation`, and
`tick::movement_pass::tests::benchmark_empty_lane_claim_reset` under
`simulation::economy::agents`, with `METRUM_DEBUG=0` and the worker settings above. Rust is
1.98.1 (`48a229cea`, 2026-09-01). Baseline binary SHA-256
`ed8ee8df8150244d3637cbebbf74318d1e0676a887e6c2e36ab17df7f280e138`; accepted binary SHA-256
`71402a5b38110a6ce1b15ff553e012d0d9814dc52530fe9a0ba292cac6ca18fc`.

## Known Limits

- normal road bends do not yet use curvature speed limits
- full IDM approach-speed interaction is not implemented because lead-vehicle speed is not tracked
- overtaking has no driver personality, urgency, emergency behavior, or multi-car prediction
- there are no traffic lights, stop signs, yield priorities, or priority-road rules yet
- lane changes are centerline S-curves, not full swept-body collision geometry
- parking, driveways, curb queues, and building entrance reservations are not modeled
- connector lanes are generated from lane necks; if road geometry changes materially, lane rebuild
  must keep connector lanes and `next_lanes` in sync
- network entry at a node does not claim the entry slot (`TRAFFIC-03`)
- walkers keep car spacing (2.7 m) in lane buckets, connector entry claims and overlap correction
