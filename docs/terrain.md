# Terrain / World Terrain Spec

## Purpose

This document owns world extent, terrain storage, water storage, and the deterministic
implementation path from the current chunk-aware runtime to a real large-world authoring pipeline.

It answers questions like:

- what `WorldConfig` means today
- what terrain and water state is authoritative
- when dense buffers are still allowed
- how terrain and water render/upload boundaries must stay local as world size or terrain density
  increases
- what is already implemented
- what the next deterministic implementation slices must do

It does not own zoning legality, building placement rules, or multi-tier inactive-region
simulation behavior. Those remain owned by their respective docs.

## Document Conventions

Interpretation rules:

- Sections under `Implemented Runtime Contract` describe the live code unless explicitly marked as a
  compatibility gap.
- Sections under `Implemented Editor / Rendering Slices` describe shipped editor, renderer, and
  authored-world behavior.
- Sections under `Remaining Planned Deterministic Implementation` are intended next steps, not
  shipped behavior.
- `must` means required for the owning contract.
- `should` means intended unless a better measured implementation replaces it.
- `may` means allowed but optional.

Terminology:

- `world extent`: the authored width and height of the map
- `runtime terrain cell`: one live terrain sample in the current in-memory terrain grid
- `authored terrain chunk`: the canonical chunk span described by `WorldConfig`
- `source terrain`: the authoritative player- or importer-authored terrain surface
- `visual terrain`: the derived terrain surface after engineered-ground earthworks
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
- `visual terrain`: derived terrain after engineered-ground earthworks

Current deterministic rules:

- untouched sparse terrain cells are implicit and read back as the configured base elevation
- setting terrain directly writes both source and visual buffers
- sculpting writes both source and visual buffers
- `reset_visuals_from_source()` discards the current visual terrain and clones the authoritative
  source terrain into it
- save/load persists the authoritative source terrain only
- renderer uploads use the visual terrain
- procedural hillshade is a render-only derivation generated from the uploaded visual terrain
  heightmap
- procedural hillshade must never become authored world data or save-game data
- terrain sculpting, DEM import, world load, and road-earthwork visual refreshes must all update
  hillshade automatically because it derives from the same visual terrain upload

### 4. Surface Queries Distinguish Source Terrain From Visible World Surface

Terrain-only height queries are authoritative against the source terrain surface, not the
engineered-ground-derived visual terrain. Separate world-surface queries read the current
engineered-ground client surface first and fall back to visual terrain only when no client-owned
surface owns the queried location.

Current deterministic rules:

- `get_height(x, y)` reads source terrain
- `sample_height_world(x, z)` reads source terrain
- `intersect_terrain()` and terrain height queries use source terrain interpolation in world space
- `get_world_surface_height()` returns visible client-owned surface height when an engineered-ground
  client owns the queried XZ location, otherwise visual terrain height
- `intersect_world_surface()` raycasts the visible client-owned surface first and falls back to
  visual terrain when no client-owned triangle is hit
- engineered-ground earthworks are a visual derivation, not an edit to source terrain

Editor interaction rule:

- authored-ground editing tools must use the terrain-only query family
- visible-surface placement, inspection, and selection tools must use the visible-world query
  family
- terrain-authoring tools must not implicitly move already placed engineered-ground client surfaces

This preserves the rule that engineered-ground placement must not feed back into the terrain that
grade and slope calculations treat as authored ground.

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
- temporary compatibility scratch buffers inside water ticking
- undo snapshots

### 8. Engineered Ground Is A Chunk-Local Visual Derivation Step

Shared engineered-ground semantics now live in [`earthworks.md`](earthworks.md). This document owns
the terrain-storage side of that boundary: source terrain stays authored ground, visual terrain is
the derived buffer, and only touched chunks are reset and restamped.

Current deterministic sequence:

1. reset the touched visual terrain chunks from source terrain
2. compile or refresh the affected engineered-ground client surface inputs
3. rasterize footprint support plus deterministic outer earthwork-margin transitions from those
   inputs into the touched visual chunks
4. leave untouched visual chunks and all source terrain chunks unchanged
5. rebuild dependent caches against the updated client state plus visual terrain

Current runtime client state:

- roads are the first live engineered-ground client
- grounded roads no longer stamp ordinary `Standard` footprints or margins into visual terrain;
  road-touched terrain patches receive stitched mesh topology from Rust
- grounded road-owned asphalt, shoulder / curb, and sidewalk footprints also provide exact clip
  polygons to terrain and water render patches so neither terrain nor water remains a visible carrier
  under the committed road footprint
- grounded roads use Rust-generated stitched terrain patch topology from the clipped footprint edge
  to nearby terrain, so ordinary `Standard` roads do not need a visible closure strip
