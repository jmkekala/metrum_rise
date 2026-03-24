# Metrum Rise — Project State

**Scale target**: ≥ 1,000,000 concurrent agents on a 20 km × 20 km map.
**Current milestone**: v0.01 — playable and correct at 10,000 agents, 500 buildings, 50 road edges, ≥ 30 FPS.

Severity tags: `[BLOCKER]` = must fix before v0.01. `[BUG]` = correctness failure, fix in v0.01. `[v0.01]` = strong target for v0.01 quality. `[v0.1]` = 100k-agent milestone. `[v1.0]` = 1M-agent milestone.

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
- `TransitGraph` in `simulation/network/graph.rs` — adjacency list as two parallel `Vec`s (`nodes`, `edges`) plus a `HashMap<(i32,i32), Vec<usize>>` spatial chunk index (512 m chunks) and a union-find alias map.
- Supported road types: 2-lane standard (10 m total: 7 m asphalt + 1.5 m sidewalk each side).
- `TransitNetwork` (`network/mod.rs`) — `add_road`, `split_edge`, `merge_nodes`.
- Topology: intersection detection and edge splitting in `topology.rs`.
- Road and junction mesh generation in `network/render/road.rs` (405 lines).
- Soft deletion: edges set `deleted = true` but never physically removed (see Bugs).
- Lane types and one-way rules in `network/types.rs`.

### Zoning
- `simulation/grid/zoning.rs` (470 lines) — edge-aligned zoning cells, 10 m × 10 m.
- Cells extend up to 10 cells deep (100 m) from the road sidewalk edge, starting 5 m from road centreline.
- Obstruction check (`is_cell_obstructed`): 5-point sampling (4 corners + centre) per cell with asphalt collision and Voronoi ownership test.
- `left_blocked` / `right_blocked` cache fields exist on `EdgeZoning` but are not correctly wired as an invalidation cache (see Bugs).
- Zone types: Residential, Commercial, Industrial, Office, Mixed.

### Environmental Grids
- `simulation/grid/pollution.rs` — industrial emission (+100/tick, **spec says +5** — see Bugs) + 4-neighbour diffusion, decay ×0.995, parallelised with Rayon.
- `simulation/grid/noise.rs` — traffic noise diffusion, parallelised.
- `simulation/grid/desirability.rs` — composite formula: `50 − pollution × 2 − noise × 1.5`, parallelised.
- All three use `DataGrid<f32>` at 2000 × 2000 resolution (current; target is 500 × 500 — see Backlog).

### Buildings
- Desirability gate enforced (> 50). Spawn throttle active (max 10 buildings per tick).
- Rendered via MultiMesh instancing: one draw call per zone type.
- Rendered via MultiMesh instancing: one draw call per zone type.
- Building deletion via swap-remove, O(1).
- Save/load via `serde_json` — **not yet wired end-to-end**.

### Agents
- `simulation/economy/agents.rs` (708 lines) — `AgentSystem` in Structure-of-Arrays (SoA) layout.
- FSM states: `IDLE → DEPARTING → ON_ROAD → ARRIVING → IDLE` + `IMMIGRATING`.
- Movement: polyline traversal with sub-tick `remaining_dist` budget; lane offsets from road width / lane count.
- Agent kill: swap-and-pop, O(1).
- **Single-threaded tick** — Rayon parallelisation is a v0.01 goal (see Backlog).

#### Agent Rules
- **Immigration**: agents spawn at highway border nodes, arrive by car. Capped at `residential_capacity × 1.1`.
- **Housing search**: immigrants drive toward city centre and claim the first residential building with free capacity (6 agents per plot, hard-coded).
- **Daily cycle**: Home (rest/happiness recovery) → Work (Industrial or Commercial, earn money) → Shop (Commercial, spend money) → Home.
- **Happiness/money**: fields initialised (happiness = 50, money = 100) but **never modified** (see Bugs).

### Pathfinding
- `simulation/pathing/astar.rs` — binary-heap A* with `(node, incoming_edge)` state key (correctly handles turn restrictions at `Node::lane_connections`).
- `simulation/pathing/cost.rs` — edge cost = `length / speed_limit`.
- A* heuristic: `euclidean_distance / 100` — admissible but weak; divisor should be `v_max` (see Bugs).
- `simulation/pathing/hpa.rs` — HPA* **build phase correct**: chunk boundary abstract nodes, per-chunk Dijkstra, stores `abstract_edges`. **Query phase optimized**: utilizes hierarchical search (local searches + abstract graph A*) and caches concrete adjacency list for O(1) fetch.

