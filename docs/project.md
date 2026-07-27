# Metrum Rise — Project Dashboard

This file is the live dashboard for current state, priorities, and links to the owning docs. It is intentionally summary-first.

The old monolithic ledger and numbered backlog are archived in [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md). That archive is historical reference only, not the current planning source.

## Snapshot

- **Scale target**: at least 1,000,000 total population across simulation tiers in one large world.
- **Simulation model**: full-FSM simulation stays inside the active area of interest; distant world regions are expected to degrade to coarser flow-field or aggregate simulation.
- **Current focus**: keep the playable small-to-medium city slice correct, deterministic, and scalable while the docs/planning cleanup continues and baseline water/render plus local road-build performance are hardened.

## Shipped Foundations

- **Road network and routing**: modular `RegionGraph`, lane system, CCH pathfinding, road rendering, border nodes, and roadway editing are all live.
- **Zoning and building allocation**: Rust-owned road-aligned parcels, parcel occupancy, roadside building placement, vacancy indexing, and no-build edge flags are live. See [`zoning.md`](zoning.md) and [`building_allocator.md`](building_allocator.md).
- **Entrance-aware movement**: the building entrance/exit rewrite is implemented through the exact-plan system described in [`entrance_and_exit.md`](entrance_and_exit.md), including the Phase 1–6 and Phase 8 slices already verified against the live code.
- **Benchmark coverage**: the Criterion suite now measures the live access phases through `ACCESS_EGRESS` and `ACCESS_INGRESS` in addition to pure `NETWORK` and idle scaling. Treat comparisons against older benchmark runs as a fresh baseline unless the benchmark shape is identical.
- **Economy foundation**: household records, building-centric daily economy, physical truck freight jobs, `OWA` fallback, exact entrance-side freight routing/ETA, unemployment benefit disbursement, two-day building bankruptcy, short private-building construction timers, baseline fiscal revenue (income tax, household/business purchase tax, business profit tax, and construction property tax), and first city-owned service-building placement/funding are all live. See [`economy.md`](economy.md).
- **Demand foundation**: the live `DemandSystem` now fully owns immigration and building growth pressure, except for the explicit gameplay cheat mode documented in [`demand.md`](demand.md). RCI telemetry, household admission, and private building actions refresh hourly, while household removal remains daily. Private spawning now uses deterministic missing-building need; legal parcels cap placement rather than scaling the spawn rate. Household admission is driven by incoming household pull from bootstrap entry, budget-backed open jobs, and authored regional migration pressure; vacant homes only cap actual move-in execution. Regional migration requires an external road connection and is damped by household affordability, stock stability, failure state, and a soft household target. Residential construction reads that same incoming pressure plus move-in viability and failure-memory damping before creating more home capacity. Non-residential spawning is not hard-blocked by pre-existing full staffing; placed workplaces create budget-backed open jobs that pull households, while output absorption prevents ordinary oversupply. Move-in acceptance estimates candidate household search runway from starter savings, budget-backed open jobs, unemployment benefit reliability, and daily essential cost. Household removal now combines a crisis-ratio outflow rule with persistent exit for households that remain unhoused and destitute long enough. Daily city-flow diagnostics now summarize net household flow, job openings, resident employment, household failure state, vacant homes, and treasury in one economy log line. The static R/C/I pioneer demand floor has been removed entirely — unemployment benefit provides early-city solvency instead. Commercial demand now anticipates missing shop capacity before household stock collapses using short-run household buying power, and industrial demand is driven by commercial input coverage rather than household `goods_shortage`. See [`demand.md`](demand.md).
- **Persistence and runtime**: SQLite save/load, background simulation thread, render snapshots, debug flags, asset editor, and economy editor are live. The asset editor now supports multi-part building assets, driveway/parking/loading-bay site anchors, WYSIWYG flat lot preview, and authored polygon yard surfaces for textured asphalt and concrete. Runtime building placement registers required flat support footprints at construction start, clips visual terrain through the shared terrain/CDT path, and keeps zoning terrain-neutral. Vehicle parking / freight stop behavior remain later runtime hooks.

## Current Priorities

For active tracked work, use [`roadmap.md`](roadmap.md).

- `ROAD-05`: fixed world-aligned refined-terrain CDT tiles and immutable prior-generation reuse are
  implemented, including cached tile render buffers, bounded incremental road undo with exact
  pre-edit surface-cache restoration, stable exact-XZ `JunctionN` contact reuse, and uniform-height
  ownership reuse. Topology-changing junction rebuilds now also reuse unchanged exact
  same-material contour-pair contacts, cross-kind raised-step contour-pair output, indexed
  raised-step source/group contributors, final noded contact components, and retained-contact
  decisions/authority. Fixed world-aligned source/target tiles and semantic source deduplication
  make later raised-step passes visit only new sources or changed target groups; point incidence no
  longer scans every source. Retained-contact authority uses reverse-indexed immutable buckets, and
  diagnostics separate current-generation duplicate lookups from previous-generation reuse.
  Node-local keys ignore raw edge-ID churn. Canonical ownership cleanup including final self-touch
  splitting, seam extraction/materialization, and final-boundary point provenance now promote exact
  unchanged contributors from the immutable prior generation. An exact final owned-shape/constraint match
  also replays the complete footprint, seams, boundary arrangement, and diagnostics while retaining
  the nested seam cache for the next edit; removed entries are dropped after changed builds,
  Bend-to-JunctionN transitions retain contributor state, and indexed boundary-reference
  construction replaces full point scans. Remote third-road splits now
  recompile the existing and newly created `JunctionN` pieces as one atomic surface generation,
  retaining the last complete render generation on any required-node failure. World-definition
  and save replacement publish their final road generation before terrain/water workers resume,
  while water-only query revisions retain valid unchanged road clipping. Semantic node export now
  reuses final explicit-step topology, exact-XZ height-conflict cohorts, top-boundary contributors,
  and raised-step spans/faces with current-generation index rebinding; final-step misses use
  compact edge-local authority keys and spatially indexed compatible overlap candidates. Complete
  assembled node export buffers and fresh release gameplay measurements remain.
