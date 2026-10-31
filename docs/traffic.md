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
and validates each current lane against its owner and travel mode. A live session lifts lanes the
same way without waiting for the renderer: rebuilds only append lanes, so `LaneSystem` keeps a
watermark of lanes already on the visible surface, and `TransitNetwork::sync_new_lane_heights`
lifts the lanes past it at the start of every simulation frame and after each road mesh
precompute (O(new lane points); a frame with nothing pending returns at once). Before this, a lane
rebuilt outside a drawn road chunk, such as a crosswalk connector rebuilt by a building edit, kept
its flat build heights and so a shorter length until the next load, and a session continued
differently after save and reload. Pedestrian lanes remain active
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
- geometry from the inbound lane's end to the outbound lane's start, as below
- samples at most `1 m` apart along the path and at most `0.15 rad` apart along an arc

**Lane ends come from the roadbed's mouth frames (`TRAFFIC-12`).** Every vehicle lane ends at a
node on its arm's `MouthFrame` (`roadbed/walkways.rs`): the arm's cross-section line at the edge's
clip there (`start_clip` / `end_clip`, the station every system that maps edge stations to lanes
reads), built from the arm's own knots exactly as the junction layout collects them
(`NodeArms::single`, `ArmInput::section_line`). The graph sets the clip to the layout's mouth
station at a bend or junction, where the node piece ends and the span begins (rounded to `f32`, so
a lane end lies within a micrometre of the mouth row), and to zero at a terminal, a pass-through
node and a node whose layout fails. A lane's end point is its lateral offset on that line, at the
centreline height there; the lane polyline ends exactly on it (`LaneEnd::Mouth`), keeping only
the points more than `1 mm` beyond the station, since the roadbed profile puts a knot at every
mouth station that would otherwise stay as a zero-length end piece. The connector leaves and joins
along the frame's direction, square to the line, and never reads a lane polyline's end piece:
those pieces were zero-length, millimetres long or reversed at mouths and sharp bends, and a
zero-length one sent the old connector off along `+x`. Building a frame is `O(edge points)`, once
per edge end in the lane rebuild and once per incident edge end in the connector builder.

**Curve.** One construction that always exists (`vehicle_junctions/turn.rs`, `VehicleTurn`), of
straights and circular arcs only, so the heading is continuous and the curvature piecewise constant:

- **Tangent arc**, where the entry direction and the exit direction meet ahead of both ends: a
  straight along the entry, one arc and a straight along the exit, a fillet of the two tangent
  lines. Its radius is the largest that fits between the ends, reduced by bisection (24 halvings)
  until the arc keeps half a lane from every kerb corner of the layout (the fitted carriageway-edge
  circles of its concave sectors): at a right-angle kerb-lane turn that is the arc concentric with
  the kerb, and a large fillet no longer cuts a skewed junction's obtuse corner. The longer lead
  keeps its straight. The arc never reaches past the tangent lines' intersection, so it neither
  overshoots nor counter-steers. Construction and kerb bound from Veeti's road-chain branch
  (`1ae3b22f`).
- **Biarc** otherwise: straight on, a lateral shift, directions that meet behind an end, a U-turn,
  or a tangent arc that no radius keeps off the kerbs (none on the swept maps). Two arcs with a
  common tangent at their joint; the joint is the equal-tangent choice: both ends' tangent lines
  are cut at the same distance `d > 0`, the joint is the midpoint of the two cut points and its
  tangent runs between them. `d` exists for every pair of ends except parallel directions with the
  exit level with or behind the entry, which no junction produces; coincident ends (a user
  connection through a degree-two node) give a zero-length connector.

There is no cubic and no fallback; the old junction-span cubic, whose handles (`0.55` of the
footprint span) overshot short turns, is gone. Building a connector is `O(c)` for `c` kerb corners at
the node, `O(24 c)` when the radius is reduced, only on a lane rebuild, and allocates nothing but
its polyline.

**Check.** Each connector is checked once per lane rebuild against the node's kerb corners
(`NodeWalkways::collect_with_kerbs`, from the same layout as the walkways): the
smallest distance from a corner's centre to the connector's straights and arcs, less the corner's
radius, must not be negative (`O(c)`). A violation is logged on the `traffic` debug channel with the
node and both lane ids and is counted by the save sweep; nothing clamps or repairs the curve.

Acceptance record (`TRAFFIC-12`, 2026-10-31; commands and results; the artifact folders stay
local):

