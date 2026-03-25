# Metrum Rise — Project State

**Scale target**: ≥ 1,000,000 concurrent agents across a multi-city region. City tiles are variable size (default 20 km × 20 km, player-configurable up to at least 100 km × 100 km); cities are connected by highways, rail, ships, and air routes via a single unified `RegionGraph`. Background cities run as statistical models; only the active city runs full agent simulation.
**Current milestone**: v0.01 — playable and correct at 10,000 agents, 500 buildings, 50 road edges, ≥ 30 FPS.

Severity tags: `[BLOCKER]` = must fix before v0.01. `[BUG]` = correctness failure, fix in v0.01. `[v0.01]` = strong target for v0.01 quality. `[v0.1]` = 100k-agent milestone. `[v1.0]` = 1M-agent milestone.

See [`docs/analysis.md`](analysis.md) for a detailed algorithmic and data-structure analysis, including scaling assessment for the 1M-agent target and comparisons to alternative approaches.

## Core Principles

**Performance first.** The 1M-agent target is the fixed constraint that all other decisions must satisfy. A feature that works correctly but cannot scale is not done — it is a future blocker.

**Document as you write.** Every new public item must have a `///` doc comment at the time it is written. Every source file must have a `//!` module-level header. `#![warn(missing_docs)]` is enabled — a missing doc on a public item is a compiler warning. Check coverage with `cd rust && cargo doc --no-deps 2>&1 | grep "warning\[missing_docs\]" | wc -l`.

**Reuse before you build.** Before adding any new system, data structure, or abstraction, check whether an existing one already covers the need:
- Need a spatial grid? Use `DataGrid<T>`.
- Need a spatial query? Use the 512 m chunk index (or extend it).
- Need bulk parallel iteration? Use `rayon::par_iter` over the existing SoA layout.
- Need to store per-edge or per-cell state? Add a field to the existing struct rather than creating a parallel collection.

Adding a new structure when an existing one fits is never neutral — it is a maintenance cost, a memory cost, and a future source of divergence. The default answer to "should I add a new X?" is no.

---

## Implemented Systems

### Time
- Fixed-timestep tick engine in `simulation/core/time.rs`.
- Supports play, pause, and fast-forward from Godot.
- One in-game day per fixed real-time interval; speed multiplier configurable.

### Terrain
- `simulation/terrain/` — heightmap as a flat `DataGrid<f32>`.
- Raycasting against the height grid for terrain-picking and road grade.
- Taubin smoothing (50 iterations) applied per edge on placement and sculpt.

### Water
- `simulation/water/` — shallow-water equations (SWE), parallelised with Rayon.

### Road Network
- `TransitGraph` — refactored into a modular package in `simulation/network/graph/`.
    - `data.rs`: `Node`, `Edge`, and `TransitGraph` struct definitions.
    - `spatial.rs`: 512 m spatial edge grid and 16 m node grid logic.
    - `topology.rs`: intersection detection, edge splitting, and node merging.
    - `rebuild.rs`: batch remapping, soft-deletion compaction, and intersection clipping.
    - **Road Network (`TransitGraph`)**: Adjacency-list based directed graph with spatial acceleration. Pathfinding via HPA*. Now supports multi-modal queries via `allowed_mask`.
- `TransitNetwork` (`network/mod.rs`) — `add_road`, `split_edge`, `merge_nodes`.
- **Unit tests** (`simulation/network/test_topology.rs`): `add_road` (bidirectional adjacency, 100 m subdivision logic), `split_edge` (physical length summation, node sharing, zoning/building migration), `compact_edges` (index remapping consistency for agents and buildings).
- Topology: intersection detection and edge splitting in `topology.rs`.
- Road and junction mesh generation in `network/render/road.rs` (405 lines).
- Soft deletion with compaction: edges marked `deleted = true`; `compact_edges()` removes them and remaps all indices (agents, zoning, routing graph).
- Lane types and one-way rules in `network/types.rs`.
- Edge geometry is 3D (`Vec<Vector3>`) — grade-separated roads are natively representable as elevated or depressed polylines. Node snapping uses 3D Euclidean distance, so bridge abutments and underpass nodes with ≥ 2 m vertical separation will not snap together.
- **No `EdgeClass` yet**: the renderer assumes all edges are ground-level. Bridge decks and tunnel bores require an `EdgeClass` field and a renderer branch. See Backlog.

### Zoning
- `simulation/grid/zoning.rs` (470 lines) — edge-aligned zoning cells, 10 m × 10 m.
- Cells extend up to 10 cells deep (100 m) from the road sidewalk edge, starting 5 m from road centreline.
- Obstruction check (`is_cell_obstructed`): 5-point sampling (4 corners + centre) per cell with asphalt collision and Voronoi ownership test.
- Obstruction cache correctly wired: `recalculate_obstructions` is parallelised with Rayon and spatially invalidated on nearby road edits.
- Zone types: Residential, Commercial, Industrial, Office, Mixed.

### Configuration
- `simulation/core/config.rs` — `MapConfig` struct replacing all hardcoded map-size constants. Fields: `width_m`, `height_m`, `env_cell_m` (environmental grid cell size, default 40 m), `zone_cell_m` (zoning cell size, default 10 m).
- All `DataGrid` initialisations and grid-dimension calculations (`PollutionSystem`, `NoiseSystem`, `DesirabilitySystem`, `ZoningSystem`) derive their dimensions from `MapConfig` at construction time. No hardcoded grid sizes remain in simulation code.
- Benchmark mode passes the default 20 km × 20 km config; player-configurable map sizes are supported without code changes.