### Demand
- `simulation/economy/demand.rs` — global R/C/I demand counters. Demand increments globally; buildings consume it on spawn.

### Godot Bridge
- `nodes/simulation_node.rs` (1,655 lines) — all Godot `#[func]` API, rendering helpers, editor tools, undo stack, benchmarking. `[BLOCKER]` for splitting (see Backlog).

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
| B2 | `pathing/hpa.rs::find_path` | [DONE] Hierarchical search implemented; concrete adjacency list cached. | `[BLOCKER]` |
| B5 | `buildings/allocator.rs` | [DONE] Desirability gate enforced (> 50). | `[BLOCKER]` |
| B6 | `buildings/allocator.rs` | [DONE] Spawn throttle (max 10/tick) and HPA* batching implemented. | `[BLOCKER]` |
| B7 | `simulation_node.rs` | God-object: 1,655 lines mixing API, rendering, editing, undo, benchmarking — must split | `[BLOCKER]` |
| B8 | `network/graph.rs::find_or_add_node` | [DONE] O(N) scan replaced with 16m spatial node grid | `[BLOCKER]` |
| B9 | `pollution.rs` line 30 | [DONE] Emission corrected: +100 → +5 per tick | `[BUG]` |
| B10 | `agents.rs` | [DONE] Happiness and money wired: commute penalties, daily activity rewards, and pollution effects implemented. | `[BUG]` |
| B11 | `pathing/hpa.rs` A* heuristic | [DONE] Divisor precomputed from max speed limit during graph build | `[BUG]` |
| B12 | `simulation_node.rs` | `undo_stack.remove(0)` is O(N); replace `Vec` with `VecDeque` | Minor |
| B13 | `simulation_node.rs::get_node_connection_count` | O(E) scan; should use `graph.adjacency` | Minor |
| B14 | `network/graph.rs::remove_from_spatial_index` | O(chunks × edges/chunk) full scan on delete | Minor |
| B15 | `network/graph.rs` | Soft deletion: edges marked `deleted = true` but never compacted — all O(E) scans degrade over a session | Minor |

---

## Backlog

### v0.01 Blockers — fix before tagging

1. [DONE] **Fix HPA* query** (B2): rewrite `find_path` to run A* on the pre-built abstract graph for inter-chunk traversal, then local A* within source and destination chunk. Cache shared read-only adjacency list inside `HpaGraph` post-`build()`.
2. [DONE] **Fix `pollution.tick()` double-call** (B1): remove one of the two calls in `simulate_tick`.
4. [DONE] **Add desirability gate to `allocator.tick`** (B5): read `desirability.grid.get(cx, cy) > 50` before spawning.
5. [DONE] **Add spawn throttle to `allocator.tick`** (B6): max ~10 buildings per tick; batch-dirty HPA*; rebuild once at end of tick.
6. [DONE] **B7: Split `simulation_node.rs`** into modules.
7. [DONE] **B12: Fix Undo O(N) performance** by using `VecDeque`.
8. [DONE] **REGRESSION: Zoning cell overlap** (FIXED - Graph consistency & Cache refresh)
9. [DONE] **REGRESSION: Building-Road overlap** (FIXED - Sidewalk offsets added)
10. [DONE] **REGRESSION: Frontage Split Zoning Loss** (FIXED - Cache migration)
11. [DONE] **Fix `find_or_add_node`** (B8): replaced O(N) scan with a 16m spatial node grid.

### v0.01 Goals — strong targets for v0.01 quality

8. [DONE] **Wire happiness and money** (B10): happiness +1/day at home, −commute_time/60 per trip, −pollution × 0.1/day; money +10/day at work, −20 per shop.
9. [DONE] **Fix pollution emission** (B9): change +100 to +5 in `pollution.rs`.
10. [DONE] **Fix A* heuristic** (B11): replace divisor `100.0` with `graph.max_speed_limit()` precomputed at HPA* build time.
11. [DONE] **Optimize Zoning Cache & Parallelize** (B5/B11ish): Fixed $O(Cells \times E \times L)$ regression in visualization by correctly using the obstruction cache. Parallelised `recalculate_obstructions` with Rayon and implemented spatial invalidation for nearby roads.
12. [DONE] **Coarsen environmental grids to 500 × 500**: run diffusion at 1 MB instead of 16 MB per grid; bilinear upsample for display. 16× memory and compute reduction.
13. [DONE] **Incremental HPA* rebuild** on road edit: mark affected 512 m chunks dirty, rebuild only those. O(E_chunk) instead of O(E_total).
14. **Split `agents.rs`** (708 lines) into `agents/data.rs`, `agents/decisions.rs`, `agents/tick.rs`.
15. **Split `graph.rs`** (801 lines) into `graph/data.rs`, `graph/spatial.rs`, `graph/topology.rs`, `graph/rebuild.rs`.
16. **Add pathfinding unit tests**: highway cheaper than dirt road; slope penalty forces bypass; flow field Dijkstra < 5 ms on a 1,000-node graph.