- terrain-only queries still read source terrain, while visible-world queries use the client-owned
  surface first, structural local earthwork geometry second, and visual terrain third; ordinary
  grounded seams are terrain topology, not a separate road-owned query surface
- future flat building pads and other engineered-ground clients should extend the same shared model
  from [`earthworks.md`](earthworks.md) instead of inventing a separate terrain-flattening path

Current deterministic editor rule:

- terrain authoring edits source terrain first
- after the source edit, touched engineered-ground clients rebuild derived terrain outputs; ordinary
  grounded roads regenerate CDT terrain patch meshes while structural clients may restamp visual
  terrain
- terrain brushes do not directly sculpt roadbeds, flat pads, or future local earthwork geometry

Remaining limitation:

- road-touched terrain patches now use the accepted Spade CDT seam representation, but live visual
  validation on varied authored maps is still required before the terrain / road integration can be
  treated as fully shipped
- terrain density alone is no longer the target fix for road / terrain gaps
- the live road-touched seam path is the Spade CDT patch builder described in
  [`roads.md`](roads.md):
  road footprint loops become hard constraints, terrain faces inside those loops are omitted, and
  road seam constraint edges are preserved exactly
- current grounded-road terrain editing must keep placed `Standard` road geometry fixed and rebuild
  derived terrain outputs around the committed roadbed instead of silently resynchronizing the road
  to later source-terrain edits

Authoritative rule:

- engineered-ground earthworks change the visual terrain only
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

### 10. Live Water Runtime Is Split Into Baseline Water And Dynamic Water

The live repository now distinguishes still-water ownership from flowing runtime water.

Current repository state:

- `Lake Fill` and `Open Water` rebuild into a baseline-water layer
- `Source` and `Sink` drive the dynamic water layer only
- baseline water stores flat still-water depth above terrain
- dynamic water stores additional depth above the local support surface plus:
  - velocity
  - flux
- support surface is:
  - terrain world height on dry land
  - baseline water surface elevation where baseline water exists
- rendering consumes a derived visible total depth from baseline plus dynamic water, not the old
  raw dynamic solver field by itself

Current first continuous-runtime rules:

- source/sink-driven dynamic water advances as a fixed-step runtime pass, not only during authored
  preview rebuilds
- the first shipped continuous runtime cadence is `5 Hz` (`dt = 0.2 s`) with internal dynamic
  substeps for stability
- gameplay dynamic water follows simulation speed and advances only while the gameplay clock is
  unpaused
- world-editor dynamic water may advance in real time while the authored operational clock remains
  paused
- each continuous runtime water step must set `water_dirty` so Godot refreshes the rendered water
  surface

This is the first correct ownership split, but the dynamic runtime is still compatibility-dense
inside its tick and is not yet the final large-world water runtime.

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
  - authored water records
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

### 13. Godot Bridge Uses Patch Snapshots For Terrain / Water Rendering

The live Godot render bridge now consumes chunk-local terrain / water patch snapshots instead of
whole-map render buffers.

Current deterministic rules:

- `terrain.gd` consumes `get_terrain_patch_layout()`, `get_dirty_terrain_patches()`,
  `get_terrain_patch()`, and `get_terrain_border_loop()`
- `water.gd` consumes `get_dirty_water_patches()`, `get_water_patch()`, and
  `get_water_border_depths()`
- terrain and water now keep patch identity stable while choosing a deterministic mesh-detail tier
  per resident patch from camera distance, so zoomed-out views do not pay near-field vertex
  density for every resident patch
- road-touched terrain patches switch from cached rectangular `PlaneMesh` topology to
  Rust-generated baked local `ArrayMesh` topology
- visible water patches always build depth-owned local `ArrayMesh` topology; dry cells emit no water
  mesh instead of relying on shader discard, and road-touched water patches suppress every water
  cell touched by the road footprint after a network edit instead of emitting partial transparent
  clip fragments
- the old whole-map terrain / water Godot render APIs were removed from the steady terrain / water
  bridge
- dense terrain or water materialization may still exist at save/load, undo, or other explicit
  compatibility boundaries, but it is no longer the gameplay or WorldEditor terrain / water render
  path

This is a rendering boundary, not an excuse for simulation systems to depend on dense storage.

## Implemented Compatibility Gaps We Should Not Extend

The following are live behaviors, but they are not the intended long-term ownership model.

### 1. Dense Scratch Buffers Still Exist In Hot Adjacent Systems

These are compatibility-only.

Current gap:

- terrain renderer upload still materializes one full dense visual-terrain buffer every refresh
- water tick materializes full dense water buffers
- render upload materializes full dense buffers every refresh

Required direction:

- these paths should later localize to chunk windows or active areas instead of whole-map buffers
- denser authored terrain must not be adopted by extending this whole-map upload path

