# Metrum Rise — Reference

Stable lookup tables for architecture constants, runtime vocabulary, memory budgets, and data formats. Update this file when specs change. For current status see [`project.md`](project.md); for active tracked work see [`roadmap.md`](roadmap.md); for doc ownership see [`README.md`](README.md).

Terminology note: this file mirrors durable code-facing names on purpose. When subsystem specs use broader gameplay terms such as `build site`, this reference may still show names such as `lot_width_cells` until the underlying runtime data model changes.

---

## Architecture Reference

### Spatial And Grid Specifications

| Parameter | Value | Notes |
|-----------|-------|-------|
| World extent | Player-configurable | Fallback gameplay world is `20 km × 20 km`; authored blank worlds can be larger. |
| Terrain sample cell | `10 m` (`terrain_cell_m`) | Configurable independently from `zone_cell_m` in `WorldConfig`. |
| Terrain chunk | `512 m` (`terrain_chunk_m`) | Canonical authored terrain chunk size in `WorldConfig`. |
| Zoning cell | `10 m × 10 m` (`zone_cell_m`) | Configurable via `WorldConfig`. |
| Runtime terrain grid (default map) | `2001 × 2001` | Derived from `round(width_m / terrain_cell_m) + 1`; terrain samples include both world edges. |
| Building footprint in zoning cells | Asset-authored | `lot_width_cells × lot_depth_cells`; authored building footprint measured from `zone_cell_m`. |
| Reference zoning depth | `12` cells | `DEFAULT_ZONING_DEPTH`; tooling / fade heuristic only, not a hard cap. |
| Lane width | `3.5 m` | `LANE_WIDTH`. |
| Sidewalk width | `1.5 m` each side | `SIDEWALK_WIDTH`. |
| Standard 2-lane road width | `10 m` | `7 m` asphalt + `1.5 m` sidewalk on each side. |
| Edge spatial query index | `RTree` | `spatial_edge_rt` handles edge AABB lookup. |
| Node lookup grid | `16 m` chunks | `spatial_node_grid` for nearest-node queries. |
| Routing / CCH dirty chunk | `512 m` | `RegionGraph::CHUNK_SIZE`; used for chunk tagging and edge-to-chunk overlap. |
| Environmental grid cell | `40 m` (`env_cell_m`) | Configurable via `WorldConfig`. |
| Environmental grid (default map) | `500 × 500` | Derived from map size and `env_cell_m`. |

### Movement Speeds

Detailed vehicle movement behavior, including junction turn caps and lane changes, is owned by
[`traffic.md`](traffic.md).

| Mode | Speed | Status | Notes |
|------|-------|--------|-------|
| Walking | `4.0 m/s` (`14.4 km/h`) | Implemented | Used by pedestrian agents on the network. |
| Driving (car) | `13.89 m/s` (`50 km/h`) | Implemented | Default free-flow speed for the current urban road presets. |
| Junction car turn | `6.0 m/s` (`21.6 km/h`) | Implemented | Cap while traversing junction connector lanes. |
| Bicycle | `5.5 m/s` (`20 km/h`) | Planned | First post-car transport mode; see `MOB-01` in [`roadmap.md`](roadmap.md). |
| Bus | `10–15 m/s` (`36–54 km/h`) | Planned | Lower effective speed due to stops and dwell time; see `TRANSIT-01` in [`roadmap.md`](roadmap.md). |
| Train / Metro | `20–40 m/s` (`72–144 km/h`) | Planned | Metro at lower end, intercity rail at higher end; see `TRANSIT-02` in [`roadmap.md`](roadmap.md). |
| Ship / Ferry | `5–10 m/s` (`18–36 km/h`) | Planned | Harbor-to-harbor routing; see `TRANSIT-03` in [`roadmap.md`](roadmap.md). |
| Airplane | `~200 m/s` (`720 km/h`) | Planned | Near-teleport at city scale; see `TRANSIT-04` in [`roadmap.md`](roadmap.md). |

