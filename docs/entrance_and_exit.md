# Metrum Rise - Building Entrance and Exit System

This document describes the current building entrance/exit behavior in the Rust
simulation, why it has become brittle, and the recommended replacement architecture.

The target audience is future work on `simulation/economy/agents/` and
`simulation/buildings/allocator/`. This is a standalone reference linked from
`docs/project.md`; `project.md` tracks implementation status, while this file owns the
detailed entrance/exit design and implementation spec.

This document is now intended to be a deterministic implementation spec for the
entrance/exit rewrite. If the implementation changes, update this file and
`docs/project.md` together so they stay aligned.

## Current System

### Core data model

Buildings do not have explicit doors, lobbies, or interior navigation state.

Each building is attached to one road edge and stores:

- `edge_idx`
- `side`
- `cell_x`
- `cell_y`
- `width_cells`
- `depth_cells`
- `frontage_t`
- `center_x`, `center_y`
- `facing_dir`
- `side_offset`

The authoritative placement meaning of those fields belongs to [`building_allocator.md`](building_allocator.md).
This document only owns how entrance and trip-planning logic consume them.

`frontage_t` is currently overloaded. It acts as:

- the approximate frontage position along the road edge
- the basis for choosing which endpoint node is the building's "depart node"
- the lane insertion position for mid-edge entry
- the arrival target for the frontage-edge snap check

`building_depart_node(building, graph)` chooses one of the two edge endpoints only:

- `edge.start_node` when `frontage_t < 0.5`
- `edge.end_node` when `frontage_t >= 0.5`

This means the path planner discards the true mid-edge location and compresses the
building's network attachment to one bit of information.

### Agent state model

Agents store three building references:

- `current_building`: the building the agent is currently inside
- `target_building`: the building the agent is currently travelling toward
- `planned_target_building`: the next destination chosen while the agent is idle

An agent is considered "inside a building" when:

- `transit == TRANSIT_IDLE`
- `current_building != usize::MAX`
- `is_visible == false` in the common settled case

The household planner only schedules departures for agents already in `TRANSIT_IDLE`.

### Current trip lifecycle

The implemented FSM is:

- `IDLE -> DEPARTING -> ON_ROAD -> ARRIVING -> IDLE`
- `IMMIGRATING` is a special entry path that eventually joins the same arrival logic
- `INTERSECTION` is the junction-lane phase for road traffic

The actual building access flow is:

1. While idle, the economy sets `planned_target_building` and `planned_activity`.
2. The agent tick resolves `origin_node` and `target_node` from `building_depart_node(...)`.
3. A road-network path is built between those nodes using flow fields or CCH.
4. The agent enters `TRANSIT_DEPARTING`.
5. During `TRANSIT_DEPARTING`, the agent walks in a straight line from its current world position, usually the building centre, to a computed curb point on the frontage edge.
6. When the curb point is reached, the agent is inserted directly into a frontage lane partway along the edge.
7. The agent travels normally through the lane/junction/pathing FSM.
8. On the destination frontage edge, the system watches for the agent to come within about 4 meters of the building's `frontage_t`.
9. The agent snaps to the curb point and enters `TRANSIT_ARRIVING`.
10. During `TRANSIT_ARRIVING`, the agent walks in a straight line from the curb point to the building center.
11. On arrival, `current_building` becomes `target_building`, the agent becomes invisible, and the transit state returns to `IDLE`.

### Sidewalk and lane rules

Pedestrian entry and exit are restricted to the building's frontage side when a
matching sidewalk exists. The implementation currently does this by selecting a
sidewalk lane whose sign matches `building.side`.

Cars do not use a modeled driveway or parking system. They also use the same
curb-point handoff, then insert directly into a vehicle lane on the frontage edge.

If the expected frontage-side lane does not exist, the code falls back to a much
coarser node-based entry or arrival path.

### Special cases in the current code

- If the origin and destination resolve to the same depart node, the trip skips the network phase and goes straight to `ARRIVING`.
- If an agent finishes its network path at the correct frontage node but the building sits mid-edge, the code re-inserts the agent onto the frontage edge so the midpoint arrival check can fire.
- Immigrants spawn at a border node, target their future home's depart node, and then use the same arrival logic as ordinary trips.
- Failed local access or failed pathfinding usually ends by forcing the agent to `IDLE` and often `is_visible = false`.

## Why The Current System Is Brittle

### 1. `frontage_t` is doing too many jobs

One scalar currently stands in for:

- building-footprint attachment
- path-planning attachment
- lane insertion point
- arrival trigger

That makes every bug harder to reason about because placement, routing, animation, and save/load all depend on the same approximate value.

### 2. The node attachment is too coarse

`frontage_t < 0.5` versus `>= 0.5` throws away the actual entrance position. Buildings near the middle of an edge are still planned as if they were attached to one endpoint only.

This is the root of many awkward cases:

- same-edge trips needing special treatment
- agents reaching a frontage node and then being shoved back onto the edge
- path plans that are "correct" at the node level but wrong at the building level

### 3. Building access is reconstructed ad hoc every tick

The local building-to-road handoff is not part of the trip plan. Instead, departure and arrival rebuild the logic from building geometry each tick and try to stitch that onto a node path.

The road network trip and the local access trip are not represented as one coherent plan.

### 4. The system uses fuzzy heuristics where it should use exact targets

The arrival handoff currently depends on a frontage-edge proximity check:

- same edge
- matching side constraints
- within roughly 4 meters of `frontage_t`

That is a heuristic, not an exact contract.

### 5. The visible movement is tied to the building center, not an entrance

Both `DEPARTING` and `ARRIVING` use straight-line movement to or from the building center. That means:

- there is no explicit door position
- no distinction between front wall and center of footprint
- no way to queue or reserve entrances later
- no clean upgrade path to better access geometry

### 6. Failure handling masks bugs instead of expressing state

The current fallback behavior frequently hides the agent when planning or access reconstruction fails. That keeps the world from filling with obvious artifacts, but it also means the system has no durable "waiting at building" or "replan from access point" state.

### 7. The building occupancy counters are not entrance state

`occupancy` currently tracks household count (for residential) and assignment bookkeeping. It is not:

- who is physically inside this building right now
- who is waiting at the entrance
- whether the entrance is blocked

So the system has no place to store real building access flow.

### 8. The current model is awkward to extend

Future features like these all become harder under the current design:

- explicit doors
- driveway-only access
- parking or curb pickup
- side-specific pedestrian entrances
- loading bays and freight doors
- different access rules by asset type

## Recommended Replacement

The clean fix is to make building entrances first-class, edge-local access objects and to stop routing buildings through `building_depart_node()` as the primary attachment abstraction.

### Design goals

- Do not split road edges for buildings.
- Do not reintroduce virtual frontage nodes into `RegionGraph`.
- Keep CCH and flow-field pathing on the road graph.
- Represent local building access explicitly and cache it.
- Allow targeted SoA growth where it removes per-tick reconstruction and ambiguous state.
- Remove heuristic arrival checks and replace them with exact access targets.
- Keep the per-agent hot path allocation-free.
- Keep the movement logic compatible with the SoA agent layout.

### Proposed data model

#### 1. Make the road attachment authoritative from building footprint coordinates, not `frontage_t`

Keep these as authoritative building attachment fields:

- `edge_idx`
- `side`
- `cell_x`
- `cell_y`
- `width_cells`
- `depth_cells`

Make `frontage_t` derived only, or remove it entirely from authoritative save logic.

The road-side attachment should be recomputed from building placement plus the asset-authored primary entrance anchor the same way center/facing are already recomputed on load. This removes one stale source of truth.

#### 2. Add a derived `BuildingEntrance` cache

Each building should have one derived entrance/access record with exact edge-local geometry and legal access lanes. The building asset's primary entrance anchor is authoritative for the building-side entry point; the runtime derives only the road-side attachment from live edge and lane topology.

Required fields:

- `edge_idx`
- `side`
- `vehicle_frontage_access`: `SameSideOnly` or `BothSides`
- `entrance_s_m`: exact distance in meters along the parent edge centerline
- `door_pos`: world-space canonical building entry point on the building frontage
- `curb_pos`: world-space canonical pedestrian network/building-frontage handoff point
- `foot_lane_fwd`
- `foot_lane_bkw`
- `car_lane_fwd`
- `car_lane_bkw`
- `flags`: derived validity bitfield for foot/car access and frontage-lane availability

This cache should be rebuilt whenever:

- building placement changes
- building transforms are recomputed after load
- lane topology for the attached edge changes

That rebuild is off the hot path and can remain O(B) over buildings.

Do not cache endpoint-distance helper fields like `*_d_to_start` or `*_d_to_end` in the current `BuildingEntrance` contract.

In this spec, endpoint legality and frontage-travel cost are derived from:

- the chosen attach or detach lane id
- the chosen attach or detach lane distance
- the lane's directed origin and terminal nodes
- the lane length and owning-edge travel speed

If a future implementation wants to cache endpoint-distance helpers as an optimization, that can be added later, but it must come with an exact derivation rule and must not silently reintroduce the older long-to-node access model.

##### Vehicle frontage access policy

The parent road edge must expose a deterministic vehicle frontage-access policy. The entrance cache may copy that value for fast legality checks, but the edge remains authoritative.

Use this exact enum:

- `SameSideOnly`
- `BothSides`

Interpretation:

- `SameSideOnly`: direct car access may use only the curb-most vehicle lane on the building's frontage side.
- `BothSides`: direct car access may use the curb-most forward vehicle lane and the curb-most backward vehicle lane if those lane groups exist.

This policy applies symmetrically to both halves of a trip:

- origin-side lane selection for `ACCESS_EGRESS`
- destination-side lane selection for `ACCESS_INGRESS`

It is a planning and legality rule, not a lane-network maneuver. The chosen attach or detach lane already encodes whether the trip uses same-side or opposite-side frontage access. For `BothSides`, opposite-side car access uses a deterministic short local crossover polyline defined below, not a diagonal one-segment cut across the full road width.

##### Exact `vehicle_frontage_access` ownership and persistence contract

`vehicle_frontage_access` is authoritative edge state. It does not live in agent SoA, building save data, or asset metadata.

Use this exact rule:

1. Add one authoritative enum to the network layer:
   - `VehicleFrontageAccess::SameSideOnly = 0`
   - `VehicleFrontageAccess::BothSides = 1`
2. Store `vehicle_frontage_access: VehicleFrontageAccess` directly on `network::graph::Edge`.
3. `BuildingEntrance.vehicle_frontage_access` is a derived cache copy of `edge.vehicle_frontage_access` only. It must never be saved independently.
4. New edge creation initializes the field from the road tool that created the edge.
   - current road creation defaults are:
     - `TransitType::Road` => `BothSides`
     - `TransitType::Foot` => `SameSideOnly`, though car access remains invalid on those edges regardless