### 2. Dynamic Water Runtime Is Still Dense Inside The Compatibility Boundary

Current gap:

- dynamic water still materializes dense depth, velocity, and flux scratch buffers inside each
  tick
- renderer upload still materializes dense combined water depth and dense dynamic velocity every
  refresh

Required direction:

- dynamic water should localize to active chunk windows or other bounded regions instead of
  whole-world dense buffers
- render upload should later stop refreshing full-water textures for every dynamic change

### 3. Dense Water Snapshots Are Still Treated As Authoritative Runtime Persistence

Current gap:

- city saves still persist dense water depth, velocity, and flux snapshots
- authored-world rebuild still depends on runtime-compatible water snapshots rather than a clean
  authored baseline plus dynamic overlay model

Required direction:

- this refactor may intentionally break existing saves and authored worlds
- no migration is required for the current dense water snapshot layout
- dense runtime water blobs should be removed from the long-term authoritative ownership model

### 4. Undo Is Not Yet Fully Authoritative For Terrain Ownership

Current repository state:

- undo snapshots currently capture visual terrain and water depth snapshots for compatibility with
  the pre-sparse mutation path

Required direction:

- terrain undo must become authoritative over source terrain and any derived visual state it
  invalidates
- world-authoring workflows must not depend on visual-only terrain snapshots

### 5. Terrain Height Storage Is Still Scaled At Query / Render Boundaries

Current gap:

- terrain samples are still multiplied by `HEIGHT_SCALE` when converted to world-space `y`
- `terrain_base_elevation_m` still enters raw sample storage before that scale is applied

Required direction:

- terrain base elevation and sample values should eventually become direct world-space metres
- no future authored-world format should assume the current scaled-height compatibility contract

### 6. `WorldDefinition` V1 Keeps The First Authored-Water Slice

Current state:

- a dedicated `--world-editor` launch mode and `WorldEditor` scene now exist
- world-editor UI now calls:
  - `create_blank_world`
  - `save_world_definition`
  - `load_world_definition`
- `WorldDefinition` now stores authored water records and rebuilds into baseline-water plus dynamic
  boundary-point state
- `WorldDefinition` still has no richer hydrology ownership beyond the first authored-water slice,
  no preview image, and no richer metadata

Remaining direction:

- the current authored-water slice is the shipped water-authoring baseline and does not require a
  separate richer hydrology system to be considered valid
- richer metadata and optional later water-authoring extensions may still happen, but only if the
  current `Source` / `Sink` / `Lake Fill` / `Open Water` workflow later proves insufficient

## Implemented Editor / Rendering Slices

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

### 2. WorldEditor V1 Terrain And Water Authoring Is Live

The current world editor uses the shared `SimulationNode` runtime but does not expose gameplay
simulation controls or gameplay HUD surfaces.

Current deterministic rules:

- world editor starts with the simulation thread available
- world editor does not expose pause / speed controls or gameplay HUD widgets
- world editor terrain authoring is live through:
  - `Raise`
  - `Lower`
  - `Level`
  - `Smooth`
  - `Slope`
- world editor water authoring is live through:
  - `Source`
  - `Sink`
  - `Lake Fill`
  - `Open Water`
- terrain brush picking uses `intersect_terrain()` and therefore targets authored source terrain,
  not the visible engineered surface
- `Raise`, `Lower`, `Level`, `Smooth`, and `Slope` write authoritative source terrain only
- completing a terrain brush stroke rebuilds touched engineered-ground clients and derived terrain
  outputs from the updated source terrain
- terrain brushes must not directly deform road top surfaces, future flat foundation pads, or
  future local earthwork meshes
- selecting `Raise`, `Lower`, `Level`, `Smooth`, or `Slope` opens a terrain brush submenu on the bottom toolbar
- that terrain brush submenu owns the shared editor `Diameter m` and `Strength` controls
- active terrain brushes show their footprint directly on the terrain so brush diameter is visible before and during sculpting
- `Level` captures the clicked source-terrain height at the start of the brush stroke and moves terrain toward that height while the stroke remains active
- `Smooth` moves terrain toward the local neighborhood average inside the brush footprint and is intended for relaxing jagged cuts, banks, and shorelines after carving
- `Slope` is a two-phase terrain brush:
  - first click captures the slope start anchor and its source-terrain height
  - second click captures the slope end anchor and its source-terrain height
  - after both anchors exist, brushing moves terrain toward the clamped linear grade between those two anchor heights
- `Slope` must not extrapolate beyond the two captured anchors; samples before the first anchor clamp to the start height and samples beyond the second anchor clamp to the end height
- `Lake Fill` and `Open Water` use a preview-first workflow:
  - first click seeds the preview
  - `Surface +m` adjusts the previewed surface
  - `OK` confirms
  - `Cancel` / `Esc` dismisses the preview without writing authored state
