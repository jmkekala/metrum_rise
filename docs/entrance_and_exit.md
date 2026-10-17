# Metrum Rise - Building Entrance and Exit System

This document describes the current building entrance/exit behavior in the Rust simulation, why it has become brittle, and the recommended replacement architecture.

The target audience is future work on `simulation/economy/agents/` and `simulation/buildings/allocator/`. This is a standalone reference linked from `docs/project.md`; `project.md` tracks implementation status, while this file owns the detailed entrance/exit design and implementation spec.

This document is now intended to be a deterministic implementation spec for the entrance/exit rewrite. If the implementation changes, update this file and `docs/project.md` together so they stay aligned.

## Current System

Lane attachment and ingress rollback publish occupancy through the shared
[`TRAFFIC-02` lane contract](traffic.md#shared-lane-occupancy-contract-traffic-02).
Entrance planning still owns exact attachment legality; movement no longer independently writes
the lane ID, road owner, distance and transit state at those handoffs.

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

This means the path planner discards the true mid-edge location and compresses the building's network attachment to one bit of information.

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
5. During `TRANSIT_DEPARTING`, the agent walks in a straight line from its current world position, usually the building center, to a computed curb point on the frontage edge.
6. When the curb point is reached, the agent is inserted directly into a frontage lane partway along the edge.
7. The agent travels normally through the lane/junction/pathing FSM.
8. On the destination frontage edge, the system watches for the agent to come within about 4 meters of the building's `frontage_t`.
9. The agent snaps to the curb point and enters `TRANSIT_ARRIVING`.
10. During `TRANSIT_ARRIVING`, the agent walks in a straight line from the curb point to the building center.
11. On arrival, `current_building` becomes `target_building`, the agent becomes invisible, and the transit state returns to `IDLE`.

### Sidewalk and lane rules

Pedestrian entry and exit are restricted to the building's frontage side when a matching sidewalk exists. The implementation currently does this by selecting a sidewalk lane whose sign matches `building.side`.

Cars do not use a modeled driveway or parking system. They also use the same curb-point handoff, then insert directly into a vehicle lane on the frontage edge.

If the expected frontage-side lane does not exist, the code falls back to a much coarser node-based entry or arrival path.

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

`activity` and `planned_activity` describe the owned task, including its in-building stops:

- `0 = home`
- `1 = work`
- `2 = shopping task or other visit`

Household replenishment shopping must not add a new `TRANSIT_*` state. The economy selects one
eligible household member, records that carrier on the household request, writes
`planned_target_building` to the store, and writes `planned_activity = 2`. The normal trip planner
then moves the agent through `IN_BUILDING -> ACCESS_EGRESS -> NETWORK -> ACCESS_INGRESS ->
IN_BUILDING`.

After store arrival, the household economy pass observes the carrier in-building at the store,
updates the household request, and schedules the same agent back home with `planned_target_building`
set to the home building and `planned_activity = 2`. Shopping activity survives home arrival until
the household pass credits the delivered stock and restores home activity, clearing any queued
return reminder. Work scheduling must not interrupt either pickup or delivery acknowledgement.
Cancellation also releases the task through the normal home-return path. Movement code is
responsible only for the trip; household stock, store inventory, budget reservation, refunds, and store revenue are owned by
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
- `network_replan_failures`: watchdog counter for consecutive failed live network/access replans; reset on successful planning, trip completion, recovery and road-edit route invalidation; preserved by format-75 save/load

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

##### Render-height ownership

Local access movement owns XZ positions; it does not cache an independent elevation profile.
For both cars and pedestrians, `ACCESS_EGRESS` and `ACCESS_INGRESS` use the same current
world-surface query: owned road top, then flat building support, then accepted generation-matched
graded terrain/CDT, with visual terrain as the unowned fallback. Pedestrians sample at their
origin; rigid vehicles sample their footprint as described below. Keep 0.02 m clearance above
the supporting surface. Original source terrain and a maximum of road,
pad and terrain heights are not valid substitutes: terrain cut away beneath a yard must not
lift an agent above that yard. This applies equally to zoned and explicitly placed buildings.

Network-bound agents retain lane geometry heights; sidewalk walkers retain the existing curb
step offset. Ground heights are resampled when snapshots are built, so new terrain/site
products need no route rewrite or cached-access-height invalidation. Terrain publication and
pending-product limitations remain owned by [`earthworks.md`](earthworks.md).

Off-lane vehicles use one Rust rigid-footprint support solver in snapshots and after visual
interpolation. Mesh loading registers model-local bounds and bottom contacts once; it does not
change simulation movement or save data. Four yaw-aligned footprint corners define a least-squares
pitch/roll plane. Preserve the existing XZ heading when constructing its orthonormal basis;
model -Z still faces travel. Query the final rotated XZ of the prepared contacts, then choose the
smallest origin height that clears them all by 0.02 m. Contacts include a 3×3 bounds grid and up
to 64 distinct mesh-bottom vertices. Registration rejects invalid geometry; the renderer then
uses its matching procedural fallback instead of mismatched dimensions.

A centre height and triangle normal alone cannot support a rigid car spanning road/apron/pad
grade breaks. The footprint contract applies to both sides and both access directions, remains
level when the entire footprint is on a level pad, and permits rigid-body clearance above lower
parts of a concave transition. It is not suspension simulation or continuous chassis collision.
Lane-bound cars retain lane geometry; pedestrians retain their existing orientation.
The superseded optional-normal query pipeline is removed. The production ownership query returns
height only; vehicle orientation continues to derive from the footprint, not a centre-face normal.
The old direct, non-snapshot car/pedestrian exporters are removed; they had no callers and bypassed
the shared surface contract. Instance rendering consumes the coherent snapshot exports only.

Car transforms, stable IDs and off-lane flags are exported from a single snapshot read. After
interpolating XZ and yaw, Godot requests the same Rust solve for flagged cars. This read uses a
nonblocking core lock: if busy/unprepared, only those cars use their supported snapshot poses;
ordinary lane interpolation continues. Interpolation caches the pose actually uploaded. Changes
between lane and access ownership snap to the target pose instead of blending unsupported
geometry across owners. No per-agent persistent height cache or additional route state is added.

The allocator prepares its existing site/chunk index once before the snapshot loop. Repeated
off-lane height queries allocate nothing, compile nothing and visit only local road owners,
nearby indexed sites and containing fixed terrain tiles, not the entire building or terrain
store. Each vehicle solve performs four fit queries plus at most 73 contact queries, independent
of city population. Rayon fills existing transform buffers in parallel without changing instance
order or allocating per vehicle; the snapshot iterator borrows buckets without a staging vector.
Existing camera culling happens before those queries; lane-bound agents avoid footprint probes.

If a later feature needs temporary render suppression unrelated to transit state, add a narrowly scoped visual/debug flag then. Do not keep `is_visible` as a general-purpose state bucket just because the old system used it.

That separation is the main reason to accept the SoA edit.

##### Memory budget

At 1,000,000 agents, the added raw field cost is approximately:

- `u32 × 4` = about 16 MB
- `f32 × 3` = about 12 MB
- `u8 × 1` = about 1 MB

Total: about 29 MB plus normal `Vec` overhead.

That is a meaningful increase, but it is still cheap compared to the cost of keeping ambiguous access state and rebuilding it in the hot path every tick.

##### Save/load and lifecycle impact

Save format 58 remaps both planned lane IDs through the saved directed lane identities, alongside
the active lane. Building entrances rebuild using the borrowed asset catalogue before agent
restoration; no catalogue clone is required. Network and ingress trips may have no origin attach
after a network replan, but must retain a valid destination detach. Version 57 destination lanes
are reconstructed from the target building's legal frontage lanes and planned directed node;
stale runtime-array IDs are never interpreted as rebuilt lane IDs.

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

##### Migration rule

The rule for this redesign is:

- accept new compact scalar SoA fields when they encode exact trip/access state
- reject new SoA fields when they duplicate building geometry or derived lane topology already cached elsewhere

### Proposed execution model

Replace the current building access behavior with four explicit phases:

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
- When home and work are the same farm, the authored shift changes activity in place. A same-building
  target clears the pending transition without entering `ACCESS_EGRESS` or planning a road path.
  Changing that job preserves any home-bound shopping trip and returns an on-site idle worker to
  home activity. Home/work assignment changes invalidate the existing schedule cache (`ECON-08`).
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
- Each failed replan, the recovery and its target, junction turn or road misses, invalid access plans and departures that cannot be planned are logged on `events` with the planner's own rejection reasons ([trip failures](debug.md#trip-failures), `DEBUG-06`).

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
- Arrival preserves shopping activity at both the store and home until the economy acknowledges
  pickup or delivery. Other arrivals at home set home activity; other destinations preserve purpose.
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
     - `origin_frontage_penalty_time_s = ((lane.length - planned_attach_lane_d) / lane.length) * lane.live_frontage_delay_s`
     - `destination_frontage_penalty_time_s = (planned_detach_lane_d / lane.length) * lane.live_frontage_delay_s`
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
   - direct frontage penalty time is `((planned_detach_lane_d - planned_attach_lane_d) / lane.length) * lane.live_frontage_delay_s` when `lane.length > 1e-6`; otherwise `0.0`
   - the node-path portion of `current_path` for that candidate is empty, but `ACCESS_ZERO_HOP_NODE_PATH` remains clear because `planned_attach_node != planned_detach_node`
   - this rule exists to allow same-edge same-lane travel between the exact attach and detach points without forcing a fake endpoint wrap

##### Exact frontage congestion-penalty cache

The authoritative road-graph path cost remains free-flow. Only `MODE_CAR` frontage-segment candidate cost incorporates aggregated live congestion.

Use this exact rule:

1. Each vehicle lane owns one derived scalar:
   - `lane.live_frontage_delay_s: f32`
2. `lane.live_frontage_delay_s` is updated at fixed cadence:
   - `FRONTAGE_DELAY_UPDATE_S = 1.0`
3. Only `lane_type == Vehicle` lanes participate. All foot lanes always have `live_frontage_delay_s = 0.0`.
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
   - `lane.live_frontage_delay_s = 0.75 * lane.live_frontage_delay_s + 0.25 * raw_lane_delay_penalty_s`
8. If `edge.speed_limit <= 1e-6` or `lane.length <= 1e-6`, `lane.live_frontage_delay_s = 0.0`.
9. Candidate evaluation uses only the latest cached `lane.live_frontage_delay_s`. It must not inspect raw live IDM neighbors, gap state, or leader-follower chains during planning.
10. `network_path_time_s` remains the exact CCH or accepted flow-field node-path cost in seconds with no dynamic congestion multiplier.

This keeps dynamic traffic influence cheap and stable: full-network routing stays free-flow and deterministic, while building-adjacent car choices still see a low-frequency congestion signal.

Economy prices use a second scalar, `lane.frontage_delay_penalty_s`, with the same frontage
formulas. Freight, customer and construction quotes read it. `TransitNetwork::publish_frontage_prices`
copies every lane's live delay into it at the start of each operational hour, before the hourly
economy runs. `frontage_delay_revision` advances only when a published value changes. The live delay
changes almost every simulated second, so pricing it directly would rebuild citywide censuses for
every queued-construction check (`ECON-13-D`). Agent trip planning, including job-search commutes,
keeps reading the live delay.

Save format 75 preserves each live lane's smoothed delay and the network's fractional delay-update
clock. Restore delays through the existing directed-lane/connector identity remap, after rebuilding
lanes. The smoothing history and clock are continuation state: recreating them as zero shifts
traffic sampling and therefore freight quotes even when road geometry and current traffic agree.
Older saves lack this history and initialize it to zero. Save/load remains O(live lanes).
Save format 85 stores the live and published delays separately; formats 75–84 load both from the
single saved delay. Publication advances the quote revision only when a published value changes,
without another traffic scan or allocation. Hiring refreshes its border quotes on that revision.

CCH customization uses `edge.base_cost` alone, matching flow-field costs and the free-flow
contract above. Freight caches border sweeps and node-to-node costs. Complete quotes carry the
traffic revision at which they were priced: unchanged traffic reuses them in expected O(1), while
a changed revision lazily reprices only requested quotes, without clearing or reallocating the
whole cache. A warmed cache and a cache rebuilt after load therefore price the same current
traffic. Repricing a local quote evaluates at most four entrance combinations, with expected O(1)
cached node-pair lookups; border quotes use the existing sweep plus at most two entrance options.
Node-pair retention is bounded by O(roads + nodes) prepared storage. Missing pairs, including
misses after that cache fills, use the exact cost-only CCH search with reusable generation-stamped
labels and heaps; no unused node path is built. Complete building/building and border/building
maps likewise have prepared O(roads + nodes + buildings) capacity; saturation computes an exact
uncached quote. Entrance/index invalidation advances their generation in O(1), with a storage
clear only on generation wrap. Border sweeps are prepared before the quotation loop, including
physical dispatch, rather than allocated by a border quote. The saturation regression compares
every directed fixture pair with independent CCH/border costs across invalidation and changing
frontage delays, while all map capacities stay fixed. Recurring supply distinguishes geometry
revisions from CCH metric revisions: price changes retain source/resource/component buckets and
prepared replacement doors, but still invalidate quote freshness and refresh the maximum-speed
search bound. Proposed suppliers use their own directed door route and current frontage delays
through the same prepared node-cost scratch; their private supply handles never enter allocator-ID
route caches. Exact rollback restores their quantity eligibility. The 453-test release economy run
includes different proposed doors, unreachable variants, changing frontage delay and warm reuse
across a CCH metric change. Runtime replacement admission still requires its affected-buyer proof
before activating these projected sources. Clearing query labels is O(1), not a scan over the
prepared city capacity. Household customer feasibility preserves the ordinary direct frontage/node
rules and checks directed pedestrian component reachability with a reusable stack when the
existing constant-time cyclic-component proof does not apply. Base-cost customization invalidates
those tables through a separate CCH metric revision.

Freight delivery quotations also retain fixed lane projection and local access geometry in
indexed, revision-stamped route-cache slots. Batch preparation reserves only added slots;
quotations lazily prepare each requested entrance once per revision without allocation. Road
or entrance invalidation advances the existing generation; it does not sweep building slots.
Business-customer quotes reuse live building endpoints only when their supplied doors match
exactly, so replacement owner IDs cannot alias proposed geometry. Frontage prices remain live.
The same rank order, local access arithmetic and directed route checks price cached and proposed
doors. `demand.md` owns the current tests, timing comparisons and remaining acceptance work.

Commute quotations retain the ordinary planner's path and connector costs while reusing
thread-local CCH search/reconstruction buffers and generation-stamped pedestrian lane labels.
Cold sizing walks O(roads + CCH arcs + lanes/connectors) per caller/Rayon thread. Heap capacity
additionally reflects boundary-edge states: its CCH bound is the sum over nodes of
`(road degree + 1) * upward arc count`, rather than just the number of arcs. Subsequent
queries reuse that storage, including the exact directed connector fallback; actual agent plans
still own their returned path. Commute quotations additionally retain exact network-leg results
in direct slots bounded by O(roads + nodes) per worker. Keys include mode and both nodes.
Pedestrian fallback retains both lane constraints. Car legs, and walking legs covered by the
complete-turn proof below, use the CCH's canonical incoming-edge context and no arrival-lane
constraint: pure CCH costs do not depend on the arrival lane, unrestricted origins ignore the
incoming edge, and FOOT queries ignore vehicle turns. Restricted vehicle origins retain the
exact incoming edge. Source bucket queries share that same context. A collision or miss runs
the ordinary exact query. Topology, CCH metric and physical-lane revisions clear
these results, while door-local and frontage costs remain priced separately. Repeated job scans
retain visited labels per worker instead of allocating them per Rayon task. Prepared-storage
regressions compare paths/costs across metric changes and broken pedestrian connectors, test
cached failures and changed door/frontage terms, and check that query buffers do not grow.
Positive shopping and household-customer walking proofs prepare only pedestrian entrance
ends, sharing each projection across departure and arrival while pricing each direction's
frontage separately. Full candidate searches likewise prepare their four distinct ends once,
retaining the original rank-comparison order. A failed proof lazily prepares the ordinary walking/driving ends and retains its exact
fallback. This avoids car-lane projection when proving a walking trip; it does not remove car
choices from ordinary trip planning. Directed connectivity is scoped to the CCH/lane revision,
while callers validate their current building entrances separately.
Cost forecasts also prepare an ordered, compact copy of pedestrian connector targets per
worker and topology revision, in O(roads + lanes + connections) time/storage. An exhaustive
positive proof skips witness traversal only when every physical incoming foot lane connects
to every outgoing foot lane at its junction, and every live road edge has both pedestrian
directions. This covers the node-pair representative used for parallel edges too. Distinct-target
counts prevent duplicate connectors from concealing a missing turn. The proof preserves a
nonempty lane frontier by induction along any CCH node path and reaches the exact destination
lane at its final junction. Otherwise forecasts retain the complete connector check and
fallback. Stored routes still materialize and validate their witness. The routing oracle checks
exact cost bits across complete, incomplete and rebuilt lane graphs; economy measurements are
in the [ECON-13 follow-up](economy.md#routing-follow-up-2026-10-11).
Full trip selection now uses the same prepared exact quotations. Only the winning building
trip reconstructs an owned route; border and network-replan candidates retain their winning
path in worker scratch and copy it once after selection. Pedestrian fallback predecessors,
connector frontiers and flow-field paths also reuse prepared buffers. Cost-only callers do not
prepare these additional full-route buffers, and invalid/zero-hop requests avoid graph-sized
preparation. Warm storage remains bounded by roads, nodes, lanes and connector links, alongside
the existing degree-weighted CCH query bound. The final stored agent route remains an owned
product. A test-only copy of the former allocating planner compares exact cost bits, ranks,
paths and access flags; the current source audit finds owned query wrappers only in tests.
`demand.md` records validation for the active follow-up; the figures below are historical.

The current DEM-02 performance follow-up, with 25 passing pathing tests, indexes CCH labels directly
by the original road endpoint. Each direction prepares two slots per road plus one slot for
its exact query-origin context, including nonincident incoming edges. Generation stamps keep
warm clearing and lookup O(1), with no hash lookup or allocation. Owned-path callers reuse
thread-local search/reconstruction scratch as well; their returned node vector remains owned.
Existing graph-generation checks rebuild this derived storage after topology changes. Invalid
and same-node requests bypass city-sized preparation. The final build passes 2,264 library tests,
all growth/reload checks and the unchanged saved-town year, with only the known utility assertion
failing. Matched hourly timing improves 7.1613 → 6.9396 ms; daily timing shows no significant
change. The larger DEM-02 performance regression remains open.
`demand.md` owns the measured profile, correctness results and performance acceptance boundary.

Format 75 also preserves each agent's speed, planned target/activity, network-replan failure
count, active lane-change origin/distance/length, overtaking timers and employment lock/unpaid
day counters. These affect arrival or employment decisions and are continuation state. Replacing
them with initial defaults changes shopping completion and freight timing after load. Lane-change
origins use the same lane identity remap as current/attach/detach lanes; building targets use the
existing snapshot building map. This adds O(agents) scalar save/load work and no simulation-tick
work. Exact continuation is validated for freshly written format-75 saves; older saves do not
contain the missing history.

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

When a worker loses or changes jobs while inside the former workplace, scheduling first requests
an ordinary trip home. Shopping activities are not overridden. Demand-owned demolition evacuates
physical occupants with the existing street-handoff routine before deleting the origin entrance;
zero employment slots do not prove physical vacancy. Route targets and building-index remaps must
remain valid together, so a displaced worker can return home and become a household shopper.

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

##### CCH routing performance track (`NET-02`–`NET-05`)

The routing follow-up to `ECON-13-B` proceeds in order: exact hot-path layout and turn-cache
improvements (`NET-02`), nested dissection and a turn-aware elimination-tree query (`NET-03`),
bucket many-to-many commute queries (`NET-04`), then restricted CPU sweeps (`NET-05`).
`NET-02` must preserve cost bits, distance bits, paths, economic records and growth reports.
Its baseline is `03c1cf45`, which contains the earlier hourly frontage-price/profit-proof work.
The hierarchy change can alter f32 summation and equal-cost path choices. The user approved
`NET-03` and the confirmed `NET-06` fix after the `NET-02` measurements, then deferred 1M runs
for this pass. Bucket joins must use compatible turn states and preserve the complete
frontage/entrance and 24-option ranking contracts.
The existing one-to-all sweep is the base for RPHAST; its float summation difference from point
queries must remain explicit. GPU work, funding-census changes and job-lock scheduling are
outside this routing implementation. The [roadmap](roadmap.md) owns each stage's exit criteria.

`NET-02` compiles the exact vehicle whitelist into sorted unique `(incoming edge, outgoing edge)`
pairs at hierarchy construction. An empty vehicle entry still activates whitelist mode; only
source lane IDs ±100 are ignored, preserving the existing target-lane semantics. Sentinel query
edges and FOOT-only queries retain their bypass. Open-node checks are O(1), restricted checks
O(log T) for T permitted edge pairs, and both allocate nothing. Turn tables retain O(nodes + T)
storage; independent node compilation uses Rayon. Live lane edits already rebuild the hierarchy,
so the table and shortcuts share the same topology revision. Metric customization leaves both
unchanged; the standalone path validator continues checking the authoritative graph.

Upward adjacency uses node offsets into flat shortcut-ID arrays. Construction scatters in the
former insertion order, in O(nodes + shortcuts); queries borrow a node's slice in O(1). Costs
occupy a separate f32 array indexed by the same shortcut IDs. Customization order, addition
order, selected boundaries, heap ordering, meeting ties and predecessor updates are unchanged.
The ordinary prepared-query storage bound still applies; no query allocation is introduced.

The ignored `benchmark_populated_cch_queries` builds the existing economy fixture outside query
timing, then measures 4,096 seeded CAR/FOOT queries with and without path materialization. Set
`METRUM_CCH_BENCH_RESIDENTS=10000|100000` and `METRUM_CCH_QUERY_REPORT` for exact cost/distance-bit,
node-path and initial city-record comparisons. The existing build/storage benchmark preserves
its topology/metric fingerprint across the layout change.

Fresh matched query measurements (2026-10-11), `03c1cf45` versus the `NET-02` working tree
(source fingerprint `0f14270494525f3dc6117e266d451ab7ea103d9c50ed7b33a667f68ad5c4dfdb`):
release, i9-12900K, `RAYON_NUM_THREADS=8`, `taskset -c 0,2,4,6,8,10,12,14`, no concurrent
builds/tests/benchmarks. The baseline adds only the identical populated-query measurement harness.
Both builds use Rust 1.98.1 and Cargo from the repository root with
`--manifest-path rust/Cargo.toml`: `cargo test --release --lib --test growth_scenario --no-run`,
`cargo bench --bench economy_tick_benchmark --no-run` and `cargo build --release`, with that
manifest option on each command. `CARGO_TARGET_DIR` and `TMPDIR` point to local build storage.
This root invocation uses the default target CPU, not the nested Cargo configuration's
`x86-64-v2` override; the two measured builds have identical compiler settings.
Two pairs per tier run baseline/candidate then candidate/baseline. Each run averages eleven
passes over the same 4,096 queries; the table averages those two run means. City setup and report
generation are outside query timing. Run command:

```bash
METRUM_CCH_BENCH_RESIDENTS=10000 METRUM_CCH_QUERY_REPORT="$ARTIFACTS/net02/queries.txt" \
  RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 \
  "$TEST_BINARY" --ignored --nocapture benchmark_populated_cch_queries
```

Repeat with `100000` and separate outputs for each build/run. Logs, exact reports and raw samples
are retained under `$ARTIFACTS/net02`.

| Fixture | Query | Baseline µs | NET-02 µs | Time reduction |
|---|---|---:|---:|---:|
| 10k: 247 nodes, 446 edges | Cost only | 31.29 | 19.49 | 37.7% |
| 10k | Reconstructed path | 31.40 | 19.72 | 37.2% |
| 100k: 2,220 nodes, 4,283 edges | Cost only | 192.24 | 124.29 | 35.3% |
| 100k | Reconstructed path | 191.01 | 124.80 | 34.7% |

All four reports within each tier are byte-identical, including every cost/distance bit, node
path and initial city record. The existing `benchmark_cch_build_storage`, run with
`METRUM_CCH_BENCH_GRAPH=car|mixed|restricted|one_way`, also preserves every topology/metric
fingerprint at sides 8/16/32. At side 32 its retained-storage accounting changes from
5,076,752 → 4,969,936 bytes (car/mixed), 6,810,816 → 6,694,952 (restricted) and
9,036,144 → 8,868,832 (one-way). This accounting excludes allocator metadata and the pre-existing
node-order buffer on both builds. These isolated query gains do not establish `ECON-13-B`;
the full 100k economy hour/day benchmark has not been rerun for this stage.

The same builds also complete four alternating 10k economy pairs with the standard recipe:
`RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$BENCH_BINARY" --bench --noplot /10k`.
Mean operational hour: **35.50 → 25.13 ms (29.2% lower)**; mean daily settlement:
**9.22 → 9.01 ms (2.2% lower)**. The two-sided exact permutation test over the four run means
per build gives **p = 0.0286** for each phase (70 assignments). Hourly run means are
35.547/35.251/35.409/35.795 ms versus 25.164/25.114/24.946/25.288 ms; daily run means are
9.289/9.137/9.151/9.304 ms versus 8.958/9.049/9.029/9.019 ms. Every run ends with the same
day-15 record: 5,341 households, 10,668 residents, 7,134 employed, 13,777 jobs and 7,395 buildings.
`NET-02` meets its exact-result and measured-performance exit criteria; `ECON-13-A` passes on
this build. `ECON-13-B` remains open, and the earlier C/D measurements remain historical evidence
for their own build rather than freshly rerun acceptance here.

Fresh correctness validation (2026-10-11): `cargo test --release --lib`'s executable passes
2,298 tests (112 ignored); all four `growth_scenario` tests pass. The ordinary routing filter
passes 27 tests. Against a freshly compiled `03c1cf45` baseline, all five growth reports
(`METRUM_GROWTH_REPORT`, `METRUM_GROWTH_LONG_REPORT`, `METRUM_PRODUCER_TOWN_REPORT`,
`METRUM_GROWTH_RECOVERY_REPORT`, `METRUM_GROWTH_EXPANSION_REPORT`) compare byte-identically.
`cargo build --release` and the economy benchmark build produce no warnings. `cargo doc --no-deps`
reports zero missing-doc warnings and the existing unresolved `VegetationGenerator` link.
`./run.sh --test` was not run: the handover reports existing terrain-fixture and unrelated
benchmark-compilation blockers, which were not revalidated here. No real Godot profile was used.

**`NET-06`: confirmed pre-existing positive-cost self-loop bug.** An independent isolated probe
uses three nodes and three bidirectional CAR edges: `e0: 0→2` costs 1 s, `e1: 2→2` costs 5 s,
and `e2: 2→1` costs 1 s. Node 2 whitelists only `e0→e1` and `e1→e2`. The only legal route from
0 to 1 is `[0, 2, 2, 1]`, costing **7 s**. Both an extracted `03c1cf45` CCH and the frozen
`NET-02` library return that path but report **2 s**. The probe's independent expected-cost assertion
fails on both; its source/output are under `$ARTIFACTS/net02/loop_probe.rs` and
`$ARTIFACTS/net02/loop-probe.log`. This is correctness evidence, not a timing measurement.
Contraction excludes equal-rank endpoints from `customization_order`, so direct self-loops
retain their initial zero cost, while backward queries can still traverse them. The existing
loop test compares point queries with the sweep, which shares the same wrong metric.
Fixing this changes route costs and belongs with the explicitly approved semantic work after
`NET-02`; it must gain an independent regression before the fix. The ordinary suite's passing
status does not establish correctness for this newly exposed case.

Historical `NET-03` design checkpoint (approved 2026-10-11; implemented below): the
[turn-aware CCH paper, sections 3–4](https://drops.dagstuhl.de/entities/document/10.4230/OASIcs.ATMOS.2020.9)
distinguishes expanded edge-state graphs, on which standard CCH applies, from compact turn-table
graphs, for which its elimination-tree query is not applicable. The current node hierarchy with
boundary-edge labels must therefore not be treated as textbook node-only CCH. The proposed next
stage is deterministic nested dissection, followed by a proved turn-state query model (explicit
directed-edge states if the compact representation cannot support that proof), then allocation-free
elimination-tree queries. Preserve source-edge context, directed FOOT traversal, mode filtering,
unpacking and all derived quote invalidations. Keep f32 seconds; any changed addition order and
equal-cost paths require explainable growth-report differences. Validate against independent
edge-state Dijkstra, resolve `NET-06`, cover restricted detours and loops, and measure preprocessing memory as
well as queries at 10k/100k before acceptance. The user approved this stage and `NET-06` on
2026-10-11, then deferred all 1M runs for this pass; 1M acceptance remains unverified.

`NET-03` implementation and correctness argument (validated 2026-10-11):

- The road-node order is deterministic nested dissection. For each median x/z partition,
  collect the endpoints covering every crossing edge and choose the smaller separator;
  coordinate/ID ties and the x-before-z comparison are stable. Recurse into the disconnected
  halves before the separator, lifting each node to its arrival states. Each surviving half
  contains at most half the vertices rounded up. Coordinates select only an ordering; they
  never approximate route costs or discard roads. Independent large halves use Rayon.
- Restricted junctions have one state per incident incoming edge. Unrestricted junctions
  have one state: all incoming edges permit exactly the same exits there. Thus merging those
  arrivals is an exact quotient of the edge-state automaton. A transition traverses one
  directed road and pays its base cost. A required self-loop changes arrival state and pays
  its full cost; a nonnegative same-state loop cannot improve any route. A restricted query
  origin seeds only departures permitted by its supplied incoming edge. Targets accept every
  arrival state; a zero-hop query remains zero.
- Fill every higher-neighbor clique in this graph and customize both directed costs using
  every lower triangle. Parent is the lowest higher neighbor, computed before metric filtering.
  Higher-neighbor clique induction puts every higher neighbor on the ancestor chain. Therefore
  an ascending pass over the union of seed ancestor chains settles the complete upward DAG.
  Lower-triangle elimination converts a shortest route into an up/down route, so the minimum
  forward/backward meeting recovers the shortest cost in exact arithmetic. Concrete witnesses
  unpack to permitted original roads. No node-only elimination-tree assumption is applied to
  the old compact turn-table hierarchy.
- Metric profiles share storage only when every directed road-type decision and the turn
  bypass agree. FOOT-only queries bypass vehicle rules. Other masks require permitted turns;
  each traversed road must intersect the mask. Costs stay f32. Strict improvement, ascending
  state/arc order and lowest-state meeting ties are deterministic. Independent f64 Dijkstra
  tests use relative tolerance `2e-5` (absolute `2e-5` below one second); integer fixtures match
  exactly. This is a tested numeric contract, not a claim that floating-point sums are associative.
- With N road nodes, E roads, S arrival states, A filled edges, T lower triangles and P distinct
  profiles, ordering costs O((N log N + E) log N), fill visits O(T) higher-neighbor pairs plus
  ordered-set maintenance, and customization costs O(E + P(A + T + base transitions)).
  For H visited ancestors and A_H outgoing arcs, a prepared point query costs
  O(source degree + target states + H log H + A_H), with no heap or query allocation.
  Worker labels occupy O(S). The one-to-all sweep costs O(S + A) after its upward phase;
  its outward f32 summation may differ from a point query. Source/target parallel work uses Rayon.

Fresh release validation on the frozen `NET-03` build: 32 active routing tests and all 2,303
library tests pass (112 library tests ignored); all four growth integration tests pass. All five
growth/recovery/expansion reports are byte-identical to `NET-02`. Release and benchmark builds
produce no warnings; rustdoc has zero missing-doc warnings and only the existing unresolved
`VegetationGenerator` link. The independent positive-loop regression fails with 2 s before the
change and passes at 7 s after it, including recustomization to other positive loop costs.
The required return-detour regression also agrees with independent Dijkstra.

Matched, unprofiled `NET-02` → `NET-03` results (same compiler/worker/affinity settings and commands
as above; no concurrent compilation/tests). Baseline source fingerprint is the `NET-02` identity
above; `NET-03` is `a74f85b09b46370bcb8a29bb9e66a964bafed6320717f027d0ffa5eb4124c7e7`.
Source diff/new files, binaries, raw logs, query reports and analysis are under
`$ARTIFACTS/net03/{baseline,candidate}` and `$ARTIFACTS/net03/summary.json`.

| Workload | NET-02 | NET-03 | Comparison |
| --- | ---: | ---: | --- |
| 10k cost query, µs | 19.40 | 0.677 | 28.6× faster |
| 10k path query, µs | 19.62 | 0.871 | 22.5× faster |
| 100k cost query, µs | 127.09 | 5.368 | 23.7× faster |
| 100k path query, µs | 126.64 | 6.158 | 20.6× faster |
| 10k operational hour, ms | 24.99 | 11.14 | 55.4% lower |
| 10k daily settlement, ms | 9.09 | 14.15 | **55.6% higher; fails ECON-13-A's 10 ms daily bound** |

Queries use two matched pairs per tier, with reversed build order for the second pair and
11 samples of 4,096 queries per process. Both repeat reports are identical within each build.
Economy uses four alternating pairs and the ordinary 23-hour/10-day Criterion recipe;
exact two-sided permutation p = 0.0286 for both hour and day run means. These economy timings
compare the same configured fixture, **not identical evolved simulation states**. The daily
regression is retained as an open performance finding; faster routing alone does not accept
ECON-13. In the first candidate run its last sampled settlement costs 66.76 ms; the other nine
range from 5.89 to 14.82 ms. Phase diagnostics and the subsequent bucket stage must resolve this.

Query changes are limited to route choices and f32 sums on the sampled populated graphs:
all 4,096 queries at each tier preserve reachability and physical distance. At 10k, 2,018 cost
bit patterns and 3,081 node paths differ; at 100k, 1,952 costs and 3,776 paths differ. Maximum
cost deltas are 5 ULP / 0.00003815 s and 4 ULP / 0.00024414 s, respectively. The largest relative
delta is 3.42e-7, below the independent-oracle tolerance. A concrete rounding boundary is the
FOOT query 35→5 with incoming edge 416: 108.00000762939453 s becomes 108 s, changing the integer
ceiling. Changed path ties also distribute physical traffic differently. These are the approved
mechanisms by which the populated city can evolve differently; economic logic and balancing
parameters were not changed. The separate five growth reports do not differ.

The 10k populated day-3 record changes from 10,612 residents / 7,225 employed to 10,594 / 7,224.
At day 15 it changes from 5,341 households, 10,668 residents, 7,134 employed and 13,777 jobs to
5,330 households, 10,647 residents, 7,134 employed and 13,803 jobs. Buildings remain 7,395;
both have zero open jobs, unhoused households and departures. The 100k day-3 record changes
from 104,716 residents / 71,644 employed to 104,563 / 71,484, with identical road/building counts.
All records repeat exactly within their build. `NET-04` must preserve the new hierarchy's records.

Preprocessing/storage tradeoff, side-32 synthetic fixtures (1,024 nodes; 21 builds per process):

| Graph | Retained bytes NET-02 / NET-03 | Build median ms NET-02 / NET-03 |
| --- | ---: | ---: |
| CAR | 4,969,936 / 6,610,304 | 13.72 / 8.04 |
| Mixed modes | 4,969,936 / 6,610,304 | 13.73 / 7.98 |
| Restricted | 6,694,952 / 7,585,608 | 17.02 / 7.98 |
| One-way | 8,868,832 / 7,574,208 | 24.14 / 8.20 |

Retained capacity rises 33% on open grids and 13% on the restricted grid, and falls 15% on the
one-way grid. The replacement retains a filled-edge topology, lower triangles and per-profile
witnesses. Counts exclude allocator metadata and are not peak RSS; the new count also includes
the small road-node order array omitted by the old benchmark. The table is one matched process
pair per case, not a significance claim about construction. The 1M tier remains unmeasured by
user instruction.

`NET-04` implementation (correctness and workload measurements complete):

- Each distinct open-workplace attachment node runs the same backward ancestor search as a
  point query. Per-state buckets retain its finite cost and a backward path witness. A source
  runs one forward ancestor search, then joins matching state buckets in ascending state order.
  Strict improvement preserves the point query's lowest-state meeting tie, exact f32 operands
  and reconstructed node path. Invalid and zero-hop cases keep their ordinary query semantics.
- `SharedNetworkQuotes` owns target tables per walking/driving mode, alongside its existing
  exact leg cache. Topology, customized costs or lane rebuilds invalidate both; a changed
  open-workplace set rebuilds tables while retaining valid pair quotes. The shared leg cache
  now uses two exact entries per fixed bucket, bounded by O(lanes), with O(1) lookup/replacement
  and no query allocation. It survives midnight; complete door-to-door quotes still expire daily
  to preserve their original frontage-pricing cadence. Save format 86 retains complete quotes
  with valid topology stamps: these contain historical frontage prices, so recomputing them
  after loading would change workplace choices. Rebuilt network-leg caches remain transient.
  Lane epochs cover terrain-driven rebuilds
  even when lane count and CCH generation are unchanged. Worker scratch caches
  the two source attachment ranks per mode. A prepared source query allocates nothing and costs
  O(H log H + A_H + visited bucket entries + targets); retained tables cost O(sum of target
  ancestor labels), and each worker costs O(states + targets) per source slot.
- `PERF-01` now gives sorted workplace-query slices stable ownership of this scratch.
  Shared network quotes remain read-only during each parallel phase; shards publish the last
  retained value in each changed local slot in fixed shard order between phases. Quote values
  and complete frontage-sensitive cache insertion order stay unchanged. Movement likewise
  owns route scratch by fixed agent ranges, with a separate serial scratch. This removes
  scheduling-dependent memoization work without making route selection approximate or adding
  an unbounded pair cache; fresh work-count and memory evidence lives in `economy.md`.
- Full frontage arithmetic, entrance combinations, the exact 24-option ranking, current-job
  fallback and rounded commute seconds remain unchanged. Pedestrian forecasts use the exhaustive
  turn-completeness proof above when available; otherwise they materialize the identical witness
  and retain the existing directed lane-connector check/fallback. The
  optimization replaces network searches; it does not relax pedestrian feasibility.
- Fresh frozen-build validation: all 2,304 library tests and four growth integration tests pass.
  All five growth reports are byte-identical to `NET-03`. Seeded mixed-mode/whitelist tests
  compare bucket costs and paths exactly with point queries; the targeted 43-workplace test
  compares every estimate and all 24 ranked options for walking/car-owning homes. It includes
  rounded costs, capped commute penalties and wage/capacity/ID ties. Prepared scratch capacities
  do not grow. Targeted entrance tests cover connector fallback and metric/lane invalidation.

The frozen measurement build has source fingerprint
`7a2098c94cc69dc99bc5593582c40fdeeb1efc85ff3486d333c756df24caf9f5`;
its source, binaries, raw logs and reports are retained under `$ARTIFACTS/net04/candidate`.
Matched timings compare it with the frozen `NET-03` build, using the compiler/worker/affinity
settings above. Four alternating 10k pairs and one complete 100k pair run the ordinary
`"$BENCH_BINARY" --bench --noplot /10k` or `/100k` recipe; setup is excluded from tick timings.
Raw Criterion samples, logs and analysis are under `$ARTIFACTS/net04`.

| Tick | NET-03 mean ms | NET-04 mean ms | Evidence |
| --- | ---: | ---: | --- |
| 10k hour | 10.858 | 10.789 | Four run means per build; p = 0.429 |
| 10k day | 14.368 | 14.333 | Four run means per build; p = 0.914 |
| 100k hour | 214.444 | 219.810 | One pair; descriptive, no significance claim |
| 100k day | 1,286.162 | 1,206.616 | One pair; 6.2% lower, all ten days lower |

The 100k daily samples cover days 6–15, including initial job-lock expiry. NET-03 / NET-04
milliseconds by day are 355/276, 186/168, 230/210, 445/319, 528/439, 728/647, 480/475,
2,494/2,441, 3,869/3,728 and 3,545/3,365. The large days 13–15 remain expensive.
Both builds finish with exactly the same 100k record: 54,388 households, 108,669 residents,
69,963 employed, 138,091 jobs, 76,039 buildings, treasury 1,000,000,017,473,930.5,
3,239 vacant homes, reserve 5,439, two unhoused households, zero open jobs and zero departures.
All eight 10k final records also match exactly. These comparisons establish no 10k timing
change and a descriptive late-day improvement, not an overall hourly speedup from batching.

`ECON-13` remains unaccepted: A's daily mean exceeds 10 ms, and B's same-build ratios are
20.37× for hours and 84.18× for days. B has one current pair, not the two repeated accepted
runs required to close it. C/D were rerun on this same frozen production revision:
`benchmark_combined_financing_cascades` retains two censuses at lengths 4/16/64, cold and warm;
`benchmark_queued_project_validation` measures 0.946 / 1.012 ms for 1,024 warm checks at
16,384 / 131,072 background buildings (1.07×). Cold first checks are 1.425 / 14.202 ms;
checks after publication are 1.505 / 14.272 ms. Logs are under `$ARTIFACTS/net05/econ13-c.log`
and `$ARTIFACTS/net05/econ13-d.log`; those two tests use the frozen `NET-04` test executable.

Diagnostic logging uses `economy_phase` and `economy_workplace` separately
from unprofiled acceptance runs. The 10k day-15 spike is in midnight demand work
(62.03 ms): dirty cell-lot regeneration takes 55.97 ms, planning 5.97 ms and execution
0.05 ms; household settlement takes 5.33 ms. Temporary phase timers were removed after
measurement; their patch and logs are retained under `$ARTIFACTS/net05-block`.

`NET-05` prototype (test-only; measured production no-go): collect every target arrival
state's elimination-tree ancestors, then run the ordinary upward source search followed by a
descending pass restricted to that closure. Every higher filled neighbor is an ancestor, so
every dependency of a retained descending label is also retained. Descending rank order and
the existing higher-neighbor order therefore reproduce the full PHAST arithmetic at requested
targets; omitted labels cannot feed a retained label. The independent Dijkstra regression also
checks restricted source-edge contexts, while the full-sweep comparison requires identical bits.
The closure is topology-dependent, so customization does not require rebuilding it.

Prepared restricted queries cost O(H log H + A_H + closure arcs + target arrival states), with
O(states + targets) reusable worker scratch and O(closure + targets) retained target storage.
Closure construction additionally uses O(states) temporary marks. The ignored
`benchmark_populated_cch_batches` compares buckets, restricted sweeps and the existing full
PHAST API using actual populated-city borders, open-workplace nodes and dense funded-workplace
nodes. It samples at most 512 distinct home attachment/context pairs across the whole city.
Preparation, worker reservation and repeated queries are reported separately; source batches
use Rayon. Cost-bit and rounded-second differences against bucket queries are explicit outputs.
Freight already caches full border sweeps per routing revision: repeated O(buildings × borders)
frontage quotations are outside these network-query measurements and cannot be credited as
RPHAST savings. Proposed building doors also consume the full cached tables.

The prototype also processes eight sources per block, storing eight contiguous f32 labels per
state. Each arc broadcasts its cost across those lanes; Rayon distributes independent blocks.
It preserves scalar restricted-sweep bits, including incoming-edge contexts, without unsafe
code or explicit SIMD dependencies. Both variants match full PHAST bits and the independent
turn-aware Dijkstra tolerance; prepared scratch capacities remain fixed.

Fresh measurements use the same release/worker/affinity settings as above, two independent
processes per tier, eleven samples of eight repetitions per process. The table reports the
mean of the two process means in microseconds for each complete source/target batch; setup
is excluded and measured separately. Source fingerprint:
`162f1096bd39043f293c5e672fed9788ebcae6cdc5621337ffea7fabd20d8ff1`.
Source snapshots, binaries, logs and analysis are under `$ARTIFACTS/net05-block`.
Command: `METRUM_CCH_BENCH_RESIDENTS=100000 RAYON_NUM_THREADS=8 taskset -c
0,2,4,6,8,10,12,14 "$TEST_BINARY" --ignored --nocapture benchmark_populated_cch_batches`
(use `10000` for 10k). Optional post-measurement diagnostic hours are excluded from timings.

| Tier / workload | Sources / targets | Buckets µs | Scalar RPHAST µs | Eight-source RPHAST µs | Existing point/full µs |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10k freight | 1 / 246 | 0.69 | 4.31 | 5.08 | Full: 4.54 |
| 10k open jobs | 271 / 4 | 25.30 | 38.02 | 23.62 | Point: 105.28 |
| 10k dense jobs | 271 / 63 | 67.04 | 153.45 | 45.03 | — |
| 100k freight | 10 / 2,210 | 125.32 | 195.26 | 130.64 | Full: 220.26 |
| 100k open jobs | 480 / 77 | 345.94 | 1,141.07 | 356.41 | Point: 21,097.08 |
| 100k dense jobs | 480 / 407 | 1,072.95 | 4,761.84 | 918.67 | — |

NET-04's open-job matrix reduces 36,960 point searches to 480 forward plus 77 target
searches (66× fewer), taking 61× less query time at 100k. Runtime pair-cache hits reduce
how often that full saving applies; the full economy measurements above remain decisive.

RPHAST is not adopted in production. Although blocking improves dense throughput, PHAST's
f32 grouping differs from the point/bucket contract: at 100k, 88 open-job and 286 dense-job
pairs change rounded seconds (maximum cost delta 0.0003052 s). This violates exact commute
ranking. Freight's closure contains 2,210 of 2,220 states, and target discovery plus closure
construction costs 12.30 + 0.99 ms, compared with 0.220 ms for the existing cached full sweep.
Its warm query saving does not repay preparation or preserve proposed-door coverage. Keep
the scalar/block implementations and comparison harness test-only; GPU and 1M remain deferred.

After removing the temporary phase timers, the routing build freshly passes all 2,304 library
and four growth integration tests (113 library diagnostics ignored). Its source fingerprint is
`92c59a1562167df4069c3e94d51e7cb456e23e53f273c0634a5421033d3c0c95`;
source snapshots, executables and the test log are under `$ARTIFACTS/routing-final`.
This validates the routing cleanup; subsequent ECON-13 fixes require their own checks.

##### Vehicle U-turn routing contract (`NET-07`, `NET-08`)

**Rule.** A car may reverse onto the road it arrived on only at a dead end (one arm). Bends,
pass-throughs and junctions of any size have no U-turn. Roundabouts are ordinary junctions,
so driving around one is the legal way to turn back. Junctions with explicit vehicle turn
connections keep following their whitelist. The lane builder already implements this rule
(`vehicle_junctions.rs` creates a same-edge connector only when the node has one arm).

**Mismatch (`NET-07`).** The router does not follow it. NET-03 keeps one CCH state per
unrestricted node because "unrestricted arrivals are equivalent". That holds only if every
turn, including the U-turn, is legal there. As a result, routes U-turn for free at every open
node, while movement finds no connector. Car routes also never check the final turn into the
detach lane: `network_path` skips connector validation for cars, and
`path_has_valid_vehicle_turns` checks only interior nodes. Cars whose attach and detach nodes
match are priced as a free zero hop without any connector check, and outbound border trips
quote without their origin edge. Each failed car drops its route and replans from the same
position, receiving the same route again.

Evidence: debug dump `dump-2026-10-14_20-28-16` (road generation 13) logs repeated
`trip failed ... no turn` events at 15 nodes over about 240 s, many repeating for the same
agents. At node 2 (three arms: edges 1, 2, 9), vehicle lanes 60/61 of edge 9 connect only to
lanes 8, 14 and 15. Routes ask for lane 61 → 63 (edge 9's opposite direction) and edge 9 →
edge 9. Nodes 16 and 26 are two-arm bends with the same pattern. The dumped graph has 125
nodes: 36 dead ends, 25 bends, 52 three-arm and 12 four-arm junctions.

**Why endpoint checks alone are insufficient.** Banning the U-turn only at the first or last
node moves it: the cheapest route then turns into a neighbouring open node, U-turns there for
free, and comes back. An exact route must forbid U-turns wherever the lanes do. In the CCH
that means every non-dead-end node needs one arrival state per incoming edge, the existing
restricted-node model with an "every turn except the U-turn" rule.

**Cost with the current ordering (2026-10-14).** Matched unprofiled release runs of
`benchmark_populated_cch_queries` at 10k residents with temporary hierarchy-size/build timers
(not retained), two alternating pairs, `RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14`,
working tree based on `abb37aa5` with the pending `TRAFFIC-02` changes:

| Variant | States | Filled arcs | Build ms | Cost query µs | Path query µs |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline (one state per open node) | 247 | 1,770 | ~0.8 | 0.68 | 0.85 |
| Split every non-dead-end node (this rule) | 892 | 23,564 | 16–18 | 6.5 | 6.8 |
| Split two-arm bends only | 251 | 1,778 | ~0.8 | 0.68 | 0.86 |

On the dump's road graph (temporary ignored test; 4,096 random car queries), splitting bends
only changes 125 → 150 states, 344 → 473 arcs, 0.17 → 0.22 µs per query and 0.21 → 0.24 ms
per build. The dead-end-only rule is therefore unaffordable with the current ordering.

**Ordering (`NET-08`).** `order.rs` runs nested dissection on road nodes, using median
coordinate cuts with junction separators. `topology.rs` then places each restricted node's
arrival states consecutively at that node's rank. That is appropriate for one state per
node. When junctions split, every separator junction contributes three or four states and
filled arcs grow roughly quadratically. A turn-aware ordering must order the expanded state
graph, so that separators cut roads (one or two states) rather than junctions. Turn-expanded
CCH in the literature typically costs about 2–3× the node-based query. That figure is
unverified here and is what `NET-08` must measure.

**`NET-08` first pass (2026-10-14, branch `u-turns`, uncommitted on `0ba23dc8`).** The router
splits every node with more than one arm, using `RegionGraph::open_node_allows_u_turn`. The
lane builder, trip ends and planner are unchanged (`NET-07`). Three ordering changes were
measured in sequence:

1. **State order.** `topology.rs` orders the arrival states themselves: nested dissection over
   the state transition graph, with each state placed 0.5 m inside its node along the arrival
   edge. Before, states inherited their node's rank.
2. **Minimum-cover separators.** The median cut's separator is the minimum vertex cover of the
   crossing links (maximum bipartite matching, König). Before, it was the smaller side's whole
   boundary. A cut across one road then costs that road's two arrival states.
3. **Flow cuts.** Four directions (x, z and both diagonals) per level. For each, the smallest
   vertex separator between the first and last quarter of the vertices, by unit-capacity max
   flow (inertial flow). It is kept when smaller than the median cover, or equally small and
   better balanced.

All are exact and deterministic: the ordering changes speed, never legality. The 35 pathing
tests pass at each step, including the randomized edge-state Dijkstra oracles, which now apply
the dead-end-only rule. The final state also passes 2,418 release library tests and the 4
growth integration tests.

Measurements:
- `benchmark_populated_cch_queries`: populated 10k/30k fixtures.
- `benchmark_saved_cch_queries`: a save's road graph, read-only via `METRUM_CCH_GRAPH_SAVE`,
  run on a copy.
- Both print `CCH_HIERARCHY`, `CCH_POPULATED_BENCH` and `CCH_BUCKET_BENCH`, each run as
  `RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" <test> --exact --ignored
  --nocapture --test-threads=1`.
- Medians of 11 passes: 4,096 mixed car/foot point queries; car buckets with 64 targets and
  256 sources (per-source µs).
- Baseline and the final variant ran twice in alternating order (shown: the second round,
  within about 5% of the first). The intermediate variants ran once.

| Variant | Graph | States | Arcs | Build ms | Cost µs | Path µs | Bucket µs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline (no split, node order) | 10k | 247 | 1,770 | 0.68 | 0.667 | 0.837 | 0.97 |
| | 30k | 642 | 6,321 | 2.66 | 1.632 | 1.961 | 1.88 |
| | dump | 125 | 344 | 0.19 | 0.176 | 0.289 | 0.23 |
| | 26-days | 161 | 396 | 0.24 | 0.174 | 0.296 | 0.25 |
| Split, node order | 10k | 892 | 23,564 | 16.0 | 6.546 | 6.797 | 5.10 |
| | 30k | 2,402 | 89,808 | 101.2 | 15.56 | 16.13 | 10.54 |
| | dump | 290 | 2,373 | 0.91 | 0.892 | 1.051 | 0.73 |
| | 26-days | 362 | 3,031 | 1.22 | 1.003 | 1.183 | 0.85 |
| Split, state order | 10k | 892 | 18,926 | 11.6 | 5.612 | 5.868 | 4.16 |
| | 30k | 2,402 | 62,626 | 50.6 | 12.15 | 12.60 | 8.34 |
| + minimum-cover separators | 10k | 892 | 15,517 | 10.0 | 3.761 | 4.026 | 2.83 |
| | 30k | 2,402 | 50,962 | 33.9 | 8.602 | 8.953 | 5.80 |
| + flow cuts (final) | 10k | 892 | 14,640 | 14.0 | 3.447 | 3.694 | 2.75 |
| | 30k | 2,402 | 48,435 | 51.7 | 7.864 | 8.341 | 5.58 |
| | dump | 290 | 1,766 | 2.35 | 0.637 | 0.775 | 0.44 |
| | 26-days | 362 | 2,165 | 2.86 | 0.623 | 0.786 | 0.48 |
| Final ordering, no split | 10k | 247 | 1,755 | 2.30 | 0.670 | 0.847 | 1.06 |
| | 30k | 642 | 6,163 | 5.68 | 1.751 | 2.022 | 2.05 |
| | dump | 125 | 264 | 0.78 | 0.133 | 0.240 | 0.18 |
| | 26-days | 161 | 313 | 1.08 | 0.133 | 0.249 | 0.18 |

The intermediate rows' saved-city values are in the same run logs and follow the same trend.
The final ordering cuts the split's cost from 9.5× to 4.8× the baseline at 30k (5.2× at 10k;
3.6× on the saved cities). Bucket sources cost 3.0× and builds about 20× (52 ms at 30k).
Without the split, the new ordering is neutral on the fixtures, 24% faster on the saved
cities, and builds 2–4× slower. This is still above the 2–3× literature range, and acceptance
is the user's decision. Further options:
- A stronger partitioner (repeated/multi-direction flow cuts, FlowCutter-style refinement).
- Contracting chains of two-arm bends before ordering.
- Running the `--perf-contract` runner to see what 5× routing cost means for whole-city
  hourly and daily time.

**Whole-city cost (`--perf-contract`, 2026-10-14).** Command:
`RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 python3 tools/perf_contract.py --binary
"$TEST_BINARY" --artifacts "$ARTIFACTS/<run>"`, two alternating pairs. Both executables read
identical copies of the existing snapshot cache, which was reused, not regenerated.

| Workload | Baseline (pair 1 / pair 2) | Final split variant | Change |
| --- | --- | --- | ---: |
| 10k hour | 17.57 / 16.91 ms | 18.98 / 18.99 ms | +10% |
| 10k day | 7.19 / 6.75 ms | 7.04 / 7.01 ms | ≈0 |
| 30k hour | 52.41 / 52.81 ms | 62.46 / 62.47 ms | +18% |
| 30k day | 35.46 / 34.58 ms | 41.63 / 41.05 ms | +18% |
| Fiscal stress 10k/30k hour | 27.6 / 76.5 ms | 27.8 / 78.1 ms | +1–2% |
| Isolated routing phase 10k/30k (4,096 queries) | 2.84 / 6.76 ms | 13.34 / 29.83 ms | 4.4–4.7× |

The failure list is unchanged: the pre-existing `PERF-03`–`PERF-05` budget and scaling
failures. The 30k daily budget, already failing at 35 ms against 30 ms, rises to 41 ms. CCH point
searches rise 11–12% in the baseline day (30k: 288,059 → 319,613). Arrival-specific source
contexts at every non-dead-end origin split the quote and endpoint caches, which previously
shared one key per node. Growth records differ only in treasury cents and, under fiscal stress at
30k, one vacant home and one unhoused household: different legal routes change commutes.

**`NET-07` implementation (2026-10-14, branch `u-turns`).** One rule,
`RegionGraph::open_node_allows_u_turn`, is read by the lane builder
(`vehicle_junctions.rs`) and the router (`VehicleTurns`, which `NET-08` already splits at
every non-dead-end node). Every addition below is O(1) or O(degree) per query and allocates
nothing.

- **Exit turn at the destination.** Point queries (`find_cost`, `find_path_in`, `find_path`)
  take `outgoing`, the edge left on after `end`. The backward search seeds only the arrival
  states whose turn onto it is legal. `CchGraph::target_context` canonicalizes it like
  `source_context`, so retained backward labels and quote keys stay exact. A start equal to
  its end costs zero only when the turn between the two ends is legal; otherwise the search
  returns the shortest legal loop.
- **Buckets.** Targets are (node, canonical exit context) pairs.
  `BuildingTripEnds::network_targets` supplies each end's frontage edge. Same-node targets
  are zero only when the turn is legal. The NET-05 RPHAST comparison benchmark keeps
  unconstrained exits, so it still compares like with like.
- **Planner.** Every car query passes both trip-end edges: quote keys and
  `network_seconds`, `network_path`, `cch_cost` (immigration now passes the destination
  frontage, border-out trips the origin frontage), and `network_leg_is_reachable`.
  Node-to-node logistics pricing, the benchmark helper and the debug car keep unconstrained
  ends.
- **Same-node car trips.** Pricing (`combine_trip_ends`, `price_trip_without_network`), route
  building (`build_exact_path_for_candidate`) and replanning (`plan_network_replan`) treat a
  shared node as a free zero hop only when `CchGraph::vehicle_turn_permitted` allows the
  turn. Otherwise the trip goes through the network query as the shortest legal loop, and
  replanning keeps that loop path.
- **Flow-field paths.** `path_has_valid_vehicle_turns(path, from_edge, to_edge, graph)` also
  checks the attach and detach turns.
- **Saved routes.** `AgentSystem::clear_routes_with_illegal_turns` runs after load. It drops
  any car route on the road whose remaining turns, including the final turn into its exit
  lane, the current rules forbid. The car then replans before reaching that turn. A car saved
  on a junction connector has committed to the one lane the connector leads into, so that
  lane's road is the incoming road at its next turn. Road edits already clear routes through
  edited nodes.

Correction to the earlier diagnosis: on a straight frontage road, banning junction U-turns
does not change which way a car departs. A door on the opposite-direction lane must be
reached through the far junction in either case. What changes is where the car turns
around: a dead end or a loop instead of a free U-turn at the nearest junction (building 651
in the dump). The departure direction flips when the cheap direction needs an illegal turn
into the exit lane further on, as in the departure regression below.

Tests (all pass; the four marked "fails before" fail on `9e70526f`, the commit before these
changes):
- `exit_constrained_routes_match_dijkstra_in_point_and_bucket_queries`: random graphs with
  whitelists, deleted edges, self-loops and one-way roads; every arrival and exit edge. Point
  costs match the edge-state Dijkstra oracle with the exit constraint. Witness paths have
  only legal turns, both trip ends included. Bucket paths equal point paths.
- `across_the_road_at_a_three_arm_junction_turns_at_a_dead_end` (fails before): building
  651's shape.
- `bend_never_hosts_a_u_turn` (fails before).
- `departure_direction_flips_when_the_cheap_side_needs_a_u_turn` (fails before): the door is
  on the next road B–C, served only by the C→B lane. Leaving east and turning back at the
  three-arm junction C is about 200 m but illegal. The car now leaves west around the block
  (about 500 m) instead of the 4.2 km stub loop. Every departure and arrival rank pair yields
  a lane-drivable plan.
- `car_facing_away_from_its_destination_turns_back_legally` (fails before): end-to-end
  movement; the car drives home with no trip failure.
- `loaded_routes_needing_a_forbidden_turn_are_dropped_for_replanning`.
- `loaded_routes_from_a_junction_connector_check_their_next_turn` (fails before the
  2026-10-15 follow-up below).
- The planner tests' `drivable` check follows only the lanes' own continuations and junction
  connectors.

Dump replay: `replay_saved_city_without_impossible_turns` (`METRUM_REPLAY_SAVE`,
`METRUM_REPLAY_MODS`, `METRUM_REPLAY_DAYS`). It loads a copy of a save with copied asset
packs and runs ordinary frames. On `dump-2026-10-14_20-28-16` (a byte-identical copy,
checked by SHA-256, with a byte-identical copy of its asset packs), the final build gives 0
trip failures over 1 and 3 simulated days. Without the load-time route check, the same replay logged 4 `no turn`
failures, once per car with a route saved by the old planner, then replanned cleanly.

Follow-up (2026-10-15). Replaying copies of two further saves, `zoned-city-23-days` and
`zoned-city-26-days`, the first logged one `no turn` failure: a car saved on a junction
connector, entering the road to node 2, whose old route U-turned at node 2 into its exit
lane. The load check treated a connector's incoming road as unknown and skipped that turn.
It now uses the road of the lane the connector leads into. All three saves then replay 1 and
3 simulated days with 0 trip failures, and `perf_contract_saved_agent_plans` passes on each.

Cost (2026-10-15). Matched unprofiled release runs, two alternating rounds, `9e70526f` against
the `NET-07` working tree, `RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14`:
- **Hierarchy and point queries** (`benchmark_populated_cch_queries`,
  `benchmark_saved_cch_queries`): states, arcs and build time are identical. Point queries
  are unchanged to 6% faster (30k cost: 7.74/7.84 → 7.65/7.77 µs; dump: 0.63/0.64 →
  0.59/0.60 µs).
- **Bucket sources** cost 9–10% more on the fixtures and about 20% more on saved cities.
  The synthetic targets now carry exit edges, so one node can hold several target rows; the
  two versions' bucket workloads are therefore not identical.
- **`--perf-contract`** (identical snapshot copies, reused not regenerated): whole-city time
  is unchanged within noise. 30k baseline hour 62.69/62.43 → 62.53/62.56 ms, day
  41.13/41.47 → 42.49/40.86 ms; 10k hour 19.34/18.92 → 18.66/18.90 ms; fiscal stress
  −1–3%; isolated routing phase 29.83/29.78 → 29.75/29.49 ms at 30k.
- The failure list is unchanged (the pre-existing `PERF-03`–`PERF-05` limits). CCH point
  searches rise 8% (30k day: 319,613 → 346,350), from exit-specific quote keys and legal
  loops. Fiscal-stress growth records are identical; baseline records differ in treasury
  cents.

##### Flow-field and CCH precedence

In this redesign, CCH is the authoritative router. Flow fields are an optional fast path only.
The current `NET-03` implementation and validation status are described above. The following
`AUDIT-01` construction/query descriptions and measurements record the preceding hierarchy;
they are historical evidence, not validation of the replacement.

CCH construction maintains exact neighbour-triangle counts for minimum-fill ordering, rejects
stale heap scores in both directions, and updates every affected score only after completing a
contraction. Node-ID ties are deterministic. Independent initial counts use Rayon; dependent
elimination stays sequential. A full-recount oracle checks the incremental accounting.

Shortcut topology retains all lower-triangle alternatives instead of keeping the first path or
discarding a detour merely because a direct arc exists. Boundary-edge states merge only where an
unrestricted endpoint makes them equivalent; restricted endpoints and travel-mode masks remain
distinct. Metric customization evaluates choices in lower-endpoint-rank order, restores the selected
concrete boundary edges, and allocates nothing: O(base edges + shortcuts + retained alternatives).
Queries stop each search frontier independently. All-pairs grid, weighted/mode/direction,
re-customization and turn-aware Dijkstra regressions protect exact costs and turn legality.

`AUDIT-01-P1` corrects nondeterministic query meetings: a five-node junction fixture with two
equally cheap permitted turns returned either `[0, 4, 1, 3]` or `[0, 4, 2, 3]` for identical inputs.
Meeting checks now reuse graph adjacency and look up only incident concrete boundary-edge states,
plus the initial origin/destination state. This costs O(degree) expected map lookups per expanded
state, replacing a scan of the entire opposite search map. The graph and CCH must share a topology
revision. Equal discovered meeting costs use node, incoming-edge and outgoing-edge IDs; heap ties
use cost, node and edge IDs with consistent total ordering/equality. Strict predecessor improvement
and independent frontier stopping remain unchanged.

`CchGraph::costs_from_each` gives the costs from a few start nodes to every node (`ALLOC-04`). For
each start it runs `find_path`'s forward search to exhaustion, then one pass down the hierarchy in
decreasing rank. That pass covers the backward query's states, with the same turn checks and the
same meeting rule (a loop at a node is relaxed to a fixed point). Each cost matches the point
query except for float summation order, because the pass adds arcs from the start outward.
Shared setup is O(A log A) over the A downward arcs, plus O(A × node degree) turn checks per start;
starts run in parallel. A regression compares it with `find_path` for every pair on restricted,
looped, one-way, mixed-mode and re-customized grids.

Both search directions share one `HashMap::entry` update and retain the already-read expansion
distance. Both path halves reuse one shortcut stack and output buffer; per-shortcut temporary
vectors are removed, and the final node vector reserves its exact size. Query heaps/maps and the
returned path still allocate; this change does not claim allocation-free routing. The obsolete
public A* state, duplicate queue priority and unused maximum-speed field/scan are removed.

All 1,757 release tests pass (53 ignored), including repeatable route identity, heap-order contracts,
and the independent turn-aware Dijkstra comparison expanded to every incident origin edge and the
origin sentinel. The regression fails against the preceding implementation.

Five alternating unprofiled release pairs run
`simulation::pathing::cch::ordering_tests::benchmark_cch_queries`. Each connected grid has 16 fixed
queries, one validation/warm-up pass and 21 measured batches. Graph/hierarchy construction is outside
timing. Every query is checked against Manhattan cost/distance; all builds/runs match shortcut count,
aggregate route cost, distance and path-node count.

| Grid side | Nodes | Before median µs/query | After median µs/query |
| ---: | ---: | ---: | ---: |
| 8 | 64 | 5.205 | 4.969 |
| 16 | 256 | 22.162 | 20.658 |
| 32 | 1,024 | 171.257 | 156.323 |

These are medians of process medians, using Rust 1.98.1, release lib-test executables,
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, CPU affinity `0`, with no competing compilation/tests.
Before/after executable SHA-256 values are
`fb8709ef79dfa43214ef50124c0c471baf8a7c7681bd76b67c297b97a7a7e296` and
`e4afc9be6f64b21e5800dda4b5060f6df8e18d0bf1cb7a87099f057c4d6f5b9d`. The measurements cover these CCH
queries, not hierarchy rebuilds, whole movement ticks or city-wide road edits. Initial
meeting/stack-only revisions regressed smaller queries; separate `perf record` runs located repeated
hashing before the shared entry update. Those preliminary timings and profiles are retained for
traceability and are not acceptance timings.

`AUDIT-01-P2` keeps order/rank arrays and shortcut endpoint indices local to construction. The
unused elimination tree and its per-shortcut maintenance are removed. Finished routers retain
only shortcuts, upward query indices, metric alternatives/order and the debug generation. Direct
edge directions share one creation loop, preserving forward-then-backward ID order. The existing
full-recount oracle also checks the returned order/rank inverse. Query and customization algorithms
are unchanged; all 1,757 release tests pass (54 ignored).

Five alternating release pairs run
`simulation::pathing::cch::ordering_tests::benchmark_cch_build_storage` with eight Rayon workers
on CPUs `0,2,4,6,8,10,12,14`. Each case builds the source grid outside timing, performs one warm-up,
then measures 21 builds and build/drop cycles. The latter includes destruction because construction
scratch now drops before the router is returned. Every process fingerprints all shortcut fields,
upward indices, metric alternatives and customization order; those products match across builds.

| Grid side | Retained bytes before / after | Build median ms before / after | Build/drop median ms before / after |
| ---: | ---: | ---: | ---: |
| 8 | 156,920 / 139,200 | 0.17645 / 0.17201 | 0.18727 / 0.18090 |
| 16 | 904,376 / 807,872 | 1.24830 / 1.24711 | 1.31684 / 1.30704 |
| 32 | 7,969,016 / 7,448,768 | 11.08564 / 11.11961 | 11.60445 / 11.65710 |

Retained bytes count the router header and vector capacities, including nested vector payloads;
they exclude allocator metadata and are not RSS or peak-memory measurements. The reduction is
6.5–11.3%; construction/cycle costs remain approximately unchanged, with a 0.45% increase in the
largest cycle median. This is a storage cleanup, not a demonstrated construction speedup.

Five separate CPU-0/single-worker query pairs reuse `benchmark_cch_queries`. Before/after medians
are 4.857/5.004, 20.747/20.808 and 156.667/157.759 µs for sides 8/16/32, respectively; cost,
distance, path-node count and shortcut count match. Query differences range from 0.3–3%; no query
speedup is claimed. All runs use Rust 1.98.1, release lib-test builds, `METRUM_DEBUG=0`, and no
competing compilation/tests. Before/after executable SHA-256 values are
`1b0e02280c2bd55bb87434a111e0721d5c99405f9aaa386b281646376653ed33` and
`c81df2ef71f842235e3c8d2979ee404492645f7bd8def4544a2b02924b608a4f`.

`AUDIT-01-P3` separates vehicle whitelists from pedestrian routing. Previously a vehicle rule sent
a walking trip with shortest cost 12 onto a route costing 60. Contraction now retains pedestrian-only
alternatives across blocked vehicle turns; those alternatives do not need vehicle boundary keys.
Queries for the pedestrian mode bypass vehicle turn checks, including the supplied origin edge.
Car queries retain their turn restrictions. The flow-field fast path applies
`path_has_valid_vehicle_turns` only to cars and continues to validate actual sidewalk connectors
for walkers. The regression compares all pedestrian pairs against independent shortest costs,
all incoming-edge car contexts against turn-aware Dijkstra, and both modes after customization.
The existing connector regression now also checks successful pedestrian flow-field reuse across
a vehicle-restricted junction before rejecting the same route when its sidewalk is disconnected.

Completed hierarchies return spare construction capacity because customization never grows their
topology arrays. This adds O(nodes + shortcuts + alternatives) build finalization, with possible
allocator moves, and no work or allocation in queries/customization. Finalization stays on the
constructing thread: an eight-worker shrinking implementation was measured and rejected because
it increased construction costs substantially. Independent ordering setup still uses Rayon.

Three alternating release pairs reuse the construction/storage and query benchmarks above, with
`METRUM_CCH_BENCH_GRAPH=car`, `mixed` and `restricted`. `mixed` enables both road modes; `restricted`
also limits the central junction to one explicit vehicle turn. Construction uses eight physical
P-cores / Rayon workers; queries use CPU 0 and one worker. Graph setup, oracle checks and product
fingerprints remain outside timing; each process takes 21 samples. All car cost/distance/path-size
products match. Unrestricted hierarchy fingerprints also match; restricted topology intentionally
gains the formerly missing pedestrian alternatives.

| Graph | Side | Retained bytes before / after | Build median ms before / after | Car query median µs before / after |
| --- | ---: | ---: | ---: | ---: |
| Mixed, open | 8 | 139,200 / 90,448 | 0.18845 / 0.17485 | 4.952 / 5.039 |
| Mixed, open | 16 | 807,872 / 657,344 | 1.36365 / 1.28718 | 20.531 / 20.693 |
| Mixed, open | 32 | 7,448,768 / 5,076,720 | 12.36192 / 11.28989 | 157.870 / 157.240 |
| Mixed, restricted | 8 | 144,512 / 107,824 | 0.20476 / 0.21238 | 5.448 / 5.495 |
| Mixed, restricted | 16 | 797,104 / 850,352 | 1.35109 / 1.69383 | 20.479 / 21.167 |
| Mixed, restricted | 32 | 7,454,368 / 6,810,784 | 12.04200 / 13.83924 | 157.214 / 164.527 |

The car-only control has the same retained byte counts as the open mixed graph, build medians
0.18291/0.17950, 1.36780/1.29123 and 12.36306/11.27561 ms, and query differences of +0.5–1.3%.
The accepted tradeoff is additional mode-correct routing topology at restricted junctions:
the 16-side fixture retains 6.7% more memory and builds 25.4% slower; the 32-side fixture retains
8.6% less memory and builds 14.9% slower, with car queries 4.7% slower. This is not a claim of a
universal speedup or city-scale timing acceptance. The memory accounting excludes allocator
metadata, RSS and peak build storage. Initial fixes without capacity finalization and the slower
parallel finalizer are preserved separately, rather than reported as the accepted build.

The final executable hashes are `be6be668c03ade4dae20adb401ba06f30ce2e25b2475c35fb2f374cbb9137417`
before and `c0095bb8ab4f07de42d4b45587c21c2d90225db7eb2da48f2fd96194f78d264d` after. Commands use
Rust 1.98.1, `METRUM_DEBUG=0`, offline release lib-test builds and no concurrent builds/tests.

##### Pedestrian direction audit measurements

`AUDIT-01-P4` shares `Edge::traversal_flags` between CCH and flow fields. Every allowed
walking edge is bidirectional, including sidewalks on one-way roads and dedicated footpaths
with zero vehicle lanes. Other modes require a vehicle lane in the requested direction.
Direct CCH arcs store that direction's mask, so adding a legal reverse walking arc cannot
accidentally admit a car. Vehicle turn restrictions and actual pedestrian connector validation
remain part of the preceding contracts.

If the direct graph contains walking-only arcs, compound states use disjoint walking/vehicle
masks. This combines walking alternatives that previously differed only in whether their
constituent roads also admitted cars. Fully shared bidirectional networks retain their combined
hierarchy. The direct-mask pass is O(E), with no new persistent index; contraction and metric
customization retain their existing per-triangle/per-alternative complexity.

The existing small-grid oracle now independently allows both walking directions and checks
all pairs through three congestion phases. The flow regression covers forward-only and
backward-only roads, zero-lane footpaths, both modes, both destinations and deleted edges.
Both routers fail the corresponding baseline tests. Exact source-building/endpoint ties are
also checked in both source insertion orders. Five unused APIs are removed: three edge-sampling
methods, `next_hop` and the unconnected `look_ahead` IDM hook. The path test now owns the
linear-chain assertions; copied setup and the machine-dependent 5 ms unit assertion are removed.

An initial attempt to walk the existing adjacency directly preserved flow products but slowed
the 99,856-node fixture from 9.856 to 15.748 ms. The compact reverse-edge cache is therefore
retained: its endpoint/cost records avoid random reads of the much larger `Edge` records.
This is temporary search data, with O(V + E) storage, and is not a second authoritative graph.
Five final alternating CPU-0 / one-worker release pairs give:

| Nodes | Flow build before / after (ms) |
| --- | --- |
| 1,024 | 0.067 / 0.068 |
| 10,000 | 0.807 / 0.818 |
| 99,856 | 9.667 / 9.717 |

Each sample includes the complete field build and temporary-cache destruction, with graph setup,
field-result destruction and functional checks outside timing. All next-node/nearest-building
fingerprints match. Each process records 21 samples; graph setup is reported separately.

Three separate alternating CCH pairs use eight physical P-cores for construction and CPU 0 for
queries, with 21 samples and independent car-cost checks outside timing. The one-way fixture
makes every fourth edge forward-only for vehicles; all edges permit walking.

| Modes / grid | Retained bytes before / after | Build before / after (ms) | Car query before / after (µs) |
| --- | --- | --- | --- |
| car / 8×8 | 90,448 / 90,448 | 0.174 / 0.176 | 5.124 / 5.000 |
| car / 16×16 | 657,344 / 657,344 | 1.274 / 1.302 | 20.928 / 20.895 |
| car / 32×32 | 5,076,720 / 5,076,720 | 11.687 / 11.623 | 158.804 / 158.416 |
| mixed / 8×8 | 90,448 / 90,448 | 0.176 / 0.172 | 5.075 / 5.076 |
| mixed / 16×16 | 657,344 / 657,344 | 1.286 / 1.306 | 20.955 / 20.964 |
| mixed / 32×32 | 5,076,720 / 5,076,720 | 11.235 / 11.758 | 159.242 / 158.571 |
| one_way / 8×8 | 71,456 / 138,320 | 0.157 / 0.261 | 4.008 / 4.234 |
| one_way / 16×16 | 545,072 / 1,102,640 | 1.143 / 2.286 | 19.881 / 22.869 |
| one_way / 32×32 | 4,369,760 / 9,036,112 | 10.247 / 20.961 | 130.549 / 156.627 |

Bidirectional hierarchy fingerprints and retained sizes match; car query cost/distance/path-length
products match in every variant. Correct one-way walking requires a distinct set of alternatives:
the largest fixture retains about twice the storage and adds 10.7 ms to construction and 26.1 µs
to a car query. Partitioning removes roughly one third of the initial corrected hierarchy's
13,511,504 retained bytes. These are cold routing-build/query measurements, not a whole-city
frame-time or million-population performance acceptance. No RSS or peak-memory claim is made.

The final test executable hashes are
`9ffdce6b1549ceba631d35ba5d763b025f75aee70b959ae04a9dd3ed9eafed9c` before and
`b89ee741845d301fb02e831bc8c3226389c049d5e1c5b8ff2f9e263338d4f519` after. Runs use Rust 1.98.1,
`METRUM_DEBUG=0`, offline release lib-test builds, `RAYON_NUM_THREADS=1` / `taskset -c 0` for flow
and queries, and `RAYON_NUM_THREADS=8` / `taskset -c 0,2,4,6,8,10,12,14` for CCH construction. The
exact ignored entries are `simulation::pathing::flow_field::tests::benchmark_flow_field_build` and
`simulation::pathing::cch::ordering_tests::{benchmark_cch_build_storage,benchmark_cch_queries}`; CCH
variants use `METRUM_CCH_BENCH_GRAPH=car|mixed|one_way`. No builds, tests or source mutations run
concurrently with accepted measurements.

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
   - car paths pass `CchGraph::path_has_valid_vehicle_turns(path, graph)`
   - pedestrian paths have the actual lane connectors required for their selected approaches
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

Walking network cost is priced at walking pace (`TRAFFIC-05`): each walkable arc costs
`base_cost * max(speed_limit, MIN_ROUTE_SPEED_MS) / AGENT_WALK_SPEED_MS`, its length over 1.4 m/s.
The CCH keeps a second, walking metric beside the driving one, customized once per build without
congestion, and `FOOT`-only queries, sweeps, flow fields and the sidewalk-connector fallback
searches read it. The exact `FOOT` mask always gets its own CCH profile, and walkers leaving a
restricted junction are seeded from the per-edge walking cost. Before, walkers were priced at the road's speed limit, as fast as a car, so with
`CAR_MODE_CHOICE_OVERHEAD_S` (180 s) on top of driving, walking won nearly every trip. Now driving
wins once the trip is longer than about 280 m.

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

### Earlier Centre-Surface Orientation Verification (2026-09-10)

These measurements describe the earlier centre-sample implementation, not current footprint
support. Its planar orientation tests passed, but later road/apron breakover diagnostics showed
buried mesh contacts despite a correctly grounded origin.

That implementation used the same supporting surface for height and basis, with no
extra spatial index, geometry rebuild, route field or per-query allocation. Work stays bounded
to nearby road/site owners and triangles in containing CDT cells. Height-only specializations
skip normals; camera-culled and lane-bound agents do not pay the new off-lane orientation cost.

At that revision, `cargo test --release`: **1,652 passed, 7 ignored**. Tests covered both windings and
highest-triangle selection, analytic visual-heightfield gradients with world-unit scaling,
orthonormal pitch/roll bases with unchanged yaw, and 128 entry/exit transforms against exported
Kuopio yard triangles. Pending terrain still cannot tilt a flat support pad. The portable yard
asset now uses the required `entrance/main` name; directional tests explicitly rebuild entrances
and supply valid lane attachments. The reference SQLite remains unchanged. `cargo check
--all-targets`, `cargo doc --no-deps`, release build, formatting and diff checks pass without Rust
warnings. The deployed library passes the headless `road_junction_preview_test.gd` bridge smoke
test; vehicle orientation itself is checked in Rust snapshot transforms, not by that road test.

Matched unprofiled runs used Rust 1.98.1, `RAYON_NUM_THREADS=24`, working-tree base
`91af8afca4eb2e9a493ab3bc7f2025a2b87195d9`, with no concurrent builds or Godot runs.
Final `$TEST_BINARY` SHA-256:
`3e8806fc9b647b0aaf2e82f828021a8478d3238c4b4b94cf7a4a885b4dfbf901`.

```bash
for measurement in graded_yard_access_snapshot_benchmark graded_yard_height_query_benchmark populated_paved_road_plan_scaling; do
  RAYON_NUM_THREADS=24 $ARTIFACTS/metrum-angle-baseline-tests "$measurement" --ignored --nocapture --test-threads=1
  RAYON_NUM_THREADS=24 $TEST_BINARY "$measurement" --ignored --nocapture --test-threads=1
done
```

Snapshot workload retains the existing 76 Kuopio positions, equal cars/walkers and ingress/egress,
10 warmups and 100 recycled snapshots per count, with setup/compilation outside timing. Its
minimal agents have no cached route attachments; real directional headings are covered by the
new correctness test separately. The asset anchor-name correction changes no fixture geometry.

| Visible access agents | Baseline p50 / p90 (ms) | Final p50 / p90 (ms) |
| --- | --- | --- |
| 100 | 0.2297 / 0.2383 | 0.2384 / 0.2457 |
| 1,000 | 0.9269 / 0.9404 | 0.9408 / 0.9727 |
| 10,000 | 7.9157 / 7.9827 | 7.9367 / 8.1087 |

The 76-point height-only query workload (10 × 100,000 queries) measures 749.1 → 582.7 ns p50,
751.0 → 595.9 ns p90. Snapshot overhead is small in this workload; these are not full-game
frame-rate guarantees. The populated-road check repeats 100 local plans at 0/1,000/10,000/100,000
background buildings, up to 600,024 agents, 100,004 parcels and 391 roads. All local products
remain identical. Compile p50 before → after is 20.078 → 20.694 / 20.128 → 20.200 /
20.638 → 20.458 / 21.258 → 21.792 ms, respectively; final worker p50 spans 21.41–22.88 ms,
compile p95 22.80–30.08 ms. Separate one-time edit snapshots cost 0.0064/0.0511/0.3698/3.3858 ms.
Repeated local work does not grow proportionally with the background city.

Deployed library SHA-256:
`7874b0f50ac54935ead1638f676278abf9661290630fa22e102bd0d56c1de1d0`.

### Rigid Vehicle Support Verification (2026-09-10)

Current verification: `RAYON_NUM_THREADS=24 cargo test --release --lib` passes **1,655 tests,
8 ignored**. Footprint tests cover mirrored rising/falling road–apron–pad transitions, asymmetric
model bounds, exact planar grades and yaw, compiled Kuopio paving contacts, post-interpolation
XZ, unchanged lane transforms, malformed batch rejection, and recycled snapshot/ID/flag alignment.
`cargo check --all-targets`, `cargo doc --no-deps`, formatting and diff checks pass without Rust
warnings. The original Kuopio SQLite is unchanged; no new saved city is needed.

`godot/tests/vehicle_ground_support_test.gd` loads all five real vehicle meshes and exercises the
production renderer plus native solver. Both headless and Xvfb/OpenGL runs pass **4,840 poses**
on mirrored uphill/downhill transitions, entry/exit, busy-core fallback and ownership handoff.
2,480 poses are tilted; minimum actual mesh-bottom contact clearance is 0.020000 m. The fixture
requires up to 0.4236 m origin lift above its centre surface, so it cannot pass with the earlier
centre-only solve. The rendered run also checks actual MultiMesh transform readback. The test
is included in `./run.sh --test`; headless mode checks cached uploaded poses, not dummy-renderer
GPU readback. These are geometric regressions, not an exact replay of the screenshot's trip.

Matched unprofiled snapshot comparison uses the same 76-position mixed-access workload above, Rust
1.98.1, 24 Rayon workers and base `91af8afca4eb2e9a493ab3bc7f2025a2b87195d9` plus working-tree
changes, with builds and other engine/benchmark runs stopped during timing.

| Visible mixed access agents | Centre-only p50 / p90 (ms) | Footprint p50 / p90 (ms) |
| --- | --- | --- |
| 100 | 0.2339 / 0.2419 | 0.4144 / 0.5665 |
| 1,000 | 0.9373 / 0.9600 | 1.3178 / 1.7300 |
| 10,000 | 7.9435 / 8.0123 | 8.3231 / 10.6078 |

The added footprint queries increase CPU work and tail latency; parallel buffer filling keeps
this workload's snapshot median increase below 0.4 ms. This is not a claim of zero overhead.
The separate current Kuopio support-batch benchmark (all cars, default support bounds, 10 warmups,
100 samples, setup excluded) measures p50 / p90 **0.4985 / 0.7690**, **1.0178 / 1.2914** and
**5.8277 / 6.2374 ms** for 100/1,000/10,000 cars. It excludes GDScript and GPU work.

The rendered update benchmark includes production GDScript interpolation/cache work, native
support and MultiMesh buffer upload, using the real sedan over the synthetic transition rather
than CDT yards. Godot 4.7.2, Xvfb, OpenGL compatibility, llvmpipe; 10 warmups and 100 attempts.
At 100/1,000/10,000 all-access cars, p50 / p90 is **0.286 / 0.380**, **2.601 / 5.558** and
**15.872 / 17.957 ms**. Busy-core attempts (1/0/1) are excluded from these grounding timings and
reported separately; correctness tests exercise the supported-snapshot fallback. The 10,000-car
stress case consumes roughly a frame budget by itself, so these are not full-city FPS guarantees.
Ordinary lane cars skip footprint probes. No road-planning path changed; the earlier populated
locality result above is historical, not rerun for this rendering-only correction.

```bash
RAYON_NUM_THREADS=24 $ARTIFACTS/metrum-support-baseline-tests graded_yard_access_snapshot_benchmark --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 $TEST_BINARY graded_yard_access_snapshot_benchmark --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 $TEST_BINARY graded_yard_vehicle_support_batch_benchmark --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 xvfb-run -a godot --display-driver x11 --path godot --rendering-method gl_compatibility --script res://tests/vehicle_ground_support_test.gd -- --benchmark-vehicle-support
```

Deployed library SHA-256: `7e17c3d1fe490619bd2cf86a1c36cacf42b87b80391a0c493ba0344abee16d68`.

## Access Movement Audit Measurements (2026-09-12)

`AUDIT-01-A8/A9` consolidate the movement column constructor, route application and recovery
resets, and remove a redundant segment-selection loop. Selection already skips reached vertices;
the remaining movement code keeps the same direction, distance and completion arithmetic. The
new constructor exposes the same fifty columns without allocation and is inlined at dispatch.
Route application moves the existing path buffer. Local access still examines at most four
points, so its per-agent complexity remains O(1); no population-sized state is added.

The existing `agent_benchmark` access fixtures run one 0.016-second tick from a fresh legal
opposite-side car access state. Setup, graph/lane construction and allocation are outside the
measured closure. These fixtures concentrate all agents at the same building; they measure
movement dispatch/access work, not normal city density or whole-game frame rate. The unchanged
Criterion settings use one second of warmup, twenty samples and five seconds of measurement.
Three alternating release process pairs run for each worker setting, with no concurrent builds
or other benchmarks. The table reports the median of the three process medians.

| Workers | Agents | Egress before → after, ms | Ingress before → after, ms |
| --- | --- | --- | --- |
| 8 | 1,000 | 0.087666 → 0.078476 | 0.047699 → 0.046742 |
| 8 | 100,000 | 5.394964 → 4.505677 | 0.974405 → 0.892640 |
| 1 | 1,000 | 0.080095 → 0.066453 | 0.072790 → 0.062632 |
| 1 | 100,000 | 6.035963 → 4.730587 | 5.330591 → 4.437969 |

Run `BINARY --bench 'AgentSystem::tick_access/access_(egress|ingress)_car/(1000|100000)$'
--noplot` with `RAYON_NUM_THREADS=8` and `taskset -c 0,2,4,6,8,10,12,14`, or one worker on CPU 0.
Set `CRITERION_HOME` to a separate output directory for each process. Correctness is verified
separately by the 1,753-test release suite, including repeated vertices, exact corners,
zero/multi-segment steps, reverse traversal, watchdog and border-route recovery.

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
