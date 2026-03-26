# Metrum Rise — Reference

Stable lookup tables for architecture constants, Godot bridge API, and data formats. Update this file when specs change (grid sizes, new scripts, new buffer layouts). For current development state, bugs, and backlog see [`project.md`](project.md).

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

- **DataGrid\<T\>**: flat `Vec<T>` with stride `width`. Row-wise parallel iteration with `rayon::par_chunks_mut`. All spatial grids (terrain, pollution, noise, desirability, safety) use this type. The planned congestion heatmap (`DataGrid<f32>`, written from per-edge average speed) will also use it.
- **Per-lane occupancy lists (planned)**: transient `Vec<Vec<u32>>` built each tick from the SoA, indexed by `edge_idx * MAX_LANES + lane_idx`, sorted by `edge_progression`. Provides O(1) car-ahead lookup for IDM without any persistent spatial structure. Thrown away after each tick.
- **SoA (Structure-of-Arrays)**: `AgentSystem` stores all fields as parallel `Vec<T>` indexed by agent ID. Cache-friendly for bulk iteration. Schema defined via `#[derive(SoA)]` on the `Agent` struct (item 59) — adding a field and omitting it from `spawn_agent` is a compile error.
- **512 m spatial chunks**: road edge AABB registered in all overlapping chunks. Used for editor queries (radius ≈ 120 m → typically 1 chunk) and spatial snapping. CCH manages its own contraction hierarchy independently of this grid.
- **`(node, incoming_edge)` pathfinding state**: required for turn restriction correctness at `Node::lane_connections`. Must be preserved in any pathfinding replacement.

### Multi-modal Transport Vocabulary

The type vocabulary for all planned transport modes already exists in `network/types.rs`:

| Type | Declared values |
|------|----------------|
| `TransitType` | `Road, Rail, Ship, Air, Foot` |
| `TransitFlags` | `FOOT=1<<0, CAR=1<<1, RAIL=1<<2, SHIP=1<<3, AIR=1<<4` (bit 5+ free for `BIKE`) |
| `NodeType` | `Junction, Station, Harbor, Airport, Transfer, Frontage` |
| `MODE_*` constants | `WALK=0, CAR=1, BIKE=2, BUS_PASSENGER=3, TRAIN_PASSENGER=4, TAXI_PASSENGER=5, SHIP_PASSENGER=6` |

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