- Save sweep: `junction_connector_save_sweep` (ignored; `METRUM_SWEEP_SAVE=<save> $TEST_BINARY
  nodes::sim::core::tests::junction_connector_sweep --ignored --nocapture`, optional
  `METRUM_SWEEP_OUT=<file>` for one line per failing connector and the node layouts) loads a save,
  whose load rebuilds every lane, and measures every vehicle connector at every node with
  `lanes::trajectory_tests::measure_connectors`: the farthest probe (every `0.25 m`) outside the
  compiled carriageway, a step heading against the chord (against both lane directions in turns
  sharper than a right angle, whose exit heads against the chord), a counter-steer where the arm
  directions meet ahead of both ends (beyond `1 mrad` and the f32 resolution of the two steps: the
  demo save lies `8.5 km` from the origin, where f32 rounds points by `0.5 mm`), and a kink (a turn
  over `0.2 rad` between steps or against a lane's end piece); and every lane end against its mouth
  frame. `dev` against the fix (failing connectors, then outside / backward / counter-steer / kinks):

  | Map | `dev` | Fix |
  | --- | --- | --- |
  | Demo save copy (39 nodes) | 117 of 156: 44 (max 4.75 m) / 31 / 89 / 103 | 0 of 161 |
  | Kuopio reference save (80 nodes) | 335 of 440: 80 (max 5.65 m) / 65 / 206 / 288 | 0 of 440 |
  | Saved 100k city (2,220 nodes) | 8,282 of 24,834: 0 / 0 / 8,272 / 8,282 | 0 of 24,834 |

  Causes on `dev` (first match): a lane end piece under `3.2 mm`, so the builder used `+x` as its
  direction (82 demo, 179 Kuopio; the knot the profile puts at every mouth station, cut on exactly);
  an end piece off its arm by more than `0.05 rad` (4 demo); a lane end more than `1 cm` off its mouth
  line, the trim by 3D lane length on curved and sloped arms (14 demo, 62 Kuopio; up to `0.71 m`);
  and the junction-span cubic with good lane ends (17 demo, 94 Kuopio, every failure of the 100k city:
  its right turns counter-steer and kink by `0.24 rad`). With the fix every lane end lies on its mouth
  line, measured against the compiled roadbed's own mouth station (largest distance `0.0000 m`,
  tolerance `1 cm`), its end piece within
  `0.0077 rad` of the arm (tolerance `0.05 rad`) and at least `9.5 mm` long; no kerb violation is
  logged; the largest step is `0.150 rad`; the tightest non-U-turn is a `1.73 m` tangent arc across a
  pass-through node's slight kink (demo node 33) and the tightest junction turn a `3.58 m` biarc at a
  50-degree bend (Kuopio node 29). RJ's six fixtures (`trajectory_tests`) are clean. The 45-degree
  bend now gets two connectors: `dev`'s `+x` end directions made its inner lanes look collinear and
  overlapping, so they were linked straight across the node piece. Span lanes (the mitred offset
  inside a span): no vehicle lane of the demo or Kuopio save turns more than `0.014 rad` between steps
  of at least `1 cm`, so no mitre spike or reversal was found inside a span.
- Product probes: only the lanes component changes (20 of 20 reference entries, 133 of 141 roadbed
  fixture entries); graph, nodes, layouts, spans, walkways and queries hashes stay bitwise equal.
- Rendered top-down shots (`road_junction_reference_test.gd` with `METRUM_ROAD_JUNCTION_CONNECTORS=1`
  and `METRUM_ROAD_JUNCTION_SAVE` / `METRUM_ROAD_JUNCTION_SAVE_FOCI` on a copy of the demo save,
  gamescope headless, isolated profile) of a 90-degree bend with a zebra on one arm (node 19), the
  skewed X (node 5) and a T (node 7): on `dev` connectors hook over the zebras and the sidewalk and
  loop at both acute corners; with the fix every connector stays inside the carriageway between
  the zebras' outer edges.
- Gates: `cargo clippy --all-targets` warning set identical to `dev`; `./run.sh --test` exit 0 with
  the debug library suite at 1,851 passed, 0 failed, 179 ignored, no Godot script skipped.
- Road-edit preview: the connector build does not run in the preview path (lanes rebuild on
  commit). `benchmark_road_preview_stages` (`RAYON_NUM_THREADS=4`, cores `0,2,..,14`, `dev`, fix,
  fix, `dev` in one session): identical vertex counts; total p50 `sloped_t` 2.920, 2.770 ms (`dev`)
  against 2.973, 2.891 ms, `sloped_branch` 2.787, 2.520 against 2.556, 2.603 ms.
- `PERF-06` (`tools/perf_contract.py --agents`, 8 workers, one run each in the same session; its
  fixture has no junction): identical work counts; network 10k 0.271 against 0.245 ms, 30k 0.861
  against 0.631 ms; egress 0.729 / 1.436 against 0.567 / 1.474 ms; ingress 0.307 / 0.272 against
  0.313 / 0.290 ms; single runs at a load average near 2 (another round shared the machine), so
  within noise and not a speed claim.
- 100k lane audit (`traffic07_lane_entry_audit`, day 12 at 32×, 8 workers, each engine its own
  city): `dev` reproduces the `TRAFFIC-11` record exactly (0 lanes over capacity and 0 stacked cars
  at every sample; 2,579 and 3,385 vehicles held at nodes, 96 and 234 vehicles at attach points at
  00:00 and 24:00). The fix: 0 lanes over capacity and 0 stacked cars at every sample; 2,682 and
  3,664 held at nodes, 140 and 192 at attach points (a different city, so the holds are not
  attributed). This city was prepared with the fix's first build, whose lane ends took the layout's
  `f64` mouth station instead of the edge's `f32` clip (at most a micrometre apart).
- Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements`, `RAYON_NUM_THREADS=6`, both engines in one session): both PASS,
  determinism identical (repeat, 1 and 3 threads, reload). Both raise the same 13 notes of the stale
  baseline; the fix moves them by at most 2.6% apart from tiny rates (`dev` against fix): 10k
  baseline out-of-stock 0.1944 / 0.1943; 10k fiscal stress deliveries 8.89 / 8.69, employment
  0.07446 / 0.07450, final employment 0.00602 / 0.00592, net income 27.31 / 27.28, out-of-stock
  0.8062 / 0.8080; 30k baseline bankruptcies 1.610 / 1.652, deliveries 128.5 / 129.4, employment
  0.6004 / 0.6021, final employment 0.5418 / 0.5436, net income 8.53 / 8.58; 30k fiscal stress
  employment 0.07682 / 0.07686, final employment 0.00624 / 0.00684. The 30k arrivals workload
  moves by at most 3% on its main metrics (out-of-stock 0.218 / 0.225).

Allowed turns are derived from `Node::lane_connections`:

- if the node has no user vehicle connections, all non-U-turn outbound lanes are open
- if the node has any user vehicle connection, the node is in whitelist mode and unspecified turns
  are blocked
- terminal nodes may connect back to the same edge

Strict degree-two corridor splits may skip materializing a vehicle connector and directly link the
aligned incoming physical lane to the outgoing physical lane. This direct pass-through is allowed
only when the node has exactly two incident vehicle arms, no user-authored vehicle lane connections,
their XZ tangents are near-collinear, and the lane mouths are coincident or overlap (on the inside of
a slight bend the outgoing lane starts behind the incoming lane's end, so any connector would run
backwards).

The pass-through degree here counts only arms with vehicle lanes (`TRAFFIC-03`), so a footpath
joining a road leaves it a pass-through for cars. The U-turn rule is the shared
`open_node_allows_u_turn` (`NET-07`), which counts every arm: a road ending in a footpath is not a
dead end, and cars do not turn around there. Counting the footpath once turned such a node into a three-arm junction: the road's
coincident lane ends got a junction-span Bézier connector that ran about 3 m backwards, so cars
turned around for a moment. Saves holding such a connector still load: a saved connector whose
incoming lane now links straight to its outgoing lane is dropped, and agents on it replan. True junctions and all
user-authored vehicle connection nodes must keep explicit connector lanes so junction speed,
spacing, and whitelist semantics still apply.

Cars enter connector lanes through `TRANSIT_INTERSECTION`. Connector lanes are lane-bucketed like
road lanes, so multiple cars may occupy the same connector when they respect separation. Connector
entry also uses a per-tick claim to avoid two cars grabbing the same zero-distance entry slot.
Car exits check outgoing-lane occupancy and acquire the same reservation (`AUDIT-01-A11`). A
blocked car waits at the connector end with its route intact until the outgoing entry clears.

A car that starts at a node (border-origin freight, an arrival carrier, a replan at a node, or a
zero-hop route onto its planned detach lane) enters its first lane under the same two checks
(`TRAFFIC-07`): `lane_entry_slot_clear` on the candidate lanes of the first edge, starting at
the stable choice and wrapping, then `claim_lane_entry` on the first clear one. A car that finds
no lane it may enter holds at its node with its path, access plan, freight shipment, target and
scheduler state untouched, and stays in the raw network counts. The claim classification marks a
car with a valid plan, no lane and no clear first lane `HeldEntry`: it skips the serial handoff
pass and is not processed at all that tick (speed 0), so a queue at a border costs one gap test per
lane of the first edge, per held car and tick (O(node degree + lanes on the first edge) lookups
and one O(log bucket) test per lane, no allocation), done in the parallel classification. The
first tick the slot is clear it classifies `Dynamic` again and takes the entry in the serial
phase. An occupancy snapshot taken at the start of the tick decides both the hold and the entry,
so the result does not depend on the worker count. Pedestrians keep entering unchecked.

Acceptance record (`TRAFFIC-07`, 2026-10-30; commands and results; the artifact folders stay local):

- Regression: `movement_pass::tests::node_origin_cars_wait_for_an_entry_gap` puts 64 trucks at the
  start node of each of 80 one-lane roads (the 5,120 agents cross the parallel classification
  chunk) and ticks 24 times with 1 and 8 workers. On `dev` all 64 trucks of a road stack at
  distance 0; with the hold none do, the waiting trucks keep path, plan, shipment id and speed 0,
  and the lane state is bit-identical for 1 and 8 workers. Debug `cargo test --lib`: 1,847 passed,
  178 ignored, 0 failed.
- Lane audit: `frame_budget::lane_entry_audit::traffic07_lane_entry_audit` (ignored, release)
  loads a prepared snapshot, runs one game day and counts, every `METRUM_AUDIT_EVERY_MINUTES`
  (default 240), the lanes holding more cars than fit (`floor(length / 2.7) + 1`), the cars within
  0.5 m of a lane's start, and the vehicles held at nodes; `METRUM_AUDIT_TRACE=1` lists every car that
  steps onto a lane that was already full in the previous frame, with its previous state. Each engine
  builds its own 100k city (`perf_contract_prepare`, `RAYON_NUM_THREADS=8`, cores `0,2,..,14`), day
  12 at 32×. (Its freight count and its trace's "truck" label tested `freight_shipment_id != 0`,
  but a vehicle with no shipment holds `u64::MAX`, so every vehicle counted as a truck; fixed in
  round R14A. The held and traced counts below are vehicles.) `dev`: 11 lanes over capacity at 00:00 and 14 at 24:00 (one lane held 868 cars where
  197 fit, then 925), 685 and 754 cars within 0.5 m of one lane start, 2,872 and 3,708 cars
  stacked on 14 and 15 lanes, 12 and 5 vehicles held. With the hold: 0 lanes over capacity and 0
  stacked cars at 00:00, 2 lanes over at 24:00 (200 cars where 140 fit; 81 cars stacked), 2,636 and
  3,463 vehicles held at nodes. None of the 81 entries that took a lane above capacity came from node
  entry (on `dev` 255 of the first 400 entries onto a full lane did, within 40 game minutes); all
  81 were vehicles that left the access-egress state onto one lane in one tick (22 and 59 vehicles at
  a single attach point), tracked as `TRAFFIC-11`.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 3 alternating `dev`/fix pairs, cores
  `0,2,..,14`, 8 workers): identical work counts; 10k network tick 0.255, 0.238, 0.255 ms
  (`dev`) against 0.239, 0.240, 0.235 ms; 30k network 0.718, 0.533, 0.468 against 0.663, 0.462,
  0.545 ms. Same fix-built 100k city, first two game hours at 1×, 3 alternating pairs: mean agent
  tick 5.065–5.091 ms with `HeldEntry` against 5.300–5.315 ms with held cars left on the serial
  pass (the earlier prototype's approach), 0.22 ms less. Each engine's own city at 1×: 4.925 and
  4.948 ms (`dev`, 12 held vehicles) against 5.044 and 5.026 ms (fix, 2,636 held); the cities
  differ, so that pair does not isolate the hold.
- `./run.sh --test`: Rust suite plus the headless and rendered Godot scripts.
- Envelope (`tools/sim02_gate.py envelope determinism --bursts departures,settlements`,
  `RAYON_NUM_THREADS=6`, fresh cities): PASS with no failures; determinism identical (repeat, 1
  and 3 threads, reload). The committed baseline is stale on `dev`: the unmodified `dev` tip raises
  13 notes of its own (10k baseline out-of-stock 0.173 against a band of 0.055–0.114; 30k baseline
  employment 0.600, net income 8.45 and deliveries 127 against 0.65–0.69, 12.2–13.6 and
  164–177), so the comparison is fix against `dev` in one session. At 30k the main metrics move
  by 1.5% or less in the baseline workload apart from out-of-stock (+3.8%), by at most 5.1% in
  fiscal stress and by at most 2.2% in the arrivals workload; at 10k the fix raises the
  out-of-stock rate in all three city sizes (0.97×, 1.00×, 1.03×) by +0.021, +0.021, +0.017 in the
  baseline workload (0.173 to 0.194 at 1.00×) and by +0.024, +0.020, +0.018 in fiscal stress (0.786
  to 0.806 at 1.00×, which crosses its band edge at 0.787): the two 10k out-of-stock notes of the
  earlier prototype. Cause: the hold shows the border's real throughput. One single-lane trunk per
  10,000 residents cannot pass the imports; the fix-built 10k city has 1,404 vehicles waiting at its
  border at 00:00 of day 12 and 1,919 at 24:00 (the road holds 1,100–2,600 vehicles), which the
  stacking used to hide. Border throughput stays a fixture question (`TRAFFIC-07` does not change
  it).

A vehicle placed on the road outside the movement pass waits at its attach point the same way
(`TRAFFIC-11`). `AgentVec::hold_for_lane_attach` leaves it off every lane (`TRANSIT_NETWORK`, no
lane, edge and origin node of the attach lane) with the attach lane and distance in
`planned_attach_lane_id` / `planned_attach_lane_d` and `ACCESS_LANE_ATTACH_PENDING` set. The claim
classification marks it `HeldEntry` while `lane_attach_slot_clear` fails at that distance (one
O(log bucket) test per held car and tick, no allocation, off the serial pass) and `Dynamic` once
the slot is clear; the serial phase then claims the lane with `claim_lane_entry` and attaches at
the driveway speed, exactly as egress does, so at most one car takes a lane per tick. Three
placements use it: a building removal's evacuation (`start_building_origin_plan_on_network`), a
road edit's reattachment when a rebuilt lane's spot is already taken by a car reattached earlier
in the same pass (every car on the edit's lanes is detached, so those cars are the lane's whole
occupancy; O(reattached × log) per edit), and a car whose ingress plan turns invalid on its
driveway (it returns to its detach point). Walkers keep entering at once. Trip-plan clears and
replans keep the hold with its attach point; a hold whose lane no longer exists, or no longer
belongs to the car's edge, is dropped and the car repairs from the lane's origin node.

Diagnosis (`TRAFFIC-11`, 2026-10-30). On the `TRAFFIC-07` branch (`3532b612`) the traced 100k
audit (`METRUM_AUDIT_TRACE=1`, which now also records the buildings removed in the frame and each
car's previous building) shows both bulk entries in a frame in which the economy removed buildings:
at game minute 16,024, 14 removals, 22 vehicles step onto lane 13,362 (edge 2,227, 380 m, 141 cars
where 140 fit) at one distance, 296.98 m; at 16,504, 13 removals, 59 vehicles onto lane 13,357 (edge
2,226) at 343.02 m. Every vehicle of a group was in `TRANSIT_ACCESS_EGRESS` at the same building
(57,760 and 57,711) the frame before, has its home as its new target and the driveway speed 3 m/s.
That is the evacuation of a removed building: `remove_evacuated_building`
(`buildings/allocator/lifecycle.rs:870`) calls `evacuate_referencing`
(`agents/building_refs.rs:415`), which plans each occupant's trip home and calls
`start_building_origin_plan_on_network` (`building_refs.rs:481`, `:509`, defined at `:601`). Its
`enter_lane` (`building_refs.rs:644`) put every occupant on the building's attach lane at the
attach distance without `lane_attach_slot_clear` or `claim_lane_entry`. The vehicles were the egress
queue waiting at that full attach slot, so a business failing beside a jammed road dumps its whole
queue onto one point. The same unchecked placement existed in the road-edit reattachment
(`agents/remap.rs:331`) and the invalid-ingress rollback (`movement/access_state/ingress.rs:168`);
the audit day has no road edit and showed no ingress rollback.

Acceptance record (`TRAFFIC-11`, 2026-10-30; the artifact folders stay local):

- Regression: `building_refs::tests::evacuated_egress_queue_waits_for_attach_gaps` puts 64 egress
  trucks at an origin building on each of 80 one-way 1 km roads (5,120 agents, across the parallel
  classification chunk), evacuates every origin and ticks 40 times with 1 and 8 workers. With
  `TRAFFIC-07` alone all 64 trucks of a road stand on the attach point after the evacuation
  ("cars stacked at 50 and 50 (64 on it)"); with the hold none does, the held trucks keep target,
  shipment id and speed 0, every lane keeps the static gap at every tick, and 1 and 8 workers give
  bit-identical lane state. `remap::tests::reattach_holds_a_car_whose_spot_is_taken` covers the
  road-edit hold.
- Save and load: `save::agents::lanes::tests::loaded_cars_held_at_an_attach_point_keep_waiting`.
  Load validation used to clear any access plan with a flag bit in `0xE0` and the plan fields of
  plan-less network cars; the first 100k prepare with the hold failed its exact save/load
  continuation check (`employed` 47,618 against 47,619 one hour after the reload) until load kept
  the hold like the running game does.
- Lane audit, same command as `TRAFFIC-07` (`perf_contract_prepare` then
  `traffic07_lane_entry_audit`, `RAYON_NUM_THREADS=8`, cores `0,2,..,14`, day 12 at 32×, the
  fix-built city): 0 lanes over capacity and 0 cars stacked at a lane start at 00:00, 06:00,
  12:00, 18:00 and 24:00; 2,579 vehicles held at nodes at 00:00 (2,349 of them carry freight,
  recounted in round R14A) and 3,385 at 24:00; 96 vehicles
  held at an attach point at 00:00 and 234 at 24:00. The trace lists 51 entries onto a lane that
  was full in the previous frame, all onto lanes of at most 100 m (connectors), at most 2 per lane
  and frame, none leaving a lane over capacity. On the `TRAFFIC-07` branch's own city the same
  binary ends the day with 0 lanes over capacity and 0 stacked cars (`TRAFFIC-07` alone: 2 lanes,
  81 stacked) and 136 vehicles held at attach points. The committed tree gives the same counts on
  the fix-built city; its held vehicles wait on 20 lanes at 00:00 and 27 at 24:00, only one of
  which holds as many cars as fit, so they mostly wait for a gap at the attach point itself (a
  queue standing through it) rather than for lane capacity. Whether any of them waits
  indefinitely is not established.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 5 alternating pairs, cores
  `0,2,..,14`, 8 workers; control: release test binary of `3532b612` built in the same session,
  SHA-256 `b17c717a…`; fix `506daff0…`): identical work counts in all ten runs. The timings are
  not comparable this session: another round's runs shared the machine (load average 4-7) and the
  samples scatter 2-5× above the quiet `TRAFFIC-07` session in both binaries (10k network
  0.33-1.22 ms control, 0.40-1.12 ms fix; medians 0.62 against 0.55 ms at 10k and 1.06 against
  0.97 ms at 30k network, 0.67 against 0.80 and 1.39 against 1.92 ms egress, 0.30 against 0.48
  and 0.49 against 0.38 ms ingress). The contract's fixture holds no car at an attach point; the
  added per-tick work is one lane test per network car in `handle_network_movement` and a flag
  test per lane-less network car in the classification. Same `TRAFFIC-07` 100k city, first two
  game hours at 1×, 2 alternating pairs: mean agent tick 5.038 and 5.054 ms (control) against
  5.078 and 5.097 ms (fix), +0.04 ms (0.8%) in both pairs. The two runs diverge at the window's
  first evacuation, so the pair mixes the check's cost with different traffic.
- Envelope (`tools/sim02_gate.py envelope determinism --bursts departures,settlements`,
  `RAYON_NUM_THREADS=6`, fresh cities): PASS, determinism identical (repeat, 1 and 3 threads,
  reload). The same 13 notes as `TRAFFIC-07`'s run; the 10k and 30k fiscal-stress values are
  unchanged, and 30k baseline moves by at most 1.1% (bankruptcies 1.593 to 1.610 per 1,000
  buildings and day, deliveries 129.15 to 128.52, employment 0.6013 to 0.6004, net income 8.57
  to 8.53).
- `./run.sh --test` (headless, isolated profile): exit code 0; debug library suite 1,850 passed,
  0 failed, 178 ignored; no Godot script skipped and no script error.

### Detach rule (`TRAFFIC-15`, `TRAFFIC-13`)

A car leaves the network for its destination's ingress in `try_network_detach`
(`tick/movement/network/detach.rs`), under one rule:

- It is on its planned detach lane, with a valid access plan.
- Its movement step has reached the planned detach distance. A step that ends inside the lane
  reaches the car's new `lane_d`. A step that reaches the lane's end passes every point left on
  the lane, so it reaches `length + ACCESS_LANE_D_TOLERANCE_M` (1e-4 m), the same bound the
  planner's access legality (`access/legality.rs`) accepts for `planned_detach_lane_d`. Every
  legal detach point is therefore reachable, the lane's end included, and the end-of-lane try
  runs before `handle_lane_end`.
- The route is on its last visit to the lane: no node of `current_path` is left beyond the lane's
  terminal node (`path_idx` names the next node, the lane's terminal while the car drives toward
  it, the same reading as `planned_next_connector`). A route that leaves its detach lane and comes
  back to a door behind the car does not detach on the first visit.
- A car claims the detach slot (`claim_lane_entry` on the detach lane). While another car holds
  it, the car waits at `min(planned_detach_lane_d, length)` with speed 0.

The speed pass does not brake for a detach point, mid-lane or at the lane's end: a car detaches in
the step that reaches the point, at the speed it has. Road-lane junction braking does not apply to
a car on its last visit to its detach lane either, since `planned_next_connector` finds no next
connector for it. Cost: O(1) per detach try (one terminal-node lookup), no allocation; the
end-of-lane try runs once per road-lane end event.

Before this rule a step had to end strictly inside the lane, so a door that projects onto the
lane's end (a corner store at `frontage_t` 0 seen from the opposite lane), or a step that crossed
the detach point and the lane's end together, was never reached: the car left the lane with a
finished route, replanned at the end node and drove round the block for ever. And a route that
came back to a door behind the car detached on its first visit and jumped back to the door.

Acceptance record (`TRAFFIC-15`, `TRAFFIC-13`, round R14B, 2026-10-31; commands and results, the
artifact folders stay local):

- Regressions (`agents::tests::detach`, debug test profile): `corner_store_door_at_the_lane_end_is_reached`
  drives 8 shoppers on each of 64 streets toward a corner store at `frontage_t` 0 of a 30 m
  street, whose door is the end of the lane they arrive on (`planned_detach_lane_d` = length).
  On `dev` the first shopper leaves that lane without detaching ("left its door's lane without
  detaching"); with the rule all 512 detach from that lane, and 1 and 8 workers give
  bit-identical outcomes. `looping_route_detaches_only_on_its_last_visit` attaches at 950 m of
  a 1 km lane with its door at 50 m of the same lane, route `[1, 0]`: on `dev` it detaches on
  the first tick; with the rule it drives to node 1, back to node 0 and detaches on the return.
- 100k built from scratch to day 12 (`lane_entry_audit::rings::traffic14_ring_formation`,
  release test binaries, `RAYON_NUM_THREADS=8`, cores `0,2,..,14`, one run at a time; wait-for
  cycles checked every 30 game minutes from day 1 08:00). Control: `dev` code with this round's
  diagnostics. The `TRAFFIC-15`/`TRAFFIC-13` commit alone:
  - `dev`: a 334-car ring from day 9 21:00 (absent at the day 10 07:30 check, back at 08:30,
    present at day 12 00:00). At day 12, 2,375 vehicles on the road plan a detach at their
    lane's end, which `dev` cannot reach (top targets: buildings 57645 578, 57646 555, 57644
    547, 57650 284, 57654 155); 68,955 vehicles on the road, 2,822 held at nodes, 140 at attach
    points.
  - With the rule: **0 cycles** at every check to day 12 00:00. At day 12, 491 vehicles plan a
    lane-end detach, all reachable (0 plan one beyond the end); 66,759 vehicles on the road,
    2,027 held at nodes, 60 at attach points.
- Looping routes (`TRAFFIC-13`), vehicles on the road whose remaining route visits the detach
  lane before its last visit, counted at each check: 0-2 during the `dev` build and 2-4 per
  6-hour checkpoint of the `dev` 100k day; 13 at day 12 with the rule (they now keep their
  route instead of detaching early).
- Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements`, `RAYON_NUM_THREADS=6`, same session as the `dev` control): PASS,
  determinism identical (repeat, 1 and 3 threads, reload). Doors are reached, so shopping and
  trips shift. 10k baseline out-of-stock 0.194 to 0.111, back inside its band (the `TRAFFIC-07`
  note is gone). 30k baseline: employment 0.602 to 0.623, final employment 0.544 to 0.583, net
  income 8.58 to 9.89, bankruptcies 1.65 to 1.26, deliveries 129.4 to 135.7, out-of-stock
  0.236 to 0.207, which is below its band: a new note in the good direction. 30k fiscal stress
  deliveries 10.97 to 10.90, a new note 0.1% under its band edge (10.91). Arrivals-30k:
  employment +2.3%, net income +7.4%, bankruptcies -10.2%, out-of-stock +7.4% (0.225 to
  0.242). 10k fiscal stress moves by at most 0.5% on its main metrics apart from final
  employment (0.0059 to 0.0052). 14 notes against `dev`'s 13, all of the stale baseline.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 8 workers, cores `0,2,..,14`,
  exclusive timing lock, load average 1.7 at the start; control `dev` then the branch with both
  this rule and `TRAFFIC-16`, then the branch, then control): identical work counts in all four
  runs. Single samples, ms, control / branch / branch / control: network 10k 0.666 / 0.839 /
  0.280 / 1.226, 30k 0.840 / 0.638 / 0.620 / 2.046; egress 10k 0.540 / 0.475 / 0.485 / 0.795,
  30k 1.252 / 1.281 / 1.699 / 1.312; ingress 10k 0.705 / 0.295 / 0.521 / 0.194, 30k 0.340 /
  0.273 / 0.318 / 0.493. They scatter within each binary as widely as between them: no speed
  claim either way. The contract's fixture has no connector and no lane-end detach; the added
  work is one O(1) detach try per road-lane end event.

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

