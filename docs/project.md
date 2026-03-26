# Metrum Rise — Project State

**Scale target**: ≥ 1,000,000 total population across a multi-city region, with a clear distinction between simulation tiers:
- **Full FSM** (individual pathfinding, real movement, all state transitions): ~300–500k agents on a 20-core machine with DDR5. This is the hardware-honest ceiling — the DDR5 memory bandwidth wall at ~190 MB/tick for 1M SoA entries limits throughput regardless of core count.
- **Flow-field tier** (group movement via shared destination maps, no per-agent CCH queries): extends the active zone to ~1M total when combined with the full-FSM layer. Requires item 18.
- **Statistical tier** (aggregate population counters, demand flow numbers, no individual simulation): background cities and distant city regions. No per-tick FSM cost.

"1M agents" in this document means 1M total population across all tiers. It does not mean 1M simultaneously running full FSM. Any claim of full individual simulation at that scale on current hardware is dishonest — the memory bandwidth arithmetic does not support it.

City tiles are variable size (default 20 km × 20 km, player-configurable); cities are connected via a single unified `RegionGraph`. Only the active city runs full-FSM simulation; background cities run as statistical models.
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
- `RegionGraph` — refactored into a modular package in `simulation/network/graph/`.
    - `data.rs`: `Node`, `Edge`, and `RegionGraph` struct definitions.
    - `spatial.rs`: `rstar` R-Tree for edges and 16 m uniform grid for nodes.
    - `topology.rs`: intersection detection, edge splitting, and node merging.
    - `rebuild.rs`: batch remapping, soft-deletion compaction, and intersection clipping.
    - **Road Network (`RegionGraph`)**: Adjacency-list based directed graph with spatial acceleration via `rstar::RTree<EdgeEntry>` spatial index (Item 24). Pathfinding via Customizable Contraction Hierarchies (CCH). Supports multi-modal queries via `allowed_mask`.
- **Bridge & Tunnel Infrastructure (Item 27)**:
    - **Structural Geometry**: Automated generation of `EdgeClass::Bridge` deck structural slabs (1m thick), vertical side walls with terrain grounding (1m sink), 10cm thick volumetric railings (1.2m tall), and supporting pillars (every 15m).
    - **Node Continuity (B_BRIDGE5)**: Bridge structural components (railings, walls, deck) are precisely clipped to the `start_clip`/`end_clip` boundaries. **End caps** (concrete faces) are generated only at dead-end terminations (node degree == 1), ensuring seamless transitions through junctions without internal obstructions.
    - **Junction Continuity**: Structural concrete railings and floor slabs now wrap through intersections, providing a continuous architectural profile across the network.
    - **Tunnel Portals**: `EdgeClass::Tunnel` hide the road mesh between portals and generate solid entrance geometry at endpoints.
    - **Editor Integration**: Real-time `EdgeClass` promotion via the `Inspect` tool and dedicated `Bridge` button. Includes validation warnings for invalid terrain clearances.
    - **Shading**: Depth-biased concrete materials to eliminate Z-fighting on structural faces.
    - **Texture Initialization**: Resolved "white road" initialization bug by ensuring `ShaderMaterial` instances are fully prepared before surface assignment.
- **`RegionGraph` rename** — `TransitGraph` renamed to `RegionGraph`; struct is now globally owned in `SimulationNode`, not city-scoped. All call sites updated. Prerequisite for CCH (item 31b/c).
- `TransitNetwork` (`network/mod.rs`) — `add_road`, `split_edge`, `merge_nodes`.
- **Pathfinding (CCH)**: `simulation/pathing/cch.rs` implements a Customizable Contraction Hierarchy with O(E) metric customization and bidirectional upward Dijkstra. Replaces HpaGraph for all agent routing.
- **Unit tests** (`simulation/network/test_topology.rs`): `add_road` (bidirectional adjacency, 100 m subdivision logic), `split_edge` (physical length summation, node sharing, zoning/building migration), `compact_edges` (index remapping consistency for agents and buildings).
- Topology: intersection detection and edge splitting in `topology.rs`.
- Road and junction mesh generation in `network/render/road.rs` (405 lines).
- Soft deletion with compaction: edges marked `deleted = true`; `compact_edges()` removes them and remaps all indices (agents, zoning, routing graph).
- Lane types and one-way rules in `network/types.rs`.
- Edge geometry is 3D (`Vec<Vector3>`) — grade-separated roads are natively representable as elevated or depressed polylines. Node snapping uses 3D Euclidean distance, so bridge abutments and underpass nodes with ≥ 2 m vertical separation will not snap together.
- **`EdgeClass` data model complete**: `Standard | Bridge | Tunnel` enum in `types.rs`; `class: EdgeClass` field on `Edge`; new edges default to `Standard`; `split_edge` copies `class` to the new half-edge.
- **R-Tree Spatial Index**: `spatial_edge_rt` replaces the uniform 512m grid for all edge queries. O(log N) insert/delete/query provides tight AABB filtering, zero manual deduplication, and eliminates long-edge false positives.

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

