# Metrum Rise — Project Dashboard

This file is the live dashboard for current state, priorities, and links to the owning docs. It is intentionally summary-first.

The old monolithic ledger and numbered backlog are archived in [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md). That archive is historical reference only, not the current planning source.

## Snapshot

- **Scale target**: at least 1,000,000 total population across simulation tiers in one large world.
- **Simulation model**: full-FSM simulation stays inside the active area of interest; distant world regions are expected to degrade to coarser flow-field or aggregate simulation.
- **Current focus**: keep the playable small-to-medium city slice correct, deterministic, and scalable while the docs/planning cleanup continues and the new baseline-water / dynamic-water split is hardened.

## Shipped Foundations

- **Road network and routing**: modular `RegionGraph`, lane system, CCH pathfinding, road rendering, border nodes, and roadway editing are all live.
- **Zoning and building allocation**: world-space zoning grid, occupancy tracking, roadside building placement, vacancy indexing, and no-build edge flags are live. See [`zoning.md`](zoning.md) and [`building_allocator.md`](building_allocator.md).
- **Entrance-aware movement**: the building entrance/exit rewrite is implemented through the exact-plan system described in [`entrance_and_exit.md`](entrance_and_exit.md), including the Phase 1–6 and Phase 8 slices already verified against the live code.
- **Benchmark coverage**: the Criterion suite now measures the live access phases through `ACCESS_EGRESS` and `ACCESS_INGRESS` in addition to pure `NETWORK` and idle scaling. Treat comparisons against older benchmark runs as a fresh baseline unless the benchmark shape is identical.
- **Economy foundation**: household records, building-centric daily economy, freight jobs, `OWA` fallback, exact entrance-side freight ETA, unemployment benefit disbursement, and two-day building bankruptcy are all live. See [`economy.md`](economy.md).
- **Demand foundation**: the live `DemandSystem` now fully owns immigration and building growth pressure through a strictly organic model. The pioneer demand floor has been removed entirely — unemployment benefit provides early-city solvency instead. Industrial demand is now driven by `commercial_input_deficit` rather than `goods_shortage`. See [`demand.md`](demand.md).
- **Persistence and runtime**: SQLite save/load, background simulation thread, render snapshots, debug flags, asset editor, and economy editor are live.

## Current Priorities

For active tracked work, use [`roadmap.md`](roadmap.md).

- `QA-01`: revalidate and root-cause the old long-run sim-thread panic.
- `CIV-01`: add service-building coverage so city stability is not only conceptual.
- `WATER-01`: harden the new baseline-water / dynamic-water split and remove the remaining dense compatibility boundaries.
- `MOB-01`: ship bicycle support as the next transport mode.
- `ALLOC-01`: harden building allocator ownership and spec limits.
- `DOC-01`: finish replacing old numbered backlog references in live docs.

`QA-01` is now parked in [`roadmap.md`](roadmap.md): the old long-run sim-thread panic has not reproduced recently, including at least one overnight run, so it is no longer treated as an active blocker.

## System Ownership

| Area                                                        | Owning doc                                             |
| -------------------------------------------------------------| --------------------------------------------------------|
| Current status / priorities                                 | [`project.md`](project.md), [`roadmap.md`](roadmap.md) |
| Stable constants / bridge API / formats                     | [`reference.md`](reference.md)                         |
| Entrance / exit / trip attachment                           | [`entrance_and_exit.md`](entrance_and_exit.md)         |
| Economy / freight / household runtime                       | [`economy.md`](economy.md)                             |
| Demand / city-growth pressure / admission-removal ownership | [`demand.md`](demand.md)                               |
| Zoning                                                      | [`zoning.md`](zoning.md)                               |
| Terrain ingest / chunked terrain runtime / world terrain    | [`terrain.md`](terrain.md)                             |
| Building placement / removal / frontage attachment          | [`building_allocator.md`](building_allocator.md)       |
| Gameplay HUD / menus / floating windows                     | [`ui.md`](ui.md)                                       |
| Asset-editor workflow and pack contract                     | [`asset_editor.md`](asset_editor.md)                   |
| Road-renderer notes                                         | [`improved_roads.md`](improved_roads.md)               |