- `QA-01`: revalidate and root-cause the old long-run sim-thread panic.
- `CIV-01`: validate and harden the first city-owned service-building slice across UI, runtime economy, and save/load.
- `WATER-01`: harden baseline-water rendering and remove remaining dense compatibility boundaries.
- `MOB-01`: ship bicycle support as the next transport mode.
- `ALLOC-01`: harden building allocator ownership and spec limits.
- `DOC-01`: finish replacing old numbered backlog references in live docs.

`QA-01` is now parked in [`roadmap.md`](roadmap.md): the old long-run sim-thread panic has not reproduced recently, including at least one overnight run, so it is no longer treated as an active blocker.

## System Ownership

| Area                                                        | Owning doc                                             |
| -------------------------------------------------------------| --------------------------------------------------------|
| Current status / priorities                                 | [`project.md`](project.md), [`roadmap.md`](roadmap.md) |
| Stable constants / formats / vocabulary                     | [`reference.md`](reference.md)                         |
| Entrance / exit / trip attachment                           | [`entrance_and_exit.md`](entrance_and_exit.md)         |
| Lane-bound vehicle traffic movement                         | [`traffic.md`](traffic.md)                            |
| Economy / freight / household runtime                       | [`economy.md`](economy.md)                             |
| Demand / city-growth pressure / admission-removal ownership | [`demand.md`](demand.md)                               |
| Zoning                                                      | [`zoning.md`](zoning.md)                               |
| Terrain ingest / chunked terrain runtime / world terrain    | [`terrain.md`](terrain.md)                             |
| Building placement / removal / frontage attachment          | [`building_allocator.md`](building_allocator.md)       |
| Gameplay HUD / menus / floating windows                     | [`ui.md`](ui.md)                                       |
| Asset-editor workflow and pack contract                     | [`asset_editor.md`](asset_editor.md)                   |
| Road surface / roadbed replacement                         | [`roads.md`](roads.md)               |

## Recent Structural Changes

- The Rust runtime bridge now keeps `simulation_node.rs` as the `SimulationNode` lifecycle and
  routing shell, with Godot APIs, async job state, and Variant export split into focused
  `nodes/simulation_node/` modules. Authoritative state, thread orchestration, snapshots, budgets,
  previews, and shared terrain/water payload computation are split under `nodes/sim/core/`;
  `CODE-08` tracks the remaining Godot-independent terrain/CDT work still below the node boundary.
- Rust production monoliths now route through ownership-focused modules: building-site support is
  split into model, derivation, grading, terrain clipping, query, and geometry; graph rebuilds into
  adjacency, compaction, clips, junction profiles, and terrain sync; compiled standard-road
  rendering into coverage, top surface, bridges, markings, earthwork, and geometry; and asset
  manifests into class models plus centralized validation. Large allocator, agent, and household
  test modules are also split by behavior. The audit additionally made incremental junction clips
  proportional to incident edges, removed repeated road-render coverage sorting, made island
  counting iterative and deterministic, and fixed asymmetric building-site broad-phase radius and
  geometry tolerance errors without changing the owning subsystem contracts.
- Gameplay bulldoze is now a dedicated Rust-backed tool instead of a selection special case. The
  bottom-right HUD action activates a one-click delete cursor with Rust-owned deterministic
  targeting (`building` before `road`). Bulldoze and undo are queued onto the simulation thread so
  Godot never performs road-surface, road-mesh, or refined-CDT rebuilding in an input callback.
  Road deletion uses local graph and attached-parcel deltas, while building deletion uses the
  allocator lifecycle path with a bounded inverse journal for only the touched
  building/site/economy records; derived render caches are regenerated instead of cloned. See
  [`ui.md`](ui.md), [`roads.md`](roads.md), and [`building_allocator.md`](building_allocator.md).
- Pedestrian junction lanes and visible zebra crossings now consume the same authoritative
  crossing records. Both walking directions traverse the rendered asphalt-edge segment, while
  adjacent-arm turns stay on the sidewalk perimeter instead of cutting mouth-to-mouth through the
  carriageway. Each incoming sidewalk now also has a precomputed legal route to both sidewalks of
  every reachable road arm, so exact destination-side access cannot stall or reselect a lane
  across the junction. Road-sidewalk endpoints now coincide with the crosswalk inset, removing the
  visual pass-and-backtrack discontinuity before a crossing. Adjacent-arm turns share the compiled
  road surface's sampled corner-rounding policy instead of walking through a sharp asphalt miter.
  Incremental lane updates rebuild connectors at both ends of every incident arm and invalidate
  active agents across that same closure, so no current route can target an orphaned lane ID. See
  [`roads.md`](roads.md).
- Road-edit traffic hardening now reattaches invalidated on-road agents to rebuilt physical lanes
  from their preserved world positions using authoritative lane arc length, while strict
  degree-two road splits may use direct vehicle lane continuity instead of zero-length junction
  connectors. True junctions keep connector lanes so speed and spacing semantics still apply, and
  building-site CDT seams now prefer road-owned heights at shared XZ vertices. See
  [`roads.md`](roads.md) and [`traffic.md`](traffic.md).