- world editor save/load is `WorldDefinition` only, not city-save persistence

Current compatibility gap:

- this is not yet the final terrain / water-only runtime boundary
- the shared runtime bundle still contains gameplay systems; world editor simply keeps gameplay
  controls and HUD surfaces absent
- after a terrain stroke, the current runtime still allows placed `Standard` roads to resync to
  edited source terrain; the intended long-term contract is to keep placed roads and future
  foundations fixed and reform terrain / earthworks around them instead

### 3. Baseline Water And Dynamic Water Split Is Now The Required Stable Model

This ownership split is now live and becomes the stable contract other systems should target.

Authoritative rule:

- still water and flowing water must no longer be represented as one shared runtime field

Required runtime split:

- `BaselineWaterSystem` owns hydrostatic authored water bodies
- `DynamicWaterSystem` owns transient flowing water on top of terrain or baseline water

Deterministic baseline-water rules:

- baseline water is authored-world state, not emergent solver output
- `Lake Fill` and `Open Water` belong to baseline water only
- each connected baseline water body owns one flat `surface_elevation_m`
- baseline water depth is always derived as:
  - `max(surface_elevation_m - terrain_world_y, 0.0)`
- baseline water does not own velocity or flux buffers
- baseline water is rebuilt from authored records and current terrain; it is not advanced by the
  dynamic shallow-water solver
- terrain edits that affect a baseline water body must recompute that baseline body immediately

Deterministic dynamic-water rules:

- dynamic water is runtime-only transient simulation state
- `Water Source` and `Water Sink` are dynamic-water boundary conditions, not baseline water bodies
- dynamic water may flow across dry terrain or on top of baseline water
- dynamic water owns:
  - transient additional depth above its local support surface
  - velocity / flux state needed by the dynamic solver
- the dynamic solver must not be responsible for making lakes or seas appear flat
- the first dynamic runtime may still be compatibility-dense internally, but only for the dynamic
  layer, not for authored still-water bodies

Deterministic support-surface rules:

- every water cell has one local support surface used by the dynamic solver
- for dry terrain, support surface is the terrain world height
- for baseline-water cells, support surface is the baseline water surface elevation
- dynamic water depth is measured above that support surface, not directly from bare terrain where
  baseline water already exists

Deterministic rendering rules:

- renderer must stop treating one raw runtime depth grid as the authoritative visible water surface
- renderer must consume:
  - baseline surface height
  - dynamic additional depth above the support surface
- visible still water must render as flat at the authored baseline surface elevation
- visible flowing or transient water may vary above that baseline, but it must not deform the whole
  lake or sea surface into solver stripes or cell ridges

Deterministic authored-water rules:

- `Water Source` is an authored dynamic boundary point with:
  - one world-space position
  - one authored inflow rate
- `Water Sink` is an authored dynamic boundary point with:
  - one world-space position
  - one authored outflow rate
- `Lake Fill` is an authored baseline-water record with:
  - one world-space seed position
  - one target water surface elevation
- `Open Water` is an authored baseline-water record with:
  - one world-space seed position
  - one target water surface elevation
- authored water records belong to `WorldDefinition`, not to renderer state or solver scratch
- `Lake Fill` and `Open Water` preview state remain transient editor runtime state and must never
  be serialized unless confirmed

Deterministic world-load rules:

- loading a `WorldDefinition` must rebuild baseline water from authored `Lake Fill` and
  `Open Water` records
- loading a `WorldDefinition` must also restore authored `Source` and `Sink` points as dynamic
  boundary conditions
- loading a world must not require running the dynamic shallow-water solver to reconstruct still
  lakes, coasts, or seas

Deterministic preview rules:

- `Lake Fill` preview must show a flat baseline surface preview, not a solver-generated depth field
- `Open Water` preview must show a flat edge-connected baseline surface preview, not a
  solver-generated depth field
- preview validity stays:
  - `Lake Fill` valid only if the filled region stays off the world edge
  - `Open Water` valid only if the filled region reaches the world edge
- confirming a valid preview writes authored baseline-water state only
- canceling the preview restores committed authored baseline water only

Deterministic persistence rules:

- this refactor may intentionally break existing city saves and existing `WorldDefinition` assets
- migration of the current dense water runtime blobs is not required
- the long-term authoritative format must not persist dense runtime water `depth`, `velocity`, and
  `flux` as the definition of still water

Legacy features removed:

- rebuilding `Lake Fill` or `Open Water` through the old shared runtime solver field
- using the dynamic shallow-water solver to keep authored lakes or seas visually flat
- treating one shared runtime depth field as the source of truth for both authored still water and
  dynamic flowing water

Legacy features still to remove:

- treating dense runtime water `depth`, `velocity`, and `flux` snapshots as the long-term water
  persistence model