## Recent Structural Changes

- Transitioned the residential simulation to a **household-centric occupancy model**, replacing legacy per-resident capacity with family slots (`household_capacity`).
- Added `flat_size_m2` to building assets to control household compatibility.
- Enforced authoritative `worker_capacity` derivation from Economy Profiles, removing redundant asset-level overrides for businesses.
- Updated the Inspector UI to display both Household occupancy and total Agent counts.
- `project.md` was reduced from a monolithic implementation ledger into this dashboard.
- `roadmap.md` now owns active tracked work through stable IDs instead of positional numbering.
- `README.md` now serves as the docs index and ownership map.
- Added [`terrain.md`](terrain.md) as the owning spec for GeoTIFF terrain ingest, chunked terrain
  runtime, and large-world terrain rules.
- Added the first terrain chunk importer slice: `tools/build_terrain_chunks.py` now reads the
  Kuopio world manifest and exports internal `512 m` chunk assets with raw `f32` height payloads
  plus `2 m / 4 m / 8 m / 32 m` LOD files. See [`terrain.md`](terrain.md).
- Added the first Rust-side terrain chunk loader in `rust/src/simulation/terrain/chunks.rs`,
  including strict `chunk.toml` validation and `.f32` payload loading for partial border chunks as
  well as full-size interior chunks. See [`terrain.md`](terrain.md).