- Pedestrian runtime characters now use Quaternius-derived VAT bakes for the shipped adult male
  and adult female archetypes. The bake path selects the explicit walk action from the source
  `.blend`, normalizes the rest mesh to `1.8 m`, preserves outfit color through vertex colors, and
  keeps the renderer on the existing GPU VAT MultiMesh path instead of per-agent skeleton playback.
  Walker MultiMeshes now use the same centralized dynamic shadow-caster policy as vehicles.
  See [`asset_editor.md`](asset_editor.md).
- Terrain/water streaming now smooths remaining activation spikes by keeping speculative prewarm
  local to the resident halo, deferring LOD/prewarm work when earlier render stages have already
  consumed the frame, skipping no-op baked/CDT terrain LOD mesh rebuilds, and running water mesh
  refresh as poll-ready, apply-ready, submit-new work. Water mesh apply/poll now also reports and
  gates by estimated payload bytes, fully wet unclipped water grids reuse shared Godot mesh
  resources by LOD/topology and prewarm the regular full-grid variants during load, regular
  terrain mesh variants are prewarmed from the active world layout, terrain/water patch
  nodes/materials/images/textures are pooled before first visible activation, Rust asynchronously
  prepares terrain/water non-mesh patch payloads for residency and resident dirty uploads, and
  ready residency work can now burst up to `12` terrain plus `12` water patches per frame under
  separate `4 ms` safety budgets instead of being forced through a two-patch drain. Water mesh
  publication has matching bounded apply headroom, while perf summaries include the active
  residency limits/budgets plus viewport, draw-call, primitive, memory, vsync, FPS-cap, and
  resource-pool stats. See [`terrain.md`](terrain.md).
- Refined terrain payload preparation no longer performs road clipping, building-site grading, or
  CDT input construction while holding the central simulation mutex. A perf capture exposed a
  `16.7 s` terrain-input lock hold that stopped simulation ticks, camera handling, and log output
  together as buildings spawned. Workers now take bounded patch-local terrain/site snapshots,
  perform indexed road/site work off-lock, use patch-local source revisions, and coalesce revision
  churn behind one physical build per patch/render-step. Refined publication is also atomic across
  local CDT windows: site geometry cannot mask a failed/missing road clip, and raw terrain payloads
  are refused for all road- or building-site-owned patches. Generation-tagged acknowledgements
  cannot erase a mutation newer than the uploaded terrain, water, or road revision. See
  [`terrain.md`](terrain.md) and
  [`earthworks.md`](earthworks.md).
- Terrain/water presentation now has a documented runtime contract: terrain and building-site
  grass use the Grass002 world-space material stack with luminance-preserving macro/mid/micro
  detail fade, water uses a dark Baltic-blue depth palette with less terrain bleed and restrained
  downward-view sky reflection through a tuned Fresnel/foam/normal material path. Grazing views
  receive a smooth sky response that does not expose procedural normal cells, and the sun
  reflection uses a conservative softened shoulder around its bright core; fine ripple detail is
  deferred until it can use a seamless mipmapped normal texture. Scene lighting / shadow policy is
  centralized through the Godot rendering bridge. Gameplay and WorldEditor now
  share a continuous procedural hemisphere sky with no literal horizon seam and a sun driven by
  that same directional light. A static 2K equirectangular cloud source is reduced to a restrained
  half-resolution cloud cover in the sky shader; its baked lower hemisphere is excluded and its
  sun opening is aligned to the shared directional sun. Gameplay now exposes the renderer's full
  normal-height `8 km` terrain range through a `9 km` camera far plane, then fades all world
  geometry into the sky before the cull boundary; the existing desired/resident/prewarm patch bands
  remain behind that fade. The `run.sh` debug launch flags and terrain/water/building visual modes
  are now listed in [`reference.md`](reference.md), while the rendering invariants live in
  [`terrain.md`](terrain.md).
- Building-site earthworks now keep the derived required support footprint flat, reject placement
  when the surrounding terrain/road cannot tie in within the deterministic apron envelope, and
  derive sample-only apron guides from the actual support edges, so site grading cannot add hard
  CDT rails across neighboring roads or sites. Road-facing access anchors stay behind the exact
  sidewalk/road boundary so the frontage strip remains tie-in space instead of a conflicting hard
  site loop. Near-road tie-ins sample the nearest visible road surface, stale parcel/building
  frontage attachments are repaired after road topology edits, and `--debug site-grading`
  combines road and site diagnostics. See
  [`earthworks.md`](earthworks.md), [`roads.md`](roads.md), and [`reference.md`](reference.md).
- Standard road placement now prepares a dense terrain-aware vertical profile before preview or
  commit: the player's XZ alignment is preserved, terrain / visible-road support samples become
  height targets, endpoints and road connections are pinned, and `physical_geometry` stores the
  solved dense profile that section compilation and earthworks consume. Degree-1 terminal road
  extensions re-solve the previous terminal edge plus the new edge as one corridor, so building in
  pieces and one-stroke placement share the same vertical validity. Placement preview and commit
  now also dry-run the local surface compile so degenerate tight bends are rejected with a visible
  reason instead of landing as missing roadbed. Grounded `Standard` roadbeds that touch authored
  water are now rejected in hover preview, exact preview, live commit, and edge-class editing;
  explicit `Bridge` spans remain legal. Road-locked terrain payload queries now preserve the same
  safety pad as grading-envelope patch selection, preventing source-less terrain holes where a
  bridge approach returns to grounded road. Degree-two pass-through bridge/road handoffs now retain
  the preview-validated vertical profile and share one exact endpoint cross-section, removing the
  post-commit bump and narrow transverse cap. A continuation from either end of a degree-one
  elevated bridge terminal down to source terrain now remains a structural bridge ramp across its
  full approach, rather than becoming `Standard` earthwork that raises the terrain to meet the
  deck. Ground-contact bridge-ramp sections now join the adjacent node / standard-road terrain
  cutout with source-owned abutment boundaries, preventing coplanar terrain from z-fighting through
  connected bridge landings without clipping elevated midspans. Padded terrain-CDT queries now
  discard margin-only road loops whose patch-clamped bounds collapse to a line, preventing a bridge
  landing from invalidating and hiding an adjacent terrain patch. Road geometry dumps now include
  compact cut/fill summaries. See [`roads.md`](roads.md).