5. Geometry-only edge edits preserve the existing `vehicle_frontage_access` value.
6. Split operations copy the source edge value to every produced child edge.
7. Existing edit-history and state-restoration flows must preserve the exact per-edge value.
8. Automatic merge/replacement of adjacent edge segments is allowed to preserve `vehicle_frontage_access` only when all merged source edges share the same value.
   - if merged source edges disagree, do not silently invent a merged value
   - keep the edit boundary instead
9. Persist the field directly in `network_edges.vehicle_frontage_access`.
    - save as the integer enum value above
    - load by decoding that integer back into `VehicleFrontageAccess`
10. Save migration for older snapshots is exact:
    - if `network_edges.vehicle_frontage_access` is absent, add it with default value `BothSides`
    - this is the compatibility default for pre-frontage-policy saves
11. After load and after any road edit that changes or recreates an edge, rebuild the derived `BuildingEntrance` cache before agent planning resumes.
12. This field belongs to the road/network editor, not the v1 asset editor.
    - the building asset pipeline does not own frontage-crossing legality

##### Deterministic derivation rules

The entrance cache must be rebuilt from the authoritative building attachment fields, asset metadata, and the live lane topology. It must not be saved as authoritative state.

Use this exact derivation:

1. Validate the parent edge.
   - `edge_idx` must exist
   - `edge.deleted == false`
   - `edge.physical_geometry.len() >= 2`
   - `edge.physical_length > 1e-6`
   - if any check fails, the entrance is invalid and all lane IDs are set to the invalid sentinel
2. Validate the building asset metadata.
   - runtime asset lookup for the building's `asset_id` must succeed
   - exactly one `[[anchors]]` entry with `type = "entrance"` and `name = "main"` must exist
   - optional `service` anchors may exist, but they do not participate in this generic entrance/exit system
   - if any check fails, the entrance is invalid and all lane IDs are set to the invalid sentinel
3. Compute the world-space primary building entry point from the asset anchor.
   - use the building's world transform derived from authoritative placement fields
   - `facing_dir` is road-facing: for parcel-owned buildings it is `-parcel.normal`
   - align the asset-local `main` entrance anchor `forward` vector to runtime `facing_dir`; canonical `+Z`-front assets are the common case, but authored non-`+Z` frontage vectors are honored deterministically
   - apply that transform to the `main` entrance anchor's local `position`
   - `door_pos = transformed_anchor_position.xz()`
4. Project `door_pos` onto the parent edge centerline to define the canonical frontage reference:
   - `entrance_s_m = project_point_to_polyline_s(edge.physical_geometry, door_pos)`
   - `entrance_t = entrance_s_m / edge.physical_length`
5. Sample edge geometry:
   - `edge_pos = sample_pos_on_edge(edge_idx, entrance_t)`
   - `tangent = sample_tangent_on_edge(edge_idx, entrance_t)`
   - `normal = Vector2(tangent.y, -tangent.x) * side as f32`

`door_pos` is asset-authored and authoritative. There is no frontage-midpoint fallback in this spec. If a building asset does not provide a valid primary entrance anchor, that asset is invalid for the entrance/exit system and must be fixed in the asset pipeline rather than repaired at runtime.

##### Exact polyline sampling helpers

All edge and lane sampling must use one shared arc-length rule. Do not implement separate "close enough" samplers for edge centers, lane points, and tangents.

Use this exact helper definition:

- `sample_pos_on_polyline(points, total_len, s_m) -> Vector2`
- `sample_tangent_on_polyline(points, total_len, s_m) -> Vector2`
- `project_point_to_polyline_s(points, world_pos) -> f32`

Sampling rules:

1. If `points.len() == 0`, `sample_pos_on_polyline` returns `Vector2::ZERO` and `sample_tangent_on_polyline` returns `Vector2::RIGHT`.
2. If `points.len() == 1` or `total_len <= 1e-6`, position is `points[0].xz()` and tangent is `Vector2::RIGHT`.
3. `target_s = clamp(s_m, 0.0, total_len)`.
4. Segment lengths are measured in 3D using the same metric as `edge.physical_length` and `lane.length`.
5. Traverse segments in index order.
   - `seg_len = distance(points[i], points[i + 1])`
   - segments with `seg_len <= 1e-6` are treated as degenerate and skipped for arc-length accumulation
6. For `sample_pos_on_polyline`, return the first non-degenerate segment where `acc_len + seg_len >= target_s`.
   - `local_t = (target_s - acc_len) / seg_len`
   - return `lerp(points[i].xz(), points[i + 1].xz(), clamp(local_t, 0.0, 1.0))`
7. If no non-degenerate segment contains `target_s` because the polyline ends in degenerate segments, return `points[last].xz()`.
8. For `sample_tangent_on_polyline`, use the same containing segment search as position sampling.
9. If `target_s` lies strictly inside a non-degenerate segment whose XZ projection length is `> 1e-6`, tangent is that segment's normalized XZ direction.
10. If `target_s` lands exactly on a vertex, or the containing segment has projected XZ length `<= 1e-6`, choose tangent by this exact search order:
   - first search forward from that vertex for the first segment with `seg_len > 1e-6` and projected XZ length `> 1e-6`
   - if none exists, search backward from the previous segment toward index `0` for the first segment with `seg_len > 1e-6` and projected XZ length `> 1e-6`
   - if neither search finds one, return `Vector2::RIGHT`
11. `sample_pos_on_edge(edge_idx, t)` is exactly:
   - `sample_pos_on_polyline(edge.physical_geometry, edge.physical_length, clamp(t, 0.0, 1.0) * edge.physical_length)`
12. `sample_tangent_on_edge(edge_idx, t)` is exactly:
   - `sample_tangent_on_polyline(edge.physical_geometry, edge.physical_length, clamp(t, 0.0, 1.0) * edge.physical_length)`
13. `project_point_to_polyline_s(points, world_pos)` is exactly:
   - measure closest-point distance in XZ only
   - for every segment `i`, compute the closest point from `world_pos` to `points[i].xz() -> points[i + 1].xz()`
   - choose the segment with the smallest squared XZ distance
   - if distances tie, choose lower segment index first, then lower local `t`
   - return the cumulative arc length from `points[0]` to that chosen closest point using the same segment-length metric as the owning polyline
14. `sample_pos_on_lane(lane_id, lane_d)` is exactly:
   - `sample_pos_on_polyline(lane.geometry, lane.length, clamp(lane_d, 0.0, lane.length))`

These helpers are authoritative for entrance derivation, exact attach/detach world points, and local-access cost evaluation.

##### Valid frontage lane rules

All lane selection must be deterministic and based only on the live `LaneSystem` plus the building's `side`.

Foot lanes:

- If `edge.allowed_types` does not include `FOOT`, both foot lanes are invalid.
- If `edge.primary_type == TransitType::Foot`, then:
  - `foot_lane_fwd` is the unique lane on `edge_idx` with `lane_type == Foot`, `is_fwd == true`, `lane_idx == 0`
  - `foot_lane_bkw` is the unique lane on `edge_idx` with `lane_type == Foot`, `is_fwd == false`, `lane_idx == 0`
  - this is a dedicated foot corridor, not a road with two separate frontage sidewalks
  - `building.side` still controls which side of the path `door_pos` is offset toward, but it does not select a different foot lane
  - both frontage sides attach to the same centered foot corridor during `ACCESS_EGRESS` and `ACCESS_INGRESS`
- Otherwise, for a standard road frontage:
  - `foot_lane_fwd` is the unique lane on `edge_idx` with `lane_type == Foot`, `is_fwd == true`, `lane_idx == side * 100`
  - `foot_lane_bkw` is the unique lane on `edge_idx` with `lane_type == Foot`, `is_fwd == false`, `lane_idx == side * 100`
- If either required lane is missing on a standard road, foot access is invalid and both foot lanes are set invalid.

Car lanes:

- If `edge.allowed_types` does not include `CAR`, both car lanes are invalid.
- If `edge.primary_type == TransitType::Foot`, both car lanes are invalid.
- First derive the curb-most candidate lane in each traffic direction:
  - `car_lane_fwd` is the vehicle lane on `edge_idx` with `is_fwd == true` and the highest `lane_idx`
  - `car_lane_bkw` is the vehicle lane on `edge_idx` with `is_fwd == false` and the lowest `lane_idx`
- Then apply `vehicle_frontage_access`:
  - `SameSideOnly`:
    - right-side buildings (`side == -1`) may directly access only forward traffic on the frontage edge
    - left-side buildings (`side == 1`) may directly access only backward traffic on the frontage edge
    - for right-side buildings, set `car_lane_bkw` invalid
    - for left-side buildings, set `car_lane_fwd` invalid
  - `BothSides`:
    - keep both `car_lane_fwd` and `car_lane_bkw` valid if they exist
- If a retained candidate lane does not exist, that candidate is invalid.
- Car access is invalid only if both retained car candidates are invalid.

This defines direct car access as the curb-most legal vehicle lane in each direction permitted by the parent edge's frontage-access policy. `SameSideOnly` forbids direct driveway crossing over the opposite-direction lane group. `BothSides` allows both-direction direct frontage access for both ingress and egress.

##### Exact lane-distance mapping

For every valid frontage lane, map `entrance_s_m` to lane distance by geometric projection, not by ratio scaling.

Use this exact rule:

1. Let `P = edge_pos`, the sampled edge-centerline point at `entrance_s_m`.
2. For every segment `i` in `lane.geometry[i] -> lane.geometry[i+1]`, compute the closest point to `P`.
3. Choose the segment with the smallest squared distance.
4. If distances tie, choose:
   - lower segment index first
   - then lower local `t` along that segment
5. `lane_d` is the cumulative distance from `lane.geometry[0]` to that chosen closest point.

This produces deterministic `planned_attach_lane_d` / `planned_detach_lane_d` values even when offset lane geometry is longer than the parent edge centerline.

##### `curb_pos` rule

`curb_pos` is the canonical pedestrian handoff point, not a second guessed world-space offset.

Use this exact rule:

- If foot access is valid, compute `curb_pos` from `foot_lane_fwd` at the projected lane distance from the rule above.
- If `foot_lane_fwd` is invalid but `foot_lane_bkw` is valid, use `foot_lane_bkw` instead.
- If foot access is invalid, set `curb_pos = door_pos`.

No world-space equality test between forward and backward sidewalk projections is allowed here. `curb_pos` is defined only by the ordered lane-choice rule above.

`curb_pos` is pedestrian-only. Car trips do not use `curb_pos` for local access; they use the exact planned lane point derived from `planned_attach_lane_id` / `planned_attach_lane_d` or `planned_detach_lane_id` / `planned_detach_lane_d`.

##### `BothSides` car crossover rule

Opposite-side `BothSides` car access must not be rendered or simulated as a single diagonal local segment from `door_pos` to the chosen opposite-side lane point.

Use this exact derived rule:

- `same_side_car_lane_id = car_lane_fwd` and `opposite_side_car_lane_id = car_lane_bkw` when `side == -1`
- `same_side_car_lane_id = car_lane_bkw` and `opposite_side_car_lane_id = car_lane_fwd` when `side == 1`
- `same_side_cross_point = edge_pos + normal * (edge.width * 0.5)`
- `opposite_side_cross_point = edge_pos - normal * (edge.width * 0.5)`

Then:

- if `vehicle_frontage_access == SameSideOnly`, car local access is always same-side
- if `vehicle_frontage_access == BothSides` and the chosen car lane id equals `same_side_car_lane_id`, car local access is same-side
- if `vehicle_frontage_access == BothSides` and the chosen car lane id equals `opposite_side_car_lane_id`, car local access is an opposite-side crossover
- if the chosen car lane id is neither `same_side_car_lane_id` nor `opposite_side_car_lane_id`, the access plan is invalid and must not be built

Exact local path shapes:

- same-side car local access:
  - `door_pos -> chosen lane point`
- opposite-side crossover car local access:
  - `door_pos -> same_side_cross_point -> opposite_side_cross_point -> chosen lane point`

No diagonal fallback is allowed for opposite-side `BothSides` access, even if one of the directional frontage lane groups is missing. The crossover anchor points are derived from edge geometry, not from the existence of an intermediate same-side drive lane.

This rule applies identically to both:

- `ACCESS_EGRESS` from building to network
- `ACCESS_INGRESS` from network to building

The crossover remains off-network and allocation-free. It is not a lane change, not a temporary network insertion, and not a CCH path segment.

#### 3. Concrete SoA migration

The redesign should not try to preserve the current agent field layout unchanged.

The present SoA is still the right storage model, but building access becomes much simpler if the SoA carries an explicit compact trip/access plan instead of overloading:

- `current_building`
- `target_building`
- `current_node`
- `target_node`
- `current_lane_id`
- `current_path`

with multiple meanings at once.

The recommended migration is targeted expansion, not a wholesale structure change.

##### Keep these existing fields in the final model

These continue to make sense and should remain after the redesign:

- household/building identity: `home_building`, `household_id`, `work_building`
- world state: `pos_x`, `pos_y`
- agent status: `activity`, `planned_activity`, `happiness`, `money`, `journey_start_time`
- location references: `current_building`, `target_building`, `planned_target_building`
- live network state: `current_edge`, `current_lane_id`, `lane_distance`, `speed`, `transit_mode`
- route buffer: `current_path`, `current_path_index`
- mobility/visual state: `has_car`, `vehicle_type`, `pedestrian_type`, `walk_phase`

##### Keep these existing fields only during migration

These are useful while old and new code paths coexist, but they should not survive as authoritative state in the final entrance/exit model:

- `target_node`
- `is_visible`

##### Reuse the existing `transit: u8` slot, but change its meaning

Do not add a second transit-state field.

Instead, replace the current building-access meanings with:

- `0 = TRANSIT_IN_BUILDING`
- `1 = TRANSIT_ACCESS_EGRESS`
- `2 = TRANSIT_NETWORK`
- `3 = TRANSIT_ACCESS_INGRESS`
- `4 = TRANSIT_IMMIGRATING`
- `5 = TRANSIT_INTERSECTION`

This keeps the SoA compact and avoids save/load churn from adding another per-agent state field.

`TRANSIT_IMMIGRATING` is not a building-origin access phase. It is a network-origin border-entry state for agents spawned at a border connection with no origin `BuildingEntrance`.

##### Activity and household shopping intent

`activity` and `planned_activity` describe the purpose of the in-building stop, not the movement
phase:

- `0 = home`
- `1 = work`
- `2 = shopping or other non-home stop`

Household replenishment shopping must not add a new `TRANSIT_*` state. The economy selects one
eligible household member, records that carrier on the household request, writes
`planned_target_building` to the store, and writes `planned_activity = 2`. The normal trip planner
then moves the agent through `IN_BUILDING -> ACCESS_EGRESS -> NETWORK -> ACCESS_INGRESS ->
IN_BUILDING`.

After store arrival, the household economy pass observes the carrier in-building at the store,
updates the household request, and schedules the same agent back home with `planned_target_building`
set to the home building and `planned_activity = 0`. Movement code is responsible only for the
trip; household stock, store inventory, budget reservation, refunds, and store revenue are owned by
[`economy.md`](economy.md).

##### Add these new SoA fields

Each active trip needs exact, preplanned local-access targets.

Add:

- `planned_attach_node: u32`
- `planned_detach_node: u32`
- `planned_attach_lane_id: u32`
- `planned_detach_lane_id: u32`
- `planned_attach_lane_d: f32`
- `planned_detach_lane_d: f32`
- `access_flags: u8`
- `next_replan_time: f32`
- `network_replan_failures: u8`

Purpose:

- `planned_attach_node`: chosen road-graph endpoint toward which the origin frontage departure is routed
- `planned_detach_node`: chosen road-graph endpoint from which the destination frontage approach begins
- `planned_attach_lane_id`: exact frontage-side lane to enter after `ACCESS_EGRESS`
- `planned_detach_lane_id`: exact frontage-side lane used for the final destination-side network approach and the exact network exit point
- `planned_attach_lane_d`: exact lane distance where the agent leaves short egress and enters the origin frontage lane
- `planned_detach_lane_d`: exact lane distance where the agent leaves the destination frontage lane and enters short ingress
- `access_flags`: compact authoritative trip metadata for plan validity, zero-hop routing, flow-field provenance, immigration-origin handling, and freight border destinations
- `next_replan_time`: absolute `sim_time` gate; the planner may only build or rebuild a trip when `sim_time >= next_replan_time`
- `network_replan_failures`: transient watchdog counter for consecutive failed live network/access replans; reset on successful planning, trip completion, recovery, road-edit route invalidation, and load

This is the minimum set that turns building access into exact planned legs, while also preventing failed trips from calling pathfinding every tick.

Use `u32` rather than `usize` for the planned lane IDs:

- `LaneSystem::lanes` still uses `Vec<Lane>` and is indexed with `usize` at the implementation boundary
- the SoA field should store a compact lane ID with `u32::MAX` as the invalid sentinel
- convert `u32 -> usize` only after a bounds check when reading from `LaneSystem::lanes`

For this project, `u32` is the correct tradeoff. Road lane counts per edge are already tiny, active lane counts are nowhere near 4 billion, and if long-session tombstone growth ever makes lane IDs uncomfortably large, the right fix is lane compaction/remapping or slot reuse, not widening every agent field to `usize`.

Use `u32::MAX` as the invalid sentinel for `planned_attach_node` and `planned_detach_node` too.

##### Exact flag layouts

`BuildingEntrance.flags` is derived cache state. It must use this exact bit layout:

- `0x01 = ENTRANCE_FOOT_VALID`
- `0x02 = ENTRANCE_CAR_VALID`
- `0x04 = ENTRANCE_FOOT_FWD_VALID`
- `0x08 = ENTRANCE_FOOT_BKW_VALID`
- `0x10 = ENTRANCE_CAR_FWD_VALID`
- `0x20 = ENTRANCE_CAR_BKW_VALID`
- `0x40 = reserved`, must be zero
- `0x80 = reserved`, must be zero

Consistency rules:

- `ENTRANCE_FOOT_VALID` is set iff either `ENTRANCE_FOOT_FWD_VALID` or `ENTRANCE_FOOT_BKW_VALID` is set
- `ENTRANCE_CAR_VALID` is set iff either `ENTRANCE_CAR_FWD_VALID` or `ENTRANCE_CAR_BKW_VALID` is set

Reset and save rules:

- rebuild starts from `flags = 0`
- rebuild recomputes every flag from authoritative building + edge + lane data in one pass
- `BuildingEntrance.flags` is derived cache state and must never be saved

`access_flags` is authoritative per-trip agent state. It must use this exact bit layout:

- `0x01 = ACCESS_PLAN_VALID`
- `0x02 = ACCESS_ZERO_HOP_NODE_PATH`
- `0x04 = ACCESS_PATH_FROM_FLOW_FIELD`
- `0x08 = ACCESS_IMMIGRATION_ORIGIN`
- `0x10 = ACCESS_FREIGHT_BORDER_DESTINATION`
- `0x20 = reserved`, must be zero
- `0x40 = reserved`, must be zero
- `0x80 = reserved`, must be zero

Reset and save rules:

- `access_flags = 0` in `spawn_housed_agent()` and `spawn_border_arrival_agent()`
- `access_flags = 0` in `kill_agent()`
- `access_flags = 0` whenever the agent returns to `IN_BUILDING`
- failed planning and failed replanning must clear `ACCESS_PLAN_VALID`
- successful plan build must rewrite the full used bitset from scratch
- successful `NETWORK` replan must rewrite `ACCESS_ZERO_HOP_NODE_PATH` and `ACCESS_PATH_FROM_FLOW_FIELD`
- successful immigration planning must set `ACCESS_IMMIGRATION_ORIGIN`
- successful freight export planning or freight border replanning must set `ACCESS_FREIGHT_BORDER_DESTINATION`
- `access_flags` must be saved and loaded together with the other trip-plan SoA fields

##### Exact planned-node sentinel and lifecycle rules

Use `u32::MAX` as the invalid sentinel for both `planned_attach_node` and `planned_detach_node`.

Use these exact lifecycle rules:

- if `access_flags & ACCESS_PLAN_VALID == 0`, then both planned nodes must be `u32::MAX`
- if `access_flags & ACCESS_PLAN_VALID != 0` and `ACCESS_FREIGHT_BORDER_DESTINATION == 0`, then both planned nodes must be `< graph.node_count()`
- for an ordinary building-origin trip:
  - `planned_attach_node` must equal either `origin_edge.start_node` or `origin_edge.end_node`
  - `planned_detach_node` must equal either `destination_edge.start_node` or `destination_edge.end_node`
- for an immigration trip:
  - `planned_attach_node = border_node`
  - `planned_detach_node` follows the ordinary destination rule
- for a freight export trip with `ACCESS_FREIGHT_BORDER_DESTINATION`:
  - `target_building = usize::MAX`
  - `freight_target_border_node = planned_detach_node`
  - `planned_detach_node < graph.node_count()`
  - `planned_detach_lane_id = u32::MAX`
  - during the initial building-origin export plan, `planned_attach_node` follows the ordinary origin rule
  - after road-edit route invalidation, `planned_attach_node` and `planned_attach_lane_id` may be `u32::MAX` while `ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION` remains set; the current lane or node anchor plus `freight_target_border_node` is then the authoritative replan context
- `planned_attach_node` is written once on successful initial planning and remains immutable until the trip completes, aborts, or is cancelled
- `planned_detach_node` is written on successful initial planning and may be replaced by successful `NETWORK` replans
- entering `IN_BUILDING`, trip cancellation, trip completion, `ACCESS_EGRESS` abort back to building, and `kill_agent()` must clear both planned nodes to `u32::MAX`