A connector head (no car ahead on its connector) brakes for its exit lane's entry slot the same
way (`TRAFFIC-16`): while `lane_entry_slot_clear` fails for the connector's exit lane (the lane
`exit_connector_lane` enters), its target speed is capped at `braking_speed_for_distance(0,
distance to the connector's end)`, so it stands at the end at speed 0 until the exit clears, and
starts from 0. One O(log bucket) slot test per connector head and tick, no allocation. Before,
connector cars followed only their own bucket, and the head standing at a full exit kept its
turn speed (the movement step re-clamps to the turn speed but never raises it).

Acceptance record (`TRAFFIC-16`, round R14B, 2026-10-31; the artifact folders stay local):

- Ring fixture (`movement_pass::ring_tests::traffic16_connector_heads_stop_at_a_blocked_exit`):
  the locked four-lane ring's four connector heads read 0; on `dev` all four report their turn
  speed. `traffic14_full_ring_locks` still locks (no ring rule yet) and stays ignored.
- 1-day 100k audit of the `dev`-built snapshot (`traffic07_lane_entry_audit`, day 12 at 32×,
  `RAYON_NUM_THREADS=8`, cores `0,2,..,14`): unmoved cars reporting more than 0.5 m/s at the
  6-hour checkpoints, `dev` against the fix: 2,003 / 10, 2,027 / 23, 2,119 / 36, 2,140 / 29
  (`dev`: 1,992-2,124 connector heads at the connector's end; fix: 10-34 connector heads and up
  to 2 road-lane heads). The two connector heads followed frame by frame (`METRUM_AUDIT_FOLLOW`)
  found their exit slot clear at the tick's start, accelerated (3.2 m/s) and lost the entry to
  another car in that frame (`TRAFFIC-18`); the next frame they read 0. The road-lane heads were
  not followed. Both engines: 0 lanes over capacity and 0 stacked
  cars at every checkpoint; held at nodes 2,682 at 00:00 (2,210 of them freight) and 3,742
  (fix) / 3,664 (`dev`) at 24:00. The snapshot already holds `dev`'s locked ring, which neither
  fix releases.