- Corrected road-speed units so the current urban road presets use `50 km/h` as `13.89 m/s`,
  and capped car movement through junction connector lanes at `6 m/s`. See
  [`reference.md`](reference.md).
- Traffic movement now uses connector curvature to cap junction turn speed, acceleration/braking
  limits for speed changes, and target-lane gap checks plus speed-scaled S-curve poses for
  same-edge car lane changes. Clear lane changes preserve road speed; blocked target lanes are
  treated as traffic and can force braking. Conservative same-edge overtaking is live for
  multi-lane vehicle roads: cars pass only after being traffic-blocked, only toward the center
  lane, and return outward after a cooldown when the cruising lane is clear. See
  [`traffic.md`](traffic.md).
- `ROAD-04` is closed for the current node top-surface quality pass: `Bend` / `JunctionN`
  carriageway triangulation now canonicalizes same-owner / same-height / same-provenance numeric
  dust, can insert road-owned interior guide support before CDT, and validates visible
  pathological top-surface triangles with source-rich diagnostics. Road-edit rebuilds also regrade
  affected junction mouths through an authority-corridor-aware horizontal-distance profile solve
  that keeps the best stable through corridor as the whole-`JunctionN` base grade, prevents
  secondary opposite branch pairs from rotating that plane, and blends edited branches into it with
  a small dynamic mouth pin, one solve/control sample, sparse transition support vertices, and
  protected handoff sampling. Section compilation now applies the same small profile hard pin to
  sparse grounded `Standard` two-mouth `Bend` vertical-curve blending instead of treating the
  farther material ownership handoff as a flat platform extent, while preserving source-sampled
  Bend, terrain, and earthwork footprint provenance. Edge span sampling, visible queries, and
  grounded `Standard` earthwork section ranges now consume the same node-mouth ownership policy
  rather than separate local clip helpers. `Bend` / `JunctionN` adjacent-mouth side joins now emit
  rounded mouth-to-mouth asphalt-to-curb and sidewalk-to-terrain ownership boundaries instead of
  routing visible corners through the shared graph endpoint, even when a split carriageway slice
  collapses against the centerline. Non-terminal carriageway owner carriers are part of the same
  rounded ownership policy, so production asphalt cannot keep old miter endpoints while side-join
  helper contours look rounded.
  See
  [`roads.md`](roads.md).
- Removed the zoning paint-surface runtime: zoning now lives under `simulation::zoning`, stores
  Rust-owned parcels only, and no longer exposes dense zoning patch/texture APIs. See
  [`zoning.md`](zoning.md).
- Transitioned the residential simulation to a **household-centric occupancy model**, replacing legacy per-resident capacity with family slots (`household_capacity`).
- Household operational ticks now use a fused parallel agent reduction for household membership and worker counts, skip hot-path repair of stale household references in favor of debug validation, progress household stock / utility drain / replenishment classification in one household pass before deterministic reservation apply, and plan workplace assignment from hourly job-supply snapshots plus per-home ranked job options instead of per-agent job scans.
- Household replenishment now uses a bounded one-shopper household task: it waits for an eligible
  member at home before claiming store stock, then uses ordinary building-origin trips for
  `Home -> Store -> Home`. Store reservations now require exact trip feasibility before stock or
  budget is claimed, failed searches continue through farther deterministic supplier windows on
  later coarse retries, active shopping legs have explicit timeouts, and repeated failures surface as
  unresolved shortages instead of silent retry loops. See [`economy.md`](economy.md) and
  [`entrance_and_exit.md`](entrance_and_exit.md).
- Resident age groups are live for the baseline economy: adults can work and shop, elders can shop
  only, children consume household resources but do not work or shop, immigrant households cap at
  two adults and two elders, and children only appear with adult households. Demand admission now
  estimates move-in jobs and benefits from expected adult workers rather than total residents.
  Starter household sizing now uses `flat_size_m2` with adult and lighter child area weights, so
  larger single-family homes can admit larger families while still consuming one household slot. See
  [`economy.md`](economy.md) and
  [`demand.md`](demand.md).
- Baseline city revenue is live: gross wage payments withhold authored income tax, household store
  pickup and business freight delivery split buyer-paid purchase tax from seller revenue, positive
  daily commercial/industrial budget growth pays authored business profit tax, fresh private
  construction pays one-time property tax, and economy debug logs include daily fiscal
  buckets. See [`economy.md`](economy.md).
- First explicit Services placement is live: the bottom toolbar discovers city service assets from
  the asset registry, places them on road frontage through Rust allocator validation, charges the
  city treasury, funds municipal utility wages from the treasury, and routes local city-owned
  utility fees back to the treasury. Coal-fired power plants now request coal through ordinary
  freight, draw fuel/input purchases from the city treasury, accumulate produced power from staffed
  fueled operation, route covered household utility payments into matching local utility revenue,
  and expose uncovered private demand as OWA fallback spend. The Economy Overview window now reads
  Rust-owned daily budget ledgers, graphs city income/expenses/net/treasury, and exposes a live
  electricity funding slider that sets default staffed power-plant worker slots; individual power
  plants can override that default from their inspector without being reset by later citywide
  changes. Production follows the staffed workers and fuel availability. See
  [`economy.md`](economy.md).
