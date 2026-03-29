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
- **Lane System (`LaneGraph`)**: Replaced all runtime trigonometric dynamic offset calculations with a pre-computed spline system generated during graph topology updates. 1,000,000 agents now traverse roads via an O(1) 1D array length tracker on static 3D splines (`LaneSystem`), eliminating all rendering frame normal-vector computations. Agent logic includes strict `is_fwd` orientation inversions, Bezier constraint calculations for intersections based on `p1_base`, active `MODE_WALK` interpolation rendering states for path fails, and strict `frontage_t` recalibrations when topology changes via `split_edge`, curing visual disconnected agent models rendering offset from the road.
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

### Zoning
- `simulation/grid/zoning.rs` — edge-aligned zoning cells, 10 m × 10 m.
- **Player-painted zones** (item 66): automatic edge-aligned zone generation removed. Players click and drag along road edges in `zoning_tool.gd` to paint cells onto either side of a road. Drag can span multiple connected edges; side is determined by mouse position relative to the road centreline and flips automatically at reversed junctions. Scroll wheel adjusts zone depth (2–12 cells, 20–120 m). Right-click clears all zones on an edge. Zone paint is committed on mouse release via `set_zoning_range(edge_idx, side, t_start, t_end, depth, zone_type)` on the Rust side. A MultiMesh preview (grid + painted + brush overlays) updates every frame during hover and drag.
- Cells extend up to 12 cells deep (120 m) from the road sidewalk edge, following road curvature.
- Obstruction check (`is_cell_obstructed`): 5-point sampling (4 corners + centre) per cell with asphalt collision and Voronoi ownership test. Splay check applied only to `y=0` cells (inner row) — applying it to all rows incorrectly rejected entire building footprints on curved roads.
- Obstruction cache correctly wired: `recalculate_obstructions` is parallelised with Rayon and spatially invalidated on nearby road edits. `BuildingAllocator::tick()` reads the precomputed `is_blocked()` cache (O(1) per cell) instead of rerunning `is_cell_obstructed()` (O(geometry² × nearby_edges)) on every tick — the per-tick 1–2 s freeze was caused by the latter pattern.
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
- **Single-threaded tick** — Rayon parallelisation is a v0.1 goal (see Backlog).
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

### Godot Bridge
- `nodes/simulation_node.rs` — entry point; split into `sim/editing.rs` (road/zoning mutations), `sim/query.rs` (read-only `#[func]` API), `sim/undo.rs` (VecDeque-backed undo stack, O(1) push/pop), `sim/render_helpers.rs`, `sim/benchmark.rs`.
- Save/load bridge is live: `SimulationNode.save_game(path)` and `SimulationNode.load_game(path)` call the SQLite serializer/loader, and `godot/scripts/input_manager.gd` binds `Ctrl+S` / `Ctrl+L` to `user://savegame.sqlite`. After load, the Godot layer rebuilds terrain and water mesh resources and refreshes the road/building/agent visuals against the newly loaded world.

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

| ID | Severity | Location | Description | Status |
|----|----------|----------|-------------|--------|


## Backlog

### Infrastructure


58. **Split `buildings/allocator.rs`** — 689 lines mixing two concerns. Split into `buildings/allocator/lifecycle.rs` (spawn, kill, evict, remap) and `buildings/allocator/index.rs` (zone_index, vacancy_index, claim_vacancy, release_vacancy). Low complexity, no behaviour change. Do when next working in that file.


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

At ~200k agents three independent performance walls converge: (1) the single-threaded agent tick saturates one core, (2) the O(B) building scan in every IDLE activation becomes the dominant tick cost as the city fills, and (3) per-agent pathfinding accumulates to an unacceptable fraction of frame time even with CCH (flow fields are the answer — item 18). All three must be resolved before the v1.0 path is smooth — deferring any one of them past v0.2 means hitting a hard wall instead of a gradual ramp.

The multi-modal angle: v0.01 goals 3 and 4 (`transit_mode` and `allowed_mask`) install the two-wire harness. v0.2 validates it by shipping bicycle support — the simplest possible new mode (no VehicleSystem, no WAITING state, no timetables). If bicycles work correctly under load, every subsequent mode (taxi, bus, rail) is an incremental addition, not a structural change.

**Implementation order matters.** Item 73 (simulation thread separation) must come first — it establishes the safe threading boundary that item 19 runs inside. Item 19 (parallel tick) depends on the zone index (B16a fix) existing and providing O(1) vacancy lookup — the parallel tick needs atomic vacancy counters that the index maintains. Item 18 (flow fields) requires item 19 to be done first — flow field queries need to run inside the parallel tick loop. Item 25 (IDM) is independent. Item 30 (bicycle) builds on the `transit_mode` and `allowed_mask` infrastructure already in place. Items 54–56 (multi-city region) require CCH (item 31, v0.1 blocker) to be complete — the `RegionGraph` rename and CCH query are the foundation the region system builds on.

73. **Simulation thread separation + double-buffered render snapshot**: move `AgentSystem::tick` and all grid ticks (`PollutionSystem`, `NoiseSystem`, `DesirabilitySystem`, `WaterSystem`) to a dedicated background thread that runs continuously at its own rate, decoupled from Godot's render frame. At the end of each completed tick, the sim thread writes agent transforms, building states, and grid dirty flags into a read-only snapshot buffer; Godot's render callbacks read only from that buffer. The sim thread never touches Godot objects; the render thread never touches live simulation state. Required because at v0.2 scale (250k–500k agents) tick time will exceed 16 ms and a synchronous tick blocks the render thread. Also eliminates the `experimental-threads` data-race risk that currently exists latently in the `gdext` bridge. Prerequisite for item 19.

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

- When `transit_mode` migration (v0.01 goal 3) is complete, this MultiMesh extends naturally to buses (`transit_mode=BUS`), taxis, etc. by filtering the transform array by mode. (DONE)
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