##### Do not add these to SoA

These belong in the per-building entrance cache, not on every agent:

- `door_pos`
- `curb_pos`
- `entrance_s_m`
- lane candidate lists
- side-specific geometry vectors
- any other entrance shape data

The building cache is shared O(B) data. Putting it in SoA would multiply memory traffic by agent count for no gain.

##### Field semantics after migration

After the migration, the important fields should mean exactly this:

- `current_building`: valid while the agent is inside a building and during `ACCESS_EGRESS` from that origin building; invalid otherwise
- `target_building`: the building being approached or reserved as the current destination
- `current_node`: last graph node physically reached on the live network
- `current_lane_id` and `lane_distance`: live network position only, never a planned access target
- `current_path`: network leg only, never local building access
- `planned_*`: immutable trip-plan scalars for the current trip until a replan occurs
- `next_replan_time`: earliest allowed retry time after a planning or replanning failure

##### Exact `current_node` contract

`current_node` has one meaning only:

- the last graph node physically reached by the agent on the live road/path network

Use this exact rule:

1. `current_node` is authoritative only while the agent is on the live network in `TRANSIT_NETWORK`, `TRANSIT_IMMIGRATING`, or `TRANSIT_INTERSECTION`.
2. While the agent is in `TRANSIT_IN_BUILDING`, `TRANSIT_ACCESS_EGRESS`, or `TRANSIT_ACCESS_INGRESS`, `current_node = u32::MAX`.
3. While the agent is lane-bound between two graph nodes, `current_node` remains the last node actually reached. It must not advance early just because `current_lane_id` implies a future terminal node.
4. `current_node` advances only when the agent actually reaches a graph node and consumes that hop in network movement.
5. Replanning from a lane may compute a future `replan_start_node`, but that does not mutate `current_node` by itself.
6. Save/load must preserve this exact meaning. `current_node` is never a synonym for "next node", "planned detach node", or "current lane terminal node".
7. When the agent leaves the live network and enters `TRANSIT_ACCESS_INGRESS` or `TRANSIT_IN_BUILDING`, clear `current_node` back to `u32::MAX`.

In the final model:

- `target_node` should be deleted once network planning and replanning read from `planned_detach_node` plus `current_path`
- `is_visible` should be deleted once rendering derives visibility from transit state

The intended visibility rule is simple:

- `TRANSIT_IN_BUILDING` => hidden
- `TRANSIT_ACCESS_EGRESS`, `TRANSIT_NETWORK`, `TRANSIT_ACCESS_INGRESS`, `TRANSIT_IMMIGRATING`, `TRANSIT_INTERSECTION` => visible

The intended visible-model rule is also simple:

- `MODE_WALK` => render a character
- `MODE_CAR` => render a car

The entrance/exit rewrite is mode-preserving:

- `MODE_WALK` remains a character during `ACCESS_EGRESS`, `NETWORK`, and `ACCESS_INGRESS`
- `MODE_CAR` remains a car during `ACCESS_EGRESS`, `NETWORK`, and `ACCESS_INGRESS`

The local access phases must not silently swap a car trip into a pedestrian visual or a foot trip into a vehicle visual.

If a later feature needs temporary render suppression unrelated to transit state, add a narrowly scoped visual/debug flag then. Do not keep `is_visible` as a general-purpose state bucket just because the old system used it.

That separation is the main reason to accept the SoA edit.

##### Memory budget

At 1,000,000 fully simulated agents, the added raw field cost is approximately:

- `u32 × 4` = about 16 MB
- `f32 × 3` = about 12 MB
- `u8 × 1` = about 1 MB

Total: about 29 MB plus normal `Vec` overhead.

The 20,000,000-agent world target does not multiply this by twenty. Only the
full-FSM agents inside the active area of interest carry these fields; agents at
distance run at a coarser aggregate fidelity. This budget bounds the size of that
active set.

That is cheaper than keeping ambiguous access state and rebuilding it in the hot
path every tick. No optimizations have been explored yet.

##### Save/load and lifecycle impact

The SoA migration requires coordinated updates in all agent lifecycle code:

- `spawn_housed_agent()` and `spawn_border_arrival_agent()`
- `kill_agent()`
- save/load of agent state
- benchmark/test setup that manually populates agent fields
- any code that assumes the old transit-state names or meanings
- render/snapshot code that currently reads `is_visible`
- any pathing or replanning code that currently reads `target_node`

There will be two save/load passes:

- migration pass: support both the legacy fields and the new trip-plan fields while the old and new logic overlap
- cleanup pass: drop `target_node` and `is_visible` from the authoritative agent save schema once all readers are removed

The SoA invariant remains unchanged: every field must still have exactly `self.count` elements at all times.

The rule for this redesign is:

- accept new compact scalar SoA fields when they encode exact trip/access state
- reject new SoA fields when they duplicate building geometry or derived lane topology already cached elsewhere

Proposed execution model: replace the current building access behavior with four explicit phases:

1. `IN_BUILDING`
2. `ACCESS_EGRESS`
3. `NETWORK`
4. `ACCESS_INGRESS`

`INTERSECTION` can remain as the vehicle-only junction phase inside `NETWORK`.

##### Replan timing contract

Trip planning and replanning must follow one deterministic timing rule:

- at most one planning attempt per agent per tick
- planning is only allowed when `sim_time >= next_replan_time`
- `next_replan_time` uses the same time base as `journey_start_time` and `AgentSystem::sim_time`

Use these fixed retry delays:

- `BUILDING_REPLAN_DELAY_S = 30.0`
- `NETWORK_REPLAN_DELAY_S = 5.0`

The longer building delay is acceptable because the agent is hidden inside a building. The shorter network delay keeps stranded visible agents responsive without reintroducing per-tick CCH thrash.

Successful planning or replanning must:

- write a complete new trip plan
- clear any stale failure state
- set `next_replan_time = 0.0`

Failed planning or replanning must:

- clear the invalid active network/path targets for that failed attempt
- set `next_replan_time = sim_time + retry_delay`
- never immediately loop into a second planning attempt in the same tick
- for live network/access replans, increment `network_replan_failures`; after three consecutive failures, cancel the active trip via watchdog recovery instead of retrying forever

#### `IN_BUILDING`

- Agent is hidden or represented as inside the building.
- `current_building` is valid.
- No local path is being reconstructed.
- If the economy has selected a destination and `sim_time >= next_replan_time`, the agent may attempt exactly one trip plan build at the start of the tick.
- For household shopping, the economy-owned request must already have selected this agent as the
  carrier before writing `planned_target_building`; the entrance/exit system does not choose the
  shopper.
- A successful ordinary building-origin plan build from `IN_BUILDING` must perform these exact writes before the agent begins `ACCESS_EGRESS`:
  - `target_building = chosen destination building`
  - `planned_attach_node`, `planned_detach_node`, `planned_attach_lane_id`, `planned_detach_lane_id`, `planned_attach_lane_d`, and `planned_detach_lane_d` from the chosen candidate
  - `access_flags = ACCESS_PLAN_VALID`, plus:
    - `ACCESS_ZERO_HOP_NODE_PATH` iff the chosen node-path portion is zero-hop
    - `ACCESS_PATH_FROM_FLOW_FIELD` iff the accepted `current_path` came from the flow-field fast path
    - `ACCESS_IMMIGRATION_ORIGIN` cleared
  - `current_path = chosen node path including both endpoint nodes`, or `[]` if the node-path portion is zero-hop
  - `current_path_index = 1` iff `current_path.len() >= 2`; otherwise `0`
  - `current_node = u32::MAX`
  - `current_edge = usize::MAX`
  - `current_lane_id = usize::MAX`
  - `lane_distance = 0.0`
  - `speed = 0.0`
  - `pos_x`, `pos_y = origin door_pos`
  - `transit = TRANSIT_ACCESS_EGRESS`
- If that plan build fails, the agent stays in `IN_BUILDING`, keeps its requested destination/activity, and sets `next_replan_time = sim_time + BUILDING_REPLAN_DELAY_S`.

#### `IMMIGRATING`

- `IMMIGRATING` starts at an external border-node origin, not at a building door.
- It has no origin entrance cache and never enters `ACCESS_EGRESS`.
- The destination is the immigrant's claimed home building, which uses the ordinary destination entrance cache and destination-side `planned_detach_*` fields.
- Immigration trips are node-anchored at spawn:
  - `current_building = usize::MAX`
  - `target_building = home_building`
  - `current_node = border_node`
  - `planned_attach_node = border_node`
  - `planned_attach_lane_id = u32::MAX`
  - `planned_attach_lane_d = 0.0`
- Immigration uses `MODE_CAR` in the current design because border connections are highway-style external road links.
- Immigration planning uses CCH only. Flow fields are never used for the initial immigrant trip because the destination is a specific home building and the origin is not `IN_BUILDING`.
- The origin local-access cost is exactly `0.0`.
- If the initial immigration plan succeeds, transition immediately from `IMMIGRATING` to `NETWORK` with these exact writes:
  - `access_flags = ACCESS_PLAN_VALID | ACCESS_IMMIGRATION_ORIGIN`
  - additionally set `ACCESS_ZERO_HOP_NODE_PATH` iff `planned_attach_node == planned_detach_node`; otherwise clear it
  - clear `ACCESS_PATH_FROM_FLOW_FIELD`
  - `current_path = chosen node path including both endpoint nodes`, or `[]` if the node-path portion is zero-hop
  - `current_path_index = 1` iff `current_path.len() >= 2`; otherwise `0`
  - `current_node = border_node`
  - `current_edge = usize::MAX`
  - `current_lane_id = usize::MAX`
  - `lane_distance = 0.0`
  - `speed = 0.0`
  - `pos_x`, `pos_y = border_node position`
  - `transit = TRANSIT_NETWORK`
- If the initial immigration plan fails, remain visible at the border node, keep `current_node = border_node`, and set `next_replan_time = sim_time + NETWORK_REPLAN_DELAY_S`.
- After the handoff to `NETWORK`, all normal `NETWORK`, zero-hop, and `ACCESS_INGRESS` rules apply.

#### `ACCESS_EGRESS`