- **Traffic cost at saturation.** A head that stands at a blocked exit now starts from 0 when
  the slot clears; it used to leave at its turn speed from a standstill. The exit lane's entry
  slot therefore clears later and a saturated junction discharges more slowly. The 100k city
  built from scratch with both fixes forms a 1,690-car wait-for cycle at day 10 10:30: two
  columns of 35 full 30 m cross-street lanes, two full 380 m row lanes and their connectors.
  It is absent at two later checks, present without a break from day 10 19:30 and at all five
  checkpoints of that city's next day (47,630-50,086 cars unmoved per 6 hours, against
  33,400-36,865 in the `dev` city). With `TRAFFIC-15`/`TRAFFIC-13` alone the same build has no
  cycle. Envelope (both fixes, deterministic, PASS, 13 notes) against `TRAFFIC-15` alone: 30k
  baseline deliveries 135.7 to 117.3, employment 0.623 to 0.601, net income 9.89 to 8.27,
  bankruptcies 1.26 to 1.90, out-of-stock 0.207 to 0.265; 30k fiscal-stress deliveries 10.90
  to 10.79; the other workloads move by at most 2.3% on their main metrics. Landing this before
  the junction-box and gridlock rules of `TRAFFIC-14` is the user's decision (round R14B
  handoff).

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