### Environmental Grids
- `simulation/grid/pollution.rs` — industrial emission (+5/tick) + 4-neighbour diffusion (explicit finite-difference), decay ×0.995, parallelised with Rayon.
- `simulation/grid/noise.rs` — traffic noise diffusion, parallelised.
- `simulation/grid/desirability.rs` — composite formula: `50 − pollution × 2 − noise × 1.5`, parallelised.
- All three use `DataGrid<f32>` with dimensions calculated dynamically from `MapConfig` (default 500 × 500 at 20 km); bilinear upsampled for display.
- `PollutionSystem` and `NoiseSystem` use pre-allocated `swap: DataGrid<f32>` fields; each tick calls `std::mem::swap()` instead of `clone()` — hot-path allocation is zero. `DesirabilitySystem` computes derived values from the pollution and noise grids directly and never reads from its own previous state, so no swap buffer is needed there.

### Buildings
- Desirability gate enforced (> 50). Spawn throttle active (max 10 buildings per tick).
- Rendered via MultiMesh instancing: one draw call per zone type.
- Building deletion via swap-remove, O(1).
- Save/load via `serde_json` — **not yet wired end-to-end**.
- **Building Index**: Inverted zone-type index (`zone_index: [Vec<usize>; 6]`) and vacancy index (`vacancy_index: [Vec<usize>; 6]`, `vacancy_pos: Vec<usize>`) implemented in `BuildingAllocator`. `find_available_home()` is O(1) random selection from the vacancy index. `claim_vacancy`/`release_vacancy` maintain the index incrementally in O(1); `kill_agent` calls `release_vacancy` before swap-remove. Building deletion triggers a full `rebuild_zone_index()` via `dirty_index`. Prerequisite for parallel tick.
- **Unit tests** (`buildings/allocator.rs`): desirability gate (no spawn when grid value < 50.0), demand subtraction (residential demand decreases on spawn), occupancy clearing (3×3 zoning cells cleared on building removal).