- Agent moves from `door_pos` to the exact mode-specific local access handoff point.
- `ACCESS_EGRESS` covers only that short local building-to-outside segment.
- The target lane id and target lane distance are already known from the trip plan.
- For `MODE_WALK`, the local handoff point is the sidewalk or foot-lane access point implied by `planned_attach_lane_id` and `planned_attach_lane_d`, and the visible model remains a character.
- For `MODE_CAR`, no additional side filtering happens during movement. The planned attach lane already reflects the parent edge's `vehicle_frontage_access` policy, so `SameSideOnly` and `BothSides` are both handled by trip planning rather than by per-tick egress heuristics.
- For `MODE_CAR`, the final local handoff target is the exact vehicle-lane attach point implied by `planned_attach_lane_id` and `planned_attach_lane_d`, and the visible model remains a car.
- For opposite-side `BothSides` car access, the local movement path must follow the deterministic crossover polyline from the rule above rather than a single straight diagonal segment.
- When the local access target is reached, the agent attaches to the exact lane and exact distance from the plan.
- No replanning is allowed during ordinary egress movement.
- If the origin entrance cache becomes invalid, or the planned attach lane/node is no longer legal before attachment, abort the trip.
- Trip abort from `ACCESS_EGRESS` is exact:
  - snap the agent back to the origin building's cached `door_pos`
  - `current_building` remains the origin building
  - `target_building = usize::MAX`
  - clear `current_path`
  - `current_path_index = 0`
  - `current_node = u32::MAX`
  - `current_edge = usize::MAX`
  - clear `current_lane_id` and live network position
  - `lane_distance = 0.0`
  - `speed = 0.0`
  - `planned_attach_node = u32::MAX`
  - `planned_detach_node = u32::MAX`
  - `planned_attach_lane_id = u32::MAX`
  - `planned_detach_lane_id = u32::MAX`
  - `planned_attach_lane_d = 0.0`
  - `planned_detach_lane_d = 0.0`
  - `access_flags = 0`
  - transition to `IN_BUILDING`
  - set `next_replan_time = sim_time + BUILDING_REPLAN_DELAY_S`

No midpoint heuristics are needed here.

#### `NETWORK`

- Agent travels the road/pathing network normally.
- The network leg starts from the exact planned attach point, not from a guessed endpoint-only abstraction.
- The network leg owns the origin frontage-lane travel from the exact attach point on `planned_attach_lane_id` at `planned_attach_lane_d` to `planned_attach_node`.
- The network leg also owns the destination frontage-lane travel from `planned_detach_node` to the exact detach point on `planned_detach_lane_id` at `planned_detach_lane_d`.
- For `MODE_CAR`, a same-edge direct frontage candidate is also valid when all of these are true:
  - `planned_attach_lane_id == planned_detach_lane_id`
  - `planned_attach_lane_d <= planned_detach_lane_d`
  - both buildings use that same legal frontage lane
  - in that case, the network leg is the direct lane segment from `planned_attach_lane_d` to `planned_detach_lane_d` on that lane, with no endpoint wrap
  - the node-path portion of `current_path` is intentionally empty even though `planned_attach_node != planned_detach_node`
- If `planned_attach_node == planned_detach_node`, the trip has a zero-hop network leg:
  - the node-path portion of `current_path` is intentionally empty
  - this is not a planning failure
  - the agent still enters `NETWORK` after the origin-side handoff and follows any remaining planned frontage/network segments for that trip
- Replanning is checked at the start of the tick, before movement.
- A `NETWORK` replan is triggered only when one of these becomes true:
  - `current_path` is empty before the trip has reached its detach target, unless this is either:
    - the valid zero-hop case with `planned_attach_node == planned_detach_node`
    - the valid same-edge direct frontage case where the agent is already attached to `planned_detach_lane_id` and still has not reached `planned_detach_lane_d`
  - the next unread path hop references a deleted or non-adjacent graph element
  - the current lane or current node anchor is no longer valid
  - the destination entrance cache no longer provides the planned detach lane/node
- If a trigger fires and `sim_time >= next_replan_time`, attempt exactly one replan from the current network anchor:
  - use `(current_lane_id, lane_distance)` when the current lane is still valid
  - otherwise use `current_node`
- If that replan succeeds, replace the full network leg and the destination-side `planned_detach_*` fields with these exact writes:
  - if replanning from a valid current lane:
    - `replan_start_node = lane_terminal_node(current_lane_id)`
    - `current_path = chosen node path including replan_start_node and planned_detach_node`, or `[]` if that node-path portion is zero-hop
  - if replanning from `current_node`:
    - `replan_start_node = current_node`
    - `current_path = chosen node path including replan_start_node and planned_detach_node`, or `[]` if that node-path portion is zero-hop
  - `current_path_index = 1` iff `current_path.len() >= 2`; otherwise `0`
  - do not mutate `current_node` during replan success unless the agent has physically reached a new node during movement earlier in the same tick
  - overwrite `planned_detach_node`, `planned_detach_lane_id`, and `planned_detach_lane_d`
  - preserve `planned_attach_node`
  - clear `ACCESS_PATH_FROM_FLOW_FIELD`
  - set `ACCESS_ZERO_HOP_NODE_PATH` iff `current_path.is_empty()`
  - keep `ACCESS_PLAN_VALID`
  - keep the current lane/node anchor unchanged
  - keep `speed` unchanged
  - remain in `NETWORK`
- If that replan fails, the fallback is exact:
  - clear `current_path`
  - keep the agent at its current valid lane/node anchor
  - keep the current travel mode
  - keep the current destination
  - set movement speed to zero until the next allowed retry
  - set `next_replan_time = sim_time + NETWORK_REPLAN_DELAY_S`
- If repeated live network/access replans hit the watchdog threshold, cancel the active trip instead of preserving the visible stall:
  - for ordinary housed non-freight citizens, place the agent back inside `home_building`, clear destination/access/path/lane state, set home activity, and reset `next_replan_time` plus `network_replan_failures`
  - for freight carriers, immigrant carriers, pending household arrivals, or agents without a valid home, recover to a connected border node, preferring the freight target border/current anchor/planned anchors before the first connected border; keep a valid building target in `TRANSIT_IMMIGRATING`, otherwise remain route-less in `TRANSIT_NETWORK`
- If `sim_time < next_replan_time`, do not attempt another replan and do not hide the agent.

#### `ACCESS_INGRESS`

- Agent begins ingress from the exact mode-specific local access handoff point derived from `planned_detach_lane_id` and `planned_detach_lane_d`.
- The destination frontage segment is no longer part of `ACCESS_INGRESS`; it has already been consumed by `NETWORK`.
- `ACCESS_INGRESS` covers only the short outside-to-`door_pos` local access segment.
- For `MODE_WALK`, the ingress origin is the sidewalk or foot-lane handoff point and the visible model remains a character.
- For `MODE_CAR`, no additional side filtering happens during movement. The planned detach lane already reflects the parent edge's `vehicle_frontage_access` policy, so `SameSideOnly` and `BothSides` are both handled by trip planning rather than by per-tick ingress heuristics.
- For `MODE_CAR`, the ingress origin is the exact vehicle-lane detach point and the visible model remains a car.
- For opposite-side `BothSides` car access, the local movement path must follow the deterministic crossover polyline from the rule above rather than a single straight diagonal segment.
- When the door is reached, the agent becomes `IN_BUILDING` with these exact writes:
  - `pos_x`, `pos_y = door_pos`
  - `current_building = target_building`
  - `target_building = usize::MAX`
  - clear `current_path`
  - `current_path_index = 0`
  - `current_node = u32::MAX`
  - `current_edge = usize::MAX`
  - `current_lane_id = usize::MAX`
  - `lane_distance = 0.0`
  - `speed = 0.0`
  - `planned_attach_node = u32::MAX`
  - `planned_detach_node = u32::MAX`
  - `planned_attach_lane_id = u32::MAX`
  - `planned_detach_lane_id = u32::MAX`
  - `planned_attach_lane_d = 0.0`
  - `planned_detach_lane_d = 0.0`
  - `access_flags = 0`
  - `transit = TRANSIT_IN_BUILDING`
- No replanning is allowed during ordinary ingress movement.
- Arrival only changes the agent's location and activity state. Economy-specific side effects such
  as household-stock fulfillment, store revenue, reservation release, or return-trip scheduling must
  be applied by the owning economy pass after it observes the agent in `TRANSIT_IN_BUILDING`.
- If the destination entrance cache becomes invalid before the door is reached, abort ingress and return to the last network-side detach anchor:
  - restore the agent to the exact detach point on `planned_detach_lane_id` at `planned_detach_lane_d`
  - `current_node = planned_detach_node`
  - `current_edge = parent road edge of planned_detach_lane_id`
  - `current_lane_id = planned_detach_lane_id`
  - `lane_distance = planned_detach_lane_d`
  - `speed = 0.0`
  - clear `current_path`
  - transition to `NETWORK`
  - if `sim_time >= next_replan_time`, attempt exactly one `NETWORK` replan immediately
  - otherwise wait in `NETWORK` until `next_replan_time`
- If that immediate `NETWORK` replan fails, apply the normal `NETWORK` fallback and cooldown.

No `abs(agent_prog - frontage_t * edge_len) < 4.0` arrival test is needed.

##### Exact `NETWORK -> ACCESS_INGRESS` trigger

The final design keeps the long destination frontage approach inside `NETWORK`. `ACCESS_INGRESS` is only the short outside-to-building-entry phase.

Use this exact rule:

1. `current_path` is authoritative only for the road-network leg between `planned_attach_node` and `planned_detach_node`.
2. After the node-path portion is exhausted, `NETWORK` continues to own the destination frontage-lane segment from `planned_detach_node` to `planned_detach_lane_d` on `planned_detach_lane_id`.
3. Transition from `NETWORK` to `ACCESS_INGRESS` occurs exactly when both are true:
   - `current_lane_id == planned_detach_lane_id`
   - `lane_distance >= planned_detach_lane_d`
4. If the node-path portion is exhausted at `planned_detach_node`, the agent must stay in `NETWORK` until the destination frontage-lane segment is also consumed.
5. If `current_path` is exhausted at any other node and this is not the valid zero-hop case, do not enter `ACCESS_INGRESS`.
   - stay in `NETWORK`
   - clear the invalid path
   - follow the normal `NETWORK` replan policy
6. On the transition tick:
   - compute the exact local ingress origin from `planned_detach_lane_id` and `planned_detach_lane_d`
   - if `transit_mode == MODE_WALK`, use the sidewalk or foot-lane handoff point
   - if `transit_mode == MODE_CAR`, use the exact vehicle-lane detach point
   - snap the world position to that exact local ingress origin
   - clear `current_path`
   - `current_node = u32::MAX`
   - `current_edge = usize::MAX`
   - `current_lane_id = usize::MAX`
   - `lane_distance = 0.0`
   - `speed = 0.0`
   - keep the already chosen `planned_detach_lane_id` and `planned_detach_lane_d` as the ingress origin reference
   - enter `ACCESS_INGRESS`
7. During `ACCESS_INGRESS`, the agent moves from that exact local ingress origin to `door_pos` without changing `transit_mode` or render model. No second network insertion occurs.

This removes the old "reach frontage node, then push back onto frontage edge" hack completely while also keeping `ACCESS_INGRESS` short. The frontage edge is now an explicit planned network-side approach primitive, and ingress begins only at the exact mode-specific local access origin.

