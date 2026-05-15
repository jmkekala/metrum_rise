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
| Road surface / roadbed replacement                         | [`improved_roads.md`](improved_roads.md)               |

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
- Tightened the shipped `ROAD-01` contract so compiled standard-road sections now follow the
  solved edge elevation profile already stored on the graph instead of silently re-sampling source
  terrain during render / earthwork compilation. This keeps preview, committed surface mesh, and
  terrain earthworks on the same longitudinal grade solve in authored sloped worlds. See
  [`improved_roads.md`](improved_roads.md).
- `ROAD-01` core roadbed ownership is live: `TransitNetwork` now owns one `RoadSurfaceSystem`
  cache that deterministically compiles preview geometry, committed road / sidewalk surfaces,
  bridge decks, tunnel portals, lane-divider markings, terrain earthworks, and world-surface
  picking from the same roadbed ownership model. Bridge earthworks are endpoint-only, tunnel
  earthworks are portal-only, dirty terrain rebuilds stay bounded to touched chunks, the network
  tools can visualize compiled sections / bands / piece boundaries / earthwork chunks through the
  debug overlay, and the old widened-ribbon renderer plus dense centerline flattening
  compatibility path were removed. Phase 9 and Phase 10 are now live as well: grounded roads
  now replace visual terrain with the owned top surface through the grounded footprint, and the compiled
  carriageway keeps a bounded design crossfall instead of rolling to match the full hillside
  slope. Terrain under the owned footprint is now specified as road-following support, not as an
  independent visible surface or trench carrier, terrain render patches intersecting compiled road
  ownership now stay at full mesh resolution and use a denser visible mesh step near roads so
  terrain triangles cannot simplify back through the roadbed. Road-locked terrain patch selection is
  bounded to the road-owned footprint rather than the wider earthwork envelope, road-locked patches
  now carry explicit road footprint clip polygons instead of a road-ownership shader mask, and
  road-touched terrain patches now build clipped double-sided `ArrayMesh` topology instead of
  relying on fragment discard. The clipped-patch renderer now fast-paths untouched / fully
  road-owned cells, and visible water patches now use depth-owned local topology plus the same road
  footprint clips so full water patch planes cannot leak through dry terrain or under grounded
  asphalt, shoulder / curb, and sidewalk.
  Terminal cap topology now also lives outside the node input extractor: `surface::terminal`
  generates canonical cap carriers, including side-to-end corner closures, consumed by rail
  ownership and height fields, so the retired endpoint end-band helper is no longer part of the
  ROAD-01 path.
  `ROAD-01` remains open until that clipped topology is validated against flat, diagonal, sloped,
  water-overlap, and junction cases.
  `ROAD-01` is still open because the recent roads-first earthworks prototype was
  taken far enough to prove the representation problem: the thin corridor-sheet plus
  terrain-stamp approach does not produce acceptable flat-ground or arbitrary-angle tie-ins.
  Phase 11 remains the deterministic `10 m` versus `5 m` characterization gate, Phase 12 is the
  fixed-roadbed-under-later-terrain-edits follow-up, and the hardcut road geometry target is now
  road-owned top surfaces plus Rust-stitched terrain topology rather than more polishing of the
  corridor-sheet prototype or visible closure meshes. That rewrite now has one explicit target
  split: the logical graph stays as connectivity/routing authority,
  while the visible road system becomes a separate deterministic piece/profile carrier built from
  `Span`, `Bend`, `Terminal`, and `JunctionN` pieces. The hard-cut carrier replacement is partially
  live in the road-surface runtime: renderer output, visible-surface queries, road-surface debug
  overlays, and road-driven earthwork stamping all consume explicit visual pieces instead of a
  node-patch carrier, but clipped terrain topology still needs visual validation and hardening.
  `Terminal`, `Bend`, and `JunctionN` now compile explicit road / sidewalk
  polygons from mouth profiles, width changes are no longer treated as a separate visual node
  piece, cached visual polygons now carry deterministic triangles for render/query/stamp reuse,
  span pieces now also own the earthwork chunk coverage and terrain-stamping carrier instead of
  falling back to raw section windows after compile, `Bend` and `JunctionN` no longer share one
  generic connected-node builder, adjacent mouth-side sectors no longer collapse to one fallback
  quad when side profiles differ, node incident ordering now reads inward directions from compiled
  span mouth profiles instead of from section tangents, `JunctionN` adjacent-gap sectors now use
  the ordered-mouth ownership rule directly (`current.left` with `next.right`) instead of a
  heuristic gap-facing side selector, and node
  pieces now also own explicit earthwork polygons and outer earthwork boundaries instead of
  borrowing visible polygons for node earthwork bounds and terrain stamping. Span outer boundary
  loops now also compile directly from section ranges instead of being extracted from emitted
  polygons. The `Bend` path no longer borrows the generic junction-style center asphalt core
  either: two-way corners now use direct sampled mouth-to-mouth sector geometry with fixed
  `<= 1 m` connector steps, `Terminal` outer boundary loops now come directly from explicit
  sidewalk / curb cap geometry instead of generic polygon extraction, bend outer boundary
  loops now come directly from compiled bend sectors instead of a generic polygon extraction pass,
  and footpath mouths now compile directly in the incident-mouth builder instead of through a
  separate fallback helper.
  `Bend` and `JunctionN` no longer share one connector-strip polygon builder either. `JunctionN`
  also no longer relies on one global angle-sorted center asphalt polygon; its carriageway core is
  now assembled from adjacent-mouth wedges around the node, its outer boundary loops now come
  directly from compiled adjacent-gap sectors instead of a second-pass mouth reconstruction, and
  it no longer shares the bend-side sector builder as its final geometry carrier. The shared
  node-piece assembler no longer infers node earthwork ownership from visible geometry either:
  `Terminal`, `Bend`, and `JunctionN` now pass explicit earthwork polygons and explicit earthwork
  outer loops directly from their own builders. Node earthwork stamping no longer regenerates
  tie-in faces from boundary loops at stamp time, span pieces now do the same, and combined
  visible-world queries can now hit compiled span and node earthwork geometry before falling
  through to terrain. The render path still compiles a cleaner render-only earthwork face set for
  structural or intentionally exposed cases, but suppressing that visible layer for grounded
  `Standard` roads is not accepted as complete until terrain patches are clipped to the
  road-owned seam. Gentle tie-in faces stay on the earthwork material path only when they are
  intentionally surfaced, while steep faces route deterministically to the retaining / wall
  concrete path. Dirty surface and road-touched terrain chunk rebuilds now use piece-owned chunk
  coverage indices, so changed `Span`, `Terminal`, `Bend`, and `JunctionN` pieces rebuild
  `old_coverage union new_coverage` instead of relying on edge-centerline chunk guesses or global
  node-piece scans. Visual node handoffs now use a conflict-first ownership hardcut: local profile
  width is the minimum handoff, shallow-angle arms extend shared visual ownership as far as their
  roadbed / asphalt materials would otherwise overlap, and exact graph clip points remain section
  carrier samples for height and routing metadata.
  The paired adjacent-mouth strip candidate model has been hard-cut out of node-piece ownership.
  Conflict-bounded full-roadbed corridor unions now define node footprints, conflict-bounded
  carriageway corridor unions define asphalt, and non-road ownership is split into explicit curb /
  shoulder and sidewalk owned regions. `Terminal`, `Bend`, and `JunctionN` top-surface heights now
  use owner-local `NodeBandHeightField` surfaces identified by `NodeBandHeightFieldId`; the old
  source-vector plumbing, boundary rail snapping, shared post-overlay grade sampler, and derived
  curb-transition fallback have been removed. Logged terminal and 2-arm bend curb / sidewalk join
  ownership is now hardened through the canonical path, and raised-step contact rails now use a
  generic source-owned owner-pair constraint instead of material-specific asphalt/curb and
  curb/sidewalk contact kinds; arrangement and debug seam output now reports generic
  `RaisedStepContact` sources, with bend side-join final-owner handoff limited to endpoint contacts
  from exact source rails. Non-terminal side-join ownership now enters through the dedicated
  `surface::joins` adapter rather than the input extractor; the rail contour set consumes terminal
  cap bands and side-join bands as separate carriers, and `JunctionN` side joins no longer add
  carriageway bubble fill or contribute to `node_footprint`. `JunctionN` side-join paths are now
  Cavalier-cleaned adjacent-mouth non-road joins. Generated node contours now carry explicit
  footprint / asphalt / non-road authority roles, so boolean ownership no longer infers primary
  material authority from band kind alone and clips asphalt authority to `node_footprint` before
  residual checks. Bend / JunctionN raw full-roadbed and carriageway corridor authority is now
  separated from per-band owner carriers before boolean splitting. Generated contact contours and
  final owned-region rings no longer use projected-key or overlay-neighbor repair; owner-pair
  contacts must stay exact-source authorized, backend drift may canonicalize only through the
  owning source rail key, and node raised-step face export emits generic owner-pair faces only
  from exact canonical arrangement-key support instead of overlay-sibling edge matching. The
  render mesh payload now exposes those faces as `raised_step_*` buffers rather than curb-specific
  vertical buckets. Post-boolean `node_non_road`
  subdivision now requires every final curb /
  shoulder and sidewalk owned region to carry explicit profile seam-rail evidence, and
  carrier-only leftovers are reported as deterministic boolean-ownership residual diagnostics.
  Span output now also routes through resolved top-region records and generic owner-pair
  raised-step constraints before exporting the existing render, query, terrain-clip, earthwork, and
  chunk-coverage fields, so span rendering is no longer the authority layer for material ownership.
  Road-touched terrain support now uses the lower
  road-owned top-surface envelope when grounded support overlaps terminal caps or raised bands, and
  bridge / tunnel earthwork ranges are class-aware so bridge midspans are not flattened while
  visible tunnel portals still stamp. Road-touched terrain CDT diagnostics now expose source
  samples omitted to widen over-steep cut / fill tie-ins, and over-budget road-seam faces are
  exported as explicit retaining-wall tie-in geometry instead of ordinary terrain. Synthetic DEM
  validation now covers supportive ordinary tie-ins, near-road source-sample widening, raised and
  lowered retaining-wall cuts, and mixed widening-plus-wall cases while preserving exact road seam
  constraints. The remaining blocking work is authored / extreme DEM validation and any richer
  closure variants needed beyond the retaining-wall path.
  The shared engineered-ground contract now lives in
  [`earthworks.md`](earthworks.md), with road-specific rules staying in
  [`improved_roads.md`](improved_roads.md) and terrain storage / chunking rules staying in
  [`terrain.md`](terrain.md).
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
- Added the first `WorldEditor` launch mode and scene, with a reduced File/Help top menu, bottom terrain and water authoring toolbars, shared brush controls, on-map brush previews, a two-anchor slope brush workflow, and direct blank-world `WorldDefinition` create/open/save flows on the shared paused runtime. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Extended `WorldEditor` with the first authored-water slice: bottom-toolbar `Water` subtools for `Source`, `Sink`, `Lake Fill`, and `Open Water`, `WorldDefinition` persistence for authored water boundary points, inland lake fills, and edge-connected open-water fills, and editor-only 3D markers for committed water features plus active surface-fill previews. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Replaced the legacy shared-depth water ownership model with a baseline-water / dynamic-water split: authored `Lake Fill` / `Open Water` now rebuild into flat baseline still water, `Source` / `Sink` now drive a separate dynamic runtime overlay, and the old dense water-save layout was intentionally broken rather than migrated. The first continuous dynamic-water runtime still advances at a fixed low-rate `5 Hz` pass, gameplay follows simulation speed, and WorldEditor can advance water in real time while its authored clock remains paused. See [`terrain.md`](terrain.md).
- The current `Source` / `Sink` / `Lake Fill` / `Open Water` workflow is now treated as the shipped water-authoring baseline; richer river-path or hydrology ownership is optional future work rather than a required next milestone. See [`terrain.md`](terrain.md).
- Reworked `WorldEditor` surface fills into a two-phase preview workflow: click once to seed a transient basin or open-water preview, adjust `Surface +m`, then use the dedicated `OK` / `Cancel` flow to confirm or dismiss it. Unconfirmed preview state is runtime-only and never serialized into `WorldDefinition`, and terrain sculpting now rebakes authored water so previewed/committed water reacts to basin changes. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Terrain rendering now adds procedural hillshade directly from the live heightmap in both gameplay and WorldEditor, so imported DEM worlds and hand-sculpted worlds get better relief readability without any separate hillshade asset pipeline. See [`terrain.md`](terrain.md).
- Terrain and water rendering now use the first render-only realism pass: slope-aware terrain coloring, shoreline-aware terrain tinting, macro terrain breakup, depth-aware water color, fresnel-style water highlights, and mild procedural surface variation, all without introducing authored material data or external texture requirements. See [`terrain.md`](terrain.md).
- Terrain coloring now follows a surface-classification-first and absolute-height-second model, so blank worlds and imported DEM worlds no longer depend mainly on one global elevation ramp for their palette. See [`terrain.md`](terrain.md).
- Added an offline DEM-to-`WorldDefinition` importer in `tools/import_dem_world_definition.py`, validated against the Kuopio `324 km²` Maanmittauslaitos `Korkeusmalli 2 m` tiles under `maps/raw/Kuopio/324km2/`, producing a ready-to-open authored world asset at `maps/processed/Kuopio/kuopio_324km2_10m.sqlite`. See [`terrain.md`](terrain.md).
- Water shoreline rendering on the existing `10 m` grid now derives its visible coast from the linearly interpolated live water field instead of whole-cell shoreline masks, giving contour-style diagonal coastlines and channels without a denser authored map. See [`terrain.md`](terrain.md).
- Terrain rendering on the existing coarse authored grid now also includes render-only cliff breakline / cliff band treatment derived from the live terrain field, improving steep cuts and man-made cliffs without changing authored world data or forcing a denser map. See [`terrain.md`](terrain.md).
- Terrain rendering now adds a render-only terrain-border skirt derived from the live terrain edge, with a side wall, bottom cap, and contour continuation down the cut surface so the world reads as a visible slice instead of a paper-thin plane. See [`terrain.md`](terrain.md).
- Water rendering now also adds a render-only edge curtain where water reaches the map boundary, so outside views do not see straight through to the submerged terrain plane at the border. See [`terrain.md`](terrain.md).
- `TERRAIN-01` is now live: terrain and water rendering no longer use one whole-world mesh plus one whole-map dense runtime upload. Both renderers now consume chunk-local patch snapshots aligned to the terrain patch grid, terrain/water roots now own per-patch child meshes instead of a single mesh boundary, dirty patch uploads stay local, and the old whole-map Godot render bridge methods were removed from the steady-state render path. This makes the `10 m` versus `5 m` terrain-density decision measurable on the actual large-world render boundary instead of on the old overlay-era compatibility path. See [`terrain.md`](terrain.md).
- Earthworks cleanup note: the old whole-map terrain render boundary is no longer the active
  blocker for engineered ground. The remaining blocker is now the near-road representation itself:
  the current corridor-sheet prototype is retired, and [`earthworks.md`](earthworks.md) plus
  [`improved_roads.md`](improved_roads.md) now reset the target to a closed road-owned earthwork
  mesh carried by a separate piece/profile visual road layer rather than by graph-derived node
  fills.
