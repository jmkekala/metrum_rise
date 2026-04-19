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

### 4. `WorldDefinition` V1 Is Terrain-Only And WorldEditor V1 Is Still Incomplete

Current state:

- a dedicated `--world-editor` launch mode and `WorldEditor` scene now exist
- world-editor UI now calls:
  - `create_blank_world`
  - `save_world_definition`
  - `load_world_definition`
- `WorldDefinition` v1 still stores terrain only; it has no hydrology, preview image, or richer
  metadata

Remaining direction:

- authored-world assets still need richer metadata and later hydrology ownership

## Planned Deterministic Implementation

The next slices should be implemented in this order.

### 1. Blank-World WorldEditor V1 Is Live

The first dedicated authored-world shell now exists around `WorldDefinition`.

Current deterministic rules:

- `--world-editor` routes to a dedicated `WorldEditor` scene
- world-editor UI is separate from gameplay save/load UI
- the world-editor top menu exposes:
  - `New World`
  - `Open World`
  - `Save`
  - `Save As`
  - `Quit`
- the world-editor bottom toolbar is the primary authoring surface
- a newly created blank world is dry and flat except for the authored base elevation

### 2. WorldEditor V1 Is Terrain-Only And Launches Paused

The current world editor uses the shared `SimulationNode` runtime but does not expose gameplay
simulation controls.

Current deterministic rules:

- world editor starts with the simulation thread available
- world editor does not expose pause / speed controls or gameplay HUD widgets
- world editor terrain authoring is live through:
  - `Raise`
  - `Lower`
- world editor save/load is `WorldDefinition` only, not city-save persistence

Current compatibility gap:

- this is not yet the final terrain / water-only runtime boundary
- the shared runtime bundle still contains gameplay systems; world editor simply leaves the
  simulation paused for terrain-only v1

### 3. The First Authored-Water Slice Is Live

WorldEditor no longer stops at terrain-only authoring.

Current deterministic rules:

- water authoring lives on the existing world-editor bottom toolbar surface
- authored water ownership remains separate from gameplay save/load UI
- the first implemented authored-water tools are:
  - `Water Source`
  - `Water Sink`
  - `Lake Fill`
- authored water records are saved in and loaded from `WorldDefinition`
- loading a `WorldDefinition` rebuilds runtime water preview from those authored records

Deterministic first-slice scope:

- the first authored-water slice must add only:
  - `Water Source`
  - `Water Sink`
  - `Lake Fill`
- this first slice must not start with freehand water-depth painting
- this first slice must not require river-path drawing in order to be useful on imported or
  hand-sculpted maps

Deterministic ownership rules:

- `Water Source` is an authored inflow point with a world-space position and authored inflow rate
- `Water Sink` is an authored outflow point with a world-space position and authored outflow rate
- `Lake Fill` is an authored basin-fill record with:
  - one world-space seed position
  - one target water surface elevation
- authored water records belong to `WorldDefinition`, not only to live city saves
- authored water records must be engine-owned data types; they must not store raw runtime scratch
  buffers or renderer-only state

Deterministic runtime-application rules:

- loading a `WorldDefinition` must rebuild the runtime water state from authored water records
- point sources and point sinks become live runtime water boundary conditions for that world
- lake fills seed the initial water state for the contiguous basin that contains the authored seed
  position and lies below the authored surface elevation
- the authored water layer is the baseline; later runtime simulation may modify local details on
  top of it

Current compatibility behavior:

- the current first slice rebuilds a preview water state from authored records immediately after
  each authored-water edit
- this preview rebake is not yet the final large-world dynamic water runtime
- authored sources and sinks are persisted separately even though the current preview solve uses the
  shared runtime water system underneath

Deterministic non-goals of the first water slice:

- no arbitrary "paint some water depth into the map" authoring mode
- no automatic river extraction from imported DEMs
- no automatic hydrology extraction from hydrography vectors or rasters
- no full river-channel authoring tool yet
- no requirement that the whole world run one dense shallow-water solve at all times