- The legacy numbered backlog and bug table were preserved in the archive rather than kept half-live in the dashboard.
- `rust/benches/agent_benchmark.rs` now includes access-phase microbenchmarks for `ACCESS_EGRESS` and `ACCESS_INGRESS`, so old Criterion result history is no longer strictly apples-to-apples with the updated suite.
- Added a shared top menu scaffold across gameplay and editor scenes, with gameplay File/View/City/Tools/Help menus and reduced editor File/editor-action menus. See [`ui.md`](ui.md).
- Migrated the Building Inspector and SelectTool road-properties UI onto draggable Godot `Window` surfaces instead of custom anchored panels. See [`ui.md`](ui.md).
- Building Inspector now supports multiple simultaneous per-building windows and refreshes open inspectors on each in-game hour boundary. See [`ui.md`](ui.md).
- Added an in-game Pack Manager window through the gameplay `Mods` toolbar action. See [`ui.md`](ui.md).
- Reworked the zoning toolbar from one flat profile row into Residential / Commercial / Industrial family buttons with a second profile row above for the selected family. See [`ui.md`](ui.md).
- Added a compact bottom-left R/C/I demand meter beside the clock, driven by live normalized demand pressures from `SimulationNode`. See [`ui.md`](ui.md).
- Replaced the legacy `MapConfig` type with chunk-aware `WorldConfig`, added terrain chunk metadata to saves, added explicit `terrain_cell_m`, restored canonical metre-based world coordinates for terrain / water / zoning tooling, removed the old `10 km` versus `20 km` gameplay startup split, and moved terrain plus water runtime storage onto sparse chunk-backed buffers with dense materialization only at save/render boundaries.
- Added blank-world `WorldDefinition` persistence as a separate authored-world asset path, with deterministic SQLite metadata plus sparse-authored terrain chunk storage and runtime methods to create, save, and load blank worlds independently from city saves. See [`terrain.md`](terrain.md).
- Added a first terrain-only `WorldEditor` launch mode and scene, with a reduced File/Help top menu, a bottom terrain toolbar (`Raise`, `Lower`, `Level`, `Smooth`, `Slope`), shared `Diameter m` / `Strength` terrain-brush controls, an on-map terrain brush preview, a two-anchor slope brush workflow, and direct blank-world `WorldDefinition` create/open/save flows on the shared paused runtime. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Extended `WorldEditor` with the first authored-water slice: bottom-toolbar `Water` subtools for `Source`, `Sink`, `Lake Fill`, and `Open Water`, `WorldDefinition` persistence for authored water boundary points, inland lake fills, and edge-connected open-water fills with immediate preview rebakes into the runtime water map, and editor-only 3D markers for committed water features plus active surface-fill previews. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Replaced the legacy shared-depth water ownership model with a baseline-water / dynamic-water split: authored `Lake Fill` / `Open Water` now rebuild into flat baseline still water, `Source` / `Sink` now drive a separate dynamic runtime overlay, and the old dense water-save layout was intentionally broken rather than migrated. The first continuous dynamic-water runtime still advances at a fixed low-rate `5 Hz` pass, gameplay follows simulation speed, and WorldEditor can advance water in real time while its authored clock remains paused. See [`terrain.md`](terrain.md).
- Reworked `WorldEditor` surface fills into a two-phase preview workflow: click once to seed a transient basin or open-water preview, adjust `Surface +m`, then click again to confirm. Unconfirmed preview state is runtime-only and never serialized into `WorldDefinition`, and terrain sculpting now rebakes authored water so previewed/committed water reacts to basin changes. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Terrain rendering now adds procedural hillshade directly from the live heightmap in both gameplay and WorldEditor, so imported DEM worlds and hand-sculpted worlds get better relief readability without any separate hillshade asset pipeline. See [`terrain.md`](terrain.md).
- Terrain and water rendering now use the first render-only realism pass: slope-aware terrain coloring, shoreline-aware terrain tinting, macro terrain breakup, depth-aware water color, fresnel-style water highlights, and mild procedural surface variation, all without introducing authored material data or external texture requirements. See [`terrain.md`](terrain.md).
- Added an offline DEM-to-`WorldDefinition` importer in `tools/import_dem_world_definition.py`, validated against the Kuopio `324 km²` Maanmittauslaitos `Korkeusmalli 2 m` tiles under `maps/raw/Kuopio/324km2/`, producing a ready-to-open authored world asset at `maps/processed/Kuopio/kuopio_324km2_10m.sqlite`. See [`terrain.md`](terrain.md).
- Water shoreline rendering on the existing `10 m` grid now derives its visible coast from the linearly interpolated live water field instead of whole-cell shoreline masks, giving contour-style diagonal coastlines and channels without a denser authored map. See [`terrain.md`](terrain.md).
- Added a dedicated `MainMenu` front-door scene and `LaunchState` startup handoff so normal launch no longer boots an empty fallback gameplay map. `New Game` now begins from `user://worlds/`, `Load Game` begins from `user://saves/`, and gameplay only opens after one of those selections. See [`ui.md`](ui.md).
- Gameplay `File -> New Game` now opens a `user://worlds/` picker and loads the selected `WorldDefinition` into the live gameplay scene, pausing immediately after the refresh. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Gameplay `Save` and `Load` now open file pickers rooted at `user://saves/` instead of using one fixed `savegame.sqlite` path. See [`ui.md`](ui.md).
- Added a compact city-status HUD panel between the clock and R/C/I meter for treasury balance and live agent count, backed by continuously refreshed snapshot values. See [`ui.md`](ui.md).
- **Pioneer demand floor removed**: the static 0.70 floor on `ResidentialGrowth`, `CommercialGrowth`, and admission pressure has been removed from `demand.rs`. Unemployment benefit now provides early-city bootstrap solvency through real economic activity.
- **Demand formula changes**: `ResidentialGrowth` no longer gates on `job_availability` (people can settle before jobs exist). `IndustrialGrowth` now uses `commercial_input_deficit` instead of `goods_shortage` to avoid OWA-suppression of farm spawning. `NonResidentialSpawnLimit` changed from `resident_presence` to `1.0` to break the commercial/industrial bootstrap deadlock.
- **Unemployment benefit live**: `pay_unemployment_benefits` implemented in `households.rs`, tuning in `economy/profiles.toml` (`15.0/member/day`, 30-day max).
- **Building bankruptcy live**: two-day `budget_distress` check implemented in `households.rs`, `budget_distress: bool` persisted in SQLite schema.

## Reference

- Stable technical lookup data: [`reference.md`](reference.md)
- Live work tracker: [`roadmap.md`](roadmap.md)
- Historical numbered ledger: [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md)