##### Exact `ACCESS_EGRESS -> NETWORK` trigger

The final design also keeps the long origin frontage departure inside `NETWORK`. `ACCESS_EGRESS` is only the short building-entry-to-outside phase.

Use this exact rule:

1. `ACCESS_EGRESS` ends when the agent reaches the exact mode-specific local handoff point implied by `planned_attach_lane_id` and `planned_attach_lane_d`.
   - if `transit_mode == MODE_WALK`, this is the sidewalk or foot-lane handoff point
   - if `transit_mode == MODE_CAR`, this is the exact vehicle-lane attach point
2. On the transition tick:
   - snap the world position to that exact attach point
   - `current_building = usize::MAX`
   - `current_node = lane_origin_node(planned_attach_lane_id)`
   - set `current_lane_id = planned_attach_lane_id`
   - `current_edge = parent road edge of planned_attach_lane_id`
   - set `lane_distance = planned_attach_lane_d`
   - if `transit_mode == MODE_CAR`, set `speed = min(AGENT_DRIVEWAY_SPEED_MS, parent edge speed limit of planned_attach_lane_id)`
   - if `transit_mode == MODE_WALK`, set `speed = 0.0`
   - clear any stale local-only state
   - enter `NETWORK`
3. After the transition, `NETWORK` continues along the origin frontage lane until `planned_attach_node`, then follows the node-path portion of `current_path`, then performs the destination frontage-lane approach.
4. `ACCESS_EGRESS` must not absorb the long origin frontage segment from the attach point to `planned_attach_node`.

This keeps egress symmetric with ingress:

- `ACCESS_EGRESS` = short `door -> exact outside handoff point`
- `NETWORK` = origin frontage departure + node path + destination frontage approach
- `ACCESS_INGRESS` = short `exact outside handoff point -> door`

At both boundaries, `current_node` stays a last-reached-node anchor:

- on `ACCESS_EGRESS -> NETWORK`, it becomes the origin node of the attached lane because the agent has entered that lane segment but has not yet reached its terminal node
- on `NETWORK -> ACCESS_INGRESS`, the agent has already physically consumed the detach-node approach while still in `NETWORK`, so after detaching into local ingress `current_node` is cleared back to `u32::MAX` with the rest of the live-network anchor state

##### Exact local-access geometry, distance, and time rules

Planning and runtime movement must use the same local-access math.

Use these exact helper definitions:

- `local_access_point(mode, lane_id, lane_d, curb_pos) -> Vector2`
- `local_access_speed(mode) -> f32`
- `local_access_distance(...) -> f32`
- `local_access_time_s(distance, mode) -> f32`

Rules:

1. `local_access_point(MODE_WALK, ..., curb_pos) = curb_pos`
2. `local_access_point(MODE_CAR, lane_id, lane_d, ...) = sample_pos_on_lane(lane_id, lane_d)`
3. `local_access_speed(MODE_WALK) = AGENT_WALK_SPEED_MS = 1.4`
4. `local_access_speed(MODE_CAR) = AGENT_DRIVEWAY_SPEED_MS = 3.0`
5. Segment distance is planar Euclidean distance in XZ:
   - `segment_distance(a, b) = sqrt((a.x - b.x)^2 + (a.y - b.y)^2)`
6. `local_access_distance` must use the exact local path shape for the chosen mode and lane:
   - `MODE_WALK`: `segment_distance(door_pos, curb_pos)`
   - `MODE_CAR` with same-side access: `segment_distance(door_pos, chosen lane point)`
   - `MODE_CAR` with opposite-side `BothSides` crossover: `segment_distance(door_pos, same_side_cross_point) + segment_distance(same_side_cross_point, opposite_side_cross_point) + segment_distance(opposite_side_cross_point, chosen lane point)`
7. `local_access_time_s(distance, mode) = distance / local_access_speed(mode)`
8. Local-access cost is identical to local-access time in seconds.
9. `ACCESS_EGRESS` movement must follow the exact local path shape above at constant `local_access_speed(mode)`, clamping the final step onto the last segment endpoint.
10. `ACCESS_INGRESS` movement must follow the same exact local path shape in reverse at constant `local_access_speed(mode)`, clamping the final step onto the last segment endpoint.
11. No pathfinding, lane search, or heuristic offset is allowed inside `ACCESS_EGRESS` or `ACCESS_INGRESS`.

##### Local-access capacity policy

V1 of the generic entrance/exit system has no building-side reservation, queue, or contention model.

Use this exact rule:

1. Every `BuildingEntrance` used by this spec has effectively unlimited local-access throughput.
2. `ACCESS_EGRESS` never waits for a driveway slot, bay reservation, or building-side queue token.
3. `ACCESS_INGRESS` never waits for a driveway slot, bay reservation, or building-side queue token.
4. No per-building or per-anchor occupancy counter participates in trip legality, trip start, or local-access movement.
5. Multiple agents may traverse the same local-access segment at the same time. This is accepted v1 behavior.
6. If a future freight or service system needs building-side contention, it must be added as a separate extension that binds to asset-authored anchors rather than changing the base local-access geometry rules in this document.
7. Until that future extension exists, optional `service` anchors are ignored by the generic entrance/exit system; ordinary trips use only the required `entrance` anchor named `main`.

This keeps the base rewrite cheap and deterministic. Building-side queues, bay reservations, and spillback are explicitly deferred instead of being approximated implicitly.

##### Exact frontage-lane legality and cost rules

Candidate legality and frontage-lane cost must use the lane's directed origin/terminal nodes.

Define:

- `lane_origin_node = edge.start_node` if `lane.is_fwd == true`; otherwise `edge.end_node`
- `lane_terminal_node = edge.end_node` if `lane.is_fwd == true`; otherwise `edge.start_node`

Use these exact rules:

1. An origin attach candidate is legal only if `planned_attach_node == lane_terminal_node(planned_attach_lane_id)`.
2. A destination detach candidate is legal only if `planned_detach_node == lane_origin_node(planned_detach_lane_id)`.
3. Origin frontage-lane travel distance is `lane.length - planned_attach_lane_d`.
4. Destination frontage-lane travel distance is `planned_detach_lane_d`.
5. Origin frontage-lane free-flow time is:
   - `origin_frontage_free_flow_time_s = (lane.length - planned_attach_lane_d) / edge.speed_limit` for `MODE_CAR`
   - `origin_frontage_free_flow_time_s = (lane.length - planned_attach_lane_d) / AGENT_WALK_SPEED_MS` for `MODE_WALK`
6. Destination frontage-lane free-flow time is:
   - `destination_frontage_free_flow_time_s = planned_detach_lane_d / edge.speed_limit` for `MODE_CAR`
   - `destination_frontage_free_flow_time_s = planned_detach_lane_d / AGENT_WALK_SPEED_MS` for `MODE_WALK`
7. If the required frontage segment distance is `<= 1e-6`, its frontage cost is exactly `0.0`.
8. If the required travel speed is non-finite or `<= 1e-6`, the candidate is illegal and must be skipped.
9. For `MODE_WALK`, frontage congestion penalty is always `0.0`.
10. For `MODE_CAR`, frontage congestion penalty is:
   - if `lane.length <= 1e-6`, both frontage penalty terms are exactly `0.0`
   - if `lane.length > 1e-6`:
     - `origin_frontage_penalty_time_s = ((lane.length - planned_attach_lane_d) / lane.length) * lane.frontage_delay_penalty_s`
     - `destination_frontage_penalty_time_s = (planned_detach_lane_d / lane.length) * lane.frontage_delay_penalty_s`
11. Final frontage travel time is free-flow plus penalty:
   - `origin_frontage_time_s = origin_frontage_free_flow_time_s + origin_frontage_penalty_time_s`
   - `destination_frontage_time_s = destination_frontage_free_flow_time_s + destination_frontage_penalty_time_s`
12. A candidate with an invalid lane id is illegal and must be skipped before cost comparison.
13. `MODE_CAR` has one additional same-edge direct frontage candidate:
   - if `planned_attach_lane_id == planned_detach_lane_id`
   - and `planned_attach_lane_d <= planned_detach_lane_d`
   - then do not price that candidate as `(lane.length - planned_attach_lane_d) + planned_detach_lane_d` plus a node-path between endpoints
   - instead price the live network segment directly as `planned_detach_lane_d - planned_attach_lane_d`
   - free-flow direct frontage time is `(planned_detach_lane_d - planned_attach_lane_d) / edge.speed_limit`
   - direct frontage penalty time is `((planned_detach_lane_d - planned_attach_lane_d) / lane.length) * lane.frontage_delay_penalty_s` when `lane.length > 1e-6`; otherwise `0.0`
   - the node-path portion of `current_path` for that candidate is empty, but `ACCESS_ZERO_HOP_NODE_PATH` remains clear because `planned_attach_node != planned_detach_node`
   - this rule exists to allow same-edge same-lane travel between the exact attach and detach points without forcing a fake endpoint wrap

##### Exact frontage congestion-penalty cache

The authoritative road-graph path cost remains free-flow. Only `MODE_CAR` frontage-segment candidate cost incorporates aggregated live congestion.

Use this exact rule:

1. Each vehicle lane owns one derived scalar:
   - `lane.frontage_delay_penalty_s: f32`
2. `lane.frontage_delay_penalty_s` is updated at fixed cadence:
   - `FRONTAGE_DELAY_UPDATE_S = 1.0`
3. Only `lane_type == Vehicle` lanes participate. All foot lanes always have `frontage_delay_penalty_s = 0.0`.
4. At each update, derive one raw observed lane delay from live lane traffic:
   - `lane_vehicle_count = number of MODE_CAR agents currently on that lane`
   - `lane_mean_speed_mps = arithmetic mean of current agent speeds for those cars`
5. If `lane_vehicle_count == 0`, `raw_lane_delay_penalty_s = 0.0`.
6. Otherwise:
   - `observed_speed_mps = clamp(lane_mean_speed_mps, 1.0, edge.speed_limit)`
   - `free_flow_lane_time_s = lane.length / edge.speed_limit`
   - `observed_lane_time_s = lane.length / observed_speed_mps`
   - `raw_lane_delay_penalty_s = clamp(observed_lane_time_s - free_flow_lane_time_s, 0.0, 30.0)`
7. Smooth the cached penalty with a fixed deterministic low-pass filter:
   - `lane.frontage_delay_penalty_s = 0.75 * lane.frontage_delay_penalty_s + 0.25 * raw_lane_delay_penalty_s`
8. If `edge.speed_limit <= 1e-6` or `lane.length <= 1e-6`, `lane.frontage_delay_penalty_s = 0.0`.
9. Candidate evaluation uses only the latest cached `lane.frontage_delay_penalty_s`. It must not inspect raw live IDM neighbors, gap state, or leader-follower chains during planning.
10. `network_path_time_s` remains the exact CCH or accepted flow-field node-path cost in seconds with no dynamic congestion multiplier.