- Freight shipments now dispatch physical truck carrier agents through the existing lane movement
  system. Local freight, `OWA` imports, and `OWA` exports settle only when the carrier reaches the
  destination building or border terminal; empty carriers then return to their source building or
  border base before being removed. The Godot renderer maps the freight vehicle type to
  `assets/models/vehicles/freight/delivery.glb`. See [`economy.md`](economy.md).
- Industrial exports now hold affordable local commercial input demand before selling to `OWA`,
  repeated same-resource exports saturate to a lower outside bid, and commercial store jobs/input
  targets scale from the larger of recent household sales and local household demand/stock
  recovery instead of immediately using full authored capacity.
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
  [`roads.md`](roads.md).
- `ROAD-01` core roadbed ownership is live: `TransitNetwork` now owns one `RoadSurfaceSystem`
  cache that deterministically compiles preview geometry, committed road / sidewalk surfaces,
  bridge decks, tunnel portals, lane-divider markings, terrain earthworks, and world-surface
  picking from the same roadbed ownership model. Bridges render structural concrete supports
  instead of terrain earthworks, tunnel
  earthworks are portal-only, dirty terrain rebuilds stay bounded to touched chunks, the network
  tools can visualize compiled sections / bands / piece boundaries / earthwork chunks through the
  debug overlay, and the old widened-ribbon renderer plus dense centerline flattening
  compatibility path were removed. Phase 9 and Phase 10 are now live as well: grounded roads
  now replace visual terrain with the owned top surface through the grounded footprint, and the compiled
  carriageway keeps a bounded design crossfall instead of rolling to match the full hillside
  slope. Terrain under the owned footprint is now specified as road-following support, not as an
  independent visible surface or trench carrier, terrain render patches intersecting compiled road
  ownership now stay at full mesh resolution and use a denser visible mesh step near roads so
  terrain triangles cannot simplify back through the roadbed. Road-locked terrain patch selection
  is bounded to each road-owned footprint plus its required grade-limited tie-in envelope,
  road-locked patches now carry explicit road footprint clip polygons instead of a
  road-ownership shader mask, and
  road-touched terrain patches now build clipped double-sided `ArrayMesh` topology instead of
  relying on fragment discard. The clipped-patch renderer now fast-paths untouched / fully
  road-owned cells, and visible water patches now use depth-owned local topology plus the same road
  footprint clips so full water patch planes cannot leak through dry terrain or under grounded
  asphalt, shoulder / curb, and sidewalk.
  Terminal cap topology now also lives outside the node input extractor: `surface::terminal`
  generates canonical cap carriers, including side-to-end corner closures, consumed by rail
  ownership and height fields, so the retired endpoint end-band helper is no longer part of the
  ROAD-01 path.
  `ROAD-01` is now closed for the roadbed / terrain handover: clipped topology is validated
  against flat, diagonal, sloped, water-overlap, bridge / tunnel, terminal, bend, junction,
  production authored DEM, and compact imported Kuopio DEM cases. The first imported DEM closure
  failure was fixed in the terrain-CDT ownership stage by removing only road-owned internal chords
  from the terrain seam constraint set after both sides classify against the final footprint.
  Phase 11 remains the deterministic `10 m` versus `5 m` characterization gate, Phase 12 is the
  fixed-roadbed-under-later-terrain-edits follow-up, and the hardcut road geometry target is now
  road-owned top surfaces plus Rust-stitched terrain topology rather than more polishing of the
  corridor-sheet prototype or visible closure meshes. That rewrite now has one explicit target
  split: the logical graph stays as connectivity/routing authority,
  while the visible road system becomes a separate deterministic piece/profile carrier built from
  `Span`, `Bend`, `Terminal`, and `JunctionN` pieces. The hard-cut carrier replacement is live in
  the road-surface runtime: renderer output, visible-surface queries, road-surface debug overlays,
  road-driven earthwork stamping, and clipped terrain topology all consume explicit visual pieces
  instead of a node-patch carrier.
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
  from exact canonical arrangement-key support instead of overlay-sibling edge matching. `JunctionN`
  final owned asphalt / curb step edges now materialize after boolean ownership from exact
  owner-pair source polyline authority before height validation / CDT export; missed
  source-authorized materialization now blocks with a canonical-keys diagnostic that names the
  final edge and source constraint. JunctionN final owned vertices now also evaluate through their
  post-boolean region-scoped band carrier, keeping same-material overlap conflicts local to the
  explicit owner instead of reviving the old node-wide grade sampler; same-height seam validation
  now keys separate materialized owner-pair seams independently even when they came from the same
  source rail index. Same-material carrier tie-breaks now require equal `SurfaceHeightMmKey`
  heights, so elevated multi-arm nodes with contradictory same-XZ carriageway owners reject
  deterministically until ownership selects one carrier before height sampling. Source-band height
  carriers now also reject one-sided explicit paths during height-field
  construction; any required opposite rail must already be materialized by the rail / topology
  stage with matching canonical path vertices before height evaluation. Source handoff and
  final-region support heights are now likewise materialized as explicit rail-owned `RoadVec3`
  support points before height-field construction, so height evaluation no longer interpolates
  along source edges to authorize contour support. Node footprint boundary
  export now resolves heights only from adjacent solved boundary provenance, with terminal
  raised-step corners accepted only when ordered source edges prove the material step. The render mesh
  payload now exposes those faces as `raised_step_*` buffers rather than curb-specific vertical
  buckets. Post-boolean `node_non_road`
  subdivision now requires every final curb /
  shoulder and sidewalk owned region to carry explicit profile seam-rail evidence, and
  carrier-only leftovers are reported as deterministic boolean-ownership residual diagnostics.
  Road geometry debug dumps now include final span / node top-region coordinates, post-boolean
  node owned-region contours and side-join trim provenance when capture is enabled, plus an
  opt-in road-surface probe for identifying the exact final triangle owner under a hovered XZ
  point.
  Span output now also routes through resolved top-region records and generic owner-pair
  raised-step constraints before exporting the existing render, query, terrain-clip, earthwork, and
  chunk-coverage fields, so span rendering is no longer the authority layer for material ownership.
  Road-touched terrain support now uses the lower
  road-owned top-surface envelope when grounded support overlaps terminal caps or raised bands, and
  bridge / tunnel earthwork ranges are class-aware so bridges do not stamp terrain while visible
  tunnel portals still stamp. Road-touched terrain CDT diagnostics now expose source
  samples omitted to widen over-steep cut / fill tie-ins, and `ROAD-03` keeps ordinary grounded
  `Standard` seams on the terrain path with `RoadSurfaceSystem` owned grade-limited guide samples
  around the final unioned road-owned footprint instead of retaining-wall teeth. Bridge abutments
  now retain the terrain material for emitted grade-compliant faces even when one nearby source
  sample must be omitted, rather than promoting the whole span boundary into triangular wall fans;
  actual over-budget bridge faces and portal-required tunnel sources retain explicit wall output.
  Convex single-loop footprints may constrain their guide rails; concave or multi-loop junction
  footprints stay sample-only so grading constraints cannot cross the roadbed. Synthetic DEM
  validation still covers structural retaining-wall classification while preserving exact road seam
  constraints.
  Production road-surface authored DEM coverage now also validates supportive spans, steep
  along-slope and extreme cross-slope spans, raised standard spans, raised terminals and bends near
  authored ridge / valley terrain, raised multiway junctions on flat and steep authored terrain,
  and edit-order-stable emitted terrain-CDT topology through final road-owned terrain loops.
  Production imported DEM coverage now bakes a compact Kuopio height window and validates ordinary
  lower-shelf tie-ins, grounded steep terrain, raised spans, raised terminals / bends, raised
  `JunctionN`, widened tie-in diagnostics, structural retaining-wall provenance, and
  edit-order-stable emitted topology through the same production path. `ROAD-02` generated helper
  hardening now covers mixed sidewalk / curb and no-sidewalk curb / shoulder profile modes across
  flat and elevated mixed-width 4-way / 5-way / 6-way `JunctionN` cases. `CODE-14` is now closed:
  road-surface long-lived geometry is `RoadVec2` / `RoadVec3` internally, Godot vectors are limited
  to graph/API input, render upload, debug output, and bridge adapters, and arrangement split
  vertices preserve source-owned height provenance at exact canonical split keys.
  The shared engineered-ground contract now lives in
  [`earthworks.md`](earthworks.md), with road-specific rules staying in
  [`roads.md`](roads.md) and terrain storage / chunking rules staying in
  [`terrain.md`](terrain.md).