### v0.1 — 100k-agent milestone

17. **Virtual Frontages**: change building address from physical edge splits to `(EdgeID, t: f32)`. Agents arrive by reaching the T-coordinate and trigger arriving state. Decouples graph size from city density.
18. **Flow fields for shared destinations**: one Dijkstra from destination per zone type per tick produces a `DataGrid<f32>` cost map. Agents query their cell instead of running individual A*. Reduces O(A × E log N) pathfinding to O(M × E log N) where M ≈ 10–100 zone types.
19. **Parallelise `AgentSystem::tick`** with `rayon::par_iter_mut` over agent chunks. Use `AtomicU32` for `parking_occupied`; accumulate congestion deltas per chunk and merge after parallel phase.
20. **Compact deleted edges** periodically: drain soft-deleted entries and remap all indices.
21. **R-Tree spatial index** (`rstar` crate) for fine-grain edge queries — O(log N) insert/delete/query vs. current O(N) scan on delete.

### v1.0 — 1M-agent milestone

22. **Contraction Hierarchies or Customizable Route Planning (CRP)**: CH for static costs; CRP if congestion changes per-tick (separates topology preprocessing from per-tick metric updates).
23. **Agent Level-of-Detail**: full FSM for camera-visible agents (~50k), flow-field only within 2 km (~500k), statistical counts only beyond 2 km.
24. **GPU compute (`wgpu`)**: move agent position update arithmetic (linear interpolation along polylines, lane offset, transform generation) to GPU. Keep FSM decisions and pathfinding on CPU.
25. **Building levels (1→3)**: upgrade driven by demand pressure history and neighbourhood desirability.
26. **Congestion heatmap**: agent spatial grid (`DataGrid<Vec<AgentID>>`) → edge congestion → dynamic cost update → routing feedback loop.
27. **Incremental road mesh updates**: rebuild only modified edges; simplified LOD geometry at camera distance.

---

## Architecture Reference

### Grid Specifications

| Parameter | Value |
|-----------|-------|
| Zoning cell | 10 m × 10 m |
| Building footprint | 3 × 3 cells (30 m × 30 m) |
| Road width (2-lane) | 10 m (7 m asphalt + 1.5 m sidewalk each side) |
| Zoning offset from centreline | 5 m |
| Zoning depth | 10 cells (100 m) |
| Road spatial chunk | 512 m |
| Environmental grid (current) | 2000 × 2000 |
| Environmental grid (target) | 500 × 500 |

### Movement Speeds

| Mode | Speed | Notes |
|------|-------|-------|
| Walking | 4.0 m/s (14.4 km/h) | ~3× real life; 10 m road takes 2.5 s |
| Driving | 20.0 m/s (72 km/h) | Standard suburban |

### Key Design Patterns

- **DataGrid\<T\>**: flat `Vec<T>` with stride `width`. Row-wise parallel iteration with `rayon::par_chunks_mut`. All spatial grids (terrain, pollution, noise, desirability, planned car collision) use this type.
- **SoA (Structure-of-Arrays)**: `AgentSystem` stores all fields as parallel `Vec<T>` indexed by agent ID. Cache-friendly for bulk iteration.
- **512 m spatial chunks**: road edge AABB registered in all overlapping chunks. Used for editor queries (radius ≈ 120 m → typically 1 chunk) and HPA* chunk assignment.
- **`(node, incoming_edge)` pathfinding state**: required for turn restriction correctness at `Node::lane_connections`. Must be preserved in any pathfinding replacement.

### Memory Budget (20 km map)

| Resource | Size |
|----------|------|
| Terrain heightmap (2000²) | 16 MB |
| Terrain source copy | 16 MB |
| 3 environmental grids at target 500² | 3 MB |
| 3 environmental grids at current 2000² | 48 MB |
| Road edges (50k × ~512 B) | 25 MB |
| Road nodes (100k × ~128 B) | 12 MB |
| Agent SoA (1M × ~120 B) | 120 MB |
| HPA* abstract graph | ~10 MB |
| Road mesh VRAM (50k edges) | ~144 MB VRAM |

**Bandwidth note**: 3 environmental grids at 2000² = 48 MB of memory traffic per tick. At 10 ticks/s this approaches practical DDR4 bandwidth. Coarsening to 500² is mandatory before raising tick rates.

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
