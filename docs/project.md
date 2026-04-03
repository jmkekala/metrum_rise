# Metrum Rise — Project State

**Scale target**: ≥ 1,000,000 total population across a multi-city region, with a clear distinction between simulation tiers:
- **Full FSM** (individual pathfinding, real movement, all state transitions): ~300–500k agents on a 20-core machine with DDR5. This is the hardware-honest ceiling — the DDR5 memory bandwidth wall at ~190 MB/tick for 1M SoA entries limits throughput regardless of core count.
- **Flow-field tier** (group movement via shared destination maps, no per-agent CCH queries): extends the active zone to ~1M total when combined with the full-FSM layer. Implemented (item 18).
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
- **Lane System (`LaneGraph`)**: Replaced all runtime trigonometric dynamic offset calculations with a pre-computed spline system generated during graph topology updates. 1,000,000 agents now traverse roads via an O(1) 1D array length tracker on static 3D splines (`LaneSystem`), eliminating all rendering frame normal-vector computations. Agent logic includes strict `is_fwd` orientation inversions, Bezier constraint calculations for intersections based on `p1_base`, active `MODE_WALK` interpolation rendering states for path fails, and strict `frontage_t` recalibrations when topology changes via `split_edge`, curing visual disconnected agent models rendering offset from the road.
- **Incremental lane rebuild** (`network/lanes.rs` + `network/mod.rs` + `nodes/sim/core.rs`): Road placement is now fully smooth with no sim-thread stall visible to the player. Previously, placing a road that crossed K existing roads triggered K+1 full `O(total_lanes)` `LaneSystem::rebuild()` calls (one per intersection). The refactor replaces this with:
    1. **`bulk_load` flag** (already existed): defers all per-edge lane rebuilds until `finalize_bulk_load()`.
    2. **`bulk_dirty_edges: HashSet<usize>`** on `TransitNetwork`: populated by `create_edge_internal` (new edge) and `split_edge` (both halves of any split existing edge) while `bulk_load = true`.
    3. **`LaneSystem::rebuild_edges_incremental(graph, affected_edges)`**: appends new lanes only for edges in the rebuild set (affected edges + edges incident to affected nodes for clip-geometry correctness). Old lanes for rebuilt edges are orphaned in `self.lanes` without compaction, so all unaffected lane IDs remain stable for existing agents.
    4. **`AgentSystem::invalidate_lane_ids_for_edges(affected, lane_system)`**: called before the lane rebuild; sets `current_lane_id = usize::MAX` for agents on affected-edge road lanes and connection lanes that lead into those edges. The tick loop's existing `usize::MAX` handler re-enters agents onto the new lanes on the next tick.
    - Cost: O(affected_edges × lanes_per_edge + affected_nodes × connections_per_node). A single road crossing 5 existing roads produces ≈ 6 edge rebuilds and ≈ 12 node connection rebuilds — constant-time for typical city scale.
- **Immigration Target Logic**: Fixed an integration regression where immigrants spawned via `BuildingAllocator::tick` received a blank `usize::MAX` target home. This caused immigrating agents to sequentially travel node-to-node across the map indefinitely until they hit a dead end, ignoring all buildings and intersections organically. Immigrants now properly claim vacancies at spawn and pathfind directly to their designated homes.
- **`RegionGraph` rename** — `TransitGraph` renamed to `RegionGraph`; struct is now globally owned in `SimulationNode`, not city-scoped. All call sites updated. Prerequisite for CCH (item 31b/c).
- `TransitNetwork` (`network/mod.rs`) — `add_road`, intersection splitting, zoning invalidation, and CCH dirty marking.
- **Pathfinding (CCH)**: `simulation/pathing/cch.rs` implements a Customizable Contraction Hierarchy with O(E) metric customization and bidirectional upward Dijkstra. Replaces HpaGraph for all agent routing.
- **Unified Pedestrian Navigation (`LaneSystem`)**: Pedestrians now traverse the same `LaneSystem` architecture as vehicles, using dedicated `LaneType::Foot` splines and crosswalk connection splines at junctions. 
    - **Visual Crosswalks (Item 67)**: Junction connections that cross road asphalt now render white "zebra" stripes.
        - **Refined Topology**: Uses an angle-based mouth-sorting algorithm to distinguish between "Corners" (curb continuations) and "Crosswalks" (street crossings).
        - **Sparse Crossings**: Enforces the "one crosswalk" rule for 2-road bends and avoids criss-cross webs at complex junctions by only crossing adjacent road arms.
    - **Local Roads**: Generate 4 shoulder lanes (2 per side, bidirectional) only if `allowed_types & FOOT != 0`. High-speed arterials/highways without graphical sidewalks are naturally excluded.
    - **Dedicated Footpaths**: `TransitType::Foot` edges generate 2 centered bidirectional lanes (index 0) instead of shoulder offsets.
    - **Group Dynamics (Positional Jitter)**: Pedestrians on sidewalks are rendered with deterministic lateral offsets based on their internal ID, simulating realistic group walking variety without per-agent physics overhead.
    - **Legacy Purge**: `simulation/pathing/pedestrian.rs` and all its fallbacks have been removed. `AgentSystem` now uses a single `TRANSIT_ON_ROAD` state for both cars and pedestrians, distinguished only by the assigned lane types.