- `ROAD-01` node earthwork visibility is now owner-scoped: mixed Standard / Bridge or visible
  Tunnel nodes retain Standard boundary roots for terrain/CDT, but render, query, and stamp only
  structural owner faces as visible earthwork.
- Added the first Rust-side terrain chunk loader in `rust/src/simulation/terrain/chunks.rs`,
  including strict `chunk.toml` validation and `.f32` payload loading for partial border chunks as
  well as full-size interior chunks. See [`terrain.md`](terrain.md).
- The legacy numbered backlog and bug table were preserved in the archive rather than kept half-live in the dashboard.
- `rust/benches/agent_benchmark.rs` now includes access-phase microbenchmarks for `ACCESS_EGRESS` and `ACCESS_INGRESS`, so old Criterion result history is no longer strictly apples-to-apples with the updated suite.
- Added a shared top menu scaffold across gameplay and editor scenes, with gameplay File/View/City/Tools/Help menus and reduced editor File/editor-action menus. See [`ui.md`](ui.md).
- Migrated the Building Inspector and SelectTool road-properties UI onto draggable Godot `Window` surfaces instead of custom anchored panels. See [`ui.md`](ui.md).
- Building Inspector now supports multiple simultaneous per-building windows and refreshes open inspectors on each in-game hour boundary. See [`ui.md`](ui.md).
- Added an in-game Pack Manager window through the gameplay `Mods` toolbar action. See [`ui.md`](ui.md).
- Asset Editor building assets now use building-only `[[mesh_parts]]` with per-part transforms and
  nested LOD entries, replacing the old top-level building `[[lods]]` contract. See
  [`asset_editor.md`](asset_editor.md).