### Foundational Grid System
- `DataGrid<T>` — flat memory-efficient 2D grid with bilinear interpolation support and bounds checking.
- **Unit tests**: Verified get/set round-trip for arbitrary coordinates, exact corner values for bilinear interpolation, and boundary condition clamping.

### Environmental Grids
- `simulation/grid/pollution.rs` — industrial emission (+5/tick) + 4-neighbour diffusion (explicit finite-difference), decay ×0.995, parallelised with Rayon.
- `simulation/grid/noise.rs` — traffic noise diffusion, parallelised.
- `simulation/grid/desirability.rs` — composite formula: `50 − pollution × 2 − noise × 1.5`, parallelised.
- All three use `DataGrid<f32>` with dimensions calculated dynamically from `MapConfig` (default 500 × 500 at 20 km); bilinear upsampled for display.
- `PollutionSystem` and `NoiseSystem` use pre-allocated `swap: DataGrid<f32>` fields; each tick calls `std::mem::swap()` and then clears the target grid with `.fill(0.0)` — hot-path allocation is zero. Correctly isolates new emissions and prevents ghost pollution/noise accumulation from stale buffers.
- **Unit tests (Item 28)**: Verified diffusion (source spreads to neighbors over time), decay (average grid value decreases after source removal), and stability (all values remain finite) for `PollutionSystem`.
- **NoiseSystem Unit Tests (Item 28b)**: Verified road-edge emission (high-speed roads emit 4x more noise), diffusion, decay, and stability.

### Buildings
- Desirability gate enforced (≥ 20). Spawn throttle active (max 10 buildings per tick).
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

### Economy

Current agent decision logic lives in `simulation/economy/agents/tick.rs` (activity selection) and `simulation/economy/agents/decisions.rs` (transit mode). The economic loop is partial — income and spending exist but the decision model is probabilistic and the transit mode selection is a hardcoded distance threshold.

**Activity cycle** (`TRANSIT_IDLE` branch in `tick.rs`):
- Each idle agent has a **5% chance per second** of triggering an activity decision.
- If `activity == 0` (at home): 40% chance to shop (if `money ≥ 20`), otherwise go to work.
- If `activity != 0` (at work or shop): always return home.
- Work building assigned lazily on first work trip — random Industrial or Commercial building; persists until destroyed.

**Economic state per agent** (SoA fields):
- `money`: starts at $100 for immigrants; `+$10/day` while idle at work; `−$20` on shop arrival.
- `happiness`: starts at 50; `+1/day` while idle at home; `−commute_time / 60` per trip; `−pollution × 0.1` per day. Clamped `[0, 100]`.

**Transit mode selection** (`decisions.rs`):
- If pedestrian CCH distance > 500 m and agent has a car, attempt car path; otherwise walk.
- `MODE_WALK`, `MODE_CAR`, `MODE_BIKE`, `MODE_BUS_PASSENGER`, `MODE_TRAIN_PASSENGER`, `MODE_TAXI_PASSENGER`, `MODE_SHIP_PASSENGER` constants are declared in `mod.rs`; only WALK and CAR are exercised.

**Planned** (see Backlog, v0.1 Economy): utility-based decision model, Maslow-inspired need hierarchy (physiological → safety → social → esteem), living standard as a derived aggregate, per-agent needs, supply chain, building economic actors.