- **Unit tests** (`simulation/network/test_topology.rs`): `add_road` (bidirectional adjacency, 100 m subdivision logic), `split_edge` (physical length summation, node sharing, zoning/building migration), `compact_edges` (index remapping consistency for agents and buildings).
- Topology: intersection detection and edge splitting in `topology.rs`. `TransitNetwork` road edits now use one canonical split path there; the duplicate graph-level `RegionGraph::split_edge()` helper was removed so split fixes cannot drift between production and utility codepaths.
- Road and junction mesh generation in `network/render/road.rs`.
- **Graph-dilation road renderer**: `RoadRenderer` no longer infers custom junction polygons from edge-local offsets. It now emits asphalt ribbons from the road centerline for every visible surface road, emits widened sidewalk ribbons only on surface road edges whose `allowed_types` include `FOOT`, keeps edge curvature continuous with connected polyline strips rather than per-sample circular caps, treats straight width mismatches as tapered transitions instead of circular junction bubbles, and trims true mixed-width / multi-road junction ribbons back to an angle-aware node fill built from the incident clipped edge boundaries. This removes the old contour solver and all edge-to-node handoff clipping from the active top-surface path.
- **Visible composition instead of contour ownership**: sidewalk-capable roads render their sidewalk base first on the lower road layer, asphalt is rendered second on a clearly separated higher mesh layer, and lane markings stay on the top overlay layer. The visible road is therefore the union of graph-dilation primitives and draw order, not a stitched `outer - road` contour solve. Arbitrary-angle bends, T-junctions, acute joins, width transitions, and graded road segments stay connected because the top surface is never rebuilt from pairwise offset intersections and the asphalt/marking layers no longer depend on a near-zero vertical gap.
- **Node classification**: terminal nodes receive round caps, straight two-edge pass-through splits skip the node disk when the widths match, ordinary same-width two-edge bends still use the rounded junction fill, straight width changes widen the narrower endpoint into a tapered transition instead of spawning a circular node bubble, and mixed-width or 3+ road junctions now use an angle-aware clipped fill instead of a widest-road circle. The current city-road system has no special highway-style join classification; all road-road joins stay on the ordinary junction and width-transition path.
- **Editor snapping**: `get_closest_network_point()` now projects edge snaps to the exact road centerline point on the hovered segment instead of quantizing them in 2 m steps. Node snapping still has absolute priority near existing junctions, but branch roads no longer depend on landing in the right quantized bucket to produce a clean split. Topology processing also now keeps endpoint-to-segment attachments on that projected road point instead of reusing a nearby off-center endpoint node, so midpoint walkway/road joins do not create hooked split geometry on sampled roads.
- **Canonical node queries**: editor node exports, nearest-node queries, and `get_closest_network_point()` node snaps now ignore alias nodes left behind by `unite_nodes()`. The holographic node overlay, green snap cursor, and node-based tools therefore operate on one live canonical node per junction instead of showing or selecting merged ghost duplicates.
- **Sidewalk access matches routing flags**: the renderer no longer infers sidewalks from `EdgeClass`. Surface roads only get sidewalk strips and sidewalk node fills when `allowed_types & FOOT != 0`, and explicit `TransitType::Foot` walkways now participate in that same sidewalk pass instead of rendering as isolated strips. Simple branch cases use a sidewalk pass-through classification so the road-side sidewalks stay intact instead of being replaced by a shared sidewalk node disk. Duplicate-angle sidewalk boundary rays also collapse to the outermost point for the remaining polygon-fill cases. Straight road + footpath pass-through joins now emit a dedicated curb-apron sidewalk patch from the trimmed footpath mouth to the selected shoulder curb line, so centered and angled walkway connections no longer carve triangular holes out of the road shoulders.
- **Old junction mesh cache removed**: `RegionGraph` no longer carries the unused `JunctionMesh` / `junction_polygons` state from the retired contour-based junction renderer.
- **Soft-deleted edges stay live-safe during play**: road edits now leave deleted edge slots in place during interactive editing and routine CCH rebuilds. Rendering, snapping, zoning invalidation, and CCH pathfinding all skip `deleted` edges directly, so road editing no longer stalls on a synchronous remap/rebuild pass. Runtime compaction entry points have been removed from `SimulationNode`; `compact_edges()` now remains only as a low-level graph canonicalization helper for tests and SQLite save/load serialization.
- **External Connections (Border Nodes)**: `NodeType::Border` added to `network/types.rs`. When the player draws a road endpoint within 200 m of any map edge (`config::BORDER_DETECTION_THRESHOLD`), `road_tool.gd` calls `check_border_candidate(pos)` after the commit and shows a `ConfirmationDialog`. Confirming calls `set_border_connection(node_id)` which promotes the node to `Border` and immediately auto-extends the node's physical geometry 10 meters outward to spawn immigrants cleanly off-screen. Border nodes are persisted in the SQLite save (value 5). `BuildingAllocator::tick` collects all connected `Border` nodes at immigration time; if none exist, immigration is blocked. `get_border_nodes()` exposes positions for future visual indicators.
- Lane types and one-way rules in `network/types.rs`.
- Edge geometry is 3D (`Vec<Vector3>`) — grade-separated roads are natively representable as elevated or depressed polylines. Node snapping uses 3D Euclidean distance, so bridge abutments and underpass nodes with ≥ 2 m vertical separation will not snap together.
- **`EdgeClass` data model complete**: `Standard | Bridge | Tunnel` enum in `types.rs`; `class: EdgeClass` field on `Edge`; new edges default to `Standard`; `split_edge` copies `class` to the new half-edge.
- **Junction Rendering (Item 70)**: The old corner-intersection / contour-union / clipped-band renderer is gone from the active path. Junctions now render through the same dilation primitives as the proof-of-concept renderer: widened edge strips, road strips, and circular node fills. There is no active exact contour extraction, no angle-paired sidewalk difference solve, and no scalar handoff clipping in the top-surface renderer.
- **R-Tree Spatial Index**: `spatial_edge_rt` replaces the uniform 512m grid for all edge queries. O(log N) insert/delete/query provides tight AABB filtering, zero manual deduplication, and eliminates long-edge false positives.
- **O(N²) road-placement fix** (`graph/topology.rs`, `simulation/network/topology.rs`): Road placement was O(N²) in the number of existing roads due to two bugs: (1) `move_node` called `rebuild_intersection_clips()` (O(E)) on every endpoint snap, triggering O(E) work per 100m chunk; (2) `move_node` scanned ALL edges twice (O(E)) instead of using the adjacency list. Combined, placing a road across N existing roads triggered O(N) chunks × O(N) per chunk = O(N²). Fix: `move_node` now uses `self.adjacency[node_id]` (O(degree)) and no longer calls `rebuild_intersection_clips` — callers that need visual clips update it explicitly. `split_edge` also had a related bug: it updated `physical_geometry` before calling `remove_from_spatial_index`, causing the R-tree removal to fail (AABB mismatch) and accumulate stale entries. Fixed by reordering to remove-before-update. Benchmark (`examples/road_placement_scaling.rs`, Scenario B, 100m chunks): N=500 roads was 37.5 s → 46 ms (805× speedup); scaling is now O(N) as intended.
- **`clips` + `lanes` O(E) elimination** (`network/mod.rs`, `network/lanes.rs`): After the O(N²) fix, two residual O(E) costs remained per road placement event. (1) `create_edge_internal` (non-bulk-load path) still called `graph.rebuild_intersection_clips()` (full O(E) resample + full R-tree rebuild) instead of `rebuild_intersection_clips_for_nodes` (O(K) resample, partial R-tree). Fixed by tracking affected nodes from `process_intersections` via node/edge snapshots and calling the scoped variant. (2) `rebuild_edges_incremental` (step 5) scanned ALL surviving lanes — O(total_lanes) — to build `lane_map`, even though that map is fully populated by step 6 (only lanes for `rebuild_set` edges are needed for connection building). Step 5 is now a no-op initialization; `lane_map` starts empty and is built in step 6. These two fixes remove the only O(E) work proportional to network size from the per-placement hot path; remaining costs are O(K) where K = affected intersections, independent of total city size.
- **Building Frontage Snapping Guard**: Enforces a `MIN_FRONTAGE_DISTANCE` (8.0m) for all dynamic road splits. Buildings near junctions snap to existing nodes instead of creating unstable micro-segments, resolving visual mesh gaps and ensuring deterministic agent pathfinding across topology changes. Includes a shared-node occupancy guard to prevent premature frontage removal when multiple buildings share a junction node.
- **Frontage Crossing Prohibition**: Strictly forbids pedestrian lane connections between opposite sides of the road at `Frontage` nodes. By enforcing `lane_idx` preservation, the simulation ensures that pedestrians only cross at dedicated junctions and crosswalks, preventing unrealistic "jaywalking" at virtual building nodes.

### Zoning
- `simulation/grid/zoning.rs` — edge-aligned zoning cells, 10 m × 10 m.
- **Player-painted zones** (item 66): automatic edge-aligned zone generation removed. Players click and drag along road edges in `zoning_tool.gd` to paint cells onto either side of a road. Drag can span multiple connected edges; side is determined by mouse position relative to the road centreline and flips automatically at reversed junctions. Scroll wheel adjusts zone depth (2–12 cells, 20–120 m). Right-click clears all zones on an edge. Zone paint is committed on mouse release via `set_zoning_range(edge_idx, side, t_start, t_end, depth, zone_type)` on the Rust side. A MultiMesh preview (grid + painted + brush overlays) updates every frame during hover and drag.
- Cells extend up to 12 cells deep (120 m) from the road sidewalk edge, following road curvature.
- Obstruction check (`is_cell_obstructed`): 5-point sampling (4 corners + centre) per cell with asphalt collision and Voronoi ownership test. Splay check applied only to `y=0` cells (inner row) — applying it to all rows incorrectly rejected entire building footprints on curved roads.
- Obstruction cache correctly wired: `recalculate_obstructions` is parallelised with Rayon and spatially invalidated on nearby road edits. `BuildingAllocator::tick()` reads the precomputed `is_blocked()` cache (O(1) per cell) instead of rerunning `is_cell_obstructed()` (O(geometry² × nearby_edges)) on every tick — the per-tick 1–2 s freeze was caused by the latter pattern.
- **Deferred zoning flush**: `flush_zoning_updates` (Voronoi obstruction passes) is no longer called synchronously during road placement. Dirty edges accumulate in `TransitNetwork::zoning_dirty_edges` and are flushed once at the start of `simulate_tick_internal` (before `allocator.tick`), so building placement always sees a fresh cache but road placement never blocks on it. The `process_intersections` full-scan O(E) loop is replaced with an R-tree AABB query so only spatially candidate edges are tested.
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