- Reworked the zoning toolbar from one flat profile row into Residential / Commercial / Industrial family buttons with a second profile row above for the selected family. See [`ui.md`](ui.md).
- Added a compact bottom-left R/C/I demand meter beside the clock, driven by live normalized demand pressures from `SimulationNode`. See [`ui.md`](ui.md).
- Replaced the legacy `MapConfig` type with chunk-aware `WorldConfig`, added terrain chunk metadata to saves, added explicit `terrain_cell_m`, restored canonical metre-based world coordinates for terrain / water / zoning tooling, removed the old `10 km` versus `20 km` gameplay startup split, and moved terrain plus water runtime storage onto sparse chunk-backed buffers with dense materialization only at save/render boundaries.
- Added blank-world `WorldDefinition` persistence as a separate authored-world asset path, with deterministic SQLite metadata plus sparse-authored terrain chunk storage and runtime methods to create, save, and load blank worlds independently from city saves. See [`terrain.md`](terrain.md).
- Added the first `WorldEditor` launch mode and scene, with a reduced File/Help top menu, bottom terrain and water authoring toolbars, shared brush controls, on-map brush previews, a two-anchor slope brush workflow, and direct blank-world `WorldDefinition` create/open/save flows on the shared paused runtime. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Extended `WorldEditor` with authored baseline water: bottom-toolbar `Water` subtools for `Lake Fill` and `Open Water`, `WorldDefinition` persistence for inland lake fills and edge-connected open-water fills, and editor-only 3D markers for committed water features plus active surface-fill previews. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Removed the legacy dynamic water prototype: `Source` / `Sink`, dynamic depth, velocity, flux, source lists, and the low-rate runtime solver path are gone. `Lake Fill` / `Open Water` now rebuild flat baseline still water only, with shader-side waves kept as presentation. See [`terrain.md`](terrain.md).
- The current `Lake Fill` / `Open Water` workflow is now treated as the shipped water-authoring baseline; richer river-path or hydrology ownership is optional future work rather than a required next milestone. See [`terrain.md`](terrain.md).
- Reworked `WorldEditor` surface fills into a two-phase preview workflow: click once to seed a transient basin or open-water preview, adjust `Surface +m`, then use the dedicated `OK` / `Cancel` flow to confirm or dismiss it. Unconfirmed preview state is runtime-only and never serialized into `WorldDefinition`, and terrain sculpting now rebakes authored water so previewed/committed water reacts to basin changes. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Terrain rendering now adds procedural hillshade directly from the live heightmap in both gameplay and WorldEditor, so imported DEM worlds and hand-sculpted worlds get better relief readability without any separate hillshade asset pipeline. See [`terrain.md`](terrain.md).
- Terrain and water rendering now use the first render-only realism pass: slope-aware terrain coloring, shoreline-aware terrain tinting, macro terrain breakup, depth-aware water color, fresnel-style water highlights, and mild aperiodic procedural surface variation that does not expose repeating wave bands from high camera views, all without introducing authored material data or external texture requirements. See [`terrain.md`](terrain.md).
- Terrain coloring now follows a surface-classification-first and absolute-height-second model, so blank worlds and imported DEM worlds no longer depend mainly on one global elevation ramp for their palette. See [`terrain.md`](terrain.md).
- Added an offline DEM-to-`WorldDefinition` importer in `tools/import_dem_world_definition.py`, validated against the Kuopio `324 km²` Maanmittauslaitos `Korkeusmalli 2 m` tiles under `maps/raw/Kuopio/324km2/`, producing a ready-to-open authored world asset at `maps/processed/Kuopio/kuopio_324km2_10m.sqlite`. See [`terrain.md`](terrain.md).
- Water shoreline rendering on the existing `10 m` grid now derives its visible coast from the linearly interpolated live water field instead of whole-cell shoreline masks, giving contour-style diagonal coastlines and channels without a denser authored map. See [`terrain.md`](terrain.md).
- Terrain rendering on the existing coarse authored grid now also includes render-only cliff breakline / cliff band treatment derived from the live terrain field, improving steep cuts and man-made cliffs without changing authored world data or forcing a denser map. See [`terrain.md`](terrain.md).
- Terrain rendering now adds a render-only terrain-border skirt derived from the live terrain edge, with a side wall, bottom cap, contour continuation, irregular earth strata, restrained surface relief, and a shallow topsoil lip so the world reads as a visible slice instead of a paper-thin plane. See [`terrain.md`](terrain.md).
- Water rendering now also adds a render-only edge curtain where water reaches the map boundary, so outside views do not see straight through to the submerged terrain plane at the border. See [`terrain.md`](terrain.md).
- `TERRAIN-01` is now live: terrain and water rendering no longer use one whole-world mesh plus one whole-map dense runtime upload. Both renderers now consume chunk-local patch snapshots aligned to the terrain patch grid, terrain/water roots now own per-patch child meshes instead of a single mesh boundary, dirty patch uploads stay local, and the old whole-map Godot render bridge methods were removed from the steady-state render path. This makes the `10 m` versus `5 m` terrain-density decision measurable on the actual large-world render boundary instead of on the old overlay-era compatibility path. See [`terrain.md`](terrain.md).
- Earthworks cleanup note: the old whole-map terrain render boundary is no longer the active
  blocker for engineered ground. The remaining blocker is now the near-road representation itself:
  the current corridor-sheet prototype is retired, and [`earthworks.md`](earthworks.md) plus
  [`roads.md`](roads.md) now reset the target to a closed road-owned earthwork
  mesh carried by a separate piece/profile visual road layer rather than by graph-derived node
  fills.
- Terrain / water patch rendering now also uses deterministic distance-based mesh LOD on top of the
  split patch snapshot path, so far-field camera views can reuse the same resident patch snapshots
  without paying full near-field vertex density for every visible patch. The temporary seam /
  emissive terrain-debug visual modes used during patch-hardening were removed from the steady
  runtime after the seam-width bug was fixed. See [`terrain.md`](terrain.md).
- Water patch mesh topology now builds through async Rust/Rayon cache jobs keyed by patch, LOD,
  road-clip signature, and depth signature; Godot submits mesh requests in small time-capped
  batches, Rust owns the ready queue, and Godot polls completed buffers without resubmitting pending
  keys every frame. Uploads apply under a measured per-frame time budget with pending-job
  backpressure, stale road/depth signatures are rejected before `ArrayMesh` upload, stale queued
  water-mesh requests are compacted before submission, request/cache/job perf counters expose queue
  health, ready polling plus apply drains use a conservative headroom boost while backlog is high,
  and fully wet unclipped patches use indexed grid buffers instead of expanded per-cell triangles. See
  [`terrain.md`](terrain.md).