### Pathfinding
- `simulation/pathing/astar.rs` — binary-heap A* with `(node, incoming_edge)` state key (mandatory for correct turn-restriction enforcement at `Node::lane_connections`).
- `simulation/pathing/cost.rs` — edge cost = `length / speed_limit` (time in seconds) with exponential slope penalty for grades > 10%: `1 + (max_slope × 5)²`.
- A* heuristic: `euclidean_distance / max_v` — admissible and consistent; `max_v` is the maximum edge speed limit in the network, precomputed at graph build time.
- `simulation/pathing/cch.rs` — CCH / CRP implementation replacing HPA*. Single hierarchy with inner-path expansion (`CchShortcut::inner_edges: Vec<usize>`). Three phases: contraction (degree-based node order, shortcut generation into `fwd_up`/`bwd_up`), customisation (`customize()` walks all shortcuts and sums `base_cost * (1 + current_congestion)` — O(E), called on every congestion update), query (bidirectional upward Dijkstra, `allowed_mask` applied at expansion, path reconstructed from edge sequence). `hpa.rs` deleted; all call sites updated.
- **`simulation/pathing/cch.rs`** — initial CCH implementation (31b). Implements building a single contraction hierarchy with shortcut inner path storage. High-performance customization strategy: recomputes shortcut costs by summing underlying concrete edge costs (base_cost * (1 + congestion)) in O(path_length). Supported by bidirectional upward query phase.
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
- **`AgentSystem::tick` baseline (single-threaded, 2026-03-25)**:
  | Benchmark | 1k | 10k | 100k | 1M | Per-agent |
  |-----------|-----|------|-------|-----|-----------|
  | `on_road` (polyline traversal + lane offset) | 12.6 µs | 124.8 µs | 1.23 ms | ~12.3 ms* | ~12.3 ns |
  | `idle_scaling` (SoA iteration floor) | 5.29 µs | 52.9 µs | 537 µs | 5.72 ms | ~5.3 ns |
  *extrapolated. Near-perfect O(N) on both. Movement costs ~2.3× the idle iteration floor (~7 ns/agent for traversal maths). At 1M on_road agents single-threaded: ~12.3 ms = 74% of a 16.7 ms frame budget; Rayon parallelisation (item 19) is the next multiplier.

---

## Known Bugs

| ID | File | Description | Severity |
|----|------|-------------|----------|
| B1 | `simulation/economy/agents/data.rs` | `kill_agent` does not `swap_remove` the `has_car` field. Every kill leaves `has_car` one element longer than all other SoA fields, silently corrupting agent state after each kill+spawn cycle. Fix: add `self.has_car.swap_remove(index)` alongside the other 29 fields. The `soa_derive` migration (item 59) fixes this structurally. | [BUG] | [DONE]
| B2 | `godot/scripts/terrain.gd` | Overlay keys 8/9/0 have no effect. `overlay_mode` shader parameter and overlay texture are only updated inside `update_terrain_visuals()`, which is gated on `is_terrain_dirty()`. Pressing 8/9/0 sets the GDScript variable but the shader never changes unless terrain also happens to be dirty. Fix: in `_process`, compare `overlay_mode` to `cached_overlay_mode` and push the shader parameter + texture upload when they differ, independently of terrain dirty. | [BUG] | [DONE]
| B3 | `godot/scripts/zoning_tool.gd` | Painted zone colours are always visible regardless of active tool or overlay mode. `_process` hides `grid_mesh` when the tool is inactive but never hides `paint_mesh` — last set `visible = true` inside `_update_visuals()` and never cleared. Zone colours should only show in overlay mode 0 (zoning) or when the zoning tool is active. Fix: add `paint_mesh.visible = false` to the `not active` early-return branch. | [BUG] | [DONE]
| B4 | `simulation/economy/agents/tick.rs` | ON_ROAD pedestrian sidewalk normal is `Vector2::new(-tangent.y, tangent.x)`, the exact negative of the building placement normal `Vector2::new(tangent.y, -tangent.x)` used in both `allocator.rs` and `TRANSIT_DEPARTING`. Agents correctly exit to their building's sidewalk during DEPARTING, then immediately cross to the opposite sidewalk during ON_ROAD movement and arrive at buildings from the wrong side. Fix: in the pedestrian ON_ROAD branch, replace `Vector2::new(-tangent.y, tangent.x)` with `Vector2::new(tangent.y, -tangent.x)`. | [BUG] | [DONE]
| B5 | `simulation/network/render/road.rs` | Sidewalk ribbons z-fight on the inner side of junction corners where the angle between two roads exceeds 90°. Each edge generates its sidewalk with a raw perpendicular end at the clipped junction boundary. On acute inner corners the two adjacent sidewalk quads overlap in XZ at the same Y. Root cause: `node_miters` is only computed for 2-edge pass-through nodes (curves); junction nodes (degree ≥ 3) have no miter, so the sidewalk corner vertex is not pulled back to the angle bisector. Fix: for junction nodes, sort connected edges by departure angle, compute the bisector of each consecutive pair of edge normals, store as a per-`(node_id, edge_id)` miter, and apply it to the inner corner vertex of each sidewalk ribbon at its clipped junction endpoint. Clamp miter length to prevent near-parallel roads from producing an infinite spike. Outer corner gaps (convex side) can be filled by a separate junction cap polygon pass. | [BUG]