### Vehicle models and lamps (`RENDER-26`)

Cars are the Kenney models (`godot/assets/models/vehicles/civilian/` sedan, sports sedan, SUV,
luxury SUV; `freight/` delivery van) with their colormap atlas variants. Generic Finnish models
were tried on this branch and withdrawn as not good enough to ship; they are kept off-tree. The
Kenney meshes carry no lamp data, so `vehicle_lamps.gd` holds a measured table of front and rear
lamp anchors per model (left lamps mirror X); procedural cars derive theirs from the AABB.

At night (sun below 4 to -2 degrees, as the street lights):

- `vehicle_lamps.gd` adds a camera-facing glow quad per
  lamp (`vehicle_lamp_glow.gdshader`) that keeps at least 1.6 pixels, so traffic shows across the
  map; headlights show from ahead, tail lights from behind;
- the 12 drawn cars nearest the camera within 100 m get a real `SpotLight3D` each
  (`vehicle_headlights.gd`, 60 m range, inverse-square decay, 35 degrees with a soft rim, no
  shadows; see below), chosen in Rust by
  `nearest_car_poses` (O(drawn cars · log 12) with a bounded heap). They light the road, trees,
  walls, walkers and the car ahead by their own colour;
- every car carries a light box (`vehicle_headlight_beam.gdshader`) that rebuilds each covered
  pixel from the depth buffer and adds a small red tail glow behind, and for cars from 70-100 m
  out (where the spot lights fade) the same cone the real headlight casts: Godot's SpotLight3D
  falloff with its 60 m range, inverse-square decay, 35° angle and rim attenuation 3, lit as level
  ground. This light is added blind to surface colour, acceptable only at that distance. Near the camera the box shrinks in the vertex shader
  to the tail region; by day and past 450 m it collapses and draws nothing.