### Key Design Patterns

- **Sparse chunk-backed terrain/water storage**: runtime terrain and water keep only touched chunks resident, with dense row-major buffers materialized only for save/load and renderer upload boundaries.
- **`DataGrid<T>`**: flat row-major `Vec<T>` with width stride. Used for pollution, noise, and desirability.
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

### Benchmark Reference

| Surface | Command / coverage | Notes |
|---------|---------------------|-------|
| Criterion microbenchmarks | `cd rust && cargo bench` | Runs `rust/benches/agent_benchmark.rs`. |
| Pure road movement | `AgentSystem::tick/on_road/*` | Measures pre-pathed `TRANSIT_NETWORK` throughput. |
| Idle SoA scan cost | `AgentSystem::tick/idle_scaling/*` | Measures idle/no-trip scan overhead. |
| Access egress | `AgentSystem::tick_access/access_egress_car/*` | Measures the live `ACCESS_EGRESS` car path using a real entrance cache and lane handoff. |
| Access ingress | `AgentSystem::tick_access/access_ingress_car/*` | Measures the live `ACCESS_INGRESS` car path using a real entrance cache and lane detach/door approach. |

Benchmark-history rule:

- treat older Criterion results as a different baseline if they were captured before the access-phase benchmarks were added or before the benchmark setup was reshaped
- only call something a regression when the compared runs use the same benchmark family and the same benchmark setup shape

### Memory Budget (Default 20 km Map)

| Resource | Size | Notes |
|----------|------|-------|
| Terrain visual sparse chunks (`2001²` worst case) | `up to 16 MB` | Full-cost only when the entire runtime terrain is materialized away from the base elevation. |
| Terrain source sparse chunks (`2001²` worst case) | `up to 16 MB` | Same upper bound as visual terrain; untouched chunks stay implicit. |
| 3 environmental grids at `500²` | `~3 MB` | Pollution, noise, desirability. |
| Zoning parcels | Bounded by authored lots | Stable parcel records plus chunk lookup; no full-map zoning surface. |
| Road edges (`50k × ~512 B`) | `~25 MB` | Order-of-magnitude planning estimate. |
| Road nodes (`100k × ~128 B`) | `~12 MB` | Order-of-magnitude planning estimate. |
| Agent SoA base state (`1M`) | `~120 MB` | Approximate base scalar state; actual memory also depends on route `Vec` capacity and scratch buffers. |
| Agent speed field (`1M × 4 B`) | `4 MB` | Included in current SoA layout. |
| CCH contracted graph | `~20–30 MB` | Shortcut tables + elimination tree. |
| Road mesh VRAM (`50k` edges) | `~144 MB VRAM` | Approximate render budget. |

**Bandwidth note**: the current hot path uses pre-allocated buffers for environmental diffusion and agent tick scratch space. The old per-tick clone concern documented in earlier versions of this file is obsolete.

---

## Runtime Boundary Note

Godot is a rendering, input, and editor bridge. Authoritative simulation state and gameplay decisions live in Rust. This file intentionally does not maintain an exhaustive script-to-`SimulationNode` inventory; that API changes too quickly for a hand-written reference. Use `rust/src/nodes/simulation_node.rs` and `rg "simulation_node\.|sim\." godot/scripts` when auditing current bridge calls.

## Debug Launch Reference

`run.sh` is the canonical launch wrapper for local debug sessions. These flags set environment
variables before building Rust, deploying `libmetrum_rise.so`, and launching Godot.

### Primary Debug Flags