---

## Backlog

### Infrastructure

59. **`AgentSystem` SoA migration to `soa_derive`**: replace the 29 manually-maintained parallel `Vec<T>` fields in `data.rs` with a `#[derive(SoA)]` schema struct (`Agent`) and a generated `AgentVec`. `spawn_agent` becomes a single `push(Agent { ... })` — the compiler enforces that all fields are initialised; `clear()` becomes one call; adding a new field no longer silently corrupts state. Also fixes B1 (`has_car` missing from `kill_agent`). Implementation: add `soa_derive = "0.3"` to `Cargo.toml`; define `Agent` with all per-agent fields; wrap `AgentVec` in `AgentSystem` via `Deref/DerefMut` so all `agents.field[i]` call sites are unchanged; replace `self.count` with `self.len()` throughout (`tick.rs`, `data.rs` methods, `render_helpers.rs`, `query.rs`, `benchmark.rs`, `allocator.rs`). `sim_time` and `pathfind_count` remain as direct fields on `AgentSystem`, not part of the generated vec.

66. **Manual zoning (SimCity 2013 style)**: remove the automatic edge-aligned zone cell generation. Replace with player-painted zones. Players can start zoning next to the already existing road and then drag with mouse to create a zone. Mouse must be dragged on a direction of the road and it create 1x4 zone cells. Buildings can allocate this cells. For example 3 1x4 cells create a constructing area of 3x4. The zoning must follow the curveture of the road. Cells should not overlap. Instead the outer edge of the cells should follow as well the curveture of the road. No agent or pathfinding changes.

58. **Split `buildings/allocator.rs`** — 689 lines mixing two concerns. Split into `buildings/allocator/lifecycle.rs` (spawn, kill, evict, remap) and `buildings/allocator/index.rs` (zone_index, vacancy_index, claim_vacancy, release_vacancy). Low complexity, no behaviour change. Do when next working in that file.

67. **Crosswalk system** — data model, rendering, toggle tool, and (deferred) pedestrian crossing constraint.

   **Data model** (`network/types.rs`): add `crosswalk: bool` to `Node`. Auto-set `true` in `TransitNetwork::add_road()` whenever the connected node's degree reaches ≥ 3 (proper junction). Dead-ends and 2-way pass-through nodes default to `false`. No cascade effects — the field is read-only by all other systems until part 4 ships.

   **Rendering** (`godot/scripts/crosswalk.gd`, new): `MultiMeshInstance3D` — one instance per road arm at each crosswalk node (a 4-way junction emits 4 instances, a T-junction 3). Base mesh is a single quad. Material uses a UV-based stripe shader with no textures: `float stripe = step(0.5, fract(UV.x * 5.0)); ALPHA = stripe;` — the quad's U axis spans the road width so stripes run parallel to the kerb. Transform layout: 12 floats per instance (same `PackedFloat32Array` format as agent/building transforms). Per-instance: **origin** = node position + edge direction × ~3 m (slightly inset from the node centre, snapped to terrain height); **basis X** = edge tangent (along road); **basis Z** = edge normal (across road); **scale X** = edge road width. Rebuilt whenever road topology changes (same dirty-flag pattern as `buildings.gd`).

   **Rust export** (`nodes/simulation_node.rs`): `get_crosswalk_transforms() -> PackedFloat32Array` — iterates all nodes where `crosswalk == true`, then iterates each adjacent edge arm and emits one transform as described above. `toggle_crosswalk(node_id: i64)` — flips `node.crosswalk`, marks crosswalk data dirty.

   **Toggle tool** (`godot/scripts/crosswalk_tool.gd`, new): reuses `get_closest_node()` for click detection (already exists on `SimulationNode`). On click: calls `toggle_crosswalk(node_id)`; refreshes the MultiMesh. Displays a highlight ring around nodes where `crosswalk == true` using a small `ImmediateMesh` or a second sparse MultiMesh. Tool is registered in `InputManager` under a keyboard shortcut alongside the existing tool keys.

   **Pedestrian crossing constraint** (deferred to v0.1 — see item 68): the visual and data model ship first. Enforcement requires the pedestrian graph to be side-aware (see item 68).

