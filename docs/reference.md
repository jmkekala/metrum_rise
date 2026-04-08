# Metrum Rise — Reference

Stable lookup tables for architecture constants, Godot bridge API, and data formats. Update this file when specs change. For current status see [`project.md`](project.md); for active tracked work see [`roadmap.md`](roadmap.md); for doc ownership see [`README.md`](README.md).

---

## Architecture Reference

### Spatial And Grid Specifications

| Parameter | Value | Notes |
|-----------|-------|-------|
| City tile size | Player-configurable | Default `20 km × 20 km`, defined by `MapConfig`. |
| Zoning cell | `10 m × 10 m` (`zone_cell_m`) | Configurable via `MapConfig`. |
| World-space zoning grid (default map) | `2000 × 2000` | Derived from `width_m / zone_cell_m` and `height_m / zone_cell_m`. |
| Building lot footprint | Asset-authored | `lot_width_cells × lot_depth_cells`; no fixed global `3 × 3` footprint anymore. |
| Reference zoning depth | `12` cells | `DEFAULT_ZONING_DEPTH`; tooling / fade heuristic only, not a hard cap. |
| Lane width | `3.5 m` | `LANE_WIDTH`. |
| Sidewalk width | `1.5 m` each side | `SIDEWALK_WIDTH`. |
| Standard 2-lane road width | `10 m` | `7 m` asphalt + `1.5 m` sidewalk on each side. |
| Edge spatial query index | `RTree` | `spatial_edge_rt` handles edge AABB lookup. |
| Node lookup grid | `16 m` chunks | `spatial_node_grid` for nearest-node queries. |
| Routing / CCH dirty chunk | `512 m` | `RegionGraph::CHUNK_SIZE`; used for chunk tagging and edge-to-chunk overlap. |
| Environmental grid cell | `40 m` (`env_cell_m`) | Configurable via `MapConfig`. |
| Environmental grid (default map) | `500 × 500` | Derived from map size and `env_cell_m`. |

### Movement Speeds

| Mode | Speed | Status | Notes |
|------|-------|--------|-------|
| Walking | `4.0 m/s` (`14.4 km/h`) | Implemented | Used by pedestrian agents. |
| Driving (car) | `20.0 m/s` (`72 km/h`) | Implemented | Current free-flow / target value for civilian cars. |
| Bicycle | `5.5 m/s` (`20 km/h`) | Planned | First post-car transport mode; see `MOB-01` in [`roadmap.md`](roadmap.md). |
| Bus | `10–15 m/s` (`36–54 km/h`) | Planned | Lower effective speed due to stops and dwell time; see `TRANSIT-01` in [`roadmap.md`](roadmap.md). |
| Train / Metro | `20–40 m/s` (`72–144 km/h`) | Planned | Metro at lower end, intercity rail at higher end; see `TRANSIT-02` in [`roadmap.md`](roadmap.md). |
| Ship / Ferry | `5–10 m/s` (`18–36 km/h`) | Planned | Harbor-to-harbor routing; see `TRANSIT-03` in [`roadmap.md`](roadmap.md). |
| Airplane | `~200 m/s` (`720 km/h`) | Planned | Near-teleport at city scale; see `TRANSIT-04` in [`roadmap.md`](roadmap.md). |

### Key Design Patterns

- **`DataGrid<T>`**: flat row-major `Vec<T>` with width stride. Used for terrain, pollution, noise, desirability, and the world-space zoning textures / masks.
- **Environmental diffusion with swap buffers**: `PollutionSystem` and `NoiseSystem` use pre-allocated swap grids and `std::mem::swap()`; no per-tick `grid.clone()` in the hot path.
- **SoA via `soa_derive`**: `AgentSystem` is generated from `#[derive(StructOfArray)]` on `Agent`, producing `AgentVec` plus explicit scratch buffers around it.
- **Lane buckets for IDM**: per-lane occupancy / scratch lists are built and cleared incrementally each tick for car-following and overlap correction.
- **Edge R-tree + node chunk grid**: edge queries use `spatial_edge_rt`; node proximity uses `spatial_node_grid`; routing dirtiness still uses `512 m` chunks.
- **`(node, incoming_edge)` path state**: required for turn-restriction correctness at `Node::lane_connections`.

### Multi-modal Transport Vocabulary

The type vocabulary for current and planned transport modes lives in `simulation/network/types.rs` and `simulation/economy/agents/mod.rs`.

| Type | Declared values |
|------|----------------|
| `TransitType` | `Road, Rail, Ship, Air, Foot` |
| `TransitFlags` | `FOOT=1<<0, CAR=1<<1, RAIL=1<<2, SHIP=1<<3, AIR=1<<4` |
| `NodeType` | `Junction, Station, Harbor, Airport, Transfer, Border` |
| `MODE_*` constants | `WALK=0, CAR=1, BIKE=2, BUS_PASSENGER=3, TRAIN_PASSENGER=4, TAXI_PASSENGER=5, SHIP_PASSENGER=6` |