Measured 2026-10-09 on the M2 Pro, `no-headlights` save (7 cars), release build, 120 frames,
old (Kenney models, no lamps) and new alternated three times: by day the new models cost under
0.1 ms. At 23:00, with real lights already on for the street lights: saved view +1.15 ms,
eye level +0.8 ms, 32 m overhead +0.4 ms with a 45 m spot range. Without the spot lights the
lamps cost 0.1-0.35 ms; the spot lights' cost follows their reach, not their count (6 or 12 the
same), and a 28 m range saved 0.45 ms in the saved view (about +0.85 ms over old in total), so
the range is 30 m. The 30 m setting itself was checked visually, not re-timed.

Near and far headlights did not match: up close the lighting shrank and dimmed. The real lights
were about 100 times weaker than the far light box on ordinary surfaces (their light is scaled by
albedo, the box's is added blind to it), fell off linearly instead of with the square of distance,
and reached 30 m against the box's 45. `tools/bake_headlight_projector.py` calibrates the
two against each other and bakes the box's own low-beam pattern into a spot-light projector
(`godot/assets/textures/vehicles/headlight_projector.png`). Godot 4.7 applies a projector only to
a shadowed light (`SpotLight3D` warns so; `storage_rd/light_storage.cpp` builds the projector
matrix only for shadowed lights), and four shadowed lights cost 1.9-2.9 ms a frame on the M2 Pro,
two 1.3-2.1 ms; hard shadows or an empty caster mask saved little, so the cost is per-pixel
shading. They are off (`SHADOWED := 0`), and every light is a plain cone: 60 m range,
inverse-square decay, 35 degrees with a steep rim falloff, energy 1200, matching the box's hot
spot near 20 m and overlighting the first few metres. Against the old 30 m lights (same save,
23:00, 120 frames, alternated twice): close view -0.1 ms, saved view +0.4 ms, overhead +0.15 ms.
Shots: `screenshots/2026-10-09-roads/headlights-4-shadowed.png` and
`headlights-shadowless-cone.png`.

Far and near headlights still did not match (2026-10-14): from 70 m out a car lit a wide white
block with straight edges, and close up a narrow soft cone. The box drew its own low-beam pattern,
still strong at the box's front face (45 m) and side faces (16 m), and lit each pixel with a
normal rebuilt from depth derivatives, which broke at every kerb and depth step into bright dashes
and straight-edged patches. The box now draws the real light's cone (above), lights everything as
level ground, and reaches 60 m ahead and 42 m (tan 35° × 60 m) to each side, where that cone is
spent; it fades out below the box's 8 m top. Its energy (62) and a ground tint (1.07, 1.0, 0.85)
were fitted to the mean light the real headlights add to the same views (`light-road` save at
01:00, cars 60 and 110 m away, all-real against all-box: 86-110 % per channel before the tint).
Frame cost was not measurable apart from the street-light change it shipped with (`ROAD-59`).

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
  (`TRAFFIC-06` plans junction priority, yield and stop control, and roundabout entry)
- lane changes are centerline S-curves, not full swept-body collision geometry
- parking, driveways, curb queues, and building entrance reservations are not modeled
- connector lanes are generated from lane necks; if road geometry changes materially, lane rebuild
  must keep connector lanes and `next_lanes` in sync
- a border queue of held cars is not shortened by the entry hold: the borders' throughput (one
  single-lane trunk per 10,000 residents in the perf fixture) is a separate fixture question
- walkers keep car spacing (2.7 m) in lane buckets, connector entry claims and overlap correction