- Terrain / water patch rendering now also uses deterministic distance-based mesh LOD on top of the
  split patch snapshot path, so far-field camera views can reuse the same resident patch snapshots
  without paying full near-field vertex density for every visible patch. The temporary seam /
  emissive terrain-debug visual modes used during patch-hardening were removed from the steady
  runtime after the seam-width bug was fixed. See [`terrain.md`](terrain.md).
- The first roads-first engineered-ground prototype did useful architectural work but is no longer
  treated as the final path: later terrain edits can keep committed roads fixed, chunk-local
  rebuilds and visible-surface precedence remain required, and terrain / road ownership stays
  explicit, but the thin corridor-sheet visual carrier is now retired in favor of road-owned top
  surfaces, band-owned node geometry, and Rust-stitched terrain topology. See [`earthworks.md`](earthworks.md),
  [`improved_roads.md`](improved_roads.md), and [`terrain.md`](terrain.md).
- `ROAD-01` is now pinned to one deterministic target architecture: the next road geometry pass
  must stop treating the logical graph as the visible-shape carrier and instead compile a separate
  deterministic piece/profile geometry layer with `Span`, `Bend`, `Terminal`, and `JunctionN`
  pieces, while the elevated `Bend` / `JunctionN` hardcut replaces post-overlay height sampling
  with band-owned regions and `NodeBandHeightField` surfaces identified by
  `NodeBandHeightFieldId`. The existing graph / clip / lane ownership layers stay intact. See
  [`improved_roads.md`](improved_roads.md).
- The retired annulus/corridor prototype still produced useful conclusions that remain valid after
  the code revert: arbitrary-angle bends and multi-arm junctions need explicit road and sidewalk
  piece ownership, not one sampled outer loop plus one sampled inner loop with triangulation
  layered on afterward. See [`improved_roads.md`](improved_roads.md).
- Gameplay and `WorldEditor` now share one terrain-aware world-camera core in `CameraNode`, including a common terrain-clearance rule that keeps the camera above the terrain surface while preserving separate scene-level zoom and clip policy. See [`ui.md`](ui.md).
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