- Terrain shoreline/debug water sampling now reuses the Water renderer's resident patch depth
  texture binding instead of requesting a second terrain-aligned water snapshot and uploading a
  duplicate `ImageTexture` from GDScript. See [`terrain.md`](terrain.md).
- Gameplay world-load refresh now consumes the terrain, water, and network render-dirty flags after
  rebuilding the visible scene, so the first live frame no longer repeats resident terrain/water
  uploads that were already performed by the load coordinator. See [`terrain.md`](terrain.md).
- Terrain and water patch residency plus speculative cache prewarm now run under elapsed-time
  budgets with camera-prioritized patch order, and steady-state water residency follows the terrain
  resident-set revision instead of rebuilding desired patch lookups every frame. Terrain and water
  mesh-LOD refreshes and terrain-to-water texture sync are queued and drained under small per-frame
  time budgets; LOD refreshes are movement-gated and cap checked/changed patches per frame,
  movement-triggered LOD sweeps replace stale pending sweep entries instead of appending another
  full resident pass, only enqueue patches whose target LOD/subdivision differs from current
  state, activation removes far patches before adding new ones, and water mesh submit/poll/apply
  queues process camera-near work first so startup and camera motion favor visible activation
  before far cache warming or far-field LOD churn. See [`terrain.md`](terrain.md).
- The first roads-first engineered-ground prototype did useful architectural work but is no longer
  treated as the final path: later terrain edits can keep committed roads fixed, chunk-local
  rebuilds and visible-surface precedence remain required, and terrain / road ownership stays
  explicit, but the thin corridor-sheet visual carrier is now retired in favor of road-owned top
  surfaces, band-owned node geometry, and Rust-stitched terrain topology. See [`earthworks.md`](earthworks.md),
  [`roads.md`](roads.md), and [`terrain.md`](terrain.md).
- `ROAD-01` is now pinned to one deterministic target architecture: the next road geometry pass
  must stop treating the logical graph as the visible-shape carrier and instead compile a separate
  deterministic piece/profile geometry layer with `Span`, `Bend`, `Terminal`, and `JunctionN`
  pieces, while the elevated `Bend` / `JunctionN` hardcut replaces post-overlay height sampling
  with band-owned regions and `NodeBandHeightField` surfaces identified by
  `NodeBandHeightFieldId`. The existing graph / clip / lane ownership layers stay intact. See
  [`roads.md`](roads.md).
- The retired annulus/corridor prototype still produced useful conclusions that remain valid after
  the code revert: arbitrary-angle bends and multi-arm junctions need explicit road and sidewalk
  piece ownership, not one sampled outer loop plus one sampled inner loop with triangulation
  layered on afterward. See [`roads.md`](roads.md).
- Gameplay and `WorldEditor` now share one terrain-aware world-camera core in `CameraNode`, including a common terrain-clearance rule that keeps the camera above the terrain surface while preserving separate scene-level zoom and clip policy. See [`ui.md`](ui.md).
- Added release startup user-data bootstrap: the router creates `user://worlds/`, `user://mods/`,
  and `user://saves/`, then copies missing bundled starter entries from `res://bootstrap/worlds/`
  and `res://bootstrap/mods/` without overwriting user-owned files. See [`ui.md`](ui.md),
  [`terrain.md`](terrain.md), and [`asset_editor.md`](asset_editor.md).
- Added a dedicated `MainMenu` front-door scene and `LaunchState` startup handoff so normal launch no longer boots an empty fallback gameplay map. `New Game` now begins from `user://worlds/`, `Load Game` begins from `user://saves/`, and gameplay only opens after one of those selections. See [`ui.md`](ui.md).
- Gameplay `File -> New Game` now opens a `user://worlds/` picker and loads the selected `WorldDefinition` into the live gameplay scene, pausing immediately after the refresh. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Gameplay `Save` and `Load` now open file pickers rooted at `user://saves/` instead of using one fixed `savegame.sqlite` path. See [`ui.md`](ui.md).
- Added a compact city-status HUD panel between the clock and R/C/I meter for treasury balance and live agent count, backed by continuously refreshed snapshot values. See [`ui.md`](ui.md).
- **Pioneer demand floor removed**: the static 0.70 floor on `ResidentialGrowth`, `CommercialGrowth`, and admission pressure has been removed from `demand.rs`. Unemployment benefit now provides early-city bootstrap solvency through real economic activity.
- **Demand formula changes**: `ResidentialGrowth` no longer gates on `job_availability` (people can settle before jobs exist), and household pull now includes explicit regional migration pressure in addition to open jobs. `IndustrialGrowth` now uses the local industrial input-capacity deficit for active commercial inputs instead of `goods_shortage` or OWA import telemetry. `NonResidentialSpawnLimit` changed from `resident_presence` to `1.0` to break the commercial/industrial bootstrap deadlock.
- **Unemployment benefit and household starter tuning live**: `pay_unemployment_benefits` implemented in `households.rs`; unemployment, household starting budget/stock, household utility cost, and OWA utility costs are authored in `economy/profiles.toml` and validated by the runtime loader.
- **Building bankruptcy live**: two-day `budget_distress` check implemented in `households.rs`, `budget_distress: bool` persisted in SQLite schema.
- **Household economy cleanup**: deserted buildings are excluded from household supplier flows, forced OWA liquidation sells only unreserved inventory, utility providers must be staffed before providing local service revenue, and unemployment timers advance even when the treasury is empty.

## Reference

- Stable technical lookup data: [`reference.md`](reference.md)
- Live work tracker: [`roadmap.md`](roadmap.md)
- Historical numbered ledger: [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md)