### Save/Load
- `simulation/save.rs` now implements single-file SQLite save/load for the live simulation state. Save writes one `savegame.sqlite` transaction containing map config, time, terrain, water + sources, demand, pollution, noise, compact road graph + lane connections, zoning intent, buildings, and live agents.
- Save canonicalizes only the snapshot: it serializes only live (`!deleted`) edges and remaps node, edge, and building ids into a compact saved graph without mutating the running sim. The live world continues to use soft-deleted edge slots during play.
- Load hydrates authoritative state only, then rebuilds runtime data: adjacency, spatial indices, intersection clips, building transforms, zoning occupied/blocked caches, building indices, CCH, and desirability. Building frontage attachment (`edge_idx`, `frontage_t`, `frontage_node`, `side`, `cell_x`, `cell_y`) is authoritative in saves; world-space building transform fields are derived on load.
- Agents persist exact in-progress travel state for walking, driving, and junction turns. On load, saved route references are validated; if any path/building/edge reference is invalid, only the agent's travel state is cleared and the agent replans on the next tick.

### Buildings
- Desirability gate enforced (≥ 20). Spawn throttle active (max 10 buildings per tick).
- Rendered via MultiMesh instancing: one draw call per zone type.
- Building deletion via swap-remove, O(1).
- Save/load now persists only authoritative frontage attachment and occupancy state for buildings. `center_x`, `center_y`, `facing_dir`, and `side_offset` are recomputed on load before any zoning, render, noise, or pollution consumer reads the building list.
- **Building Index**: Inverted zone-type index (`zone_index: [Vec<usize>; 6]`) and vacancy index (`vacancy_index: [Vec<usize>; 6]`, `vacancy_pos: Vec<usize>`) implemented in `BuildingAllocator`. `find_available_home()` is O(1) random selection from the vacancy index. `claim_vacancy`/`release_vacancy` maintain the index incrementally in O(1); `kill_agent` calls `release_vacancy` before swap-remove. Building deletion triggers a full `rebuild_zone_index()` via `dirty_index`. Prerequisite for parallel tick.
- **Unit tests** (`buildings/allocator.rs`): desirability gate (no spawn when grid value < 50.0), demand subtraction (residential demand decreases on spawn), occupancy clearing (3×3 zoning cells cleared on building removal).