| Flag | Main environment | Output / effect |
|------|------------------|-----------------|
| `--debug` | `METRUM_DEBUG=1` | General debug logging to stdout. |
| `--debug <category>` | `METRUM_DEBUG=1`, `METRUM_DEBUG_FILTER=<category>` | Category-filtered logging. Common categories include `isect`, `economy`, `demand`, `road`, `border`, `terrain`, `buildings`, `visuals`, `perf`, and `world-editor`. |
| `--debug road` | `METRUM_DEBUG_FILTER=road`, `METRUM_DEBUG_ROAD_GEOMETRY_DUMP=1`, `METRUM_DEBUG_SURFACE=1` | Road placement timings, geometry dump, terrain/water patch diagnostics, and road-surface overlay. |
| `--debug terrain` | `METRUM_DEBUG_TERRAIN=1` | Terrain and water patch residency/perf summaries. |
| `--debug terrain-verbose` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_VERBOSE=1` | Terrain summaries plus residency-change logs. |
| `--debug terrain-full` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD=1` | Force full-world terrain/water residency for cost comparison. |
| `--debug terrain-lod1` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_FORCE_LOD1=1` | Force all resident terrain/water patch meshes to LOD1. |
| `--debug terrain-full-lod1` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD=1`, `METRUM_DEBUG_TERRAIN_FORCE_LOD1=1` | Force full-world residency and LOD1 meshes for seam/material debugging. |
| `--debug perf` | `METRUM_DEBUG_PERF=1` | Frame CPU summaries by renderer every 0.5 s. Reports total renderer CPU per completed frame plus per-renderer/detail averages and maxes. |
| `--debug buildings` | `METRUM_DEBUG_BUILDINGS=1` | Building-site mesh, material, height, and site metadata logs only. |
| `--debug building-sites-visual [mode]` | `METRUM_DEBUG_BUILDING_SITES_VISUAL=<mode>` | Building-site visual overlay. Current mode: `material`. |
| `--debug site-grading` | `METRUM_DEBUG_FILTER=road`, `METRUM_DEBUG_BUILDINGS=1`, `METRUM_DEBUG_BUILDING_SITES_VISUAL=material` | Combined road CDT, building-site edge/grading, and site material diagnostics for road/yard seam debugging. |
| `--debug traffic` / `--debug-traffic` | `METRUM_DEBUG_TRAFFIC=1` | Traffic/routing and road-network connectivity logging to stderr plus visual lane/connector debug labels. |
| `--pedestrian-vat-debug=<mode>` | `METRUM_DEBUG_PEDESTRIAN_VAT=<mode>` | Pedestrian VAT material debug. Modes: `rest` disables animation/VAT offsets so rigid sliding is expected, `uv` colors vertex-ID UVs, and `off`/`offset` colors VAT offset magnitude while applying offsets. Use no pedestrian VAT debug flag for normal animated character colors. |
| `--debug-world-editor` | `METRUM_DEBUG=1`, `METRUM_DEBUG_FILTER=world-editor` | WorldEditor create/open/save/tool activity. |
| `--debug-sim` | `METRUM_DEBUG_SIM=1` | Hourly simulation summaries to stdout. |
| `--debug visuals [mode]` / `--debug visual [mode]` / `--visuals [mode]` | `METRUM_DEBUG_TERRAIN_GRASS=<mode>` | Terrain grass material diagnostic view. Defaults to `material`. |
| `--debug terrain-visual <mode>` | `METRUM_DEBUG_TERRAIN=1`, `METRUM_DEBUG_TERRAIN_VISUAL=<mode>` | Terrain/water/lighting material diagnostic view. Defaults to `patch`. |

Removed debug aliases:

- `--debug road-geometry` is intentionally rejected; use `--debug road`

### Terrain Grass Visual Modes

These modes are selected with `--debug visuals <mode>` or `--visuals <mode>`.