- extending the current whole-world `5 Hz` continuous runtime pass as if it were the final water
  architecture

Deterministic non-goals of this refactor:

- no arbitrary freehand water-depth paint brush
- no automatic river extraction from imported DEMs
- no automatic hydrology extraction from hydrography vectors or rasters
- no full river-channel authoring tool yet

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

### 5. Chunk-Local Terrain / Water Rendering Is Now Live

Now that `terrain_cell_m` exists, the live render boundary is chunk-local instead of whole-world.

Authoritative rule:

- whole-map dense mesh and texture refresh is no longer the steady terrain / water render path
- it must not be reintroduced to justify denser authored terrain, larger worlds, or local
  engineered-ground geometry

Deterministic terrain-render rules:

- visible terrain now renders as chunk-local terrain patches rather than one whole-world plane
- terrain render patches follow authored terrain chunk boundaries by default
- if a different terrain render patch span is used later, it must be:
  - derived from `terrain_chunk_m`
  - a fixed integer multiple of `terrain_chunk_m`
  - stable in code rather than camera-dependent
- each terrain render patch owns local GPU resources for that patch only
- rebuilding or uploading one patch may read only:
  - the local visual-terrain window for that patch
  - one fixed border sample ring if needed for interpolation, normals, or shading continuity
- unchanged patches must keep their existing GPU resources
- camera motion alone must not rebuild or reupload already resident unchanged patch textures
- camera motion may change the mesh-detail tier of an already resident patch, but that change must
  reuse the resident patch snapshot and must not fall back to whole-map terrain or water uploads
- only dirty patches and newly required visible patches may rebuild or upload
- the visible terrain patch set must be derived from the camera or editor interest region plus one
  fixed padding margin to avoid pop-in
- road earthworks and future engineered-ground clients must continue to invalidate only touched
  terrain chunks; the renderer must reflect that locality instead of reintroducing full-world
  uploads

Deterministic water-render rules:

- visible water no longer depends on one whole-world depth texture refresh for every change
- water rendering consumes chunk-local bounded window snapshots aligned to the same fixed terrain
  render patch grid
- `WATER-01` may still keep dynamic water simulation compatibility-dense internally while that work
  remains in progress, but once this render split lands the water renderer must not require
  whole-map texture refreshes just to display local changes

Deterministic Godot-bridge rules:

- the primary terrain/water render path now uses chunk-local snapshot APIs
- whole-map Godot render APIs such as `get_heightmap_data()`, `get_water_data()`, and
  `get_water_velocity_data()` were removed from the steady-state terrain / water render bridge
- any future dense helper kept for compatibility, debug tooling, or offline export must stay
  outside the steady-state gameplay and WorldEditor render path
- `road` water diagnostics must report authored baseline depth, dynamic source/sink
  depth, and composed visible depth separately so dark water-patch regressions identify the owning
  water layer instead of only the final rendered sum
- when a road-touched water patch contains authored baseline water, `road` diagnostics
  must also list the committed `Lake Fill` / `Open Water` records or active preview that actually
  contributed non-zero samples inside that patch
- chunk-local snapshot APIs expose enough metadata for deterministic reconstruction of one
  patch window:
  - patch identity
  - local dimensions
  - world origin
  - `terrain_cell_m`
  - packed sample payloads for that window

Deterministic overlay-separation rule:

- zoning, parcel, or other editor overlays may keep their own representation if that remains cheap
- those overlays must not force terrain or water back onto one whole-world mesh or one whole-world
  texture upload path

Deterministic density gate:

- a default authored-terrain move from `10 m` to `5 m` or finer must not happen before this
  chunk-local terrain / water render split is live
- the density decision must now be re-measured on the split path using the same world-space test
  cases for:
  - system RAM
  - GPU VRAM
  - terrain upload cost
  - water upload cost
  - terrain brush cost
  - earthwork restamp cost
- the accepted road / terrain seam fix is not a density move; it is the Spade CDT terrain-patch
  hardcut in [`roads.md`](roads.md)

Deterministic transition rules:

- future engineered-ground closed local earthwork / tie-in geometry is required whether the
  implementation extends the current terrain runtime or rewrites it