This keeps dynamic traffic influence cheap and stable: full-network routing stays free-flow and deterministic, while building-adjacent car choices still see a low-frequency congestion signal.

### Path planning strategy

Do not reintroduce building nodes into the road graph.

Instead, treat each building entrance as up to two possible edge-endpoint access options per mode:

- toward `start_node`
- toward `end_node`

The actual number of legal options depends on the cached frontage lanes:

- walking on a valid sidewalk frontage exposes exactly two options
- driving on a `SameSideOnly` road frontage exposes exactly one direct option when car access is valid
- driving on a `BothSides` road frontage may expose up to two direct options
- invalid frontage access exposes zero options

For an ordinary building-origin trip, the planner must:

1. Read the origin entrance cache.
2. Read the destination entrance cache.
3. Enumerate legal access options for the chosen mode.
   - For `MODE_CAR`, origin attach candidates and destination detach candidates must both be generated under the same `vehicle_frontage_access` rule on their respective parent edges.
4. Evaluate the cheapest total trip using:
   - short egress cost from `door_pos` to `local_access_point(mode, planned_attach_lane_id, planned_attach_lane_d, curb_pos)`
   - origin frontage-lane travel cost from the exact attach point to `planned_attach_node`
   - network path cost between `planned_attach_node` and `planned_detach_node`
   - destination frontage-lane travel cost from `planned_detach_node` to the exact detach point
   - short ingress cost from `local_access_point(mode, planned_detach_lane_id, planned_detach_lane_d, curb_pos)` to `door_pos`
5. Store the selected access legs and network path in the trip plan.

This keeps the road graph unchanged while still planning with exact building access costs. Full node-path routing stays free-flow, while only car frontage segments receive the low-frequency cached congestion penalty defined above.

The exact total-cost formula for one legal candidate is:

- `total_cost_s = egress_local_time_s + origin_frontage_time_s + network_path_time_s + destination_frontage_time_s + ingress_local_time_s`

Where:

- `egress_local_time_s = local_access_time_s(local_access_distance(door_pos, local_access_point(mode, planned_attach_lane_id, planned_attach_lane_d, curb_pos)), mode)`
- `origin_frontage_time_s` follows the frontage rule above
- `network_path_time_s` is the exact CCH or accepted flow-field node-path cost in seconds
- `destination_frontage_time_s` follows the frontage rule above
- `ingress_local_time_s = local_access_time_s(local_access_distance(local_access_point(mode, planned_detach_lane_id, planned_detach_lane_d, curb_pos), door_pos), mode)`

For the origin side, the chosen endpoint is `planned_attach_node`. The agent performs short egress to `planned_attach_lane_id` at `planned_attach_lane_d`, then `NETWORK` continues along that frontage lane until `planned_attach_node`.

For the destination side, the chosen endpoint is `planned_detach_node`. `NETWORK` reaches that endpoint and then continues along `planned_detach_lane_id` until `planned_detach_lane_d`. Only the short local access segment from the exact mode-specific handoff point to `door_pos` is executed by `ACCESS_INGRESS`.

##### External-origin immigration rule

Immigration is the one supported case where a trip has no origin `BuildingEntrance`.

Use this exact rule:

1. The origin anchor is the chosen border node.
2. Origin endpoint enumeration is skipped entirely.
3. `planned_attach_node = border_node`.
4. `planned_attach_lane_id = u32::MAX`.
5. `planned_attach_lane_d = 0.0`.
6. `origin_local_access_cost = 0.0`.
7. Destination-side candidate selection is still performed normally from the claimed home building's entrance cache.
8. The initial immigration network path is built with CCH only, from `border_node` to `planned_detach_node`.
9. Flow fields are not consulted for immigration.
10. If the claimed home sits mid-edge, the final frontage-lane approach from `planned_detach_node` to `planned_detach_lane_d` is still executed inside `NETWORK`; immigration does not skip directly from the node to `ACCESS_INGRESS`.

This keeps immigration compatible with the entrance model without inventing a fake origin building or a fake origin entrance cache.

##### Future outside-gateway household arrival and departure model

This is a later explicit extension, not the baseline `v0.1` transport contract.

When the city later supports multiple outside-connection types for people as well as freight, the
transport layer should generalize narrow `TRANSIT_IMMIGRATING` into a shared outside-gateway model.

Recommended future model:

- compile every connected external people/freight connection into one `OutsideGateway` runtime
  record
- an `OutsideGateway` may be `road`, `rail`, `ship`, `air`, or a later authored external mode
- each gateway should declare which external flows it supports:
  - household arrival
  - household departure
  - freight import
  - freight export
- the same physical outside connection may serve both household movement and `OWA` freight, but
  those remain different consumers of the same gateway rather than one merged subsystem

Recommended future transport states:

- `TRANSIT_EXTERNAL_ARRIVAL`
  - generalized replacement for narrow border-road `TRANSIT_IMMIGRATING`
  - used when an admitted household is visualized as entering the map through a chosen
    `OutsideGateway`
  - starts from the chosen gateway rather than from a building door
  - keeps the current rule that demand already decided admission and economy already created the
    household record and claimed its destination home before transport visualization begins
- `TRANSIT_EXTERNAL_DEPARTURE`
  - used when a household already selected for whole-city removal is visualized as leaving through
    a chosen `OutsideGateway`
  - transport does not decide whether departure happens; it only executes the already-decided trip
  - once the departure handoff at the chosen gateway is complete, the household members are removed
    from live simulation

Ownership boundary for this future model:

- [`docs/demand.md`](demand.md) decides whether households are admitted or removed and how many
- [`docs/economy.md`](economy.md) owns the admitted household record, claimed home, and the
  underlying bad-state reasons that may lead to later removal
- this document owns only the physical outside-gateway arrival/departure trip semantics, gateway
  choice, and visible transport behavior

Shared-gateway rule:

- if the same outside connection also serves the `OWA`, it should do so through the same
  `OutsideGateway` abstraction
- freight import/export remains economy-owned `OWA` behavior
- household arrival/departure remains transport-owned movement behavior
- the shared abstraction is the gateway, not the business logic

##### Exact same-endpoint rule

If the selected candidate has `planned_attach_node == planned_detach_node`, the trip has a zero-hop network leg.

Use this exact rule:

1. The node-path portion of `network_path_cost = 0.0`.
2. `current_path` is stored empty by design for the node-path portion.
3. This does not skip `NETWORK` automatically.
4. After `ACCESS_EGRESS` reaches the exact planned attach point, the agent enters `NETWORK` normally.
5. In `NETWORK`, the agent still consumes any required origin-side and destination-side frontage-lane segments.
6. `ACCESS_INGRESS` still begins only when the exact detach target on `planned_detach_lane_id` at `planned_detach_lane_d` is reached.

The old system's "same node means skip the road phase entirely" shortcut should not be copied forward blindly. In the new model, the agent may still need to consume origin-side and destination-side frontage-lane segments even when the node-path portion is zero, so the correct abstraction is a zero-hop node path inside a still-real `NETWORK` phase, not a teleport from departure to arrival.

##### Stable candidate ordering and tie-breaks

For one chosen mode, each legal trip candidate is defined by:

- one origin endpoint choice: `start_node` or `end_node`
- one destination endpoint choice: `start_node` or `end_node`
- one exact attach lane id and attach lane distance
- one exact detach lane id and detach lane distance

Candidate enumeration must use this exact nested order:

1. origin toward `start_node`, destination toward `start_node`
2. origin toward `start_node`, destination toward `end_node`
3. origin toward `end_node`, destination toward `start_node`
4. origin toward `end_node`, destination toward `end_node`

Illegal candidates are skipped, but the surviving candidates keep that same relative order.

The planner must compare candidates by this exact lexicographic key:

1. lower `total_cost`
2. lower `origin endpoint rank`
   - `start_node = 0`
   - `end_node = 1`
3. lower `destination endpoint rank`
   - `start_node = 0`
   - `end_node = 1`
4. lower `planned_attach_lane_id`
5. lower `planned_detach_lane_id`
6. lower `planned_attach_lane_d`
7. lower `planned_detach_lane_d`

If two legal candidates have equal `total_cost`, the earlier value in this ordering wins.

This gives the planner one stable answer even when:

- both sidewalk directions are equally cheap
- the same edge can be reached from either endpoint with identical network cost
- future asset types introduce more than one legal lane-level access candidate on the same frontage

##### Flow-field nearest-building tie-break

If a flow field stores `nearest_building`, equal-cost source competition must use this exact lexicographic key:

1. lower total reverse-Dijkstra cost
2. lower source building id
3. lower source endpoint node id

`nearest_building[node]` must always be the building id from the winning tuple above. No hash-map iteration order, insertion order, or floating-point epsilon comparison may affect this result.

##### Flow-field and CCH precedence

In this redesign, CCH is the authoritative router. Flow fields are an optional fast path only.

Use this exact precedence:

1. Choose `target_building`, travel mode, origin endpoint, and destination endpoint first.
   - `target_building` is authoritative before any network path is written.
   - mode selection uses the exact CCH-based totals defined below in `Mode policy`.
   - access-candidate selection uses the deterministic ordering defined above.
2. After those choices are fixed, try a flow-field path only if all of these are true:
   - the agent is starting a new trip from `IN_BUILDING`
   - this is not a mid-trip replan
   - the chosen mode has a built flow field for the destination zone
   - the chosen origin anchor has no incoming-edge turn-restriction context
   - `flow_field.nearest_building[planned_attach_node] == target_building`
   - `flow_field.build_path(planned_attach_node, ...)` succeeds
   - the resulting path ends at `planned_detach_node`
   - `CchGraph::path_has_valid_turns(path, graph)` returns true
3. If every check above passes, accept the flow-field path as `current_path`.
4. Otherwise, call CCH and use the CCH result as `current_path`.
5. During `NETWORK` replans and `ACCESS_INGRESS` recovery, skip flow fields entirely and use CCH only.

This gives one stable owner per decision:

- CCH owns exact travel cost, turn legality, and all replans.
- Flow fields may only reuse a precomputed node chain when it already matches the exact chosen building and exact chosen destination approach node.

Flow fields must not:

- change `target_building`
- change travel mode
- change `planned_detach_node`
- bypass the deterministic access-candidate selection
- become the authoritative path source during replans

If flow fields are retained for this redesign, their source sets must be rebuilt from the legal destination endpoint nodes that correspond to the final entrance model. Legacy `building_depart_node()` sources are not precise enough for the final system.

### Mode policy

The current "has_car means car, otherwise walk" rule is too coarse for a purged system.

Recommended deterministic policy for ordinary building-origin trips:

1. Resolve legality first.
   - `walk_legal = origin entrance has foot access AND destination entrance has foot access`
   - `car_legal = agent.has_car AND origin entrance has car access AND destination entrance has car access`
