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
- what is intentionally still compatibility-only
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
- `WorldDefinition`: the future reusable authored-world asset; not yet implemented

## Implemented Runtime Contract

### 1. `WorldConfig` Is The Authoritative World Metadata

The live runtime now uses `WorldConfig` instead of the old `MapConfig`.

```rust
pub struct WorldConfig {
    pub width_m: f32,
    pub height_m: f32,
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
- runtime sparse chunk span in cells is:
  - `ceil(terrain_chunk_m / terrain_cell_m)`

Current compatibility rule:

- the live runtime still measures world-space positions in zoning-derived world units
- `terrain_cell_m` is converted into those runtime units through `terrain_cell_m / zone_cell_m`

Implication:

- terrain sample density is now configurable independently from zoning density
- the coordinate-system compatibility layer still exists and should be removed later

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
  - `half_w = ((width - 1) * terrain_cell_world_units) * 0.5`
  - `half_h = ((height - 1) * terrain_cell_world_units) * 0.5`
- world-to-grid conversion for terrain queries uses that centered origin convention

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

### 12. Godot Bridge Uploads Dense Snapshots On Demand

The Godot render bridge still consumes full dense buffers.

Current deterministic rules:

- `get_heightmap_data()` materializes dense visual terrain
- `get_water_data()` materializes dense water depth
- `get_water_velocity_data()` materializes dense water velocity

This is a rendering boundary, not an excuse for simulation systems to depend on dense storage.

## Implemented Compatibility Gaps We Should Not Extend

The following are live behaviors, but they are not the intended long-term ownership model.

### 1. Terrain Resolution Is Still Tied To Zoning Resolution

This ownership problem is resolved at the storage layer but not yet at the coordinate-system layer.

Current gap:

- terrain and water dimensions no longer derive directly from `zone_cell_m`
- runtime world-space still uses zoning-derived world units and converts terrain through
  `terrain_cell_m / zone_cell_m`

Required direction:

- terrain resolution must stay independent from zoning and environment resolution
- the runtime coordinate system should later stop depending on zoning-derived world units entirely

### 2. Dense Scratch Buffers Still Exist In Hot Adjacent Systems

These are compatibility-only.

Current gap:

- road flattening materializes a full dense terrain buffer
- water tick materializes full dense water buffers
- render upload materializes full dense buffers every refresh

Required direction:

- these paths should later localize to chunk windows or active areas instead of whole-map buffers

### 3. Undo Is Not Yet Fully Authoritative For Terrain Ownership

Current repository state:

- undo snapshots currently capture visual terrain and water depth snapshots for compatibility with
  the pre-sparse mutation path

Required direction:

- terrain undo must become authoritative over source terrain and any derived visual state it
  invalidates
- world-authoring workflows must not depend on visual-only terrain snapshots

### 4. Reusable Authored Worlds Do Not Exist Yet

Current gap:

- city saves are the only persisted world-state mechanism
- there is no separate reusable `WorldDefinition`

Required direction:

- authored blank worlds must become a separate asset type from city runtime saves

## Planned Deterministic Implementation

The next slices should be implemented in this order.

### 1. Add `WorldDefinition` As A Reusable Authored-World Asset

`WorldDefinition` is the next missing ownership boundary.

It must be separate from city saves and must eventually contain:

- world dimensions
- terrain chunk span
- terrain sample spacing
- base terrain elevation
- authored terrain chunk payloads
- later hydrology data
- later metadata such as display name, seed, preview image, and source provenance

Deterministic rule:

- city saves persist runtime state
- `WorldDefinition` persists reusable authored world state
- `New Game` will instantiate a city from a selected `WorldDefinition`

### 2. Ship Blank-World Authoring First

The first world-authoring slice should support blank worlds only.

Blank-world v1 must support:

- create blank world with chosen extent
- choose base elevation
- sculpt source terrain
- save and load authored blank worlds
- start a city from that authored world

Blank-world v1 must not depend on:

- DEM import
- GeoTIFF import
- hydrology authoring
- full large-world regional simulation

Deterministic rule:

- a newly created blank world is dry and flat except for the authored base elevation

### 3. Complete The Terrain / Zoning Resolution Split

The storage-level split now exists. The next required work is finishing the coordinate and tooling
side of that split.

Required direction:

- keep `terrain_cell_m` authoritative for terrain and water sample density
- keep zoning dimensions derived from `zone_cell_m`
- keep environmental dimensions derived from `env_cell_m`
- progressively remove remaining zoning-derived world-unit assumptions from runtime helpers and UI

Deterministic rule:

- terrain, zoning, and environment grids remain independently sized from their own cell spacing
- no new runtime system may assume terrain cell count equals zoning cell count

### 4. Move From Whole-Map Dense Compatibility Buffers To Chunk Windows

Now that `terrain_cell_m` exists, dense scratch buffers must be localized.

Required direction:

- road flattening should operate on touched chunk windows, not full-world dense terrain
- water simulation should operate on active chunk windows or other bounded regions, not full-world
  dense arrays
- renderer upload should be able to refresh only the chunks the camera or editor currently needs

Deterministic rule:

- whole-map dense materialization is a temporary compatibility path, not the target large-world
  runtime

### 5. Heightmap / DEM Import Comes After Blank Worlds

Imported terrain must write authoritative source terrain only.

Deterministic rules:

- imported terrain writes source terrain, never visual-only terrain
- visual terrain is always derived from source terrain plus road or water derivations
- import validation must reject malformed or dimension-mismatched source rasters

### 6. Add Authored Hydrology As A Separate Layer

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

### 7. Use A Hybrid Water Model For Very Large Worlds

The intended large-world water model is hybrid.

Deterministic rules:

- authored hydrology defines the baseline rivers, lakes, inflows, and outflows for the whole world
- local active areas may run dynamic water simulation
- the engine must not require a full-world dense shallow-water solve for every map size

### 8. World Authoring Mode Must Become Separate From Live City Runtime

World editing is not just another gameplay tool.

Required direction:

- add a dedicated world-editor launch mode or scene
- world editing mutates `WorldDefinition`
- city runtime loads from authored world data rather than directly serving as the authoring store

## Current Deterministic Non-Goals

The following are explicitly not implemented yet and should not be assumed by other systems:

- reusable `WorldDefinition`
- blank-world editor UI
- DEM / GeoTIFF import into authored worlds
- authored hydrology layer
- chunk-streamed terrain renderer
- chunk-window water simulation
- authoritative terrain undo across source plus derived state
- fully zoning-free runtime world coordinates

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
- terrain and water are sparse chunk-backed at rest
- terrain keeps authoritative source plus derived visual buffers
- water keeps sparse depth, velocity, and flux at rest
- save/load and renderer boundaries still use dense materialization

What is next:

1. `WorldDefinition`
2. blank-world authoring
3. decoupled terrain resolution
4. chunk-window runtime processing
5. later DEM import
6. later authored hydrology
