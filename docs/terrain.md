# Terrain / World Terrain Spec

## Purpose

This document owns world extent, terrain storage, terrain-to-road interaction, water storage,
and the deterministic implementation path from the current chunk-aware runtime to a real large-world
authoring pipeline.

It answers questions like:

- what `WorldConfig` means today
- what terrain and water state is authoritative
- when dense buffers are still allowed
- what is already implemented
- what the next deterministic implementation slices must do

It does not own zoning legality, building placement rules, or multi-tier inactive-region
simulation behavior. Those remain owned by their respective docs.

## Document Conventions

Interpretation rules:

- Sections under `Implemented Runtime Contract` describe the live code unless explicitly marked as a
  compatibility gap.
- Sections under `Planned Deterministic Implementation` are intended behavior, not shipped behavior.
- `must` means required for the owning contract.
- `should` means intended unless a better measured implementation replaces it.
- `may` means allowed but optional.

Terminology:

- `world extent`: the authored width and height of the map
- `runtime terrain cell`: one live terrain sample in the current in-memory terrain grid
- `authored terrain chunk`: the canonical chunk span described by `WorldConfig`
- `source terrain`: the authoritative player- or importer-authored terrain surface
- `visual terrain`: the derived terrain surface after road-bed carving
- `WorldDefinition`: the reusable authored-world asset for blank-world v1

## Implemented Runtime Contract

### 1. `WorldConfig` Is The Authoritative World Metadata

The live runtime now uses `WorldConfig` instead of the old `MapConfig`.

```rust
pub struct WorldConfig {
    pub width_m: f32,
    pub height_m: f32,
    pub terrain_cell_m: f32,
    pub terrain_chunk_m: f32,
    pub terrain_base_elevation_m: f32,
    pub env_cell_m: f32,
    pub zone_cell_m: f32,
}
```

Current defaults:

- fallback gameplay world: `20_000 m × 20_000 m`
- editor sandbox: `500 m × 500 m`
- default terrain sample cell: `10 m`
- canonical terrain chunk span: `512 m`
- default base terrain elevation: `0.0`
- default environmental cell: `40 m`
- default zoning cell: `10 m`

Current deterministic rules:

- `WorldConfig` is saved and loaded as part of every city save.
- old save migration is intentionally not required; version mismatch is a hard rejection.
- authored terrain chunk count is:
  - `terrain_chunk_columns = ceil(width_m / terrain_chunk_m)`
  - `terrain_chunk_rows = ceil(height_m / terrain_chunk_m)`

### 2. Terrain And Water Grid Sizing Is Now Explicit

Current deterministic rule:

- `terrain_cell_m` is an explicit `WorldConfig` field
- `terrain.width = terrain_grid_width()`
- `terrain.height = terrain_grid_height()`
- `water.width = terrain_grid_width()`
- `water.height = terrain_grid_height()`
- `terrain_grid_width = round(width_m / terrain_cell_m) + 1`
- `terrain_grid_height = round(height_m / terrain_cell_m) + 1`
- runtime sparse chunk span in cells is:
  - `ceil(terrain_chunk_m / terrain_cell_m)`

Implication:

- terrain sample density is now configurable independently from zoning density
- runtime world-space XZ is now canonical metres
- terrain, zoning, and environment each use their own cell spacing explicitly

### 3. Terrain Uses Dual Sparse Buffers

The live `TerrainSystem` owns two sparse chunk-backed grids:

- `source terrain`: authoritative sculpted terrain
- `visual terrain`: derived terrain after road flattening

Current deterministic rules:

- untouched sparse terrain cells are implicit and read back as the configured base elevation
- setting terrain directly writes both source and visual buffers
- sculpting writes both source and visual buffers
- `reset_visuals_from_source()` discards the current visual terrain and clones the authoritative
  source terrain into it
- save/load persists the authoritative source terrain only
- renderer uploads use the visual terrain

### 4. Terrain Queries Read Authoritative Source Terrain

Terrain height queries are authoritative against the source terrain surface, not the road-carved
visual terrain.

Current deterministic rules:

- `get_height(x, y)` reads source terrain
- `sample_height_world(x, z)` reads source terrain
- raycasts and terrain height queries use source terrain interpolation in world space
- road-bed carving is a visual derivation, not an edit to source terrain

This preserves the rule that road placement must not feed back into the terrain that grade and
slope calculations treat as authored ground.

### 5. Terrain Height Storage Is Scaled At Query / Render Boundaries

The live runtime stores raw terrain sample values and multiplies them by `HEIGHT_SCALE` when
converting to world-space `y`.

Current deterministic rule:

- `world_y = terrain_sample * HEIGHT_SCALE`
- `HEIGHT_SCALE = 20.0`

This means the live terrain buffer is not currently a direct world-space metre heightfield.

Important note:

- `terrain_base_elevation_m` is currently forwarded into the raw terrain sample storage before
  `HEIGHT_SCALE` is applied
- the `_m` suffix is therefore ahead of the current implementation and should not be treated as a
  proof that the live terrain buffer already stores fully world-space metres

### 6. Terrain Coordinates Are Centered At World Origin