- any terrain-runtime rewrite must still preserve:
  - authoritative source terrain
  - a derived far-field terrain surface outside engineered-ground tie-in boundaries
  - chunk-local invalidation and rebuild boundaries
  - the split terrain / water render-upload path
  - Rust-generated stitched terrain topology anywhere grounded `Standard` road top surfaces own the
    visible surface; the clip boundary is the compiled road-piece outer loop, and shader discard,
    alpha masking, internal road-band clipping, or Godot-side polygon clipping must not be the
    ordinary road seam carrier
  - terrain clipping must be derived from the compiled road-piece outer loop in Rust; asphalt /
    sidewalk render triangles are not reused as terrain ownership triangles
  - road-piece and terrain-patch ownership cleanup uses `i_overlay` before triangulation; boolean
    union / difference / hole handling must produce non-overlapping asphalt, sidewalk, and terrain
    regions before Spade receives constraints
  - target road-touched patch generation uses Spade's Rust-side
    `ConstrainedDelaunayTriangulation` with a deterministic `try_bulk_load_cdt` input made from the
    terrain patch rectangle, road-owned footprint constraint loops, and deterministic
    source-terrain sample points outside those footprints
  - conflicting constraints are reported through CDT debug counters and skipped; they are treated
    as geometry bugs to fix at the road-piece source, not as a reason to panic the backend or fall
    back to legacy clipping
  - Spade CDT faces whose centroids are inside road-owned footprints are omitted; all emitted
    terrain triangles must preserve road seam constraint edges and must not cross road footprint
    loops
  - `ghx_constrained_delaunay` is not a terrain backend or fallback in this spec; Spade is the
    production hard-cut target because it gives the project a documented constrained triangulation
    API with exact geometric predicates
  - `robust` is not part of this path for now; standalone exact-predicate code is not needed unless
    a future measured gap remains after `i_overlay` boolean cleanup and Spade CDT
  - current road-touched patch emission uses the Spade CDT path directly; the old subtractive
    triangle cutter, visible seam strip, and conservative cell-triangle ownership rule are no
    longer live fallbacks
  - terrain render suppression for structural local earthwork geometry must remain bounded to true
    geometry overlap rather than acting as a substitute for missing tie-in faces; road-edge terrain
    topology must still be geometrically correct if terrain-side suppression is turned off
  - visible-world query precedence over client-owned top surfaces and closed local earthwork
    geometry
- post-placement terrain edits should rebuild earthworks around already placed client surfaces
  instead of resynchronizing those client surfaces to edited terrain

The chunk-local render path is now the live large-world terrain / water runtime boundary.

### 6. Offline Heightmap / DEM Import Is Live

Authoritative rule:

- imported terrain writes authoritative source terrain only
- visual terrain is always derived from source terrain plus later structural road or water
  derivations; ordinary grounded road seams are generated as CDT patch meshes

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

## Remaining Planned Deterministic Implementation

The remaining slices below are still intended next steps.

### 7. Optional Future: Richer Water Authoring

The current authored-water model is sufficient as the shipped baseline:

- `Source`
- `Sink`
- `Lake Fill`
- `Open Water`

We do not currently require a separate richer hydrology layer beyond that baseline.

If richer water authoring is ever added later, the rules should remain:

- any richer river/channel ownership stays separate from raw terrain elevation
- richer water authoring must not be stored as "just paint some water depth into the live runtime
  buffer"
- imported hydrography may be used as an editor reference, but not as implicit gameplay water state
- imported DEMs and imported hydrography remain separate inputs; one does not silently create the
  other
- the current `Source` / `Sink` / `Lake Fill` / `Open Water` workflow remains valid even if no
  richer river-path tooling is ever added

### 8. Use A Hybrid Water Model For Very Large Worlds

The intended large-world water model is hybrid.

Deterministic rules:

- the current authored-water baseline defines still water and boundary conditions for the whole
  world
- local active areas may run dynamic water simulation
- the engine must not require a full-world dense shallow-water solve for every map size

### 9. Keep The First Terrain / Water Realism Pass Render-Only

Terrain and water should look more natural, but the first realism pass must stay strictly on the
render side.

Deterministic rules:

- the first terrain/water realism pass must work in both gameplay and WorldEditor from the same
  live terrain and water buffers
- the first realism pass must not require external base textures
- the first realism pass must not add authored-material data to `WorldDefinition`
- the first realism pass must not add visual-material data to city saves
- realism tuning should live in renderer/shader parameters, not in simulation state
- terrain realism may derive from:
  - world elevation
  - slope / local normal
  - procedural hillshade
  - shoreline proximity
  - low-frequency procedural color breakup
- water realism may derive from:
  - water depth
  - water velocity
  - shoreline proximity
  - view-angle / fresnel-like response
  - small procedural surface breakup
- coastline smoothing may be handled visually with soft shoreline masking, shoreline foam bands,
  or similar shader-side treatment
- on coarse authored grids such as `10 m`, shoreline improvement should prefer sub-texel render
  coverage / contour-style masking before increasing `terrain_cell_m` density
- these visual passes must never change:
  - source terrain
  - visual terrain ownership rules
  - authored water records
  - runtime water depth / velocity / flux state
  - save schema

Deterministic v1 realism scope:

- terrain should move away from one flat height-color ramp toward:
  - slope-aware rock versus soil/vegetation weighting
  - stronger relief readability from hillshade
  - shoreline color transitions
  - subtle macro variation so large areas are not one uniform tint
- water should move away from one flat translucent surface toward:
  - shallow-versus-deep color separation
  - stronger coast readability
  - view-angle-dependent reflectance / specular response
  - mild procedural breakup so calm water does not look perfectly flat
- the first realism pass must stay cheap enough to share the existing terrain/water renderer path
  between gameplay and WorldEditor

Deterministic terrain color ownership after the first realism pass:

- the long-term terrain color model must be surface-classification-first and absolute-height-second
- absolute world elevation may still influence the palette, but it must not be the primary terrain
  material classifier
- the primary terrain color inputs should be derived from the live visible terrain field:
  - slope / local normal
  - local relief / ruggedness
  - shoreline or visible-water proximity
  - narrow shore-transition cues derived from visible water, not only from `0 m`
  - macro variation / large-scale breakup
- absolute world elevation should be treated as a secondary modifier that nudges an already chosen
  surface class rather than choosing the surface class by itself
- default dry terrain must read as normal inland ground / forest floor, not as marsh, shoreline,
  or tidal flats just because it is low or near one of many lakes
- shoreline influence should remain a relatively narrow visual transition near visible water, not a
  dominant lowland terrain class
- imported DEM worlds and hand-authored blank worlds must share the same terrain-color ownership
  model; terrain color must not assume a specific real-world altitude band such as Kuopio's
  imported range
- the renderer may still use absolute elevation to bias:
  - colder / harsher upland tones
  - alpine / snow transition
  - gentle broad lowland hue shifts where appropriate
  but those biases must remain weaker than the primary surface-classification cues
- the terrain-color model must remain render-only and must not introduce authored material records,
  biome records, or save-schema changes
- the terrain-color model must remain cheap enough to share the same terrain renderer path between
  gameplay and WorldEditor

Deterministic shoreline contour rendering slice:

- on coarse authored grids such as `10 m`, the preferred next shoreline-quality step is contour
  extraction from the live visible water field, not additional blur radius and not a denser
  authored map by default
- shoreline contour extraction must remain render-only and must never become authored world state
- the extraction input must be the same composed visible water depth used by the water renderer:
  baseline-water plus dynamic-water composition after terrain alignment
- shoreline extraction must use a waterline threshold at the visible shoreline boundary, not a
  post-hoc artistic painted mask
- the extraction algorithm may be marching squares or an equivalent contour/isoband method, but
  the output contract is:
  - smoother diagonal coastlines than raw cell edges
  - smoother narrow channel shorelines than raw cell edges
  - no visible whole-cell stair stepping as the primary shoreline shape in common camera views
  - shoreline position must stay tied to the live water field, not drift arbitrarily for style
- the renderer may realize that contour result as either:
  - a dedicated shoreline mesh
  - a higher-resolution shoreline mask or distance field
  - another equivalent render-only contour representation
- whichever render representation is chosen, it must update whenever terrain or visible water is
  refreshed in gameplay or WorldEditor
- the shoreline contour slice must not require any save-schema change, `WorldDefinition` schema
  change, or additional authored shoreline records
- sub-texel shoreline coverage smoothing remains an allowed fallback/interim treatment, but it is
  not the long-term primary shoreline-quality solution on coarse grids
- shoreline contour rendering may improve the visible water edge only; it does not create
  sub-cell terrain-bank geometry and must not pretend to solve blocky terrain cuts by itself

Deterministic cliff breakline / cliff band rendering slice:

- on coarse authored grids such as `10 m`, the preferred next cliff-quality step is render-only
  cliff extraction from the live terrain field, not manual authored cliff painting and not a
  denser authored map by default
- cliff rendering must remain render-only and must never become authored world state
- the extraction input must be the same live visible terrain field used by terrain rendering after
  terrain refresh, plus its derived slope / local-normal information
- cliff detection must classify steep terrain from the live terrain field, not from a post-hoc
  artist-painted mask
- the extraction algorithm may use slope thresholds, hysteresis, marching squares, contour
  extraction, or another equivalent method, but the output contract is:
  - one upper cliff breakline tied to the visible terrain top edge
  - one lower cliff breakline tied to the visible terrain toe / bottom edge
  - one rendered cliff-face band or equivalent representation between those two lines
  - smoother diagonal cliff edges than raw cell silhouettes in common camera views
  - visibly stronger cliff readability than the current raw terrain mesh alone
  - no arbitrary drift away from the live terrain field for style
- the renderer may realize that cliff result as either:
  - a dedicated cliff ribbon / band mesh
  - a higher-resolution cliff mask or distance field
  - another equivalent render-only breakline representation
- whichever render representation is chosen, it must update whenever visible terrain is refreshed
  in gameplay or WorldEditor