### 4. Gameplay `New Game` Now Loads A Selected `WorldDefinition`

Gameplay now has a first-pass authored-world handoff.

Current deterministic rules:

- gameplay `File -> New Game` opens a world picker rooted at `user://worlds/`
- selecting one `WorldDefinition` loads it into the live gameplay scene
- gameplay world load reuses the same scene refresh path as save-load:
  - terrain rebuild
  - water rebuild
  - network mesh refresh
  - building renderer refresh
  - zoning overlay refresh
  - agent renderer refresh
- gameplay pauses immediately after loading the selected world

Remaining direction:

- `New Game` still loads directly into gameplay rather than through a richer front-end menu flow
- city saves must remain runtime snapshots layered on top of that authored world baseline

### 5. Move From Whole-Map Dense Compatibility Buffers To Chunk Windows

Now that `terrain_cell_m` exists, dense scratch buffers must be localized.

Required direction:

- road flattening should operate on touched chunk windows, not full-world dense terrain
- water simulation should operate on active chunk windows or other bounded regions, not full-world
  dense arrays
- renderer upload should be able to refresh only the chunks the camera or editor currently needs

Deterministic rule:

- whole-map dense materialization is a temporary compatibility path, not the target large-world
  runtime

### 6. Offline Heightmap / DEM Import Is Live

Authoritative rule:

- imported terrain writes authoritative source terrain only
- visual terrain is always derived from source terrain plus later road or water derivations

Current implemented slice:

- real-map terrain import now exists as an offline editor-time tool:
  - `tools/import_dem_world_definition.py`
- the importer writes a normal `WorldDefinition` SQLite asset directly
- the first validated source case is the National Land Survey of Finland Kuopio `324 km²`
  `Korkeusmalli 2 m` tile batch under:
  - `maps/raw/Kuopio/324km2/`
- the default generated authored world is:
  - `maps/processed/Kuopio/kuopio_324km2_10m.sqlite`

Current source format rules:

- v1 import accepts raster DEM/DTM data in single-band `GeoTIFF`
- the first concrete target source class is tiled National Land Survey of Finland elevation data
  such as `Korkeusmalli 2 m`:
  - single-band `Float32`
  - projected horizontal CRS
  - explicit pixel size in metres
  - explicit `NoData` value
- v1 does not accept:
  - hillshade rasters
  - hydrography rasters
  - RGB orthoimagery
  - arbitrary grayscale PNG/JPEG images
  - mixed DEM/DSM source batches

Current ownership rules:

- DEM import is world-editor only
- the current importer creates a new `WorldDefinition`; it does not merge into an already edited
  world
- imported terrain becomes the new authoritative source terrain for that world extent
- runtime coordinates remain centred world-local metres after import; source georeferencing is not
  preserved as gameplay-space coordinates
- import provenance may be stored as non-authoritative metadata, but gameplay must not depend on it
- because the live runtime still multiplies terrain samples by `HEIGHT_SCALE` at render/query
  boundaries, the importer currently converts DEM elevation metres into that pre-scaled runtime
  sample space before writing source terrain

Deterministic validation rules:

- every selected source file must be readable and must expose georeferencing metadata
- all selected tiles in one import batch must share:
  - the same projected horizontal CRS
  - the same pixel size
  - the same sample type
  - the same north-up axis orientation
- v1 must reject source rasters with rotation / skew terms
- v1 must reject missing or malformed `NoData` metadata
- v1 must reject overlapping tiles that do not align exactly on pixel boundaries
- v1 must reject any selected crop extent that contains `NoData` inside the requested world area
- v1 must reject target `terrain_cell_m` values finer than the source raster pixel size; the
  importer must not invent terrain detail by upsampling to a finer authored resolution

Current deterministic import sequence:

1. select one or more `GeoTIFF` DEM tiles
2. validate source metadata and pixel-grid compatibility
3. mosaic the source tiles into one temporary import raster in source CRS
4. choose a rectangular import extent inside that mosaic
5. create a new `WorldConfig` for the imported world:
   - `width_m` and `height_m` come directly from the chosen import extent
   - `terrain_cell_m` is user-selected but must be `>=` source pixel size
   - `terrain_chunk_m` follows the normal authored-world chunk rules
   - `terrain_base_elevation_m` stays an authored-world default only; it must not be used to
     reinterpret imported heights
6. resample the import raster into the authored terrain grid in canonical world metres
7. write the resampled values into authoritative source terrain
8. reset visual terrain from source terrain
9. save the result as a normal `WorldDefinition`

Current deterministic resampling rules:

- resampling happens at terrain sample positions, not cell centres of a separate import-only grid
- v1 uses bilinear interpolation from source DEM values
- border-only nodata introduced by edge-aligned resampling is clamped from the nearest interior
  valid sample; any remaining nodata after that is a hard rejection
- source elevation values are numerically preserved apart from that resampling step; v1 does not
  apply erosion, exaggeration, or artistic normalization during import
- vertical datum conversion is out of scope for v1; import assumes the source height values are
  already the desired authored elevations

Allowed compatibility boundary:

- DEM import may materialize one dense temporary mosaic and one dense target raster because import
  is an offline editor operation, not a hot simulation path

Explicit non-goals of v1 DEM import:

- automatic extraction of rivers, lakes, or hydrology from the DEM
- automatic terrain texturing or biome painting
- importing hydrography vectors/raster as water gameplay state
- preserving external CRS coordinates as live gameplay-space coordinates
- patch-importing one DEM over part of an already edited authored world

Remaining direction:

- integrate DEM import into the WorldEditor UI instead of keeping it as an offline tool only
- remove the importer's current dependency on the pre-`HEIGHT_SCALE` runtime compatibility layer

### 7. Add Authored Hydrology As A Separate Layer

Hydrology must be authored separately from raw terrain elevation.

The first authored-hydrology data set should include:

- inflows / springs
- outflows / sinks
- lake basins or lake surface levels

Later authored hydrology may extend that with:

- river paths or channels

Deterministic rules:

- a blank world with no hydrology remains dry
- authored hydrology defines where water belongs before runtime simulation modifies local details
- hydrology must not be stored as "just paint some water depth into the live runtime buffer"
- imported hydrography may be used as an editor reference, but not as implicit gameplay water state
- imported DEMs and imported hydrography remain separate inputs; one does not silently create the
  other

Deterministic imported-map guidance:

- on real-map worlds, authors should place only the major inflows, major outflows, and major lake
  fills first
- the first authored-hydrology slice must be usable without an automatic river-generation pass
- this allows imported DEM worlds to become playable water worlds before river-path tooling exists

### 8. Use A Hybrid Water Model For Very Large Worlds

The intended large-world water model is hybrid.

Deterministic rules:

- authored hydrology defines the baseline rivers, lakes, inflows, and outflows for the whole world
- local active areas may run dynamic water simulation
- the engine must not require a full-world dense shallow-water solve for every map size

## Current Deterministic Non-Goals

The following are explicitly not implemented yet and should not be assumed by other systems:

- interactive WorldEditor DEM / GeoTIFF import UI
- river-path hydrology authoring
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
- offline DEM import can now generate a normal `WorldDefinition` from real GeoTIFF elevation tiles
- first authored-water tools are live in WorldEditor through `Source`, `Sink`, and `Lake Fill`
- `WorldDefinition` now persists authored water boundary points and lake fills
- save/load and renderer boundaries still use dense materialization

What is next:

1. interactive DEM / GeoTIFF import UI for real-map authored worlds
2. river-path hydrology after terrain import and first authored-water tools are stable
3. chunk-window runtime processing
6. later DEM import
7. later authored hydrology