The live terrain surface is centered around `(0, 0)` in world XZ space.

Current deterministic rules:

- terrain local grid coordinates span `0..width-1` and `0..height-1`
- world-space XZ is centered by:
  - `half_w = ((width - 1) * terrain_cell_m) * 0.5`
  - `half_h = ((height - 1) * terrain_cell_m) * 0.5`
- world-to-grid conversion for terrain queries uses that centered origin convention
- terrain samples sit on world-edge coordinates
- zoning and environmental cells remain centre-aligned metre cells

### 7. Sparse Chunk Storage Is Authoritative At Rest

Terrain and water now use sparse chunk-backed storage internally.

Current deterministic rules:

- a sparse chunk is allocated only when at least one cell in that chunk differs from the default
  value
- resetting a cell back to the default value may cause its chunk to be removed if the whole chunk
  becomes default again
- random cell `get` / `set` is expected-average `O(1)` against the chunk map
- dense materialization is `O(width × height)` and must remain a boundary-only operation

Allowed dense boundaries today:

- save/load
- renderer upload to Godot
- temporary compatibility scratch buffers inside terrain road-flattening
- temporary compatibility scratch buffers inside water ticking
- undo snapshots

### 8. Road Flattening Is A Visual Derivation Step

Road placement does not rewrite source terrain. The runtime derives a visual terrain buffer with
road beds carved into it.

Current deterministic sequence:

1. reset visual terrain from source terrain
2. materialize a dense visual scratch buffer
3. carve road beds into that visual scratch buffer
4. replace sparse visual terrain from the dense scratch buffer
5. sync road geometry back to terrain-dependent caches

Authoritative rule:

- road flattening changes the visual terrain only
- source terrain remains the authored ground surface

### 9. Water Uses Sparse Depth / Velocity / Flux Storage

The live `WaterSystem` stores these sparse chunk-backed layers:

- `depth`
- `velocity`
- `flux`

Water sources remain an explicit list of:

- `(grid_x, grid_y, rate_m_per_tick)`

Current deterministic rules:

- untouched water cells are implicitly dry
- `add_water` mutates water depth at one cell
- `update_source` accumulates source rate at one cell
- water save/load persists dense snapshots of depth, velocity, flux, plus the source list

### 10. Water Tick Is Still Dense Inside The Compatibility Boundary

The live water system is sparse at rest but not yet sparse during simulation math.

Current deterministic sequence:

1. materialize dense `depth`, `velocity`, and `flux`
2. run the Saint-Venant update on those dense scratch buffers
3. sparsify the results back into chunk-backed storage

This is allowed today only as a compatibility step. It is not the final large-world water runtime.

### 11. Save / Load Remains Dense At The Serialization Boundary

Sparse runtime storage does not change the save format boundary yet.

Current deterministic rules:

- terrain source data is serialized as one dense row-major `f32` blob
- water depth, velocity, and flux are serialized as dense row-major blobs
- sparse chunk topology is not currently saved as chunk records
- loading reconstructs sparse storage from those dense blobs

This is acceptable while city saves remain runtime snapshots rather than reusable authored world
assets.

### 12. Blank-World `WorldDefinition` Exists As A Separate Asset

The live runtime now has a separate authored-world asset path for blank worlds.

Current deterministic rules:

- `WorldDefinition` is stored as a single-file SQLite asset with its own schema version
- one `world_definition_meta` row stores:
  - world name
  - `WorldConfig` values needed to instantiate the world
- authored terrain is stored as zero or more `world_terrain_chunks` rows
- terrain chunk rows are keyed by zero-based `(chunk_x, chunk_z)` from the world minimum corner
- each chunk payload is one dense row-major `f32` source-terrain block
- only chunks containing at least one non-base terrain sample are persisted
- loading a `WorldDefinition` resets runtime state to a fresh blank city on that world
- `WorldDefinition` v1 stores:
  - world metadata
  - terrain config
  - source terrain chunks
- `WorldDefinition` v1 does not store:
  - roads
  - zoning paint
  - water runtime state
  - agents
  - households
  - treasury history
  - derived visual terrain

Authoritative rule:

- city saves and `WorldDefinition` are separate persistence products with different ownership

### 13. Godot Bridge Uploads Dense Snapshots On Demand

The Godot render bridge still consumes full dense buffers.

Current deterministic rules:

- `get_heightmap_data()` materializes dense visual terrain
- `get_water_data()` materializes dense water depth
- `get_water_velocity_data()` materializes dense water velocity

This is a rendering boundary, not an excuse for simulation systems to depend on dense storage.

## Implemented Compatibility Gaps We Should Not Extend

The following are live behaviors, but they are not the intended long-term ownership model.

### 1. Dense Scratch Buffers Still Exist In Hot Adjacent Systems

These are compatibility-only.

Current gap:

- road flattening materializes a full dense terrain buffer
- water tick materializes full dense water buffers
- render upload materializes full dense buffers every refresh

Required direction:

- these paths should later localize to chunk windows or active areas instead of whole-map buffers

### 2. Undo Is Not Yet Fully Authoritative For Terrain Ownership

Current repository state:

- undo snapshots currently capture visual terrain and water depth snapshots for compatibility with
  the pre-sparse mutation path

Required direction:

- terrain undo must become authoritative over source terrain and any derived visual state it
  invalidates
- world-authoring workflows must not depend on visual-only terrain snapshots

### 3. Terrain Height Storage Is Still Scaled At Query / Render Boundaries

Current gap:

- terrain samples are still multiplied by `HEIGHT_SCALE` when converted to world-space `y`
- `terrain_base_elevation_m` still enters raw sample storage before that scale is applied

Required direction:

- terrain base elevation and sample values should eventually become direct world-space metres
- no future authored-world format should assume the current scaled-height compatibility contract

### 4. `WorldDefinition` V1 Is Terrain-Only And Not Yet Wired To A Dedicated UI Flow

Current gap:

- there is no dedicated world-editor launch mode or scene yet
- no gameplay or editor UI currently calls the `SimulationNode` world-definition methods
- `WorldDefinition` v1 stores terrain only; it has no hydrology, preview image, or richer metadata

Required direction:

- authored-world editing must move onto a dedicated world-editor shell
- `New Game` must instantiate from a selected `WorldDefinition` rather than from ad hoc runtime state

## Planned Deterministic Implementation

The next slices should be implemented in this order.

### 1. Ship A Dedicated Blank-World Editor Flow Around `WorldDefinition`

The persistence and runtime reset path now exist. The next required slice is the actual authoring
shell around them.

Required direction:

- add a dedicated world-editor launch mode or scene
- wire editor UI to:
  - `create_blank_world`
  - `save_world_definition`
  - `load_world_definition`
- keep blank-world authoring separate from city runtime save/load UI

Deterministic rule:

- a newly created blank world is dry and flat except for the authored base elevation

### 2. Instantiate `New Game` From A Selected `WorldDefinition`

Required direction:

- `New Game` must clone or instantiate from one authored `WorldDefinition`
- city saves must remain runtime snapshots layered on top of that authored world baseline

### 3. Move From Whole-Map Dense Compatibility Buffers To Chunk Windows

Now that `terrain_cell_m` exists, dense scratch buffers must be localized.

Required direction:

- road flattening should operate on touched chunk windows, not full-world dense terrain
- water simulation should operate on active chunk windows or other bounded regions, not full-world
  dense arrays
- renderer upload should be able to refresh only the chunks the camera or editor currently needs

Deterministic rule:

- whole-map dense materialization is a temporary compatibility path, not the target large-world
  runtime

### 4. Heightmap / DEM Import Comes After Blank Worlds

Imported terrain must write authoritative source terrain only.

Deterministic rules:

- imported terrain writes source terrain, never visual-only terrain
- visual terrain is always derived from source terrain plus road or water derivations
- import validation must reject malformed or dimension-mismatched source rasters

### 5. Add Authored Hydrology As A Separate Layer

Hydrology must be authored separately from raw terrain elevation.

Future authored hydrology data should include:

- inflows / springs
- outflows / sinks
- river paths or channels
- lake basins or lake surface levels

Deterministic rules:

- a blank world with no hydrology remains dry
- authored hydrology defines where water belongs before runtime simulation modifies local details
- hydrology must not be stored as "just paint some water depth into the live runtime buffer"

### 6. Use A Hybrid Water Model For Very Large Worlds

The intended large-world water model is hybrid.

Deterministic rules:

- authored hydrology defines the baseline rivers, lakes, inflows, and outflows for the whole world
- local active areas may run dynamic water simulation
- the engine must not require a full-world dense shallow-water solve for every map size

## Current Deterministic Non-Goals

The following are explicitly not implemented yet and should not be assumed by other systems:

- blank-world editor UI
- `New Game` selection flow for `WorldDefinition`
- DEM / GeoTIFF import into authored worlds
- authored hydrology layer
- chunk-streamed terrain renderer
- chunk-window water simulation
- authoritative terrain undo across source plus derived state
- direct-metre terrain height storage without `HEIGHT_SCALE`

## Implementation Guardrails

These rules must stay true as the terrain/world system grows:

- source terrain is authoritative; derived terrain is never the only source of truth
- sparse chunk-backed storage is the resting runtime representation
- dense buffers are boundary tools, not the runtime ownership model
- new terrain import or authoring paths must write authoritative source terrain
- old save migration is not required unless a future change explicitly decides otherwise
- large-world support must avoid whole-world dense simulation assumptions

## Short Version

What is implemented now:

- `WorldConfig` replaced the legacy map config
- `terrain_cell_m` exists and terrain/water sample density is independently configurable
- runtime world-space XZ is canonical metres again
- terrain and water are sparse chunk-backed at rest
- terrain keeps authoritative source plus derived visual buffers
- water keeps sparse depth, velocity, and flux at rest
- blank-world `WorldDefinition` exists as a separate authored-world asset
- authored world load resets runtime state to a fresh blank city baseline
- save/load and renderer boundaries still use dense materialization

What is next:

1. blank-world editor UI
2. `New Game` from `WorldDefinition`
3. chunk-window runtime processing
4. later DEM import
5. later authored hydrology