- the cliff rendering slice must not require any save-schema change, `WorldDefinition` schema
  change, or additional authored cliff records
- cliff rendering may darken or re-shade the cliff face, but that shading must remain derived from
  the live terrain field and must not introduce authored material ownership
- cliff rendering improves visible cliff readability only; it does not create sub-cell terrain
  geometry and must not pretend to solve coarse side silhouettes by itself

Deterministic terrain-border skirt rendering slice:

- the preferred map-edge treatment is a render-only terrain-border skirt derived from the live
  terrain edge, not authored border geometry and not simulation-owned world walls
- terrain-border skirt rendering must remain render-only and must never become authored world
  state
- the skirt input must be the same live visible terrain field used by terrain rendering after
  terrain refresh
- the skirt must be built from the outer terrain edge of the current world and extruded downward
  to one fixed render-only depth
- the terrain-border skirt output contract is:
  - the map edge reads as a visible cut through the terrain instead of a paper-thin plane
  - contour lines or equivalent elevation bands visibly continue down the side surface
  - the skirt stays aligned to the live terrain edge and updates whenever terrain visuals refresh
  - the top terrain surface remains the authoritative playable surface; the skirt is presentation
    only
- the renderer may realize that skirt result as either:
  - one dedicated side-wall mesh plus one bottom cap
  - one equivalent render-only border representation
- whichever render representation is chosen, it must update whenever visible terrain is refreshed
  in gameplay or WorldEditor
- the terrain-border skirt slice must not require any save-schema change, `WorldDefinition` schema
  change, or additional authored border records
- the skirt material may use contour lines, sediment-style depth banding, or other earth-layer
  cues, but those cues must remain render-only and derived from the live terrain field
- the terrain-border skirt may improve the perceived thickness and readability of the map edge
  only; it must not introduce playable vertical terrain, collision ownership, or simulation-owned
  edge walls

Deterministic asset policy:

- external terrain/water material textures are optional later enhancements, not a prerequisite for
  the first realism pass
- if material textures are added later, they remain visual assets only
- authored worlds and imported DEM worlds must remain valid and usable even when no external
  material textures are present

Explicit non-goals of the first realism pass:

- authored material painting
- biome simulation
- erosion or sediment simulation for visual purposes
- dependency on downloaded hillshade rasters
- dependency on scanned PBR material libraries before terrain/water can look acceptable
- increasing `terrain_cell_m` density solely to get a smoother visible shoreline before contour
  rendering has been attempted
- increasing `terrain_cell_m` density solely to get smoother visible cliff edges before
  breakline/band rendering has been attempted
- adding authored border-wall geometry or persistence merely to make the map edge look thicker
- using one absolute-height ramp as the long-term primary terrain material classifier

## Current Deterministic Non-Goals

The following are explicitly not implemented yet and should not be assumed by other systems:

- interactive WorldEditor DEM / GeoTIFF import UI
- optional richer river-path / channel authoring beyond the current water tool set
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
- first authored-water tools are live in WorldEditor through `Source`, `Sink`, `Lake Fill`, and
  `Open Water`
- `WorldDefinition` now persists authored water boundary points, inland lake fills, and
  edge-connected open-water fills
- live water now splits authored baseline still water from dynamic flowing water
- save/load and renderer boundaries still use dense materialization
- terrain rendering now derives hillshade procedurally from the live heightmap in both gameplay
  and WorldEditor; it is not stored as separate world data
- terrain and water rendering now use a first render-only realism pass with slope-aware terrain
  shading, shoreline-aware terrain tinting, depth-aware water color, and shader-side shoreline /
  fresnel / procedural breakup treatment, including contour-style shoreline rendering on the
  existing `10 m` grid from the live visible water field plus render-only cliff breakline / cliff
  band treatment from the live terrain field
- terrain coloring now uses surface classification first and absolute height second, so flat blank
  worlds and imported DEM worlds share the same inland-first palette model instead of depending
  mainly on one absolute-height ramp
- terrain-border skirt rendering now adds a render-only side wall plus bottom cap derived from the
  live terrain edge, with contour continuation down the side surface so maps read as a visible cut
  through terrain instead of a paper-thin top plane
- water rendering now also adds a render-only edge curtain where visible water reaches the map
  boundary, so outside-of-map views do not expose the submerged terrain plane through the
  transparent water surface

What is next:

1. interactive DEM / GeoTIFF import UI for real-map authored worlds
2. dedicated shoreline mesh or distance-field rendering if the current contour-style shoreline
   field still is not enough for close camera work
3. chunk-window runtime processing
4. later texture-assisted terrain/water materials if the shader-only realism pass is not enough

Optional later only:

- richer river-path / channel authoring if the current water-tool workflow later proves
  insufficient for map making