2. If neither mode is legal, do not start the trip.
   - The agent remains in `IN_BUILDING` and retries later under the failure policy.
3. If exactly one mode is legal, choose that mode.
4. If both modes are legal, evaluate both candidate trips using exact CCH travel cost for mode selection.
   - `walk_total = walk_access_egress_cost + walk_network_cch_cost + walk_access_ingress_cost`
   - `car_total = car_access_egress_cost + car_network_cch_cost + car_access_ingress_cost`
5. Choose the lower total cost.
6. If the two totals are exactly equal, choose walking.

The required tie-break order is:

- lower total cost
- `MODE_WALK` over `MODE_CAR`

Rationale:

- walking is the safer fallback in a system without parking ownership, driveway reservation, or vehicle storage state
- tie-breaking to walking avoids unnecessary vehicle insertion when both plans are effectively equivalent
- using exact CCH cost for mode selection keeps the decision aligned with the authoritative exact-building router; flow fields are only an optional fast path after that choice is already fixed

Trip mode is chosen exactly once, at trip start, and written into the agent state.

Replans do not change travel mode. They only rebuild the access legs and network leg for the already chosen mode.

This removes a class of fallback bugs where the agent decides to drive and later discovers there is nowhere sensible to merge, while also preventing mode-flapping during replans.

Immigration is the exception: it does not run this mode-selection policy and instead uses the fixed external-origin rule above.

### Failure policy

Do not hide agents as the default failure response.

Use explicit failure states and exact fallback anchors:

- if the trip has not yet attached to the network, abort back to the origin door and return to `IN_BUILDING`
- if the trip has already attached to the network, remain on the network or at the detach node and retry later from that anchor
- do not switch modes during replan or fallback
- do not hide the agent for ordinary routing failure

Invisible cleanup should be reserved for true invalid data, not ordinary routing failure.

## What To Purge

If the current system is being intentionally replaced, these are the pieces worth deleting rather than carrying forward:

- `building_depart_node()` as the primary building-to-network abstraction for trips
- the `frontage_t < 0.5` endpoint decision rule in trip planning
- direct center-to-curb departure as the canonical source of local access truth
- the 4 meter frontage-edge arrival snap heuristic
- the "reach frontage node, then push back onto frontage edge" special case
- car frontage access that ignores `building.side` and accepts any vehicle lane on the chosen edge direction
- "cars do not enter buildings" behavior where vehicle travel ends at the curb and the final building entry is always converted into a walk-to-center phase
- flow-field nearest-building substitution that may replace the economy-selected destination building with a different zone-nearest building during trip start
- "hide the agent" as the normal answer to path or access failure
- `target_node` as an authoritative trip-destination field once `planned_detach_node` and `current_path` cover the network leg
- `is_visible` as authoritative agent state once visibility is derived from transit state

What should remain:

- building edge attachment without frontage split nodes
- side-specific pedestrian access constraints
- lane-based road movement
- SoA agent storage, expanded with explicit trip/access scalars
- CCH and flow fields on the road graph

## Suggested Implementation Order

### Phase 1 - Lock authoritative inputs and persistence

- Add the required `entrance` anchor named `main` to every building asset and make runtime asset lookup expose it as authoritative entry metadata.
- Add `vehicle_frontage_access: VehicleFrontageAccess` to `network::graph::Edge`.
- Persist `vehicle_frontage_access` through save/load with the documented migration default for older saves.
- Make road creation, split/merge, save/load, and existing editor/history flows preserve `vehicle_frontage_access` exactly as specified.

Goal: make the two authoritative inputs to the entrance model real before any derived cache or planner depends on them.

### Phase 2 - Introduce entrance cache without changing movement

- Add derived `BuildingEntrance` data to `BuildingAllocator`.
- Derive `door_pos` from the asset-authored `main` entrance anchor, not from frontage midpoint fallback logic.
- Copy `vehicle_frontage_access` from the authoritative parent edge into the derived cache.
- Rebuild the cache after building placement, load, lane rebuilds, and road edits that recreate or modify edges.
- Keep the old FSM temporarily.

Goal: make access geometry explicit and deterministic before changing planner or movement behavior.

### Phase 3 - Perform the SoA migration

- Add the new `planned_attach_*`, `planned_detach_*`, `access_flags`, and `next_replan_time` fields.
- Rename the transit-state semantics to `IN_BUILDING`, `ACCESS_EGRESS`, `NETWORK`, `ACCESS_INGRESS`, `IMMIGRATING`, and `INTERSECTION`.
- Keep `target_node` and `is_visible` temporarily while old and new readers still exist.
- Update `spawn_housed_agent()`, `spawn_border_arrival_agent()`, save/load, tests, benchmarks,
  and any manual SoA setup to initialise all new fields.
- Keep `current_path` for the network leg only.

Goal: make the trip/access contract explicit in the data model before changing planner or movement code.

### Phase 4 - Add the frontage-delay cache used by planning

- Add the low-frequency per-lane `frontage_delay_penalty_s` cache for vehicle lanes.
- Update it from aggregate lane traffic at the fixed cadence defined in the spec.
- Keep full road-graph path cost free-flow; only frontage-segment candidate cost should read this cache.

Goal: make the planner inputs match the final spec before exact trip planning starts using them.

### Phase 5 - Start writing the new trip plans

- Populate the new SoA trip-plan fields during departure planning.
- Stop using `building_depart_node()` as the main trip abstraction.
- Choose exact attach/detach lanes and endpoint nodes from the entrance cache.
- Apply the exact mode-selection, tie-break, and frontage-cost rules from this document.

Goal: move local access decisions out of per-tick reconstruction and into one explicit trip plan.

### Phase 6 - Replace `DEPARTING` and `ARRIVING`

- Rewrite movement states as exact `ACCESS_EGRESS` and `ACCESS_INGRESS`.
- Read the SoA trip-plan scalars instead of rebuilding frontage logic each tick.
- Enforce the exact `current_node` lifecycle contract at attach/detach boundaries and during lane-based replans.
- Remove midpoint arrival checks and curb snap heuristics.
- Remove node-only building attachment logic from normal trips.

Goal: one coherent trip from door to door.

### Phase 7 - Remove legacy fallbacks

- Delete the node-only building fallback paths that were only needed by the old model.
- Delete "hide on failure" for ordinary replannable situations.
- Delete any remaining `building_depart_node()` / frontage-node compatibility branches from tick logic, render/debug helpers, and test scaffolding once the exact entrance-plan movement is live.
- Delete the remaining no-plan compatibility branches kept only for old trips, old saves, or migration overlap, so valid runtime behavior no longer depends on "if `ACCESS_PLAN_VALID` is missing, fall back to the pre-Phase-5 frontage/curb model".
- Delete the remaining `NETWORK`-side midpoint-arrival, frontage-node arrival-lane reinsertion, and other destination frontage compatibility branches that can still bypass the exact `planned_detach_lane_id` / `planned_detach_lane_d` handoff.
- Delete any path-missing or path-exhausted compatibility handlers that rebuild a node-only destination approach for trips that already carry a valid exact access plan.
- Delete legacy arrival-side mode conversion and center-based ingress assumptions, including the old "`MODE_CAR` trip becomes `MODE_WALK` on final arrival" behavior and any fallback that still treats building center as the canonical ingress target for valid assets.
- Rewrite or delete the legacy departure/arrival tests that assume heuristic curb insertion from `current_node`, `target_node`, or frontage-node shortcuts; replace them with exact entrance-cache and planned-lane assertions.
- Remove direct-line render/debug assumptions that still treat egress as "current position -> current node" or ingress as "current position -> building center" once `ACCESS_EGRESS` and `ACCESS_INGRESS` use the exact planned local handoff points.
- Simplify tests around deterministic entrance access instead of heuristics.
- Keep non-agent systems on the same entrance-cache abstraction as ordinary trips. Freight/shipment ETA, supplier choice, `OWA` border-terminal choice, and helper-path spawning should continue to use exact entrance-side car access rather than regressing to any edge-endpoint proxy.
- `TRANSIT_IMMIGRATING` should stay transport-layer-only if retained. Ownership of whether the city admits households belongs to `docs/demand.md`; `docs/economy.md` owns the admitted household record; this document owns only the movement semantics for an optional border-origin transport trip.
- If later gameplay upgrades border-only immigration into a full multi-mode outside-gateway system,
  replace narrow `TRANSIT_IMMIGRATING` with explicit `TRANSIT_EXTERNAL_ARRIVAL` and
  `TRANSIT_EXTERNAL_DEPARTURE` states rather than overloading ordinary building ingress or direct
  household deletion.
- Write a dedicated destruction/eviction contract for agents whose current or target building disappears, then remove the out-of-band `TRANSIT_ACCESS_INGRESS` "dump onto rubble/street" behavior from `AgentSystem::evict_building()`.
- Audit any remaining debug logs, tests, and tooling helpers that still speak in terms of legacy "home_node" or edge-endpoint semantics even though ordinary trips now run on entrance-cache plans.

Goal: no ghost legacy behavior left in the movement loop or its adjacent helper, exceptional, and tooling paths.

### Phase 8 - Remove redundant SoA fields

- Remove `target_node` from agent state once network replanning uses `planned_detach_node` plus `current_path`.
- Remove `is_visible` from agent state once render/snapshot code derives visibility from transit state.
- Remove both fields from save/load, benchmarks, tests, and any compatibility shims.

Goal: finish the redesign with a smaller, clearer SoA instead of carrying legacy fields forever.

## Complexity And Performance Notes

- Entrance-cache rebuild: O(B) over buildings after placement/load/topology changes.
- Frontage-delay cache update: O(L_vehicle) over vehicle lanes at fixed low cadence (`FRONTAGE_DELAY_UPDATE_S = 1.0`), not per-agent.
- Trip planning: O(1) local access evaluation plus the existing road-path query cost.
- Per-agent tick: O(1) local access movement and existing lane movement; no hot-path allocation should be introduced.

This is compatible with the project's performance rules because:

- no extra graph mutation is required for buildings
- no extra spatial index is required
- local access data is cached, not recomputed expensively every tick
- SoA growth is limited to compact scalar trip-plan state
- two legacy SoA fields (`target_node`, `is_visible`) can be deleted at the end of the migration
- the pathing backend stays on the existing road graph

## Recommended Direction

If the system is being purged, the best direction is:

- keep the no-frontage-split graph architecture
- make entrances explicit
- make trips entrance-to-entrance
- accept a small, targeted SoA expansion for exact trip/access state
- cache access geometry once
- derive visibility from transit state instead of storing it separately
- remove `target_node` once the network leg is fully encoded by the trip plan and `current_path`
- remove all midpoint/node/curb heuristics from the hot path

That gives the project a stable base for later features like explicit doors, driveways, loading bays, parking, and richer pedestrian behavior without poisoning the road graph or breaking the 1M-agent design constraints.
