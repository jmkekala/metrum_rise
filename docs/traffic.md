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
- `rust/src/nodes/sim/render/agents.rs`
- `godot/scripts/renderers/agents.gd`

## Current Scope

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

Traffic state is stored in the agent SoA. The traffic-owned transient fields are:

- `current_lane_id`: the lane currently governing longitudinal movement
- `lane_distance`: metres travelled along `current_lane_id`
- `speed`: current car speed in m/s
- `lane_change_from_lane_id`: source lane for an active S-curve lane change, or `u32::MAX`
- `lane_change_start_d`: lane distance where the active lane change started
- `lane_change_length_m`: longitudinal length of the active lane-change S-curve
- `overtake_blocked_time_s`: time spent held below free-flow speed by lane traffic
- `overtake_cooldown_s`: cooldown before another discretionary overtake / return lane change

`lane_change_*` and `overtake_*` are transient runtime state. Save/load and benchmark setup reset
them to inactive defaults rather than preserving an in-progress visual maneuver.

## Lane Buckets

Each tick builds per-lane buckets containing `(lane_distance, agent_index)` sorted by distance.
These buckets are the hot-path structure for:

- nearest-car-ahead gap lookup
- connector entry slot checks
- lane-change target gap checks
- post-movement overlap correction
- low-frequency frontage delay measurement

The bucket fill is incremental over dirty lanes and reuses scratch buffers. This is required for
the 1M-agent scale target; traffic logic must not allocate per agent in the tick hot path.

Active lane-changing cars are inserted into their target lane bucket and, before movement, also
into their source lane bucket while the S-curve is still active. This lets cars in both lanes react
to the crossing car during the IDM speed pass. Post-movement overlap correction clamps the current
authoritative lane.

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

Allowed turns are derived from `Node::lane_connections`:

- if the node has no user vehicle connections, all non-U-turn outbound lanes are open
- if the node has any user vehicle connection, the node is in whitelist mode and unspecified turns
  are blocked
- terminal nodes may connect back to the same edge

Cars enter connector lanes through `TRANSIT_INTERSECTION`. Connector lanes are lane-bucketed like
road lanes, so multiple cars may occupy the same connector when they respect separation. Connector
entry also uses a per-tick claim to avoid two cars grabbing the same zero-distance entry slot.

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

Rust produces car transforms from lane poses in `nodes/sim/render/agents.rs`.

Lane pose sampling in `nodes/sim/render/lane_pose.rs`:

- samples position by distance along lane geometry
- samples tangent using a `2 m` look-behind / look-ahead window
- falls back to the local segment tangent when the look window degenerates
- samples active lane-change S-curves by smoothstep blending source-lane and target-lane positions
- derives S-curve tangent from blended lane tangents plus the lateral blend derivative

Godot applies render-side interpolation in `godot/scripts/renderers/agents.gd`:

- cars have stable render IDs from Rust
- pedestrian and vehicle MultiMeshes use the shared dynamic shadow-caster policy
- origin interpolation smooths per-tick position updates
- basis interpolation smooths rotation
- large jumps above `80 m` snap instead of interpolating across the map
- transform uploads happen every render frame so fast sim multipliers do not quantize cars to the
  simulation tick rate

Current render interpolation constants:

- `CAR_INTERPOLATION_RATE = 24.0`
- `CAR_ROTATION_INTERPOLATION_RATE = 18.0`
- `CAR_INTERPOLATION_SNAP_DISTANCE_M = 80.0`

## Debugging

Run:

```bash
./run.sh --debug traffic
```

or:

```bash
./run.sh --debug-traffic
```

Traffic debug logging goes to stderr through `traffic_log!`. When redirecting output, capture
stderr as well if you want `traffic.log`.

Important log markers:

- `[JUNCTION_ENTER]`
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

When traffic debug is enabled, the agent render debug path also exposes richer lane / junction
labels and path/connector visualization for visual diagnosis.

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

Current known bounded scans:

- adjacent lane lookup scans the current edge's lane list, whose count is tiny for supported road
  presets
- connector selection scans `next_lanes` from the current lane, bounded by junction fan-out
- lane bucket gap checks use sorted vectors and `partition_point`

## Known Limits

- normal road bends do not yet use curvature speed limits
- full IDM approach-speed interaction is not implemented because lead-vehicle speed is not tracked
- overtaking has no driver personality, urgency, emergency behavior, or multi-car prediction
- there are no traffic lights, stop signs, yield priorities, or priority-road rules yet
- lane changes are centerline S-curves, not full swept-body collision geometry
- parking, driveways, curb queues, and building entrance reservations are not modeled
- connector lanes are generated from lane necks; if road geometry changes materially, lane rebuild
  must keep connector lanes and `next_lanes` in sync