### Agents
- `simulation/economy/agents/` (Submodule) — `AgentSystem` in Structure-of-Arrays (SoA) layout.
- FSM states: `IDLE → DEPARTING → ON_ROAD → ARRIVING → IDLE` + `IMMIGRATING`.
- Movement: polyline traversal with sub-tick `remaining_dist` budget; lane offsets from road width / lane count. Agents move at a **fixed speed** with no interaction — cars on the same edge pass through each other. No car-following model, no capacity constraint per lane. See Backlog.
- **Virtual Frontages**: agents arrive at buildings via `(edge_id, t: f32)` T-coordinates rather than physical graph nodes. The arrival trigger is a **projected distance check along the edge tangent**, ensuring agents on wide roads or sidewalks correctly identify they have reached their destination regardless of lateral offset.
- Agent kill: swap-and-pop, O(1). Note: agent indices are not stable across ticks (swap-remove invalidates the last agent's index).
- **Single-threaded tick** — Rayon parallelisation is a v0.1 goal (see Backlog).
- **Transit Mode Enum** — migrated `is_driving: Vec<bool>` to `transit_mode: Vec<u8>` using constants (WALK=0, CAR=1, ...). This provides the multi-modal foundation for bicycles, buses, and rail.
- **Unit tests** (`economy/agents_test.rs`): `test_agent_fsm_lifecycle` verifies the complete daily cycle (Home → Work → Shop → Home) including FSM state transitions, money tracking, and arrival detection via virtual frontage T-coordinates.

#### Agent Rules
- **Immigration**: agents spawn at highway border nodes, arrive by car. Capped at `residential_capacity × 1.1`.
- **Housing search**: immigrants drive toward city centre and claim the first residential building with free capacity (6 agents per plot, hard-coded).
- **Daily cycle**: Home (rest/happiness recovery) → Work (Industrial or Commercial, earn money) → Shop (Commercial, spend money) → Home.
- **Happiness/money**: home +1 happiness/day; commute penalty −commute_time/60 per trip; pollution effect −p × 0.1/day; work +$10/day; shop −$20.

### Pathfinding
- `simulation/pathing/astar.rs` — binary-heap A* with `(node, incoming_edge)` state key (mandatory for correct turn-restriction enforcement at `Node::lane_connections`).
- `simulation/pathing/cost.rs` — edge cost = `length / speed_limit` (time in seconds) with exponential slope penalty for grades > 10%: `1 + (max_slope × 5)²`.
- A* heuristic: `euclidean_distance / max_v` — admissible and consistent; `max_v` is the maximum edge speed limit in the network, precomputed at graph build time.
- `simulation/pathing/hpa.rs` — current HPA* implementation (three-phase query). **Superseded by CCH (item 31, v0.1 blocker); will be removed when CCH lands.**
- **Unit tests** (`simulation/pathing/tests.rs`): `test_slope_cost_calculation` (50% grade edge receives a 7.25× cost multiplier vs a flat edge of equal length), `test_pathing_avoids_steep_slope` (router selects the longer flat detour A→C→B over the steep direct A→B). **Known geometry inconsistency in `test_pathing_avoids_steep_slope`**: `edge_ab`'s geometry endpoint is `(100, 50, 0)` but node `n_b` is placed at `(100, 0, 0)`. `CostCalculator` reads `edge.geometry`, so the slope penalty is computed correctly and the test passes, but the endpoint violates the invariant that edge geometry must start and end at the node positions. Fix: place `n_b` at `(100, 50, 0)`, or represent the slope with an intermediate waypoint while keeping the geometry endpoint at `n_b`'s flat position.

### Demand
- `simulation/economy/demand.rs` — global R/C/I demand counters. Demand increments globally; buildings consume it on spawn.

### Godot Bridge
- `nodes/simulation_node.rs` — entry point; split into `sim/editing.rs` (road/zoning mutations), `sim/query.rs` (read-only `#[func]` API), `sim/undo.rs` (VecDeque-backed undo stack, O(1) push/pop), `sim/render_helpers.rs`, `sim/benchmark.rs`.

### Benchmark Mode
- Launch: `./run.sh --huge-map`
- Creates a 20 km × 20 km map with a 20 × 20 road grid, zone burst-growth, and 100,000 agents.
- Logs per-tick: timestamp, version, agent count, map size, tick duration (ms), FPS, pathfind calls.
- Results written to `godot/benchmark_results.csv`. Delete the file to reset.
- Criterion micro-benchmarks: `cd rust && cargo bench` → `target/criterion/`.
- Memory note: huge-map mode uses ~1 GB+ RAM.

---

## Known Bugs

| ID | File | Description | Severity |
|----|------|-------------|----------|

---

## Backlog

### v0.01 Goals — strong targets for v0.01 quality

- Flow field Dijkstra < 5 ms on a 1,000-node graph: **deferred to v0.2** (item 18) — cannot be written until flow fields are implemented.

### v0.1 — 100k-agent milestone

Feature completion and correctness fixes. Performance tuning is not the focus here — the benchmark target is 100k agents at ≥ 30 FPS on a single core, which is achievable without parallelism given the current per-agent cost.

#### v0.1 Blockers — must complete before tagging v0.1

31. **CCH / CRP implementation** — supersedes HPA* for all long-distance routing:
    - **RegionGraph design**: `TransitGraph` must not be scoped to a single city. Rename or extend it to `RegionGraph`; it is the single graph for all current and future city tiles. Adding a second city later means adding nodes/edges to this graph and triggering a topology rebuild — not creating a separate graph instance. Do this structural rename before building the CCH data structures on top of it.
    - **Topology phase**: compute contraction order (node importance by edge-difference heuristic); add shortcut edges bypassing contracted nodes. Output: an elimination tree and a contracted graph. This phase runs once on full topology rebuild (when roads are added or removed). O(E log E).
    - **Metric customization phase**: propagate current edge weights (speed limits, congestion) through the elimination tree. O(E). Runs on every road edit or congestion update — this is CRP's key property: metric updates are cheap even though topology rebuilds are not.
    - **Query**: bidirectional Dijkstra on the contracted graph. Forward from source, backward from goal, meeting at the highest-hierarchy node. O(√E log √E) vs O((V+E) log V) for A*. The `(node, incoming_edge)` state key for turn restrictions must be preserved.
    - **Modal filtering**: `allowed_mask: u8` (v0.01 goal 4) gates which edges are visible during query — the contracted graph is built per-mask or masks are applied at query time.
    - On completion, `HpaGraph` and the three-phase HPA* query are removed. All `find_path` call sites in `decisions.rs` switch to CCH query.

#### v0.1 Goals

24. **R-Tree spatial index** (`rstar` crate) for fine-grain edge queries — O(log N) insert/delete/query. Current uniform 512 m grid degrades for edges that span multiple chunks.
26. **Bridge and tunnel support — `EdgeClass`**:
    - Add `EdgeClass` enum to `network/types.rs`: `Standard | Bridge | Tunnel`. Fits in a single byte; zero memory cost due to existing `Edge` struct padding.
    - Add `class: EdgeClass` field to `Edge`. No simulation logic changes — the 3D polyline geometry already stores correct Y positions for elevated or depressed roads.
    - `Standard`: current behaviour; road mesh follows terrain.
    - `Bridge`: renderer generates a floating deck mesh at the geometry's Y elevation; mesh does not deform to terrain. Zoning disabled on both sides.
    - `Tunnel`: renderer generates portal entrance meshes at both endpoints only; the road mesh between portals is hidden. Zoning disabled.
    - Zoning obstruction check: `is_cell_obstructed` must ignore bridge edges when checking cells at ground level directly beneath the deck (bridge Y − cell Y > clearance threshold).
    - 512 m chunk index is XZ-only and will place a bridge edge and an underpass edge in the same bucket — harmless because all chunk queries operate on graph topology, not rendered geometry.
27. **Bridge and tunnel editor tool**:
    - UI action on an existing edge to promote it to `Bridge` or `Tunnel` (single `EdgeClass` field write + CCH topology rebuild for affected edges + re-render).
    - Validation: warn if bridge endpoints are not at a higher Y than the terrain midpoint; warn if tunnel endpoints are not below the terrain surface.
28. **Environmental grid regression tests** — the diffusion PDE and decay constants are undocumented invariants with no verification. Add a test that ticks `PollutionSystem` 200 times with one industrial source cell; assert: the source cell has a positive value, a cell 5 steps away has a nonzero diffused value, no cell in the grid is infinite or NaN, and the average grid value decays over time after the source is removed. This is a smoke test for the explicit FD stability condition and the decay half-life documented in `analysis.md §2.3`.
29. **`DataGrid<T>` unit tests** — used by every environmental system and the terrain but has no direct tests. Add: get/set round-trip for arbitrary coordinates; bilinear interpolation returns the exact corner value when querying at a grid point; out-of-bounds query returns `None` (or clamps, whichever the implementation guarantees).

### v0.2 — scaling baseline, multi-modal foundation, and multi-city region

Target: ~250k–500k agents with the first non-car transport mode live and the multi-city region architecture in place.

At ~200k agents three independent performance walls converge: (1) the single-threaded agent tick saturates one core, (2) the O(B) building scan in every IDLE activation becomes the dominant tick cost as the city fills, and (3) per-agent pathfinding accumulates to an unacceptable fraction of frame time even with CCH (flow fields are the answer — item 18). All three must be resolved before the v1.0 path is smooth — deferring any one of them past v0.2 means hitting a hard wall instead of a gradual ramp.

The multi-modal angle: v0.01 goals 3 and 4 (`transit_mode` and `allowed_mask`) install the two-wire harness. v0.2 validates it by shipping bicycle support — the simplest possible new mode (no VehicleSystem, no WAITING state, no timetables). If bicycles work correctly under load, every subsequent mode (taxi, bus, rail) is an incremental addition, not a structural change.

**Implementation order matters.** Item 19 (parallel tick) depends on the zone index (B16a fix) existing and providing O(1) vacancy lookup — the parallel tick needs atomic vacancy counters that the index maintains. Item 18 (flow fields) requires item 19 to be done first — flow field queries need to run inside the parallel tick loop. Item 25 (IDM) is independent. Item 30 (bicycle) builds on the `transit_mode` and `allowed_mask` infrastructure already in place. Items 54–56 (multi-city region) require CCH (item 31, v0.1 blocker) to be complete — the `RegionGraph` rename and CCH query are the foundation the region system builds on.

19. **Parallelise `AgentSystem::tick`**: remove `&mut TransitGraph` from tick signature (currently unused as mut); switch to `rayon::par_iter_mut` over agent index ranges. Use `AtomicU32` for building vacancy counters (enabled by item 20's index); batch immigration assignments in a post-parallel sequential phase. Prerequisite for all subsequent agent-scale targets.
18. **Flow fields for shared destinations**: one reverse Dijkstra from each active destination zone type produces a next-node map of length V. Agents query O(1) instead of running individual CCH queries. Reduces O(A × CCH_cost) to O(M × (V + E) log V) where M ≈ 10–100 active zone types. Retain CCH for immigration and one-off novel destinations. Requires parallel tick (item 19) to be useful — flow field lookup is the O(1) work that fills each parallel agent slot. Add a timing test on completion: Dijkstra from one destination on a 1,000-node graph must complete in < 5 ms (originally listed in v0.01 goals but deferred here until the system exists).
25. **Car collision — Intelligent Driver Model (IDM)**:
    - Add `speed: Vec<f32>` to `AgentSystem` SoA. Current model uses a hardcoded fixed speed; IDM requires a dynamic per-agent speed state (~4 MB at 1M agents).
    - Each tick, build a transient `lane_agents: Vec<Vec<u32>>` indexed by `edge_idx * MAX_LANES + lane_idx`, each sub-Vec sorted by `edge_progression`. O(A) to build from the SoA; thrown away after the tick. Finding the car directly ahead is O(1) (adjacent element in sorted list).
    - Apply IDM per car: `a = a_max × [1 − (v/v_max)⁴ − (s*(v,Δv) / gap)²]` where desired gap `s*(v,Δv) = s_min + v·T + v·Δv / (2√(a_max·b))`. Produces realistic stop-and-go waves and jam dissolution at O(1) per car.
    - Intersection queuing: if a car is at the last polyline segment of its edge and the target node has no accepted entry slot this tick, set `speed = 0`. One entry slot per incoming lane per tick.
    - After the movement pass, write average speed per edge into `Edge::current_congestion`. Feeds directly into the CCH metric customization phase (triggers a fast O(E) weight refresh) and the v1.0 congestion heatmap.
    - Bridges and tunnels: cars on different vertical levels are on different edges and therefore different lane lists — no cross-level interaction, correct without modification.
    - Bicycles, buses: both participate in per-lane occupancy lists naturally. A bicycle is just a slow narrow vehicle; a bus is a long slow one. No mode-specific changes to IDM.
30. **Bicycle support** — first new transport mode; validates the multi-modal foundation (v0.01 goals 3 and 4):
    - Add `BIKE=1<<2` bit to `TransitFlags`. Set it on all edges with a sidewalk or dedicated cycle path — at minimum every `Standard` road edge. Bridges and tunnels: same flag if their geometry accommodates a cycle lane.
    - Speed: 5.5 m/s (~20 km/h). `decide_transit_mode` selects `BIKE` when distance < ~2 km (shorter than car threshold) or when the agent has no car.
    - No VehicleSystem, no WAITING state, no timetables — bicycles are individual agents with `transit_mode=BIKE`, routed via `allowed_mask = BIKE | FOOT`.
    - IDM applies on shared road edges. Bikes have their own lane slot, lower `v_max`, and shorter desired gap parameters.
    - After this item ships, adding taxis is one FSM state + one dispatch call. Buses add VehicleSystem + WAITING. Rail adds an `EdgeClass` variant. The foundation holds.

54. **Multi-city region — background city statistical model**: each inactive city tile is represented by ~15 numbers per tick: population, employment capacity, demand by zone type, and throughput per inter-city connection. Updated on a coarse schedule (~1/s game time, not per tick). No agent simulation runs for background cities. The `RegionGraph` (see item 31) contains all city tiles' road/rail/air nodes and edges so CCH can route through them; agents on those edges are statistical counters on the edge, not FSM objects.
55. **Multi-city region — border crossing spawn/despawn**: when an agent leaves the active city onto an inter-city edge, it is demoted to a statistical entry in a queue attached to that edge (arrival time estimated from CCH path cost). When it arrives at the destination city boundary, it is promoted to a full FSM agent and spawned at the border node. If the destination city is inactive, it is absorbed into that city's statistical population counter. Border nodes already exist as `NodeType::Border` in `types.rs`; immigration logic in `tick.rs` (highway border spawn) is the direct predecessor of this system.
56. **Multi-city region — region view**: a coarse top-level view showing all city tiles, inter-city connection throughput, and aggregate demand flow. No agent rendering at this zoom level — statistical flow numbers only. Switching the active city promotes it to full simulation and demotes the previous active city to statistical mode.
32. **Agent Level-of-Detail**: full FSM + rendering for camera-visible agents (~50k); flow-field-only routing within 2 km (~500k, no individual pathfinding); statistical aggregate counts only beyond 2 km (~450k). Promotion/demotion at LoD boundaries must preserve city-level supply/demand statistics.
33. **AoSoA agent layout**: replace flat SoA with 8-wide SIMD batches (Array of Structures of Arrays). Enables AVX2 to process 8 agents per instruction for position update, lane offset, and transform generation. Prerequisite for GPU-offload of movement arithmetic.
34. **GPU compute (`wgpu`)**: move agent position update (polyline interpolation, lane offset, transform assembly) to GPU compute shader. Keep FSM state transitions and pathfinding on CPU. Requires AoSoA layout.
35. **CSR graph for pathfinding**: convert `nodes`/`edges` + `adjacency` to Compressed Sparse Row format for the read-only pathfinding phase. Single cache line per node expansion. Road edits trigger an O(E) CSR rebuild (acceptable since edits are rare interactive events).
36. **Building levels (1→3)**: upgrade driven by demand pressure history and neighbourhood desirability.
37. **Congestion heatmap**: agent spatial grid (`DataGrid<Vec<AgentID>>`) → edge congestion → dynamic cost update → CRP metric refresh → routing feedback loop.
38. **Incremental road mesh updates**: rebuild only modified edges; simplified LOD geometry at camera distance.
39. **VehicleSystem** (shared prerequisite for buses and trains):
    - Add `VehicleSystem` alongside `AgentSystem`: SoA of vehicle state (`route: Vec<RouteID>`, `stop_idx: Vec<u8>`, `capacity: Vec<u8>`, `occupants: Vec<SmallVec<[u32; 8]>>`).
    - Vehicles follow the same CCH-computed path as agents but tick independently. Passenger agents delegate movement to the vehicle while in `BUS_PASSENGER` or `TRAIN_PASSENGER` mode — their position is overwritten from the vehicle's position each tick.
40. **WAITING FSM state** (shared prerequisite for buses, trains, and taxis):
    - Add `WAITING` to the agent FSM, between `IDLE` and `DEPARTING`.
    - Agents enter `WAITING` when their chosen mode requires an external vehicle (bus stop, train platform, taxi pickup).
    - `waiting_target: Vec<u32>` in SoA: node ID of the stop/pickup point the agent is walking toward, then standing at.
    - Agents are promoted to `ON_ROAD` when the vehicle arrives and has capacity.
41. **Taxi support**:
    - Prerequisites: items 28, 40.
    - Taxis modelled as specialised agents (`transit_mode=CAR`) with two extra FSM states: `DRIVING_TO_PICKUP` and `CARRYING_PASSENGER`.
    - No VehicleSystem needed — each taxi carries exactly one passenger group.
    - Dispatch: O(1) amortised greedy matching via the existing 512 m spatial chunk index. When a passenger enters `WAITING`, query the nearest idle taxi chunk; assign the closest idle taxi.
42. **Bus support**:
    - Prerequisites: Virtual Frontages (v0.01 blocker), items 28, 29, 39, 40.
    - Bus stops use Virtual Frontages `(EdgeID, t: f32)` — no physical graph splits at every stop. Stops are T-coordinates on existing edges; the router inserts a synthetic zero-cost node only during the relevant path query.
    - Buses use CAR-flagged edges and participate in IDM naturally (buses are large, slow vehicles on the lane list).
    - Fixed-route, fixed-headway scheduling: a `RouteID` → `Vec<StopID>` table, a departure timetable, and a next-stop pointer per vehicle.
    - Passenger routing is two-phase: walk to nearest stop → `WAITING` → `BUS_PASSENGER` (vehicle carries) → walk from stop to destination.
43. **Train and metro support**:
    - Prerequisites: items 28, 29, 39, 40. Requires `EdgeClass::Rail` variant (shares same byte field as `Standard/Bridge/Tunnel`).
    - Trains use `RAIL`-flagged edges only. The `allowed_mask` mechanism (item 29) isolates the rail subgraph from road routing automatically.
    - Rail edges: no road mesh; separate rendered track geometry. Metro: `EdgeClass::Tunnel` with `RAIL` flag and underground portal renderer branch.
    - Timetables required — fixed departure slots; stochastic headways are not appropriate at ≥ 10 min service intervals.
44. **Ship support**:
    - Prerequisites: items 28, 29. `NodeType::Harbor` already declared in `types.rs`.
    - Add `SHIP`-flagged edges for water channels and ferry routes. Ships use `NodeType::Harbor` as access nodes.
    - Pathfinding mask: `allowed_mask = SHIP | FOOT` — walk to harbor, sail to destination harbor, walk on.
    - Ship edges are graph-level only; no mesh-based interaction with the SWE water simulation.
45. **Airplane and border-node support**:
    - Prerequisites: items 28, 29. `NodeType::Airport` already declared in `types.rs`.
    - Airplanes connect `NodeType::Airport` nodes with `AIR`-flagged edges at ~200 m/s; agents effectively teleport between airports on the city graph.
    - Each map border highway node gains `NodeType::Border` flag. Immigration agents always use `CAR`-flagged edges from border nodes; the existing immigration cap logic is unchanged.

### v0.3 — 3D asset pipeline

Target: same agent scale as v0.2. No simulation changes. All work is in `render_helpers.rs` (Rust FFI layer) and `godot/scripts/` (GDScript renderers). The 12-float Transform3D format and the MultiMesh instancing architecture are retained — only the meshes assigned to those MultiMesh nodes change.

**Hard architectural constraint**: Godot's `MultiMesh` does not support per-instance skeletal animation. Bone-animated pedestrians require individual `Node3D` nodes, which does not scale past a few thousand agents. The approach for v0.3 is to use static 3D models (a mid-stride static pose for pedestrians, a static car mesh). GPU vertex animation (baked walk cycle sampled via a custom shader using a per-instance phase offset) is noted as a follow-on and can be added without any simulation or API changes once the static models are in place.

46. **Split agent MultiMesh by transit mode** — prerequisite for all other agent visual work:
    - In `render_helpers.rs`, split `get_agent_transforms_internal` into `get_car_transforms()` and `get_pedestrian_transforms()`. Both return `PackedFloat32Array` of 12-float transforms; the existing road-tangent rotation logic moves into `get_car_transforms`.
    - In `agents.gd`, replace the single `MultiMeshInstance3D` with two nodes: `CarMesh` and `PedestrianMesh`, each updated independently.
    - No change to simulation, pathfinding, or SoA layout.
47. **Car 3D model**:
    - Load a `.glb` car model and assign it to the `CarMesh` MultiMesh node. No code changes beyond the assignment.
    - The existing road-tangent basis vectors already produce correct car orientation on roads.
    - When `transit_mode` migration (v0.01 goal 3) is complete, this MultiMesh extends naturally to buses (`transit_mode=BUS`), taxis, etc. by filtering the transform array by mode.
48. **Pedestrian SDF billboard shader** — recommended over loading a 3D model for this scale:
    - Replace the `CapsuleMesh` on the `PedestrianMesh` MultiMesh with a `QuadMesh` (2 triangles). The quad is billboard-oriented in the vertex shader to always face the camera.
    - A custom `gdshader` fragment shader reconstructs the human silhouette from sphere and capsule SDF primitives (head, torso, two legs). SDF edges are anti-aliased via `smoothstep` — pedestrians stay crisp at 3 px or 30 px without a texture. Animation is driven by `sin(phase * TAU)` on the leg offsets — a walking cycle at zero model-file cost.
    - **Rust change required**: add `walk_phase: Vec<f32>` to `AgentSystem` SoA (~4 MB at 1M agents). Increment each tick: `walk_phase[i] = (walk_phase[i] + delta / stride_length) % 1.0`. Expose via `get_pedestrian_phases() -> PackedFloat32Array`. Pass as MultiMesh `CUSTOM_DATA_FLOAT` channel (Godot `per_instance_color` repurposed as 4 floats).
    - **Why SDF over a 3D model here**: MultiMesh cannot do per-instance skeletal animation, so a 3D model would be a static pose anyway. An SDF billboard looks better than a static mesh at city-view distances (3–20 px), produces clean anti-aliased edges at any resolution, requires no texture VRAM, and the walking animation is trivially added. A 3D model only wins at close range — the v1.0 Agent LoD system (item 32) handles that case by switching to individual `AnimationPlayer` nodes within ~50 m.
    - Colour the silhouette by agent state: walking = neutral tone, idle = slightly desaturated, stressed/unhappy = cooler tint. This visual feedback is free — just sample `happiness[i]` and pass it in another custom data channel.
49. **Building variant system** — small Rust change, prerequisite for building model variety:
    - Add `variant: u8` to `Building` struct. Assign during placement: `variant = (cell_x ^ cell_y ^ edge_idx) as u8 % NUM_VARIANTS` for deterministic pseudo-random variety without an RNG call.
    - Add `get_building_transforms_by_variant(zone_id: i32, variant: i32) -> PackedFloat32Array` to `render_helpers.rs`.
    - In `buildings.gd`, replace the single MultiMesh per zone type with one `MultiMeshInstance3D` per `(zone_type, variant)` pair. With 5 zone types and 3–4 variants each, this is 15–20 draw calls total — well within budget.
50. **Building 3D models**:
    - Load zone-appropriate `.glb` models per `(zone_type, variant)`. Replace the procedural `SurfaceTool` mesh in `buildings.gd`.
    - Enable Godot's built-in `GeometryInstance3D` LOD on each `MultiMeshInstance3D`: simplified mesh beyond 150 m, billboard imposter beyond 400 m. No code required — set `lod_min_distance` and `lod_max_distance` in the Inspector or via script.
    - Building facing direction is already encoded in the transform basis (from `facing_dir: Vector2` in `Building`). Models must be authored facing +Z so the existing basis assignment produces correct road-facing orientation.
51. **Environment detail assets** (stretch goal):
    - Trees, benches, streetlights placed at fixed offsets from road edges, derived from the zoning grid cell positions already computed in `ZoningSystem`.
    - One `MultiMeshInstance3D` per asset type; placement computed once when a road edge is added or removed, not per tick.
    - No simulation interaction — purely visual. No Rust simulation changes; a new `environment.gd` script reads edge geometry via existing `get_edge_geometry()` calls.
52. **Terrain built-layer ("urban density map")** — eliminates grass visible under roads and buildings:
    - Add a new `DataGrid<f32>` named `built_layer` to the environment system (same architecture as `pollution`, `noise`, `desirability`). Values: `0.0` = natural, `1.0` = fully developed.
    - Write to the grid on road placement: cells within `SIDEWALK_WIDTH` on both sides of each edge → set to `~0.8`. Write on building placement: all 9 cells of the 3×3 footprint → set to `1.0`. Clear on removal.
    - Expose as `get_built_layer_data() -> PackedByteArray` (same interface as `get_pollution_data()` etc.) for the terrain shader.
    - In the terrain `gdshader`, sample the built-layer texture and blend: `mix(grass_color, concrete_color, built_density)`. No change to terrain mesh geometry.
    - **Complexity**: O(E × W) write on road placement (E edges, W = cells per edge), O(1) per building cell. Grid tick is unnecessary — values only change on network/building events.
53. **Building foundation quads** — eliminates z-fighting and grass bleed at building perimeters:
    - For each placed building, render a flat `QuadMesh` sized to the 3×3 footprint (30 m × 30 m) at `+0.02 m` above terrain height. Material uses a concrete/pavement albedo that matches the urban density map blend at `1.0`.
    - Add a second `MultiMeshInstance3D` per zone type in `buildings.gd` (e.g. `ResidentialFoundationMesh`). Reuse the same transform data already packed for the building mesh — the foundation quad uses the same center position and facing direction, with a flat scale.
    - No Rust changes beyond what item 52 already requires. Foundation quads are driven entirely from the existing `get_building_transforms()` call in GDScript.

---

## Architecture Reference

### Grid Specifications

| Parameter | Value | Notes |
|-----------|-------|-------|
| City tile size | Player-configurable | Default 20 km × 20 km; no hardcoded upper limit. Set via `MapConfig` at construction. |
| Zoning cell | 10 m × 10 m (`zone_cell_m`) | Configurable via `MapConfig`. |
| Building footprint | 3 × 3 cells (30 m × 30 m) | Fixed relative to zoning cell size. |
| Road width (2-lane) | 10 m (7 m asphalt + 1.5 m sidewalk each side) | Fixed. |
| Zoning offset from centreline | 5 m | Fixed. |
| Zoning depth | 10 cells (100 m) | Fixed relative to zoning cell size. |
| Road spatial chunk | 512 m | Fixed; scales correctly to any map size. |
| Environmental grid cell | 40 m (`env_cell_m`) | Configurable via `MapConfig`. Grid dimensions = map size / cell size. |
| Environmental grid (default 20 km map) | 500 × 500 | Scales with map size: 60 km map → 1500 × 1500. |

### Movement Speeds

| Mode | Speed | Status | Notes |
|------|-------|--------|-------|
| Walking | 4.0 m/s (14.4 km/h) | Implemented | ~3× real life; 10 m road takes 2.5 s |
| Driving (car) | 20.0 m/s (72 km/h) | Implemented | Standard suburban |
| Bicycle | 5.5 m/s (20 km/h) | Planned (item 30) | Shares sidewalk / dedicated cycle edges |
| Bus | 10–15 m/s (36–54 km/h) | Planned (item 42) | Slower than car due to stop dwell time |
| Train / Metro | 20–40 m/s (72–144 km/h) | Planned (item 43) | Higher value for intercity; metro ≈ 25 m/s |
| Ship / Ferry | 5–10 m/s (18–36 km/h) | Planned (item 44) | Slow; used for harbor-to-harbor routes |
| Airplane | ~200 m/s (720 km/h) | Planned (item 45) | Near-teleport at city scale |

### Key Design Patterns

- **DataGrid\<T\>**: flat `Vec<T>` with stride `width`. Row-wise parallel iteration with `rayon::par_chunks_mut`. All spatial grids (terrain, pollution, noise, desirability) use this type. The planned congestion heatmap (`DataGrid<f32>`, written from per-edge average speed) will also use it.
- **Per-lane occupancy lists (planned)**: transient `Vec<Vec<u32>>` built each tick from the SoA, indexed by `edge_idx * MAX_LANES + lane_idx`, sorted by `edge_progression`. Provides O(1) car-ahead lookup for IDM without any persistent spatial structure. Thrown away after each tick.
- **SoA (Structure-of-Arrays)**: `AgentSystem` stores all fields as parallel `Vec<T>` indexed by agent ID. Cache-friendly for bulk iteration.
- **512 m spatial chunks**: road edge AABB registered in all overlapping chunks. Used for editor queries (radius ≈ 120 m → typically 1 chunk) and spatial snapping. CCH manages its own contraction hierarchy independently of this grid.
- **`(node, incoming_edge)` pathfinding state**: required for turn restriction correctness at `Node::lane_connections`. Must be preserved in any pathfinding replacement.

### Multi-modal Transport Vocabulary

The type vocabulary for all planned transport modes already exists in `network/types.rs`:

| Type | Declared values |
|------|----------------|
| `TransitType` | `Road, Rail, Ship, Air, Foot` |
| `TransitFlags` | `FOOT=1<<0, CAR=1<<1, RAIL=1<<2, SHIP=1<<3, AIR=1<<4` (bit 5+ free for `BIKE`) |
| `NodeType` | `Junction, Station, Harbor, Airport, Transfer, Frontage` |

The gap is entirely in the agent system: `transit_mode: Vec<u8>` migration is complete. The remaining pathfinding query take of `pedestrian: bool` must become `allowed_mask: u8` (backlog item 4, v0.01 goal) — CCH (item 31) inherits this same parameter. These two changes are the shared prerequisites for all multi-modal work.

### Memory Budget (20 km map)

| Resource | Size |
|----------|------|
| Terrain heightmap (2000²) | 16 MB |
| Terrain source copy | 16 MB |
| 3 environmental grids at 500² | 3 MB |
| Road edges (50k × ~512 B) | 25 MB |
| Road nodes (100k × ~128 B) | 12 MB |
| Agent SoA (1M × ~120 B) | 120 MB |
| Agent speed field (1M × 4 B, added by IDM) | 4 MB |
| CCH contracted graph (shortcuts + elimination tree) | ~20–30 MB |
| Road mesh VRAM (50k edges) | ~144 MB VRAM |

**Bandwidth note**: 3 environmental grids at 500² = 3 MB of memory traffic per diffusion pass. At 10 ticks/s this is well within DDR4 bandwidth. The remaining allocation concern is the per-tick `grid.clone()` (~1 MB each); see B18.

---

## Godot Layer

The Godot side is a thin bridge: no simulation logic lives here. All GDScript scripts call into `SimulationNode` (the Rust GDExtension) and pass results to rendering nodes.

### Scene Tree (`godot/scenes/Main.tscn`)

| Node | Type | Script | Role |
|------|------|--------|------|
| Main | Node3D | — | Root |
| SimulationNode | (Rust native) | — | Owns all simulation state; exposes `#[func]` methods |
| Terrain | MeshInstance3D | `terrain.gd` | Heightmap mesh, overlay textures, sculpt input |
| Water | MeshInstance3D | `water.gd` | Shallow-water surface renderer |
| RoadTool | Node3D | `road_tool.gd` | Road drawing (straight + spline), extends NetworkTool |
| ZoningTool | Node3D | `zoning_tool.gd` | Zone paint/fill/delete tool |
| Buildings | Node3D | `buildings.gd` | MultiMesh renderer for placed buildings |
| Agents | Node3D | `agents.gd` | MultiMesh renderer for live agents |
| LaneTool | Node3D | `lane_tool.gd` | Visual turn-restriction editor |
| MoveTool | Node3D | `move_tool.gd` | Road node drag-to-reposition, extends NetworkTool |
| InputManager | Node | `input_manager.gd` | Global keyboard/mouse routing, tool switching |
| CameraNode | CameraNode | — | Rust camera node |
| MainUI | CanvasLayer | `main_ui.gd` | All HUD panels and buttons, procedurally built |

### Script → Rust Method Inventory

| Script | SimulationNode methods called |
|--------|-------------------------------|
| `input_manager.gd` | `undo_action()`, `set_simulation_speed()` (via MainUI signals) |
| `main_ui.gd` | `get_city_demographics()`, `set_simulation_speed()`, `undo_action()` |
| `terrain.gd` | `get_heightmap_size()`, `get_heightmap_data()`, `sculpt_terrain()`, `flatten_terrain_for_roads()`, `load_heightmap_data()`, `is_terrain_dirty()`, `clear_terrain_dirty()`, `get_pollution_image_data()`, `get_noise_image_data()`, `get_desirability_image_data()` |
| `water.gd` | `get_water_data()`, `get_water_velocity_data()`, `add_water_source()`, `is_water_dirty()`, `clear_water_dirty()` |
| `agents.gd` | `get_agent_transforms()`, `get_agent_paths_debug()`, `get_city_demographics()` |
| `buildings.gd` | `get_building_transforms(zone_id)` |
| `network_tool.gd` | `add_road()`, `get_closest_network_point()`, `get_closest_node()`, `get_road_mesh_data()`, `get_network_nodes()`, `get_node_pos()`, `get_height_at()` |
| `road_tool.gd` | (inherits NetworkTool) |
| `move_tool.gd` | `get_closest_node()`, `get_node_pos()`, `move_network_node()` |
| `cul_de_sac_tool.gd` | `get_closest_node()`, `has_cul_de_sac()`, `set_node_cul_de_sac()` |
| `lane_tool.gd` | `get_node_lanes()`, `get_lane_connections_array()`, `set_lane_connection()`, `clear_lane_source()`, `clear_lane_connections()`, `get_node_pos()`, `get_closest_node()`, `get_edge_geometry()`, `get_edge_width()`, `get_lane_width()` |
| `zoning_tool.gd` | `update_zoning_visuals()`, `get_hovered_edge()`, `set_zoning_cell()`, `set_zoning_enabled()`, `get_closest_network_point()` |

### Data Format Reference

| Buffer | Type | Layout |
|--------|------|--------|
| Heightmap | `PackedFloat32Array` | Flat row-major, `width × height` f32 values (metres) |
| Water depth | `PackedFloat32Array` | Same layout as heightmap |
| Water velocity | `PackedFloat32Array` | Same layout, scalar magnitude per cell |
| Agent transforms | `PackedFloat32Array` | 12 floats per agent: `[basis.x(3), basis.y(3), basis.z(3), origin(3)]` — matches `Transform3D` |
| Building transforms | `PackedFloat32Array` | Same 12-float layout as agent transforms |
| Pollution / Noise / Desirability | `PackedByteArray` | RGBA8, one pixel per grid cell; uploaded to a shader `ImageTexture` |

# Future changes

## Zoning
- Add SimCity 4 style zoning (remove automatic zoning, use manual, no restictions on the size of the zone)