### v0.1 — Economy Foundation

Target: a closed, utility-driven economic loop at 100k agents. Activity decisions are driven by agent state via a Maslow-inspired need hierarchy. Transit mode is chosen by utility scoring over all available modes. Living standard is a derived read-only metric — an aggregate of per-level need satisfaction — used for immigration and city rating.

**Maslow mapping**:
| Level | Needs | Simulation drivers |
|---|---|---|
| 1 — Physiological | Food, shelter, rest | `hunger`, home building, rest recovery |
| 2 — Safety | Income, health, security | `money`, employment, hospital/police coverage |
| 3 — Social | Community, belonging | entertainment, parks |
| 4 — Esteem | Status, quality of life | neighbourhood desirability, housing quality |

Lower levels dominate via soft priority weighting (not hard gating): `urgency(need) = base_weight(level) × (1 − satisfaction) × (1 + w_priority × unmet_lower_needs)`. Higher needs never fully drop to zero — agents occasionally visit the park when slightly hungry — they just do so far less. Level 5 (self-actualisation) is too abstract for simulation and is omitted.

**Implementation order**: item 59 (soa_derive) first. Then 60 (utility decisions) + 61 (need levels) as a unit. Then 62 (multi-modal utility) once bicycle infrastructure (item 30) is live. Items 63–65 (needs, supply chain, services) follow in dependency order.

60. **Utility-based agent decision system**: replace the hardcoded 5%/40% activity selection in `TRANSIT_IDLE` with explicit utility scores driven by need-level satisfaction. Each activity scores against the relevant level: `score(work) = w_safety × (1 − safety_sat[i]) + w_income × (1 − money/cap)`, `score(shop_food) = w_physio × hunger[i]`, `score(stay) = w_rest × (1 − happiness[i]) + w_esteem × esteem_sat[i]`. Agent picks the highest-scoring activity. All weights live in a shared `AgentConfig` struct. Evaluation cost: ~12 multiplies per activation (~5% agents per second), negligible at any scale. Prerequisite: item 59.

61. **Need-level satisfaction fields and living standard**: add four per-agent satisfaction scalars to the SoA — `physio_sat`, `safety_sat`, `social_sat`, `esteem_sat` — each in `[0, 1]`. Updated every N seconds (not per tick) via `par_iter`. Formulas: `physio_sat` = `f(hunger, has_home)`; `safety_sat` = `f(money, employment, safety_grid[home_pos])`; `social_sat` = `f(entertainment_grid[home_pos])`; `esteem_sat` = `f(desirability_grid[home_pos], housing_quality)`. `living_standard[i]` is derived as a weighted sum across all four levels — a read-only output used for immigration gates, city rating, and the Implemented Systems description. DataGrid lookups are O(1); periodic update keeps per-tick cost zero.

62. **Multi-modal utility transit selection**: replace the 500 m car threshold in `decide_transit_mode` with utility scoring over all available modes (walk, bike, car, bus, train). Score per mode: `−w_time × estimated_time − w_cost × trip_cost + w_pref × personal_preference[i]`. Pre-screen with straight-line distance; CCH pathfind only the winning mode — keeps max 1–2 CCH calls per activation (same as today). New SoA fields: `has_bike: Vec<bool>`, `eco_preference: Vec<f32>`. Bus and train scores require `bus_access_grid` and `train_access_grid` (DataGrid lookups written when stops are placed); both default to 0 until infrastructure exists. Prerequisite: item 30 (bicycle), item 59.