### Memory Budget (Default 20 km Map)

| Resource | Size | Notes |
|----------|------|-------|
| Terrain heightmap (`2000²`) | `16 MB` | One `f32` per height cell. |
| Terrain source copy | `16 MB` | Raw imported / editable source data. |
| 3 environmental grids at `500²` | `~3 MB` | Pollution, noise, desirability. |
| World-space zoning grids at `2000²` | `~12 MB` | Zone, occupied, and distance-to-road layers. |
| Road edges (`50k × ~512 B`) | `~25 MB` | Order-of-magnitude planning estimate. |
| Road nodes (`100k × ~128 B`) | `~12 MB` | Order-of-magnitude planning estimate. |
| Agent SoA base state (`1M`) | `~120 MB` | Approximate base scalar state; actual memory also depends on route `Vec` capacity and scratch buffers. |
| Agent speed field (`1M × 4 B`) | `4 MB` | Included in current SoA layout. |
| CCH contracted graph | `~20–30 MB` | Shortcut tables + elimination tree. |
| Road mesh VRAM (`50k` edges) | `~144 MB VRAM` | Approximate render budget. |

**Bandwidth note**: the current hot path uses pre-allocated buffers for environmental diffusion and agent tick scratch space. The old per-tick clone concern documented in earlier versions of this file is obsolete.

---

## Godot Layer

The Godot side is a thin bridge: no authoritative simulation logic lives here. GDScript handles rendering, input routing, and editor UX while `SimulationNode` owns the simulation state in Rust.

The asset editor and economy editor are separate launch modes inside the same Godot project. See [`asset_editor.md`](asset_editor.md) for the asset-tool contract and [`economy.md`](economy.md) for the economy-tool contract.

### Main Gameplay Scene (`godot/scenes/Main.tscn`)

| Node | Type | Script | Role |
|------|------|--------|------|
| Main | `Node3D` | — | Root gameplay scene. |
| WorldEnvironment | `WorldEnvironment` | — | Global environment settings. |
| DirectionalLight3D | `DirectionalLight3D` | — | Primary scene light and shadow source. |
| SimulationNode | Rust native | — | Owns simulation state and exposes `#[func]` methods. |
| Terrain | `MeshInstance3D` | `terrain.gd` | Heightmap mesh, sculpting, overlay textures. |
| Water | `MeshInstance3D` | `water.gd` | Shallow-water surface renderer. |
| RoadTool | `Node3D` | `road_tool.gd` | Road authoring tool and road mesh owner. |
| ZoningOverlay | `MeshInstance3D` | `zoning_overlay.gd` | Full-map zoning / occupancy / distance overlay. |
| ZoningTool | `Node3D` | `zoning_tool.gd` | World-space zoning paint tool. |
| Buildings | `Node3D` | `buildings.gd` | One MultiMesh per registered asset ID, plus foundations by zone. |
| Agents | `Node3D` | `agents.gd` | Per-type pedestrian and car MultiMesh renderers plus debug overlay. |
| LaneTool | `Node3D` | `lane_tool.gd` | Junction lane-connection editor. |
| MoveTool | `Node3D` | `move_tool.gd` | Road node drag / reposition tool. |
| NetworkRenderer | `Node` | `network_renderer.gd` | Async network-dirty refresh coordinator. |
| InputManager | `Node` | `input_manager.gd` | Global tool selection, save/load, undo, sim-speed routing. |
| CameraNode | `CameraNode` | — | Main gameplay camera. |
| MainUI | `CanvasLayer` | `main_ui.gd` | Procedurally built HUD and road property panel. |

Runtime-spawned tools:

- `SelectTool` is instantiated by `InputManager` at runtime for road-edge selection, crosswalk toggles, and edge-class editing.
- `CulDeSacTool` is instantiated by `InputManager` at runtime for dead-end cap toggles.

### Script → Rust Method Inventory