| Mode | Aliases | Meaning |
|------|---------|---------|
| `raw` | `albedo` | Direct Grass002 albedo at terrain UV scale. |
| `macro` | | Large stochastic grass layer. |
| `mid` | | Mid-distance stochastic grass layer. |
| `micro` | | Close-up grass layer. |
| `fades` | `fade`, `visibility` | RGB visibility of macro / mid / micro grass layers. |
| `material` | `composite` | Grass material composite before hillshade and contour effects. |
| `height` | | Grass002 height map. |
| `mask` | `grass-mask` | Where grass detail is allowed. |
| `luminance` | `luma`, `brightness` | World-space bands for base, macro, mid, micro, and final brightness. |
| `footprint` | `footprints` | RGB = texture footprint, micro visibility, grass mask. |

`luminance` band colors:

- red = base brightness
- yellow = macro brightness
- green = mid brightness
- blue = micro brightness
- white = final brightness

### Terrain / Water Visual Modes

These modes are selected with `--debug terrain-visual <mode>`.

| Mode | Aliases | Meaning |
|------|---------|---------|
| `patch` | `patches` | Terrain patch identity colors with patch borders. |
| `lod` | `lods` | Terrain mesh LOD colors with patch borders. |
| `height` | | Terrain height field. |
| `relief` | | Local terrain relief. |
| `shore` | `shoreline` | Terrain-side shore mask from visible water depth. |
| `water-depth` | `water`, `depth` | Water depth field on terrain and water. |
| `water-lod` | | Water mesh LOD colors. |
| `water-patch` | | Water patch identity colors. |
| `water-material` | `water-mat`, `material-water` | Water material bands: depth tint, alpha, Fresnel, foam, and normal strength. |
| `lighting` | `light`, `sun` | Sun-facing strength, shadow/cascade bands, and water specular mask. |

### Building-Site Visual Modes

These modes are selected with `--debug building-sites-visual <mode>`.

| Mode | Aliases | Meaning |
|------|---------|---------|
| `material` | `materials`, `source`, `sources` | Tints site ground, asphalt, and concrete by material source. |

## Data Format Reference

| Buffer / return value | Type | Layout / meaning |
|-----------------------|------|------------------|
| Heightmap | `PackedFloat32Array` | Flat row-major `width × height` `f32` raw terrain sample values in current runtime storage units; multiply by `HEIGHT_SCALE` to convert to rendered world metres. |
| Water depth | `PackedFloat32Array` | Same row-major layout as the heightmap; visible baseline still-water depth in world metres. |
| Pedestrian transforms | `VarDictionary` | Keys = `pedestrian_type`; values = `PackedFloat32Array` with `12` floats per instance: `[basis.x(3), basis.y(3), basis.z(3), origin(3)]`. |
| Car transforms | `VarDictionary` | Keys = `(vehicle_type * 10 + color_variant)`; values = `PackedFloat32Array` with the same `12`-float `Transform3D` layout. |
| Building transforms | `PackedFloat32Array` | Returned per asset ID by `get_building_transforms_for_asset(asset_id)`, same `12`-float transform layout. |
| Building plot transforms | `PackedFloat32Array` | Returned per zone ID by `get_building_plot_transforms(zone_id)`, same `12`-float transform layout; used for zoned building footprint or foundation preview, not for a separate parcel system. |
| Agent path debug | `VarDictionary` | `points: PackedVector3Array`, `colors: PackedColorArray`. |
| Pollution / Noise / Desirability overlays | `PackedByteArray` | RGBA8, one pixel per heightmap cell, uploaded as shader textures. |
| Zone texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
| Occupied texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
| Distance-to-road texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
| No-build mask texture | `PackedByteArray` | R8, one byte per world-space zone cell. |
| `WorldDefinition` meta | SQLite row | One row storing world name plus `WorldConfig` values for a reusable blank-world asset. |
| `WorldDefinition` terrain chunk payload | SQLite BLOB | Dense row-major `f32` source-terrain samples for one persisted authored chunk. |
| `WorldDefinition` lake fill | SQLite row | One authored lake seed position plus one target surface elevation in rendered world metres. |
| `WorldDefinition` open-water fill | SQLite row | One authored edge-connected open-water seed position plus one target surface elevation in rendered world metres. |