63. **Agent needs — physiological level**: `hunger: Vec<f32>` decays passively each tick; shop visits targeting food buildings restore it. Fulfils `physio_sat` (item 61). Shop trips become need-driven — `score(shop_food) += w_hunger × hunger[i]`. New building field: `product: ProductType` enum (Food, Goods, …). When a food shop has no stock, `hunger` cannot be restored; `physio_sat` drops; safety and higher levels are suppressed via soft priority. Prerequisite: items 60, 61.

64. **Supply chain and building economic actors**: buildings gain `stock: f32`, `revenue: f32`. Farms/factories accumulate stock proportional to employment fill rate. Shops deplete stock on agent purchase; zero stock means the shop cannot restore the relevant need (natural demand signal). Buildings make utility-based decisions each N seconds (hire, adjust production) driven by revenue and vacancy — same utility pattern as agents, negligible cost since building count << agent count. Supply transport: goods flow along road graph connections at a rate proportional to road connectivity (no individual truck agents). Prerequisite: items 60–63.

68. **Pedestrian crossing constraint** (prerequisite: item 67 crosswalk data model): enforce that pedestrians and cyclists can only change sides of a road at a node where `crosswalk == true`. Currently pedestrian sidewalk position is a pure rendering offset — the CCH foot path has no concept of sides. Implementation: expand the FOOT pathfinding graph so each directed edge becomes two directed sidewalk lanes (`edge_id × 2 + side`). At each node, add same-side continuation edges (free) and crossing edges (zero cost, only present if `node.crosswalk == true`). Run this side-aware graph through the existing A\* or CCH foot query. Agents whose path requires a crossing at a non-crosswalk node must detour to the nearest crosswalk node. Storage: the side-aware graph is a thin wrapper over the existing `RegionGraph` — no persistent duplication of edge data, just a doubled virtual node count during pathfinding queries. Prerequisite: item 67 must ship first (data model and crosswalk flag must exist). Also fixes B4 (wrong sidewalk normal) as a side effect — once the path is side-aware the ON\_ROAD normal sign is driven by the path's recorded side, not recomputed from `bezier_t`.

65. **Service buildings — static coverage model**: police stations, hospitals, and fire departments emit influence onto `safety_grid` and `health_grid` (same `DataGrid<f32>` architecture as pollution/noise). These grids feed directly into `safety_sat` (item 61 — level 2). No event simulation, no dispatch — static coverage is the correct first model at city scale. Building-level utility decisions (resource allocation) follow the same pattern as item 64 when that ships.

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

## Speculative / Post-v1.0

Ideas that may or may not ever be worth building. Recorded here so the reasoning is not lost, not because they are planned.

### Multiple CCH metric hierarchies

Instead of one CCH hierarchy with inner-path expansion, build a separate contraction hierarchy per cost function — e.g. a time hierarchy, a congestion-weighted hierarchy, an eco hierarchy (fuel or pollution cost). Each hierarchy precomputes shortcut costs for its own metric, so query time has no expansion overhead.

**When this would be worth it:** if the game ever exposes routing preferences to players ("avoid congestion", "prefer scenic roads", "minimise emissions") or if agents genuinely optimise different objectives (trucks minimising fuel, tourists taking scenic routes, cyclists avoiding hills). Neither is currently planned — agents always take the fastest route and players have no routing controls.

**How to add it when needed:** the contraction order is topology-only and is shared across all metrics — compute it once. Each additional metric runs its own O(E) customisation pass over the shared elimination tree to produce a separate shortcut cost table. Query selects the appropriate cost table. The `inner_path` storage in shortcuts can be dropped since costs are precomputed per metric. Memory scales linearly with metric count; topology rebuild cost is unchanged.

**Why it is not worth it now:** one metric, no player routing preferences, and inner-path expansion overhead is negligible at city scale. Adding the infrastructure before the game design requires it is pure maintenance cost.

---

See [`docs/reference.md`](reference.md) for grid specs, movement speeds, memory budget, design patterns, transport vocabulary, Godot scene tree, script→Rust method inventory, and data buffer formats.