| Script | `SimulationNode` methods called |
|--------|---------------------------------|
| `input_manager.gd` | `undo_action()`, `save_game()`, `load_game()`, `set_simulation_speed()` |
| `main_ui.gd` | `get_no_building_spawn()`, `get_edge_class()`, `get_edge_geometry_3d()`, `get_height_at()` |
| `terrain.gd` | `get_heightmap_size()`, `get_heightmap_data()`, `intersect_terrain()`, `sculpt_terrain()`, `flatten_terrain_for_roads()`, `load_heightmap_data()`, `is_terrain_dirty()`, `clear_terrain_dirty()`, `get_pollution_image_data()`, `get_noise_image_data()`, `get_desirability_image_data()` |
| `water.gd` | `get_heightmap_size()`, `get_water_data()`, `get_water_velocity_data()`, `add_water_source()`, `is_water_dirty()`, `clear_water_dirty()` |
| `agents.gd` | `get_agent_cull_far_m()`, `get_agent_cull_padding_m()`, `set_camera_aabb()`, `get_agent_transforms()`, `get_car_transforms()`, `get_agent_paths_debug()` |
| `asset_editor.gd` | `is_asset_editor_mode()`, `load_asset_packs()`, `get_registered_asset_ids()`, `get_pack_manifest_json()`, `get_asset_manifest_json()`, `load_economy_project()`, `validate_and_export_asset()` |
| `buildings.gd` | `load_asset_packs()`, `get_registered_asset_ids()`, `get_lod0_native_path()`, `get_building_transforms_for_asset()`, `get_building_plot_transforms()` |
| `economy_editor.gd` | `is_economy_editor_mode()`, `load_economy_project()`, `export_economy_project()`, `run_economy_sandbox()` |
| `network_tool.gd` | `intersect_terrain()`, `add_road()`, `get_closest_network_point()`, `get_closest_node()`, `get_road_mesh_data()`, `get_network_nodes()`, `get_node_pos()`, `get_height_at()`, `get_road_ghost_guides()` |
| `road_tool.gd` | inherited `NetworkTool` methods, plus `check_border_candidate()` and `set_border_connection()` through deferred border checks |
| `network_renderer.gd` | `is_network_dirty()`, `flatten_terrain_for_roads()`, `clear_terrain_dirty()`, `clear_network_dirty()` |
| `move_tool.gd` | `get_closest_network_point()`, `get_closest_node()`, `get_height_at()`, `move_network_node()` |
| `lane_tool.gd` | `intersect_terrain()`, `get_closest_node()`, `get_node_lanes()`, `get_lane_connections_array()`, `set_lane_connection()`, `clear_lane_source()`, `clear_lane_connections()`, `get_node_pos()` |
| `zoning_tool.gd` | `intersect_terrain()`, `get_zone_subrect()`, `set_zone_rect()`, `set_zone_rect_raw()` |
| `zoning_overlay.gd` | `get_zone_grid_size()`, `get_heightmap_size()`, `get_zone_texture_data()`, `get_distance_texture_data()`, `get_occupied_texture_data()`, `get_no_build_mask_texture_data()`, `get_no_building_spawn_edge_indices()`, `get_edge_geometry_3d()` |
| `select_tool.gd` | `intersect_terrain()`, `get_closest_node()`, `get_node_lanes()`, `get_lane_connections_array()`, `get_node_pos()`, `has_crosswalk()`, `set_crosswalk_override()`, `set_lane_connection()`, `clear_lane_source()`, `clear_lane_connections()`, `get_edge_nodes()`, `get_hovered_edge()`, `set_edge_class()`, `set_no_building_spawn()`, `get_edge_geometry_3d()`, `get_edge_width()` |
| `cul_de_sac_tool.gd` | `get_closest_node()`, `get_node_connection_count()`, `has_cul_de_sac()`, `set_node_cul_de_sac()` |

### Data Format Reference

| Buffer / return value | Type | Layout / meaning |
|-----------------------|------|------------------|
| Heightmap | `PackedFloat32Array` | Flat row-major `width × height` `f32` values in metres. |
| Water depth | `PackedFloat32Array` | Same row-major layout as the heightmap. |
| Water velocity | `PackedFloat32Array` | Same row-major layout; scalar magnitude per water cell. |
| Pedestrian transforms | `VarDictionary` | Keys = `pedestrian_type`; values = `PackedFloat32Array` with `12` floats per instance: `[basis.x(3), basis.y(3), basis.z(3), origin(3)]`. |
| Car transforms | `VarDictionary` | Keys = `(vehicle_type * 10 + color_variant)`; values = `PackedFloat32Array` with the same `12`-float `Transform3D` layout. |
| Building transforms | `PackedFloat32Array` | Returned per asset ID by `get_building_transforms_for_asset(asset_id)`, same `12`-float transform layout. |
| Building plot transforms | `PackedFloat32Array` | Returned per zone ID by `get_building_plot_transforms(zone_id)`, same `12`-float transform layout. |
| Agent path debug | `VarDictionary` | `points: PackedVector3Array`, `colors: PackedColorArray`. |
| Pollution / Noise / Desirability overlays | `PackedByteArray` | RGBA8, one pixel per heightmap cell, uploaded as shader textures. |
| Zone texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
| Occupied texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
| Distance-to-road texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
| No-build mask texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