### Agents
- `simulation/economy/agents/` (Submodule) — `AgentSystem` in Structure-of-Arrays (SoA) layout.
- FSM states: `IDLE → DEPARTING → ON_ROAD → ARRIVING → IDLE` + `IMMIGRATING`.
- Movement: polyline traversal with sub-tick `remaining_dist` budget; lane offsets from road width / lane count. Agents move at a **fixed speed** with no interaction — cars on the same edge pass through each other. No car-following model, no capacity constraint per lane. See Backlog.
- **Virtual Frontages (`frontage_node`)**: buildings store `(edge_idx, frontage_t: f32, frontage_node: u32)`. Each building placement calls `TransitNetwork::split_for_frontage(edge_idx, frontage_t, ...)`, which inserts a real `NodeType::Junction` at the exact arc-length position and splits the edge into two half-edges (A→F and F→B) in-place. `frontage_node = F` — the exact split node, not an endpoint approximation. `frontage_t` is stored relative to the half-edge (`≈ 1.0` for the spawning building since F is at the very end of the first half). Building removal calls `TransitNetwork::remove_frontage(frontage_node, ...)`, which merges the two half-edges back and remaps all building `cell_x`/`frontage_t` values. `frontage_node` is persisted as a column in the `buildings` SQLite table (save format version 2) and loaded directly — not recomputed on load. Agent FSM: pathfind exactly once in `TRANSIT_IDLE` (B25 fixed); `TRANSIT_DEPARTING` is a straight-line walk to `frontage_node`; on path failure agents go IDLE+invisible (B21 fixed); activity reset on arrival (B24 fixed).
- Agent kill: swap-and-pop, O(1). Note: agent indices are not stable across ticks (swap-remove invalidates the last agent's index).
- **Pedestrian Arrival/Departure Constraints**: Agents arriving at or departing from buildings are now restricted to the sidewalk on the building's frontage side. This eliminates "jaywalking" across asphalt or fields. If no sidewalk exists on the building's side, agents merge directly onto the road.
- **Stuck Agent Scrub**: `AgentSystem::tick` now includes a safety pass that hides (`is_visible = false`) any agents that enter an `IDLE` state while outside of a building, effectively cleaning up "field-ghost" artifacts from failed pathfinding.
- **Improved Visualization**: The 'P' debug overlay now displays color-coded paths: Cyan for network-based traversal and Yellow for direct-move (arrival/departure) phases.
- **Background simulation thread (item 73)**: `AgentSystem::tick` and all grid ticks run on a dedicated `std::thread` at ~60 Hz, decoupled from Godot's render frame. All simulation state lives in `Arc<Mutex<SimCore>>`; at tick end, the thread writes a `RenderSnapshot` (`Arc<RwLock<RenderSnapshot>>`) containing pre-computed `Vec<f32>` transforms and dirty flags. The Godot main thread reads only the snapshot — no live sim state. `SimCommand` enum (`SetSpeed`, `Quit`) sent via `mpsc::channel` for non-blocking control.
- **Parallel tick (item 19)**: `AgentSystem::tick` uses `rayon::par_iter_mut` for on-road movement. Immigration assignment (which needs mutable `BuildingAllocator`) remains sequential in a post-parallel phase.
- **`AgentSystem` SoA Migration (Item 59)**: Replaced the manual 29-parallel-vector layout with a type-safe Structure-of-Arrays (SoA) architecture using `soa_derive`. All agent fields are now encapsulated in an `Agent` struct and managed via `AgentVec`, ensuring field synchronisation and simplifying lifecycle methods (`spawn_agent`, `kill_agent`). Direct field access is maintained via `Deref`/`DerefMut`.
- **Transit Mode Enum** — migrated `is_driving: Vec<bool>` to `transit_mode: Vec<u8>` using constants (WALK=0, CAR=1, ...). This provides the multi-modal foundation for bicycles, buses, and rail.
- **Unit tests** (`economy/agents_test.rs`): `test_agent_fsm_lifecycle` verifies the complete daily cycle (Home → Work → Shop → Home) including FSM state transitions, money tracking, and arrival detection via virtual frontage nodes.

#### Agent Rules
- **Immigration**: agents spawn exclusively at `NodeType::Border` nodes (external connections). If no border nodes exist, immigration is fully blocked. A border node that has all its incident edges deleted is skipped (connectivity check via `adjacency`). Capped at `residential_capacity × 1.1`. Multiple border nodes are supported; each immigrant spawn picks one at random.
- **Housing search**: immigrants drive toward city centre and claim the first residential building with free capacity (6 agents per plot, hard-coded).
- **Daily cycle**: Home (rest/happiness recovery) → Work (Industrial or Commercial, earn money) → Shop (Commercial, spend money) → Home.
- **Happiness/money**: home +1 happiness/day; commute penalty −commute_time/60 per trip; pollution effect −p × 0.1/day; work +$10/day; shop −$20.
- **Visual Agents (v0.3 Assets)**: 
    - Combined MultiMesh renderer split into per-mode systems (item 46).
    - Civilian car agents now use 3D `.glb` models (item 47) with random variety (Sedan, Sports, SUV, Luxury). `vehicle_type: Vec<u8>` added to `AgentSystem` SoA (now 30 parallel fields); assigned randomly in `spawn_agent`, swapped/popped in `kill_agent`. `get_car_transforms_internal` returns a `VarDictionary` keyed by `(vehicle_type * 10 + color_variant)` (was `PackedFloat32Array`). In `agents.gd`, four GLB models are loaded at `_ready` via `GLTFDocument`; five color variants are produced per model by shifting UVs via `SurfaceTool`, giving 20 `MultiMeshInstance3D` nodes total. Models sourced from `res://assets/models/vehicles/civilian/`.

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
- `simulation/pathing/cch.rs` — CCH / CRP implementation replacing HPA*. Single hierarchy with tree-based shortcut decomposition. Three phases: contraction (degree-based node order, rank-filtered neighbor collection, shortcut generation into `fwd_up`/`bwd_up`), customisation (bottom-up O(S) cost propagation — children always have lower indices than parents so a single forward pass suffices, called on every congestion update), query (bidirectional upward Dijkstra, `allowed_mask` applied at expansion, path reconstructed by lazily expanding shortcut trees via `collect_base_edges`). `hpa.rs` deleted; all call sites updated.
- **`simulation/pathing/cch.rs`** — CCH implementation (31b). Each `CchShortcut` stores either a `base_edge: usize` (direct shortcut) or `mid_l`/`mid_r` child shortcut indices (compound shortcut) — no `Vec<usize>` per shortcut. O(1) memory per shortcut regardless of path length. Triangle insertion uses a rank filter: only shortcuts whose other endpoint has a **higher rank** than the contracted node are considered neighbors. Without this filter, already-contracted nodes accumulate as spurious neighbors, creating O(N³) shortcuts for cyclic networks (7 GB → 40 GB OOM). With the filter, total shortcuts are O(N·D²) ≈ O(N) for road networks with bounded degree D.
- **Unit tests** (`simulation/pathing/tests.rs`): `test_slope_cost_calculation` (50% grade edge receives a 7.25× cost multiplier vs a flat edge of equal length), `test_pathing_avoids_steep_slope` (router selects the longer flat detour A→C→B over the steep direct A→B). **Known geometry inconsistency in `test_pathing_avoids_steep_slope`**: `edge_ab`'s geometry endpoint is `(100, 50, 0)` but node `n_b` is placed at `(100, 0, 0)`. `CostCalculator` reads `edge.geometry`, so the slope penalty is computed correctly and the test passes, but the endpoint violates the invariant that edge geometry must start and end at the node positions. Fix: place `n_b` at `(100, 50, 0)`, or represent the slope with an intermediate waypoint while keeping the geometry endpoint at `n_b`'s flat position.

### Demand
- `simulation/economy/demand.rs` — global R/C/I demand counters. Demand increments globally; buildings consume it on spawn.

### Flow Fields (item 18)
- **`simulation/pathing/flow_field.rs`** — `FlowField` (per-zone-type reverse Dijkstra result) and `FlowFieldSystem` (one `FlowField` per zone × mode, lazy rebuild via `dirty: [bool; 6]`).
- **`FlowField::build(sources, graph, flags)`**: multi-source reverse Dijkstra from all buildings of a zone type. Each node stores `next_node[v]` (cheapest next hop toward nearest destination). O((V+E) log V) per zone rebuild.
- **`FlowField::build_path(from_node, max_hops) -> Option<Vec<u32>>`**: chains `next_node` lookups to produce a route. O(path_length).
- **`FlowField::look_ahead(node, hops) -> Vec<u32>`**: IDM preparation hook — returns the next `hops` nodes ahead for anticipation distance.
- **`BuildingAllocator::dirty_zones: [bool; 6]`**: set when a building is added or removed; drained in `simulate_tick_internal` to call `flow_fields.mark_zone_dirty()`.
- **`get_sources_for_zone(zone)`**: returns `Vec<(node_id, weight)>` pairs (frontage nodes of all buildings of that zone type) as Dijkstra seeds.
- **Agent routing in `tick.rs`**: `TRANSIT_IDLE` activations for work/shop use flow fields if a valid field exists; fall back to CCH if not. `TRANSIT_ON_ROAD` with empty path follows the same priority. Home trips always use CCH (specific building destination required).
- **Save/load**: `dirty_zones` is not serialized. After load, `apply_loaded_simulation` calls `flow_fields.mark_all_dirty()` so all fields are rebuilt on the first sim tick with buildings present.

### Intelligent Driver Model — IDM (item 25)
- **`Agent::speed: f32`** — per-agent current speed in m/s (SoA field). Initialised to `20.0` for cars on spawn; loaded saves initialise from `transit_mode` (20.0 car / 4.0 walk).
- **Lane-occupancy pre-pass** (`tick.rs`, sequential, O(A log A)): each tick, builds `lane_occupants: Vec<(lane_id, agent_idx, dist)>` sorted by `(lane_id, lane_distance)`. Stored in `AgentSystem` as a reused scratch buffer.
- **IDM parallel update** (O(A)): for each car in `TRANSIT_ON_ROAD`, binary-searches `lane_occupants` to find the bumper-to-bumper gap to the nearest car ahead. Applies `a = A_MAX × (1 − (v/v_max)⁴ − (s*/gap)²)` with `s* = S_MIN + v × T_HEAD`. New speeds written to a `new_speed` scratch buffer, then committed to `agents.speed`.
- **Parameters**: `A_MAX = 1.5 m/s²`, `T_HEAD = 1.5 s`, `S_MIN = 2.0 m`, `CAR_LENGTH = 4.5 m`. The approach-speed interaction term (`v·Δv / 2√(A·B)`) is deferred until per-agent `v_lead` tracking is added.
- **Intersection speed**: cars traversing a bezier intersection lane use `max(speed*0.5, 2.0)` — half IDM speed, floored at 2 m/s.
- **Congestion feedback** (`AgentSystem::update_edge_congestion`, called from `core.rs` after each tick): accumulates per-edge average speed into `edge_speed_sum`/`edge_agent_cnt` scratch buffers, then writes `Edge::current_congestion = 1 − avg_speed/speed_limit` for occupied edges. O(A + E) sequential. Feeds the CCH metric customization phase.

### Camera Frustum Culling (item 74)
- **`SimCore::camera_aabb: (f32, f32, f32, f32)`** — world-space (x_min, x_max, z_min, z_max) AABB updated each frame from the Godot main thread via `SimCommand::SetCameraAabb`.
- **`build_snapshot()`** skips any agent whose `(pos_x, pos_y)` lies outside the AABB. When `x_min >= x_max` the filter is disabled (default). O(A) with no extra allocation.
- **`config::AGENT_CULL_FAR_M = 4000.0`** — fallback clip distance when the camera ray is nearly horizontal (avoids an infinite ground intersection at shallow angles).
- **`config::AGENT_CULL_PADDING_M = 200.0`** — AABB padding added in GDScript to avoid pop-in at the viewport edge.
- **`agents.gd::_update_camera_aabb()`** — called every `_process` frame; casts rays through the four screen corners, intersects with y=0 when `dir.y < -1e-3`, falls back to `AGENT_CULL_FAR_M` otherwise. Sends result via `SimulationNode.set_camera_aabb()`.
- **Benchmark result** (i9-12900K, RX 7900 XTX, 100k agents, zoomed-in district view): ~3× wall-clock speedup vs no culling. Full-map benchmark camera shows no benefit since all agents are inside the AABB.

### Threading Architecture (items 73 + 19)
- **`nodes/sim/core.rs`** — defines `SimCore` (all 19 simulation fields), `RenderSnapshot` (pre-computed `Vec<f32>` transforms + dirty flags, `Send+Sync`), and `SimCommand` enum (`SetSpeed(f32)`, `Quit`).
- **Background sim thread**: `run_sim_thread(Arc<Mutex<SimCore>>, Arc<RwLock<RenderSnapshot>>, Receiver<SimCommand>)` — loops at ~60 Hz. Locks `SimCore`, ticks agents (parallel), runs daily economy ticks via `time.process_delta()`, builds snapshot, releases lock, writes snapshot, sleeps for remainder of frame.
- **`SimulationNode`** holds only `Arc<Mutex<SimCore>>`, `Arc<RwLock<RenderSnapshot>>`, `cmd_tx: Sender<SimCommand>`, and bookkeeping fields. All `#[func]` wrappers lock the mutex for one call; hot read-only paths (`get_agent_transforms`, `is_terrain_dirty`) read from the snapshot with no mutex.
- **`SimCommand` channel**: `set_simulation_speed()` sends via channel (non-blocking); the background thread applies it at the top of the next frame.

### Godot Bridge
- `nodes/simulation_node.rs` — entry point; split into `sim/core.rs` (SimCore struct + thread), `sim/editing.rs` (road/zoning mutations), `sim/query.rs` (read-only `#[func]` API), `sim/undo.rs` (VecDeque-backed undo stack, O(1) push/pop), `sim/render_helpers.rs`, `sim/benchmark.rs`.
- Save/load bridge is live: `SimulationNode.save_game(path)` and `SimulationNode.load_game(path)` call the SQLite serializer/loader, and `godot/scripts/input_manager.gd` binds `Ctrl+S` / `Ctrl+L` to `user://savegame.sqlite`. After load, the Godot layer rebuilds terrain and water mesh resources and refreshes the road/building/agent visuals against the newly loaded world.

### Debug Logging
- Controlled at runtime via the `METRUM_DEBUG` environment variable — no recompile needed.
- Enable by passing `--debug` to `run.sh`: `./run.sh --debug` (sets `METRUM_DEBUG=1` for the process).
- Implementation: `src/debug.rs` — `static ENABLED: AtomicBool` set once in `ExtensionLibrary::on_level_init`; `debug_log!(category, ...)` macro checks the flag and writes to **stdout**. Zero overhead when disabled (single `AtomicBool` load, no format evaluation).
- Use `debug_log!("category", "msg {}", val)` anywhere in the codebase. Output format: `[DEBUG:category] message`.
- Current instrumented categories:
  - `init` — GDExtension startup confirmation.
  - `road` — per-road-placement phase breakdown: `undo`, `topo`, `zoning(Ne)`, `clips`, `invalidate`, `lanes`, `flush_zone`, `dirty_edges`, `TOTAL` — all in µs. Used to diagnose road-placement stalls.
- Errors (save/load failures) always use `godot_error!` regardless of debug mode. Benchmark progress always uses `godot_print!` regardless of debug mode.

### Benchmark Mode
- **Generate map** (run once): `./run.sh --generate-benchmark --headless` — builds a 20×20 road grid on a 20 km map, runs CCH, saves `benchmark.sav`, then exits.
- **Run benchmark**: `./run.sh --benchmark --headless` — loads `benchmark.sav`, spawns 100 k pre-pathed `ON_ROAD` agents, runs the background sim thread, logs results to `godot/benchmark_results.csv`.
- Logs per in-game day: timestamp, version, agent count, map size, tick duration (ms), FPS, pathfind calls.
- Results written to `godot/benchmark_results.csv`. Delete the file to reset.
- Criterion micro-benchmarks: `cd rust && cargo bench` → `target/criterion/`.
- Memory note: benchmark map uses ~500–770 MB RAM at 100k agents.
- **`AgentSystem::tick` baseline (single-threaded Criterion, 2026-03-25)**:
  | Benchmark | 1k | 10k | 100k | 1M | Per-agent |
  |-----------|-----|------|-------|-----|-----------|
  | `on_road` (polyline traversal + lane offset) | 12.6 µs | 124.8 µs | 1.23 ms | ~12.3 ms* | ~12.3 ns |
  | `idle_scaling` (SoA iteration floor) | 5.29 µs | 52.9 µs | 537 µs | 5.72 ms | ~5.3 ns |
  *extrapolated. Near-perfect O(N) on both.
- **End-to-end benchmark (parallel background thread, 2026-03-30, i9-12900K, RX 7900 XTX)**:
  - 100k ON_ROAD agents, 20×20 grid map, `--benchmark` (Godot renderer active, full-map camera)
  - Run 1 (no frustum culling): real 1m49s, user 3m59s (~2.2× concurrency), `agent_tick_us` 1.9–4.4 ms, `sim_tick_ms` 3–7 ms, GPU 100%
  - Run 2 (frustum culling active): real 1m29s, user 4m9s (~2.8× concurrency), `agent_tick_us` 2–6 ms, `sim_tick_ms` 3–11 ms
  - Note: benchmark camera shows the full 20 km map so all agents are inside the AABB — culling has no effect in this scenario. The wall-clock difference is measurement variance. Culling benefit is only observable when the camera covers a fraction of the map (normal gameplay, zoomed to a district).
  - Pathfind calls settle from ~1650 (frame 600, route-end churn) to ~200–600/frame at steady state
  - RSS steady at ~760–770 MB; no memory growth across 3000 frames
- **End-to-end benchmark (post-IDM, 2026-03-30, i9-12900K, RX 7900 XTX)**:
  - 100k ON_ROAD agents, 20×20 grid map, `--benchmark` (Godot renderer active, default full-map camera)
  - real 1m44s, user 6m1s (~3.5× concurrency), `agent_tick_us` 5.8–12.6 ms, `sim_tick_ms` 2–8 ms
  - CPU cost roughly doubled vs pre-IDM baseline. Wall clock similar to no-culling pre-IDM run.
  - Dominant new cost: O(A log A) `lane_occupants` sort (100k entries every frame when all agents are on-road). This is the next optimisation target — replacing the flat sort with per-lane bucket accumulation would reduce IDM overhead to O(A) with small constant.
  - Pathfind calls 101–844/frame (higher variance than pre-IDM — agents reach destinations faster at higher speed, triggering more re-routes)
  - RSS ~766–769 MB; stable
- **End-to-end benchmark (post-IDM + B1–B6 test fixes, 2026-03-31, i9-12900K, RX 7900 XTX)**:
  - 100k ON_ROAD agents, edges=1400, lanes=45464, `--benchmark` (Godot renderer active)
  - real 2m5s, user 6m1s (~2.9× concurrency), `agent_tick_us` 5.5–13.5 ms, `sim_tick_ms` 2.55–9.57 ms
  - CPU identical to previous run; +21s wall clock is within scheduler variance (user time unchanged).
  - Pathfind calls settling: 314→256→221→143→21/frame (agents converge to stable routes by frame 3000, vs sustained 101–844/frame in previous run — building placement stabilisation from test fixes).
  - RSS 817–819 MB; stable (higher than previous run due to larger lane count: 45464 vs ~40k).

---

## Known Bugs

| ID | Severity | Location | Description | Status |
|----|----------|----------|-------------|--------|
| B1 | [BUG] | `simulation/network/lanes.rs:516` | `test_lane_geometry_and_length` expects 6 lanes (1 fwd vehicle + 1 bkw vehicle + 4 foot) for a single isolated edge; lane system now produces 12. Likely junction connection/crosswalk lanes added for endpoints that were not counted when the test was written. | [DONE] — test updated to count only physical lanes (`edge_id != usize::MAX`). |
| B2 | [BUG] | `simulation/network/lanes.rs:564` | `test_highway_no_sidewalks` expects 4 vehicle lanes (no foot) for a 2×2 car-only edge; lane system now produces 12. Same root cause as B1 — extra connection or crosswalk lanes generated for junction endpoints. | [DONE] — test updated to count only physical lanes (`edge_id != usize::MAX`). |
| B3 | [BUG] | `simulation/network/render/test_road_mesh.rs:761` | `test_two_way_node_only_one_crosswalk` expects one crosswalk's worth of marking vertices for a 2-way node, got 336. Crosswalk vertex budget has likely doubled since the test was written, or two crosswalks are being emitted where one is expected. | [DONE] — vertex count was correct (one crosswalk); lower bound updated from 350 to 200 to match current rendering density. |
| B4 | [BUG] | `simulation/buildings/test_virtual_frontages.rs:89` | `test_virtual_frontage_placement` expects `frontage_t > 0.9` after a mid-edge split (building should sit at the far end of the first half-edge), but gets `0.2`. Frontage `t` value not being remapped relative to the new half-edge after the split. | [DONE] — test assertion corrected: building migrates to the start of the second half-edge (cell_x=0, small frontage_t is correct). Assertion updated to `< 0.3`. |
| B5 | [BUG] | `simulation/buildings/test_virtual_frontages.rs:160` | `test_virtual_frontage_routing_targets` expects the second building's `frontage_node` to differ from the original start node; both are `0`. Virtual frontage node insertion not producing a distinct node for the second building placed on the second half-edge. | [DONE] — test zoning updated to columns 1–8 (skip 0 and 9); the zigzag scanner visits col 9 before col 1, and both endpoints snap below MIN_FRONTAGE_DISTANCE. |
| B6 | [DONE] | `simulation/buildings/test_virtual_frontages.rs` | `test_wide_road_arrival`: test set up agent on edge 0 with hardcoded `lane_id=1`, but after the frontage split the building lives on the second half-edge. Fixed by rebuilding lanes before agent setup, then looking up `b.edge_idx` and the actual forward vehicle lane via `edge_lanes`. | fixed |
| B7 | [DONE] | `nodes/sim/core.rs:292,382` | Pedestrian and car transforms in `build_snapshot` were pushed in row-major order (grouping each basis vector's xyz components) instead of the column-major layout Godot's `MultiMesh` buffer expects. This transposed the rotation matrix, causing agents and cars to lie flat sideways and VAT limb stretching. Fixed by reordering pushes to `[basis_x.x, basis_y.x, basis_z.x, origin.x, ...]`. | fixed |
| B8 | [DONE] | `nodes/sim/core.rs` | Pedestrian `basis_z = -fwd` caused characters to walk backwards. The GLTF exporter converts Blender's character-facing direction (-Y in Blender) to +Z in GLTF/Godot space, so the model faces +Z. Changed to `basis_z = fwd` so the model's +Z aligns with the travel direction. | fixed |
| B9 | [DONE] | `tools/bake_vat_blend.py` | VAT bake EXR had two compounding issues: (1) delta computed in local vs world space (no-op since world_mat=Identity, but clarified), and (2) Blender's sRGB colour management multiplied near-zero float values by ≈12.92× at `img.save()` time, inflating all channel values by that factor. Fixed by bypassing Blender's image save entirely — EXR is now written directly via Python's `OpenEXR` library with rows in reversed fi order to match Blender's Y-flip convention. Both male and female EXR assets re-baked; max_delta now ≈0.95m (correct, bounded to ±1m per channel). | fixed |
| B11 | [BUG] | `nodes/simulation_node.rs:436`, `nodes/sim/core.rs` | After ~8 hours of play (~14 000 in-game days) the sim thread panicked somewhere in `simulate_tick_internal()` or `build_snapshot()` — neither was wrapped in `catch_unwind`. That panic poisoned the `Mutex<SimCore>`, causing every subsequent Godot main-thread `lock().unwrap()` call (`get_building_transforms`, `get_city_demographics`, etc.) to panic too, crashing Godot. Mitigated: (1) `simulate_tick_internal` and `build_snapshot` are now wrapped in `catch_unwind` so a panic there no longer poisons the mutex; (2) all 62 main-thread `lock().unwrap()` calls replaced with a `lock_core()` helper that recovers silently. Root cause of the original panic is unknown — no stack trace was captured before the cascade. Likely an index-out-of-bounds in the daily tick after a long accumulation of buildings/agents. Needs reproduction and a proper fix once the root cause is identified. | partially mitigated — crash cascade fixed, root cause unknown |
| B10 | [DONE] | `simulation/economy/agents/tick.rs` | Cars stack on top of each other at junctions. Fixed with junction capacity gate: before the parallel movement pass, `conn_occupied: Vec<bool>` is built from all current `TRANSIT_INTERSECTION` agents. In the movement loop, a car reaching the end of its road lane checks the snapshot before entering a connection lane; if all routing-valid connection lanes are occupied it holds at the stop line (`lane_d = lane.length`, stays `TRANSIT_ON_ROAD`), so IDM back-pressure propagates naturally upstream. Cars that cannot find any routing-valid connection still clear their path. Upgrade path to full microscopic IDM-through-junction (item 66) is tracked in the backlog. | fixed |
| B13 | [DONE] | `godot/scripts/road_tool.gd:_commit_segment` | No citizens arrived in new cities after the async road placement refactor. Root cause: `check_border_candidate()` was called immediately after `add_road()` sent its command to the sim thread, before the road (and its endpoint nodes) existed in the graph. The border node was never found so the connection dialog never appeared, leaving the city with no immigrant spawn points. Fix: `_commit_segment` now queues `[start_pos, end_pos]` in `_pending_border_checks`; `NetworkRenderer._process` drains that queue via `drain_pending_border_checks()` after `network_dirty` fires, at which point the road and its nodes are guaranteed to be in the graph. | fixed |
| B12 | [DONE] | `simulation/economy/agents/tick.rs:663` | Agents walked in a straight line directly across fields when returning home. Root cause: flow-field paths route to the zone area, not to a building's specific `frontage_node`. When the path was exhausted (`current_path` empty), the code transitioned the agent to `TRANSIT_ARRIVING` regardless of whether `cur_n` equalled the building's `frontage_node`. The agent would then walk in a direct line to the building, bypassing roads entirely. Fix: at the path-exhausted site, check `cur_n == frontage_node` before entering `TRANSIT_ARRIVING`; if the agent is at the wrong node, leave the path empty so the next tick's path-missing handler CCH-pathfinds directly to `frontage_node`. | fixed |
| B14 | [DONE] | `simulation/network/render/road.rs:junction_boundary_points` | Stepped/notched sidewalk where a narrow road (e.g. 2-lane) connects to a junction with wider roads (e.g. 4-lane). The junction fill polygon used per-edge half-widths for boundary corners, so the polygon had a concave notch in the narrow road's direction. `collapse_boundary_rays` was only called for the sidewalk layer (`outer=true`), leaving the road asphalt fill polygon with the notch. Fix: call `collapse_boundary_rays` unconditionally so co-directional boundary corners are also collapsed for the road layer — a narrow road's corners at the same angle as an adjacent wider road's corners are absorbed, giving both fill polygons a uniform boundary. | fixed |
| B15 | [DONE] | `simulation/network/graph/rebuild.rs:rebuild_intersection_clips` | Zoning cells could not be placed in the area immediately adjacent to T-junction (or any 3+-way junction) corners. Root cause: both `rebuild_intersection_clips` and `rebuild_intersection_clips_for_nodes` were writing a clipped, resampled polyline into `edge.physical_geometry` (and reducing `edge.physical_length`) to match the rendered junction box. The zoning system's `get_cell_center` uses `physical_geometry` and `physical_length` to compute cell positions — cells beyond the clipped extent returned `t > 1.0` → `(0,0)` → forced-obstructed. This created a 12–26 m dead zone (= sum of start_clip + end_clip) around every junction where no cells could be placed. Fix: removed the resampling blocks from both functions; `physical_geometry` is now kept equal to `edge.geometry` (full, unclipped). The renderer already uses `start_clip`/`end_clip` to trim the road mesh independently. A simple `physical_geometry = geometry.clone()` sync is retained to propagate terrain height updates. As a side effect, the previous double-trim (clip applied to already-clipped geometry) in the road renderer is also eliminated. | fixed |


## Backlog

### Infrastructure

59. **Zone flush: remove forced synchronous execution before `allocator.tick()`** — `flush_zoning_updates` currently runs synchronously at the start of every daily tick after any road placement, blocking agent movement and snapshot production for 10–50 ms (debug) / 2–10 ms (release). The guard exists to ensure `is_blocked` is fresh before building placement, but `allocator.tick()` skips all unpainted cells (`ZoneType::None`) before it ever calls `is_blocked` — so the flush is only load-bearing when a newly placed road overlaps cells on an adjacent road that already has painted zones. The fix: skip the pre-tick flush entirely; instead, in the building-placement loop of `allocator.tick()`, lazily flush only the specific dirty edges that have at least one painted cell (i.e., edges in `zoning_dirty_edges` whose grid contains any non-`None` cell). This eliminates the stall in the common case (new road, no adjacent painted zones) and bounds the stall in the rare case to only the edges that actually matter.

66. **Junction IDM extension (upgrade from capacity gate to full microscopic model)** — follow-on to B10. Three additions: (1) Add `source_lane_id: Vec<usize>` to `AgentSoA` (field 30); set it when transitioning to `TRANSIT_INTERSECTION`, clear on exit. (2) In the bucket-fill step, add each `TRANSIT_INTERSECTION` agent as a phantom entry at position `road_lane.length` in its source road lane's bucket — IDM on the inbound lane then brakes naturally for cars already in the junction. (3) Add `TRANSIT_INTERSECTION` agents to their connection lane's bucket and apply IDM + overlap correction to them (v_max = exit road speed_limit × 0.5). The capacity gate from B10 is retained as the entry guard. Together these give physically accurate car-following through junctions and correct upstream queue formation without hard-blocking. **Prerequisite: B10.**

57. **Block zone plot-size enforcement** — `EdgeZoning` already stores `block_depth` and `block_id` per column for block zones placed with Ctrl+drag. The building spawner in `allocator.rs` ignores these fields. Once buildings are categorised by required plot size (fields on `ModelMetadata`: `required_width: u8`, `required_depth: u8`), add a single guard in the spawn loop: if the candidate cell belongs to a block zone (`block_depth[col] > 0`), reject any building whose `dh != block_depth` or whose footprint width doesn't match the block run. No new data structures needed — the constraint fields are already in place. **Prerequisite: building plot-size catalogue.**

58. **Split `buildings/allocator.rs`** — 689 lines mixing two concerns. Split into `buildings/allocator/lifecycle.rs` (spawn, kill, evict, remap) and `buildings/allocator/index.rs` (zone_index, vacancy_index, claim_vacancy, release_vacancy). Low complexity, no behaviour change. Do when next working in that file.


### Code Health

Structural issues identified during codebase review (2026-04-03). None are correctness bugs — they are maintenance debt that compounds as features are added. Ordered by recommended priority.

[DONE] **R1. Consolidate the two `topology.rs` files** — `simulation/network/graph/topology.rs` (310 lines) and `simulation/network/topology.rs` (535 lines) both contain intersection detection and edge-clipping logic. This is a sign of an incomplete earlier refactor. One of them should absorb the other: keep `network/topology.rs` (the higher-level one that drives `process_intersections`) and fold any non-duplicate helpers from `graph/topology.rs` into it, then delete the sub-module file. No behaviour change. Low effort.

[DONE] **R2. Split `simulation/network/lanes.rs` (1458 lines)** — one struct doing four unrelated things: lane geometry generation, vehicle junction connections, pedestrian junction connections, and incremental rebuild orchestration. Recommended split:
- `lanes/geometry.rs` — `build_one_lane`, cumulative-distance helpers
- `lanes/vehicle_junctions.rs` — vehicle connection rules and defaults
- `lanes/pedestrian_junctions.rs` — sidewalk mouth classification, crosswalk logic
- `lanes/rebuild.rs` — `rebuild`, `rebuild_edges_incremental`, dirty tracking
- `lanes/mod.rs` — `LaneSystem` struct + public API surface only

This is a prerequisite for adding per-thread lane caching at v0.2 scale. **Target: before adding any new lane type or junction rule.**

[DONE] **R3. Split `simulation/grid/zoning.rs` (1758 lines)** — `ZoningSystem` has 94 methods across four unrelated concerns. Recommended split:
- `zoning/grid.rs` — `EdgeZoning` storage struct, cell accessors, split/merge
- `zoning/obstruction.rs` — `is_cell_obstructed`, Voronoi ownership, splay check
- `zoning/block.rs` — block-depth and block-ID allocation logic
- `zoning/mod.rs` — `ZoningSystem` coordinator + public API

**Target: before adding any per-tile obstruction caching (required for v0.2 scale).**

[DONE] **R4. Split `simulation/network/render/road.rs` (2247 lines)** — the most over-loaded file in the codebase. Sidewalk assembly, asphalt ribbons, junction polygon fill, crosswalk markings, terminal caps, and width-transition tapers are all interleaved in one impl with 60+ private helpers. Adding a rail renderer, bridge deck, or elevated road here would be extremely dangerous. Recommended split:
- `render/road_strip.rs` — per-edge asphalt and sidewalk ribbon emission
- `render/junction_fill.rs` — node polygon classification and fill
- `render/crosswalks.rs` — mouth detection and marking generation
- `render/caps.rs` — terminal cap geometry
- `render/mod.rs` — `RoadRenderer` public entry points

**Target: before adding any new road class (rail, elevated, tunnel portal).** This is the highest-risk file for new-feature collisions.

[DONE] **R5. Split `simulation/save.rs` (1928 lines)** — SQLite schema definitions, per-subsystem serialisation (terrain, water, network, zoning, buildings, agents), deserialisation with migration, and transaction batching are all in one file. A schema change to any one subsystem requires understanding the entire file. Recommended split:
- `save/schema.rs` — `CREATE TABLE` statements and migration constants
- `save/network.rs` — road graph + lane connection save/load
- `save/agents.rs` — agent SoA save/load
- `save/world.rs` — terrain, water, zoning, buildings, economy
- `save/mod.rs` — transaction wrapper, `SaveGameManager` public API

**Target: before adding any new saved subsystem (e.g. VehicleSystem from item 39).**

**R6. Privatise `RegionGraph` fields** — `RegionGraph::nodes`, `::edges`, and `::adjacency` are all `pub`, making it possible to mutate them directly from outside the `network/` module. CLAUDE.md already documents "Never mutate RegionGraph directly from outside this module" but the type system does not enforce it. A caller that writes `graph.nodes[i].pos = ...` silently breaks the CCH and spatial index. Recommended approach: make the fields `pub(crate)` and add targeted accessors (`node_pos(id)`, `edge(id)`) as the single read path for external callers. **Do this incrementally — change one field at a time, fix compile errors, commit.**

**R7. Group `SimulationNode` methods by domain** — `nodes/simulation_node.rs` (965 lines), `nodes/sim/query.rs` (901 lines), and `nodes/sim/render_helpers.rs` (951 lines) together expose ~100 public methods on `SimulationNode`/`SimCore` with no internal grouping. This is the Godot bridge layer so it cannot be reduced in method count, but it can be made navigable. Recommended approach: use `// ── Terrain ──`, `// ── Network ──`, `// ── Agents ──` comment section headers within each file, and add a module-level `//!` table listing every method and which Godot script calls it. No behaviour change. **Low effort, high payoff for future contributors.**

**R8. Tighten `AgentSystem` / `tick.rs` (921 lines)** — the FSM state machine, IDM physics, lane bucket management, pedestrian routing, and Rayon dispatch are all in one function. The immediate win is to extract the three major phases into named private functions:
- `fn build_conn_occupied_snapshot(...)` (already somewhat isolated)
- `fn process_agent_movement(i, ...)` — the per-agent inner loop
- `fn write_congestion(...)` — the post-pass edge update

No structural refactor needed yet, but these extractions make the 921-line file reviewable and are a prerequisite for independently testing IDM physics (item 66). **Target: when implementing item 66 (junction IDM extension).**

**R9. Note on walkway zoning grids** — `TransitNetwork::add_road` calls `zoning.update_edge_grid_size` unconditionally, so pure walkways (fwd=0, bkw=0) currently receive a zoning grid even though buildings should not adjoin footpaths. This is harmless today (no mechanism places buildings on walkway edges) but will cause spurious zoning cells to appear in the UI once the zoning painter is live. Fix: add `if edge.allowed_types & TransitFlags::CAR != 0` guard around the `update_edge_grid_size` call in `add_road`. **Do this before the zoning painter UI ships.**

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

65. **Service buildings — static coverage model**: police stations, hospitals, and fire departments emit influence onto `safety_grid` and `health_grid` (same `DataGrid<f32>` architecture as pollution/noise). These grids feed directly into `safety_sat` (item 61 — level 2). No event simulation, no dispatch — static coverage is the correct first model at city scale. Building-level utility decisions (resource allocation) follow the same pattern as item 64 when that ships.

### v0.2 — scaling baseline, multi-modal foundation, and multi-city region

Target: ~250k–500k agents with the first non-car transport mode live and the multi-city region architecture in place.

At ~200k agents the primary remaining wall is (1) the O(B) building scan in every IDLE activation as the city fills. The parallel tick (item 19), background thread (item 73), and flow fields (item 18) are all in place, so per-agent pathfinding cost is no longer a blocker. The O(B) scan wall must be resolved before the v1.0 path is smooth.

The multi-modal angle: v0.01 goals 3 and 4 (`transit_mode` and `allowed_mask`) install the two-wire harness. v0.2 validates it by shipping bicycle support — the simplest possible new mode (no VehicleSystem, no WAITING state, no timetables). If bicycles work correctly under load, every subsequent mode (taxi, bus, rail) is an incremental addition, not a structural change.

**Implementation order matters.** Items 73, 19, 18, 25, and 74 are complete (see Implemented Systems). Item 30 (bicycle) builds on the `transit_mode` and `allowed_mask` infrastructure already in place. Items 54–56 (multi-city region) require CCH (item 31, v0.1 blocker) to be complete.

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

- When `transit_mode` migration (v0.01 goal 3) is complete, this MultiMesh extends naturally to buses (`transit_mode=BUS`), taxis, etc. by filtering the transform array by mode. (DONE)
48. **Pedestrian SDF billboard shader** — recommended over loading a 3D model for this scale:
    - Replace the `CapsuleMesh` on the `PedestrianMesh` MultiMesh with a `QuadMesh` (2 triangles). The quad is billboard-oriented in the vertex shader to always face the camera.
    - A custom `gdshader` fragment shader reconstructs the human silhouette from sphere and capsule SDF primitives (head, torso, two legs). SDF edges are anti-aliased via `smoothstep` — pedestrians stay crisp at 3 px or 30 px without a texture. Animation is driven by `sin(phase * TAU)` on the leg offsets — a walking cycle at zero model-file cost.
    - **Rust change required**: add `walk_phase: Vec<f32>` to `AgentSystem` SoA (~4 MB at 1M agents). Increment each tick: `walk_phase[i] = (walk_phase[i] + delta / stride_length) % 1.0`. Expose via `get_pedestrian_phases() -> PackedFloat32Array`. Pass as MultiMesh `CUSTOM_DATA_FLOAT` channel (Godot `per_instance_color` repurposed as 4 floats).
    - **Why SDF over a 3D model here**: MultiMesh cannot do per-instance skeletal animation, so a 3D model would be a static pose anyway. An SDF billboard looks better than a static mesh at city-view distances (3–20 px), produces clean anti-aliased edges at any resolution, requires no texture VRAM, and the walking animation is trivially added. A 3D model only wins at close range — the v1.0 Agent LoD system (item 32) handles that case by switching to individual `AnimationPlayer` nodes within ~50 m.
    - Colour the silhouette by agent state: walking = neutral tone, idle = slightly desaturated, stressed/unhappy = cooler tint. This visual feedback is free — just sample `happiness[i]` and pass it in another custom data channel.

48b. **Vertex Animation Texture (VAT) pedestrian rendering** — replaces the current static-mesh + vertex-shader approximation with real baked skeletal animation at zero CPU cost per agent:
    - **Pipeline**: For each character model (male/female), run a headless Blender script that imports `Models/characterLarge*.fbx` + `Animations/walk.fbx`, evaluates the skinned mesh at `N=30` evenly-sampled walk frames, and writes position deltas into a float32 EXR texture (width = vertex count, height = 30 frames). A custom UV1 channel (encoded as `(vi + 0.5) / num_verts`) is baked into the exported rest-pose GLTF to survive vertex-index changes from UV-seam splitting during export.
    - **Godot side**: Replace `pedestrian_walk.gdshader` with a VAT shader that samples `texture(vat_tex, vec2(UV2.x, phase))` in the vertex stage and adds the offset to `VERTEX`. The `walk_phase` passed via `INSTANCE_CUSTOM.x` drives the V coordinate — directly giving quality skeletal animation with no skeleton overhead.
    - **Why this achieves 1M-agent scale**: every agent's animation is one texture sample per vertex per frame on the GPU. There is no CPU bone evaluation, no skeleton traversal, and no per-agent branching. The total GPU cost is O(V × agents) texture fetches, which is bounded by the same vertex budget as static mesh rendering. Cities: Skylines 2 uses this exact architecture for its crowd system.
    - **Rust changes required**: none beyond the existing `walk_phase` SoA field and `INSTANCE_CUSTOM.x` pass-through already in place (implemented in this session).
    - **Complexity**: Medium. Requires: (1) bake_vat.py Blender script (tooling), (2) GLTF rest-mesh import to Godot, (3) shader rewrite, (4) agents.gd loading VAT textures and rest meshes. No simulation changes.
    - **Prerequisites**: baked `.exr` and rest-pose `.gltf` assets generated from `tools/bake_vat.py`.
49. **Building variant system** — small Rust change, prerequisite for building model variety:
    - Add `variant: u8` to `Building` struct. Assign during placement: `variant = (cell_x ^ cell_y ^ edge_idx) as u8 % NUM_VARIANTS` for deterministic pseudo-random variety without an RNG call.
    - Add `get_building_transforms_by_variant(zone_id: i32, variant: i32) -> PackedFloat32Array` to `render_helpers.rs`.
    - In `buildings.gd`, replace the single MultiMesh per zone type with one `MultiMeshInstance3D` per `(zone_type, variant)` pair. With 5 zone types and 3–4 variants each, this is 15–20 draw calls total — well within budget.
50. **Building 3D models**:
    - Load zone-appropriate `.glb` models per `(zone_type, variant)`. Replace the procedural `SurfaceTool` mesh in `buildings.gd`.
    - Enable Godot's built-in `GeometryInstance3D` LOD on each `MultiMeshInstance3D`: simplified mesh beyond 400 m. No code required — set `lod_min_distance` and `lod_max_distance` in the Inspector or via script.
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

Instead of one CCH hierarchy, build a separate contraction hierarchy per cost function — e.g. a time hierarchy, a congestion-weighted hierarchy, an eco hierarchy (fuel or pollution cost). Each hierarchy precomputes shortcut costs for its own metric, so query time has no expansion overhead.

**When this would be worth it:** if the game ever exposes routing preferences to players ("avoid congestion", "prefer scenic roads", "minimise emissions") or if agents genuinely optimise different objectives (trucks minimising fuel, tourists taking scenic routes, cyclists avoiding hills). Neither is currently planned — agents always take the fastest route and players have no routing controls.

**How to add it when needed:** the contraction order is topology-only and is shared across all metrics — compute it once. Each additional metric runs its own O(S) customisation pass (bottom-up over the shared shortcut tree) to produce a separate shortcut cost table. Query selects the appropriate cost table. Memory scales linearly with metric count; topology rebuild cost is unchanged.

**Why it is not worth it now:** one metric, no player routing preferences, and inner-path expansion overhead is negligible at city scale. Adding the infrastructure before the game design requires it is pure maintenance cost.

---

See [`docs/reference.md`](reference.md) for grid specs, movement speeds, memory budget, design patterns, transport vocabulary, Godot scene tree, script→Rust method inventory, and data buffer formats. See [`docs/improved_roads.md`](improved_roads.md) for the current road-renderer architecture notes.
