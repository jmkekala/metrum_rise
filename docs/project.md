# Metrum Rise — Project Dashboard

This file is the live dashboard for current state, priorities, and links to the owning docs. It is intentionally summary-first.

The old monolithic ledger and numbered backlog are archived in [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md). That archive is historical reference only, not the current planning source.

## Snapshot

- **Plot material consistency (`RENDER-13`, done)**: plot grass shares terrain's palette and
  settings; frontage, yards and sidewalks share paving sampling and lighting, with one
  road/yard shader body and material setup. Removed obsolete road-stripe logic alongside
  the old plot-grass implementation and duplicate tuning. Corrected native terrain export
  winding so frontage no longer shades as an underside; shared sky reflectance removes the
  remaining morning-shadow seam. Native export and real day-cycle rendering checks pass.
  Isolated frontage GPU time improves ~0.07 ms, with no measured cost from reflectance parity. [Validation and limits](terrain.md#plot-grass-and-frontage-materials--render-13).

- **Road markings at night (`RENDER-12`, done)**: crossings and lane lines now receive
  scene lighting instead of using an unshaded material. Rendered white/yellow paint checks
  pass; isolated matched 1080p GPU delta is under 0.01 ms. See [road lighting](roads.md#road-marking-lighting--render-12).

- **Window appearance (`RENDER-11`, done)**: saved **Model → Window brightness** control
  (0–10, default 3), brighter warm-white windows, stable variation between houses, AgX
  highlights and restrained bloom. Editor round trips and rendered gameplay/LOD checks pass;
  matched 1080p look fixture adds ~0.10 ms over SSIL alone. Tonemapping also softens daytime
  colours. [Contract and measurements](asset_editor.md#window-brightness-and-variation--render-11).

- **Window light spill (`RENDER-10`, done)**: visible lit windows illuminate nearby terrain,
  site ground and buildings through shared Forward+ SSIL. Existing schedules control spill;
  no per-window lights or city scans. Off-screen/hidden windows cannot contribute. Rendered
  receiver/schedule tests and editor regression pass; matched 1080p probe adds ~0.17 ms GPU
  on RX 7900 XTX (not a whole-city measurement). See
  [the spill contract and measurements](asset_editor.md#window-light-spill--render-10).

- **Building window emission (`RENDER-09`, done)**: deterministic
  house-by-house pre-sunset activation, residential bedtime/wake fades, all-night
  non-residential lighting and dark abandoned/unfinished buildings. Shared GPU schedules
  preserve batching and also drive Automatic in the asset editor. Release, rendered
  schedule/save-load checks and editor regressions pass; measured GPU cost is comparable. See
  [the runtime contract](asset_editor.md#gameplay-window-emission--render-09).

- **Building colour schemes (authoring and gameplay done)**: optional manifest schema, confirmed
  read-only discovery, explicit per-LOD material mapping, live preview across LODs and day/night,
  and draft/export/reopen round trips. Discovery can scan the original modelling folder, because
  publication copies only the albedo a model references. Gameplay selection (`RENDER-08`) derives
  an instance's scheme from its parcel and that parcel's redevelopment generation, so a street
  varies, a standing building never changes, and demolishing a plot rerolls both its colour and
  its spawned asset; spatial batches gain a scheme group, and Godot draws it with a
  whole-instance override or, for a multi-material tier, its own mesh, built through the same
  material code as the editor preview. See
  [the authoring contract](asset_editor.md#building-colour-schemes) and
  [the runtime contract](asset_editor.md#gameplay-colour-scheme-selection--render-08).

- **Gameplay building LODs (`RENDER-07`, done)**: spatial MultiMesh groups replace repeated
  per-asset city scans; Rust shares the editor's screen-size policy, variable chains and hysteresis.
  Group identity also carries the colour scheme (`RENDER-08`).
  Graphics → Building detail applies Performance/Balanced/Quality live. Tier resources are
  cached; unchanged views perform no LOD evaluation/upload. Lifecycle/fallback/selection checks,
  1,849 Rust tests, Godot regressions and rendered Kuopio review pass. Generated release GPU
  trials improved ~0.854→0.452 ms wide and ~2.480→0.224 ms street; switching adds bounded
  CPU/upload work. Automatic gameplay night windows and metre-field migration remain separate.
  See [the contract, measurements and limits](asset_editor.md#gameplay-building-lods--render-07).

- **Asset context menus (`TOOLS-07`, done)**: unified preview/list/library actions; explicit Rotate / R;
  cancellable creation/duplication, full-chain LOD edits and one-step undo. Rust prepares lossless
  commands and safe independent copies; original pack credits survive export, and recoverable
  Trash protects active/undo sources. Legacy handlers removed; group outlines rebuild once.
  Fresh acceptance: 19 Rust tests, all six editor suites, rendered light/dark/keyboard checks and
  matched release measurements pass; idle/picking/LOD costs remain comparable.
  See [the context-menu contract](asset_editor.md#context-menus-and-explicit-manipulation--tools-07).

- **Task-oriented asset authoring (`TOOLS-06`)**: type-first creation for seven building presets;
  Overview / Model / Site / Gameplay / Validate & export; contextual fields and read-only profile
  staffing; independent incomplete drafts, grouped undo/redo, dirty guards and confirmed conversions.
  Rust now owns authoring documents/history, drafts, pack/dependency publication and shared
  lot/polygon/LOD validation; duplicated GDScript backends and write-only state are removed.
  Precommit hardening fixes list selection, lossless LOD/draft editing and export validation;
  the unused direct-write exporter is removed. Fresh verification: 1,833 Rust tests, six asset
  suites, six adjacent bridge suites and four rendered editor suites pass.
  Startup now shows a welcome screen; assets open in Model with a bounded, collapsible 420-pixel
  inspector, contextual actions, and View → Reset layout. Library/log open on demand; missing meshes
  can be relinked and thumbnails survive publication. Toolbar/File **Export asset…** now opens the
  validation/destination review even with the inspector collapsed; draft open/save are secondary File actions.
  Model exposes the LOD chain and Add/Replace controls directly; clicking a tier previews it live,
  while Placement expands below. The read-only Source materials summary and its unused catalog
  are removed; preview emission and source materials are unchanged. Any tier can be relinked with undo/redo.
  Automatic highlights and targets the actually rendered LOD on mesh selection and zoom, without
  forcing it. Add/Replace and list clicks preview a tier temporarily; camera movement resumes
  Automatic so close-ups cannot stay stuck on an imported coarse tier.
  Replacing a LOD now updates the shared, saved mesh browse directory for subsequent pickers;
  picker layout saves preserve the newer directory preference.
  Projected-size diagnostics are tucked into collapsed LOD details; active LOD/triangles stay visible.
  Direct viewport clicks select any object and
  open its settings; visible anchor guides/labels take priority over the yard beneath them. Depth
  testing now hides frontage/site guides and hover outlines behind meshes; hidden anchors no longer
  intercept ordinary clicks, while Alt+click retains deliberate access to hidden targets. Triangle
  picking, hover feedback, Alt+click overlap cycling and thresholded drags remain; inspector tasks
  no longer restrict selection. Layout saves before tree exit, avoiding the detached-window error.
  The 1.8 m scale reference is directly selectable and freely draggable across the ground, including
  over yards/outside the lot; it preserves placement through preview rebuilds without touching asset history.
  Its toolbar switch has an opaque themed background so the label remains readable over the night sky.
  Generated authoring/startup/selection and rendered
  multi-size/100–200% UI-scale regressions pass. See
  [the contract and verification](asset_editor.md#task-oriented-building-authoring--tools-06).

- **Asset previews (`TOOLS-04`)**: placeholder scene templates are replaced by working day/night,
  emission and authored-LOD inspection. Window emission now follows preview time by default:
  Night lights supported windows automatically; manual material overrides remain preview-only.
  Comparison buildings retain textured, lit source materials with a subtle cool tint instead of
  the translucent blueprint override; their supported windows follow the same night controls.
  Mesh-part state, editor views and package I/O have separate
  owners; staged saves preserve complete LOD chains and external dependencies. Frontage/access guides
  share compact filled arrows, softer colors, fine outlines and lighter labels/handles. Roadside/traffic
  scenes remain deferred; gameplay building LOD switching is owned by `RENDER-07`.
  See [the editor contract and checks](asset_editor.md#working-building-previews-and-safe-packaging--tools-04).

- **Automatic asset LOD inspection (`TOOLS-05`)**: a shared Rust screen-size policy now drives
  cached automatic preview tiers, quality presets and temporary per-part inspection. Pixel/triangle
  readouts expose the decision; saved metre bands are preserved but do not drive the preview.
  Art calibration and schema migration remain separate work; spatial gameplay integration is
  owned by `RENDER-07`.
  See [the policy and verification scope](asset_editor.md#shared-lod-policy-and-automatic-inspection--tools-05).

- **Vegetation brush navigation**: the tool now owns the four brush options; Shift-wheel cycles them in sync with the dropdown, including while its popup is open. `E` and Remove share the erase toggle, and a fading world-space label names the option or erase mode above the ring. Ctrl-wheel sizing is unchanged. See [the tool contract](terrain.md#local-vegetation-experiment-v01).

- **Crown cohesion (`RENDER-06`, visually accepted)**: near foliage cores and cards now share smooth crown-volume lighting and stop assigning independent bright faces. Two-sided cards preserve authored normals on their backs; distant radiance is recalibrated to match. Geometry, placement, density and the atlas are unchanged. Fresh appearance, rendered LOD and wind checks pass; see [the change and measurements](terrain.md#crown-cohesion-candidate--2026-09-19).

- **Vegetation LOD material candidate (`RENDER-02`, partial)**: the distant shader now thins and darkens wood and filters its deterministic mask across derivative-selected octaves, because a lit trunk at a kilometre was the brightest thing on the mesh. A new apparent-height sweep from 64 down to 2 px then found the larger defect underneath: the near crown filled only 14.2% and 15.3% of its own silhouette, against 55.3% and 65.6% for the same meshes with opaque cards, so the baked foliage mask and not the geometry was removing three quarters of the canopy, and the mean near conifer read redder than it read green. Near fill also fell with distance, because a card's cluster vanished outright once a mip held one texel per cell. The mask now fills its atlas cell, carries denser clusters, and fills rather than vanishes when it stops resolving: near fill becomes 41.6% and 50.0% and no longer falls as a tree shrinks, which is the measured form of a forest that stopped gaining weight with every step the camera takes toward it. Those coverage steps then turned out to be measured through a blind spot: coverage is the fill of a crown's own bounding box and every level is rastered at one apparent height, so both of a crown's size terms are divided out, and the distant crowns were built at half the near crown's width because the lathe used the branch base radius where the near crown reaches past it and hangs cards beyond that. The distant lathe is now sized from the measured near foliage envelope, and the harness records `footprint` beside `coverage`. Ground cover at the switch, against the near crown, moves from 52.8%/47.1% to 101.7%/93.0% for conifer and from 47.2%/57.6% to 103.9%/88.4% for broadleaf. No triangle was added at any level for either species, and no range, instance, material or atlas changed anywhere in this work. Paired `E13` GPU captures on a GTX 1060 3GB then priced it: scatter cost moves by `-0.098` to `+0.224 ms` across five views, three of them negative, against a `5.8-6.9 ms` scatter and a `0.25 ms` within-trial spread, so the change is below the noise floor; the fill-bound 1.5x render-scale trial went down. Two of five views gained one draw call and at most `0.131%` primitives, from one more patch clearing frustum culling with the wider crown bounds, and video memory is unchanged. On screen the distant canopy darkens the ground it stands on by `42%` more than before. Patch granularity, distant wind and pine/variant collapse remain open. See [the measurements and verification limits](terrain.md#vegetation-lod-reference-measurements--render-02-2026-09-13).

- **Local experiment V04**: the vegetation branch adds a decorative tree preview to Kuopio. Tree patches now regenerate from the terrain surface generation, so a road or building edit clears the trees it covered without a world reload. An art-direction pass adds aerial perspective, dark water, bushes and rocks, and rebuilt scatter meshes. A fresh E13 capture on the GTX 1060 prices the scatter at 30-41% of GPU frame time from an eye-level 300 m horizon view, superseding the earlier 22% figure; the 9% laptop figure no longer applies and the laptop is still unmeasured. The scatter is vertex- and draw-bound rather than fill-bound: at 1.5x render scale its cost does not move while the rest of the frame nearly doubles. S1 adds deterministic near mesh variants (12/12/6/6) and a per-instance tint, sharing one material and leaving placement and LOD ranges unchanged. Patch upload now depends on patch distance: a patch past the near band skips the variant split, the tints and 36 of its 38 nodes, and the two crown levels share one instance whose mesh swaps at the mid boundary rather than a second uploaded copy. On a fixed headless fixture a near patch costs 18.0 ms and a far patch 2.9 ms, against 20.5 ms and 6.1 ms when every patch built every level. A near patch still exceeds a 60 fps frame and is not accepted; what is left is the placement loop, which rescans the payload once per species. Matched E13 runs across that change give identical draw calls, so the gating is an upload win only: Godot already culled the skipped instances by visibility range. Instance colour multiplies vertex colour, so no shader was needed; the tint has not been checked on screen. S2a replaces the near conifer and broadleaf crowns with a two-level branch skeleton and opaque foliage tufts, leaving the mid, far, bush and rock meshes alone. It costs 565 and 525 triangles against caps of 600 and 550, and 51 ms of startup against 31 ms. A matched E13 pair prices it: draw calls unchanged, primitives 4.34 M to 7.01 M at yaw 180, and the scatter from 30-41% to 42-51% of GPU frame time. The silhouette now reads as a branched tree rather than a cone, but opaque tufts hold less volume than the domes they replaced, so canopy mass and trunk occlusion both dropped and scattered trees show bare leader. S2b then adds three crossed alpha-scissor cards per branch tip as a second surface on the same mesh, sharing the uploaded transforms and adding no instances, with one procedural 128x128 mask and crown-outward normals. A third matched E13 run prices it at +0.6 ms: primitives rise only 4% while draw calls rise by 407, because a second surface is a second draw per near instance. Cut-outs are the cheaper way to add apparent foliage, and the crowns are visibly fuller for it. The scatter now costs 45-55% of GPU frame time. S3 then makes near trees and bushes sway, as a vertex displacement in two lean shaders sharing one include, with the per-plant phase read from the instance origin so no per-instance data is added and the sway weight authored into vertex colour alpha. Distant crowns and rocks stay rigid because their sway would be below a pixel. A fourth matched E13 run returns 0.27 to 0.60 ms with draw calls and primitives identical, because the two shaders replace StandardMaterial3D's general-purpose one on every near plant: the first change in this experiment that made the frame faster. Motion is verified by pixel-identical repeat captures with the scatter off against a 6% frame difference with it on. Still open: the conifer keeps a long bare trunk with foliage only in its upper half, and the wind's flutter term is per-plant rather than per-branch, so a crown translates rigidly instead of its tips moving independently. The palette is not yet calibrated and canopy density is unchanged. A baked 2x2 foliage atlas now supplies distinct broadleaf clusters and conifer sprays with offline coverage-corrected mips, preserving all geometry and sway weights; fresh headless catalogue medians are 65.026 ms before and 56.281 ms after. A forest-floor ground mask was tried and then removed: `woodland_field()` mirrored the scatter's `woodland()` in GLSL, but nothing linked the two at runtime, so the ground was coloured by a sine wave rather than by trees and the mask survived disabling the vegetation renderer entirely. `RENDER-05` now derives forest-floor land cover from the accepted generated and authored canopy: fixed 8 m R8 crown coverage replaces grass hue with litter, moss and duff at unchanged albedo luminance, while actual tree shadows supply occlusion. Coverage includes neighboring crowns and follows the existing terrain and vegetation revisions through reusable terrain texture stages. Understory density is quadrupled and bushes no longer cast shadows, measured together at `+1.89 ms` of scatter on `yaw180`. Quadrupling the canopy instead cost `+19.8 ms` and was rejected: scatter cost follows the square of range, and the canopy reaches `4500 m` against the understory's `420 m`, so the far band and not the near one is what blocks canopy density (`RENDER-04`). Birch now exists as eight of the twelve broadleaf variants with banded pale bark and its own atlas cell. Reference photographs then showed the land palette was systematically wrong in hue, not value: sunlit vegetation in the photographs measures hue `59-79` degrees against `98-115` for every authored green, and a fixed `3.00` chroma gain on the grass photo drove it to full saturation, which together produced the fluorescent open ground. The palette is rotated into the measured band, the gain is now `1.15`, topographic contour lines no longer draw outside the analysis overlays. The foliage palette is rotated with it, so trees no longer read bluer than the ground. The understory's six variants stop being one 3 m faceted cone and become boreal ground layer - blueberry and lingonberry mats, a grass tuft, a fern, prostrate juniper and one spruce sapling - at `36-64` triangles against a flat `64`, measured at `+7.26 ms` of scatter on `yaw180` against `+7.48 ms`. The generator's four hard-coded constants are now `VegetationConfig` - enabled, seed, stand coverage and canopy stems per hectare - chosen per world and persisted in the save at format version 60, with player edits now stored as a sparse cell-keyed delta in save format 61 (`VEG-01`). A tree is now addressable: a Terrain-submenu tool plants one species or clears plants, with a ground-ring cursor and drag strokes, and Rust's independent per-patch vegetation revision rebuilds only the touched patch without advancing any terrain generation. An untouched patch skips the edit store entirely through a coarse cell-block index, measured at 226.4 against 247.5 us for a canopy patch and 2.294 against 2.476 ms for its understory grid, with 100000 background edits elsewhere costing an unedited patch nothing. A road, building, terraform or committed farm field placed later now clears an authored plant as well as a generated one, re-derived on every fetch rather than stored, so removing the surface restores the plant. The tool is two controls, a species choice and a clear, because ctrl and the mouse wheel size the brush and the radius also selects the dispatch: at its minimum a plant lands under the cursor, and above it the disc fills a deterministic 4 m planting lattice (625 points/ha) plus generator candidates (`VEG-05`), bounded to a 256 m planting radius. The ground ring reports both, by size and by colour. Coverage only moves a threshold and is free; density sizes the grid cell and is clamped to four times the shipped grid, a renderer limit rather than a generator one (`RENDER-04`). The two-sine stand field is replaced by three octaves of value noise at a metric `500 m` stand size, because its wavelengths were fixed in metres and put the whole `500 m` sandbox world inside one stand. Scaling that size to the world instead was tried and measured wrong: global coverage stayed correct at `0.565` while the `2 km` window at spawn fell to `0.002` and an E13 probe lost half the resident trees, so local coverage is now pinned by a test. Those four parameters are now chosen in a new-game dialog that opens after a world is picked, with its defaults, its accepted density range and its grid-spacing readout all read from Rust so the renderer's ceiling stays one number; `start_new_game` carries them in, while `load_world_definition` keeps the shipped forest for the editor, the benchmarks and the probes. Every brush action is now reversible with `Ctrl+Z`: each edit records a bounded inverse journal of the cells it is about to change, sized O(changed cells) and allocating nothing for a cell the generator still owned, and a held drag mints a stroke id that merges its stamps into one history entry rather than filling the 30-entry stack with fragments of one gesture; the id rather than a first-stamp flag is what keeps a clear from attaching itself to the planting underneath it. An authored plant can now name the tree it is, not only its species: `AuthoredPlant` carries a renderer variant pinned over the appearance seed's own choice, biased so that zero is the unpinned case the generator and every earlier save write, persisted in save format 63 and packed into the existing lane five rather than a seventh float in the stride. Spruce and aspen were already in the world and unreachable, because there was nowhere to record which conifer or broadleaf the player meant. The brush is then driven by an eleven-entry preset table in Rust - four unpinned species, pine, spruce, birch and aspen, and three numbered mixes - each carrying the fraction of its 4 m lattice it keeps, its instance scale band and a weighted species mix, with `BRUSH_OPTIONS` only naming them. The brush offers nine of the eleven: the unpinned conifer and broadleaf stay in the table as what the generator plants, but are not offered, because once pine and spruce are both nameable a "conifer" is only an unnamed two-to-one mix of them. The first four ordinals plant exactly what they planted before. Density presets thin rather than densify, because the lattice is already above a real stand, and a stroke still clears nothing. Bulldoze now answers a vegetation target after its building and road queries, over a fixed 4 m cursor disc, and deletes that one plant through the brush's own clear rather than a new mutator, so it writes the same tombstone and takes one undo entry per click (`VEG-02`). See [the local experiment contract](terrain.md#local-vegetation-experiment-v01).

- **Day/night cycle (`RENDER-01`)**: scene lighting is now a pure function of the simulation's operational clock. A real solar-position solve drives the sun arc; the palette is keyed to solar elevation rather than to the clock, so sunrise and sunset are symmetric and golden hour, blue hour and a moonlit night all fall out of one curve. One key light carries the sun by day and an antisolar moon at night. Terrain, water and site ground follow through shader globals, so one write per frame relights every resident patch. New games now start at `07:30` instead of midnight. The cycle is cosmetic, so `Tools > Time of Day` now holds the light at `Morning`, `Midday`, `Afternoon`, `Evening` or `Midnight` while the clock keeps running, and `Follow Clock` returns to the cycle. Not yet done: no seasonal sweep, no street or window lighting at night, the pinned hour is not saved with the game, and the laptop cost is unmeasured. See [the lighting contract](terrain.md#14-terrain-site-ground-water-and-lighting-materials-are-runtime-presentation).

- **Scale target**: at least 1,000,000 total population across simulation tiers in one large world.
- **Simulation model**: full-FSM simulation stays inside the active area of interest; distant world regions are expected to degrade to coarser flow-field or aggregate simulation.
- **Current focus**: keep the playable small-to-medium city slice correct, deterministic, and scalable while the docs/planning cleanup continues and baseline water/render plus local road-build performance are hardened.

## Shipped Foundations

- **Third-road placement (`ROAD-25`)**: node boundary export preserves distinct submillimetre
  segments, closing the reproduced Kuopio terrain ownership gaps. Rejected exact previews retain
  a visible red fallback and explanation. Road drawing waits for simulation acceptance; rejection
  keeps the stroke editable. Road logging no longer enables the cyan surface overlay implicitly.
  See [`roads.md`](roads.md#third-road-boundary-and-placement-feedback-road-25).

- **Machinery upkeep (`ECON-09`)**: farms use 1 and coal mines 4 Machinery per hectare/day; food processors and utilities use building-level inputs. Paid OWA imports, small-consumer stock batches, startup funding and inspector stock/rates are integrated. Business/utility customers drive matching industrial demand. The audit unified restock/supplier logic, preserved small-shop shipment buffers, corrected net production headroom and city payroll/refund reporting, removed duplicate ledger/resource helpers and production constants, and rejected duplicate recipe ports. Depleted mines now stop creating jobs and Machinery demand; distress sales protect upkeep stock, and inventory fill counts shared input/output stock once. `machinery_factory_basic` is ready for the incoming four-worker industrial asset (Steel + Metals → 40 net Machinery/day); the production model itself is still user-created content. See [`economy.md`](economy.md#machinery-upkeep-econ-09) and [`asset_editor.md`](asset_editor.md).

- **Terrain reload and yard seams**: CDT cleanup now preserves distinct near-endpoint
  intersections instead of merging them by a dimensionless tolerance. This fixes the reproduced
  missing terrain patch without disabling mesh validation; terrain debugging also reports rejected
  patch publication. Shared-side sample welding now precedes containment in both adjacent tiles,
  closing the reproduced narrow commercial-yard gap without changing assets or flattening the
  graded apron. Contract revision 14 also fixes the `farms.sqlite` rejection: site and road guides
  share source heights, and building aprons grade both sides of tile boundaries.
  See [`terrain.md`](terrain.md).
- **Level building yards (`EARTH-02`)**: buildings and usable yard interiors share a level pad; 2 m lot-edge strips provide graded road/neighbor/terrain tie-ins. Zoned, service and explicit industry placement now share support preparation, installation and site-terrain invalidation. Mixed 2D/3D frontage projection and the explicit-mode raw-height fallback are removed. Paired zoned/service tests compare final site and terrain geometry; `farms.sqlite` now reproduces and verifies the corrected terrain-rejection path; the original unsaved view remains unavailable. See [`earthworks.md`](earthworks.md).
- **Road network and routing**: modular `RegionGraph`, lane system, CCH pathfinding, road rendering, border nodes, and roadway editing are all live.
- **Zoning and building allocation**: Rust-owned road-aligned parcels, parcel occupancy, roadside building placement, vacancy indexing, and no-build edge flags are live. Road hover previews both sides using the selected zoning options; one press creates the valid lots (`ZONE-03`). Manual drags and automatic fill share a layout anchored to existing parcels, respecting mixed lot widths and the selected gap. Slopes, bends and endpoint rounding retain the shared spacing corrections. Every density uses the same terrain check for the selected lot dimensions, independent of assets. Missing compatible assets permit terrain-valid zoning while growth waits for suitable content. Actual site failures use red preview feedback without cursor warning text. See [`zoning.md`](zoning.md) and [`building_allocator.md`](building_allocator.md).
- **Entrance-aware movement**: the building entrance/exit rewrite is implemented through the exact-plan system described in [`entrance_and_exit.md`](entrance_and_exit.md), including the Phase 1–6 and Phase 8 slices already verified against the live code. Cars and pedestrians share road/pad/graded-terrain height ownership in both access directions; source terrain no longer overrides cut or filled yards. Off-lane cars now use mesh-sized rigid footprint support, including after interpolation, so grade breaks cannot bury the sampled bottom contacts despite a correctly grounded centre. The shared Rust solver preserves horizontal heading and remains level on wholly flat pads; lane/access handoffs do not blend across height owners.
- **Benchmark coverage**: Criterion covers live agent access and isolated road kernels; the chunk suite measures generation and fixed-payload Godot upload. Release gameplay measurements separate paired flat layouts, remote-grid scaling, pointer interaction, and pinned saved-city edits; authored Kuopio replays and Samply runs provide diagnostics. Comparisons require matched inputs, complete generation-matched output, and independent process pairs. See [`roads.md`](roads.md).
- **Economy foundation**: household records, building-centric daily economy, physical truck freight jobs, `OWA` fallback, exact entrance-side freight routing/ETA, household transfer disbursement (unemployment, pension, and child support), two-day building bankruptcy, short private-building construction timers, baseline fiscal revenue (income tax, household VAT, business profit tax, and daily property tax), live `CityFiscalPolicy` controls/save state, first city-owned service-building placement/funding, explicit field-backed grain farms, aggregate `service_store` commercial services, and the starter `grain -> packaged_food -> household_supplies` chain are all live. See [`economy.md`](economy.md).
- **Demand foundation**: the live `DemandSystem` now fully owns immigration and building growth pressure, except for the explicit gameplay cheat mode documented in [`demand.md`](demand.md). RCI telemetry, household admission, and private building actions refresh hourly, while household removal remains daily. Private spawning now uses deterministic missing-building need; legal parcels cap placement rather than scaling the spawn rate. Household admission is driven by incoming household pull from bootstrap entry, budget-backed open jobs after existing unemployed adults are counted first, continuous forecast-only marginal commercial worker-equivalents from one candidate household after that same local labour pool is counted, and authored regional migration pressure; vacant homes only cap actual move-in execution. Job-driven admission prefers an adult-capable claimable household over a workerless front candidate. Regional migration requires an external road connection and is damped by household affordability, stock stability, failure state, and a soft household target. Residential construction reads that same incoming pressure plus move-in viability and failure-memory damping before creating more home capacity. Non-residential spawning is not hard-blocked by pre-existing full staffing; placed workplaces create budget-backed open jobs that pull households only for the remaining workforce shortfall, while output absorption prevents ordinary oversupply. Move-in acceptance now previews the exact candidate child/adult/elder composition and estimates candidate search runway from starter savings, budget-backed current jobs plus integer or fractional forecast-only marginal commercial worker-equivalents after existing unemployed adults are counted, unemployment/pension/child-support transfer reliability, and daily essential cost. Household removal now combines a crisis-ratio outflow rule with persistent exit for households that remain unhoused and destitute long enough. Daily city-flow diagnostics now summarize net household flow, active and theoretical job openings, resident employment, household failure state, vacant homes, and treasury in one economy log line. The static R/C/I pioneer demand floor has been removed entirely — real household transfers provide early-city solvency instead. Commercial demand now anticipates missing shop capacity before household stock collapses using short-run household buying power, and industrial demand uses business and utility input coverage, including Machinery upkeep. See [`demand.md`](demand.md).
- **Persistence and runtime**: SQLite save/load, background simulation thread, render snapshots, debug flags, asset editor, and economy editor are live. The asset editor now supports multi-part building assets, driveway/parking/loading-bay site anchors, WYSIWYG flat lot preview, and authored polygon yard surfaces for textured asphalt and concrete. Runtime building placement registers required flat support footprints at construction start, clips visual terrain through the shared terrain/CDT path, and keeps zoning terrain-neutral. Vehicle parking / freight stop behavior remain later runtime hooks.

- **Asset editor ground**: the preview now has flat terrain with a 10 m × 10 m grid
  continuing beyond the lot, using the game's terrain shader and shared grass textures.
  Unpainted lot areas use that same terrain material in both UI themes.
  It receives scene lighting and shadows; lot guides and authored
  surfaces remain above it. See [`asset_editor.md`](asset_editor.md).

- **Asset export save state (`TOOLS-06`)**: successful export clears the unsaved-change state,
  so immediately creating/opening another asset or closing the editor needs no draft prompt.
  Further edits remain guarded; failed exports preserve the dirty state and existing drafts
  remain untouched. See [`asset_editor.md`](asset_editor.md#task-oriented-building-authoring--tools-06).

- **Asset thumbnails (`TOOLS-06`)**: capture opens a framing mode that dims everything outside the
  crop, then writes a fixed 1024×768 WebP regardless of window size, so one image serves the editor
  list and in-game asset details. Captures retain the model, terrain and yard surfaces while
  omitting editor guides, labels, selection overlays, grids and comparison helpers. Visibility
  and interaction are restored after capture. See [`asset_editor.md`](asset_editor.md).

## Current Priorities

- **Codebase audit (`AUDIT-01`, paused by user)**: review the economy, buildings/save lifecycle, Rust/Godot boundary, and network/terrain for obsolete code, duplicated authority, correctness and scaling problems. Coverage and fresh validation are tracked in [`code_audit.md`](code_audit.md).
  The current passes correct household/freight accounting and persistence, indexed agent removal,
  deterministic demand totals, duplicated shopping helpers, commute-clock conversions and
  save-file preservation on failed writes, shared world-size/malformed-state validation and
  redundant graph rebuilds, allocating/overflowing asset-registry queries, and shared world-reset
  cleanup and startup defaults, stale building meshes and empty pack-selection handling,
  cached building-render requests and indexed building picks. Live service funding now rejects
  non-finite input, and obsolete random-building/test-fixture code is removed.
  Agent scheduling now uses the live world's saved day length through a shared clock reference.
  Daily agent updates run in parallel; pollution and noise share their diffusion code while
  preserving their separate coefficients. Redundant environmental fixture and API state is removed.
  Environmental overlays refresh daily, sample world-space cell centres, and rasterize rows in
  parallel. Single-row/column grids interpolate and clamp correctly; shader intensity and
  world-edge sampling now match those pixels, with unused shader controls removed.
  Live speed controls share Rust-owned steps, reject unsupported values, and preserve the HUD
  when a request is rejected. Saved clocks enforce the same 32× limit and authored 60-second floor.
  Mine area edits now preserve depletion, invalid reserves are rejected on save/load, and shared
  polygon queries replace three copies. Sparse-grid loading avoids default-only payload allocations
  and rebuilds chunks in parallel. Terrain/water diagnostics share their buffer reader, and refined
  terrain exports one height buffer; water exposes one source layer. Farms and mines share ordered
  owner remapping, with logarithmic mine lookups. Claim preparation uses measured parallel batches,
  and final-agent removal clears retained traffic. Lane tests now compare actual occupancy.
  Speed updates no longer keep a second agent-sized buffer; unused whole-agent clone/clear APIs
  are removed. Movement dispatch and tests share column construction; route repairs and watchdog
  recovery reuse the same state mutators. Access steps avoid a redundant segment lookup.
  Movement fixtures share setup and require completed junction crossings. Connector exits and
  lane changes now share reservations; fixed lateral moves choose deterministic owners before
  parallel movement. Resident, arrival and freight spawning share initialization, with obsolete
  spawn arguments and copied road-edit fixtures removed. Road-edit lane reattachment now uses
  deterministic nearest-distance ordering. Retired in-place edge compaction and its disconnected
  remappers/tests are removed; live save snapshots retain their existing mapping path.
  Demand uses resource-specific shortages throughout, with the obsolete aggregate fallback and
  retired labour rejection telemetry removed. Zoning profiles share their immutable cache and
  reject malformed colours and overflowing runtime IDs. Building removal and undo now update
  parcel occupancy through the existing ID index, without scanning unrelated parcels.
  Selected building actions reuse that ownership index and validate stale keys without copying
  every building into a temporary lookup table; repeated lifecycle defaults and fixtures are removed.
  Parcel geometry repair updates only affected chunk memberships and preserves pick order;
  temporary chunk lists and copied rectangle/query helpers are removed.
  Building-site radius maintenance keeps the exact maximum locally unless that maximum decreases;
  full reductions use Rayon, and discarded derivation/duplicate removal code are removed.
  Immediate road edits preserve unrelated buildings' rezoning grace; daily maintenance supplies
  elapsed days explicitly. Duplicate expiry coverage and obsolete demand/immigration fixtures are removed.
  Zoning reserves full explicit-building lots across chunk boundaries, using the existing lot lookup
  even when imported structures have compact support footprints.
  Family and variant selection share one deterministic parcel hash; fixed vectors replace the copied
  test algorithm, and repeated startup fixtures and unused helper arguments are removed.
  Site feasibility records local road dependencies on the fixed query grid, independent of render
  chunk size/origin; local road updates invalidate cached verdicts while remote updates retain reuse.
  Exact routing breaks equal-cost ties deterministically, uses junction-local meeting lookups and
  shares search-state updates and reconstruction buffers; unused A* and heuristic state are removed.
  CCH construction scratch stays local to the build, and its unused elimination tree is removed;
  measured retained storage falls 6.5–11.3% with approximately unchanged build/drop cost.
  Road/site materials no longer load unused displacement textures or discarded road-concrete
  normal input; all 36 render references match, with 48 MiB less texture storage across six materials.
  Vehicle turn whitelists no longer reject or detour walking routes; sidewalk connector checks
  remain authoritative. Finished routing hierarchies return spare construction capacity, with the
  measured cost of additional pedestrian alternatives recorded alongside the memory savings.
  Lane edits preserve untouched far-end connections and assign deterministic IDs. Full/local lane
  builders share construction, and local updates no longer scan the surrounding city's lane maps;
  repeated fixtures and two weaker tests are removed without losing their stronger coverage.
  CCH and flow fields now permit walking both ways along one-way roads and zero-lane footpaths,
  with shared direction masks and disjoint walking alternatives where needed. Five unused routing
  APIs and repeated fixtures are removed; measured compact flow caches remain in use.
  Node edits preserve independent profile heights, update self-loops once, and repair spatial
  entries, canonical aliases and routing costs. Local merges use adjacency instead of scanning
  the city; the retired edge-merging API and redundant split/length code are removed.
  Editor and border node selection share the existing spatial index and live-node checks, with
  deterministic ties and distinct horizontal/3D distance rules. Unused snapping/projection APIs
  and duplicate query helpers are removed; local selection no longer scans every city node.
  Road hovering excludes deleted roads and searches existing spatial bounds; thirteen obsolete
  zoning/geometry query methods and their unused caller chains are removed.
  Node queries account for hash-table capacity retained after index rebuilding, keeping small
  local queries independent of historical node storage.
  Selection owns the lane editor; its inactive duplicate and unused gesture fields are removed.
  Switching tools, clearing connections and consumed releases cancel pending gestures, while
  outgoing self-loop handles remain connectable. Control picks share one terrain hit per update.
  UI settings reject non-finite scale, discard partial parse results on recovery, and refresh
  fonts/windows from one settings snapshot while preserving later setting changes.
  The latest complete release run passes 1,765 tests;
  matched subsystem timings are recorded in the owning subsystem docs linked from the audit ledger. Remaining passes are
  paused at a validated checkpoint; resume instructions and persistent evidence are in `audit-state/` at the project root.

- Residential buildability now uses the same flat-site solver as explicit buildings. Demand selects
  among geometrically feasible assets; zoning previews reject unsupported lots visibly without
  changing terrain. Local dependency caching avoids repeat grading on unchanged sites. See
  [`building_allocator.md`](building_allocator.md), [`zoning.md`](zoning.md) and
  [`earthworks.md`](earthworks.md) for hillside regression coverage and verification.
  The follow-up audit aligns terrain picking with render acceptance, validates in-place level
  changes at fixed support heights, and removes stale asset-index membership and duplicate paving
  elevation/driveway preparation state.

For active tracked work, use [`roadmap.md`](roadmap.md).

- `ROAD-24` completed: RoadEditPlan's local topology/profile/site/terrain ownership, explicit readiness,
  atomic adoption, rollback and undo are implemented. Ownership is separate from elevation;
  approaches can rise inside junctions without shortening the 32 m transition. Terrain gaps
  are fixed at their source: shared-side grading uses uncut halo contributors, guide heights
  share one terrain authority, and metric constraint incidence and strict tile bounds prevent
  slivers. Missing/discarded terrain faces block paired publication in both Rust and Godot.
  The complete profile-finalizer dirty ledger now feeds both planning and commit. Load repairs
  only rejected grounded junctions using that same finalizer; saved node 34 and all 27 reference
  terrain patches compile, without changing the checked-in SQLite or valid nonincident profiles.
  Windowed and headless replays pass all 39 strokes / 158 profiles with zero audit failures;
  117 rendered captures retain the before/preview/committed evidence. All 1,634 Rust tests
  (3 ignored), 15 Python checks, five Godot suites and the 15-test single-worker replay pass.
  Populated-worker measurements pass three processes × 100 observations per level: growing
  remote state to 100,000 buildings / 600,024 agents changes worker p50 from 20.19 to 20.52 ms
  with identical local terrain; readiness stays below 0.019 ms. Source recovery and guide
  incidence now use bounded local queries. Three larger matched timing pairs (20 repetitions,
  three warmups) put second-T preview readiness at 50.89→53.98 ms and mixed-width crossing at
  67.37→69.70 ms, with first-idle effectively unchanged. This small cost for complete terrain is
  accepted explicitly; no general speedup or 60 FPS guarantee is claimed. Details and historical runs:
  [`roads.md`](roads.md#kuopio-terrain-regression-replay-road-24).
  The September 9 audit separates CDT pre-composition checks from final buffer acceptance,
  rejects missing/invalid render products in the common validator, and removes obsolete two-pass
  regrade APIs/telemetry. Rustdoc links and load/site/undo/readiness documentation are corrected;
  historical intermediate failures are explicitly separated from current acceptance.

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
  compact edge-local authority keys and spatially indexed compatible overlap candidates. Exact
  async road-preview validation now follows the matching commit across both the Godot and simulation
  thread boundaries, guarded by generation and complete input equality. That certificate now also
  carries immutable node-topology candidates: an exact node-local rail match reuses contact output,
  exact height identity reuses boolean ownership and the validated triangulated arrangement, and
  every mismatch takes the cold compiler path. Preview topology/profile solving now also consumes
  the same bulk split-edge dirty ledger as the matching simulation-thread commit, so close junction
  clusters retain all exact preview-produced span and node artifacts. Refined-terrain planning now
  sends only contributor-bearing tiles through Spade, lets the existing side-manifest-aware regular
  filler own empty neighbors, and shares each contributor's one exact grading-margin probe between
  coverage and guide generation. Pre-clipped road provenance now resolves exact component edges
  through a deterministic sorted index, while canonical CDT bypasses redundant source-vertex
  recovery only for loops whose source endpoints are already represented. Fresh release gameplay
  measurements pass; complete assembled node export buffers remain. Preview validation now seeds
  committed node-topology candidates through its local node map. Terrain tiles retain exact
  road-input clipping while resampling terrain/site grading. A controlled 48-tile release cache-hit
  benchmark reduced input assembly from 1.650 ms to 0.972 ms (1.70x); the headless gameplay matrix
  shows no clear end-to-end improvement. All 1,541 Rust tests pass; cache-hit timing is available via
  `cargo test --release benchmark_terrain_cdt_road_input_reuse -- --ignored --nocapture` in `rust/`.
  Road-tool UX now retains generation-checked edge targets with continuous projection, reserves
  fixed snaps for nodes, removes interior-knot snap gaps, and coalesces preview updates after fresh
  per-frame cursor sampling. Clicks use current pointer coordinates; fine movements cannot reuse
  an old exact preview. Cursor sweeps, input bursts, camera-only movement, stale targets/results,
  native cursor payloads, and click-before-frame behaviour have targeted regression coverage.
  Moving and settled previews now show shared asphalt/sidewalk textures and lane dividers.
  Valid placement is untinted; checking/rejection feedback remains amber/red. Coarse moving
  ribbons use display-only terrain draping with 15 cm clearance; completed paired previews use
  canonical unlifted road/terrain products. Prepared placement heights remain authoritative. Full-width hill,
  elevated-road, walkway-width, native payload, and rendered-material checks cover the change.
- `QA-01`: revalidate and root-cause the old long-run sim-thread panic.
- `WATER-01`: harden baseline-water rendering and remove remaining dense compatibility boundaries.
- `MOB-01`: ship bicycle support as the next transport mode.
- `ALLOC-01`: harden building allocator ownership and spec limits.
- `DOC-01`: finish replacing old numbered backlog references in live docs.

`QA-01` is now parked in [`roadmap.md`](roadmap.md): the old long-run sim-thread panic has not reproduced recently, including at least one overnight run, so it is no longer treated as an active blocker.

`ROAD-06` / `ROAD-07` are [parked historical reports](roadmap.md#parked-historical-reports),
not current blockers. Their original commit-safety gap is superseded by `ROAD-12` / `ROAD-24`;
reopening requires a current reproduction, not an assumption that the old geometry still fails.

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

- `EARTH-02`: fixed the missing farm-city terrain chunk caused by conflicting site/road guide
  heights and ungraded building aprons at tile sides. New Game advances terrain payload versions,
  resets camera framing and clears the old save filename. See
  [`earthworks.md`](earthworks.md#saved-farm-city-terrain-rejection-2026-09-11).
  Normal and debug cameras share terrain anchoring, initial framing, pan and zoom; debug only
  extends orbit pitch for upward views beneath terrain. Camera regressions compare both modes'
  zoom endpoints, terrain following, and transitions into and out of underground inspection.
  Cleanup centralizes camera setup and projection updates, removes empty native callbacks and
  obsolete API checks, and fixes orthographic zoom synchronization when distance bounds change.

- `ECON-08`: each farm provides one normal household slot alongside its area-scaled jobs.
  Resident adults can work on site without road trips; larger farms can hire commuters.
  Farm inspectors include resident age-group counts and shared household economy details.
  Field resizing preserves the family home. Audit fixes preserve farmhouse area through asset
  export, withdraw bankrupt vacancies, and keep families intact after automatic removal.
  Shared housing/worker rules and authored plot geometry replace duplicate and unused paths.
  The full changed-file audit also removes stale manifest-capacity fallbacks, shared-area math
  duplication and unused APIs; city diagnostics now include filled farm jobs consistently.
  See [`economy.md`](economy.md#farm-households-econ-08).

- `ECON-07`: fields reserve land against road, building and parcel placement, with matching
  checks when fields are created or resized. Farm inspectors now offer **Edit Field**; dragging
  existing vertices commits and recalculates on each valid release, with invalid moves restored.
  Drawing and resizing show the farm lot in yellow and the blocking building/yard in red.
  `ECON-06` keeps the existing staffing density with a minimum capacity of two workers per field.
  Saved fields retain reservations through load, building removal/remapping and demolition undo.
  Industry-tool cleanup runs on deactivation; inactive tools no longer reset every frame.
  See [`economy.md`](economy.md#field-placement-and-editing-econ-07) and [`ui.md`](ui.md).

- `EARTH-02` audit: rendering and structural support now share editor-consistent part yaw and
  allocator frontage transforms. Duplicate asset/lifecycle/bounds paths and the superseded
  centre-normal query pipeline are removed; vehicle footprint grounding remains active.
  CDT revision 13 invalidates old derived products. See
  [`earthworks.md`](earthworks.md#changeset-audit-2026-09-11).

- `EARTH-02`: small-lot frontage gaps now use actual imported model bounds instead of a fixed
  structural-pad radius. Buildings/usable yards remain flat; the surrounding terrain can join the
  sloping sidewalk. The reported shop passes 99 frontage probes and all eight terrain patches on
  read-only reload, without editing assets or the save. Pack reload invalidates derived sites and
  ground; CDT revision 11 excludes older cached pads. See [`earthworks.md`](earthworks.md#imported-structural-bounds-verification-2026-09-11).

- `ROAD-20`: road edits no longer rebuild a second whole-network ghost-snapping R-tree.
  Snapping streams local guides through the existing edge index without traversal/candidate buffers.
  Three matched release/headless pairs reduce side-32 T click-to-first-idle `199 → 141 ms` and
  snapshot work `40.6 → 6.1 ms`; warmed 112-edge first-road commits improve `54.1 → 39.5 ms`.
  Dense cursor queries cost about `0.10 ms` rather than `0.002 ms`; empty-network commit results
  remain inconclusive, and preview readiness is not uniformly faster. All 1,575 Rust tests, five
  Godot suites, single-worker junction replay and matched guide/graph/lane checks pass. This recovers
  commit responsiveness without attributing the historical regression to mesh-owner bookkeeping.
  See [`roads.md`](roads.md).

- `ROAD-23`: fixed compiled-preview terrain seams. Existing road coverage stays unlifted;
  transition triangles get explicit cutout contacts, and vacated terminal footprints receive
  temporary ground infill. Visual regressions now use actual clipped terrain rather than a solid
  plane: all nine fixtures have zero preview-only sky pixels, versus 142–355 before. All 1,573 Rust
  tests, five Godot suites and single-worker replay pass. Matched motion medians remain about
  19–37 ms; wide bends have fewer updates in two of three pairs, so this is a correctness fix,
  not a speedup. Terrain data remains unchanged. See [`roads.md`](roads.md).

- `ROAD-22`: compiled previews now include bends and straight continuations, with matching sloped
  terminal reprofiles. Hover validation no longer builds unused ribbons, and completed poses survive
  pending-check recovery. Isolated release comparisons reduce that validation step from `78 → 40 µs`
  (two lanes) and `135 → 56 µs` (eight lanes); total preview latency does not improve proportionally.
  Moving bends show `66–90` updates per 96 inputs at `18–19 ms` median new-mesh latency. All 1,569 Rust
  tests, five Godot suites and layout/rendered regressions pass. See [`roads.md`](roads.md).

- `ROAD-21`: junction previews now update throughout pointer motion, replacing the idle timer and
  unbounded request queue with one running job and latest-input coalescing. Unchanged retained road
  meshes are shared across CPU payloads/GPU instances; outdated display poses never authorize a click.
  Three headless captures show `40–72` updates per 96 moving inputs versus zero before, with median
  input-to-new-mesh latency `19–37 ms` (displayed pose age medians `20–50 ms`, not a 60 FPS guarantee).
  Pending checks preserve the visible junction; rejection/context changes retire it. All 1,566 Rust
  tests, five Godot suites, 48 matrix fixtures and interaction/scaling/saved-city/single-worker replays
  pass. The independent commit regression is addressed separately by `ROAD-20`. See [`roads.md`](roads.md).

- `ROAD-19`: the initial settled road preview added the compiled junction, connecting spans, sidewalks,
  curbs and markings using committed materials. Exact source-owner filtering preserves unrelated
  roads; cancellation restores resident meshes. Five cold-commit geometry comparisons and all
  1,562 Rust tests pass, plus four native Godot suites including water-only revision changes.
  Three matched process pairs put T preview readiness at about `67 ms`, versus `61–67 ms` for
  the former ribbon. The 48-layout matrix, interaction/saved-city/single-worker replays and rendered
  comparisons also pass. Side-32 preview readiness stays near `62 ms`, but T commit latency rises
  `182 → 207 ms`; the later `ROAD-20` follow-up recovers responsiveness. See [`roads.md`](roads.md).

- `ROAD-12`: road acceptance now requires complete road/terrain products before routing changes or
  charging, with bounded rollback of graph and split building references. Complete-boundary dust
  cleanup fixes the latest iso `(23, 1)` failure; the 52-road save rebuilds all five engineered
  patches through save/load. All 1,535 Rust tests and both Godot bridges pass.
  `ROAD-13` now fixes the connector's delete/redraw workflow: incremental preview retains valid
  required pieces despite unrelated frontier failures, duplicate detection ignores deleted roads,
  and bulldozing preserves distant lane identities. Exact/certified and uncached commits restore
  52 roads, six road chunks, and five terrain patches through save/load, also with one Rayon worker.
  All 1,555 Rust tests pass. See [`roads.md`](roads.md).

- `SIM-01`: terrain/water payload jobs no longer block Rayon workers on the simulation mutex.
  Busy snapshots use the existing retry protocol; completed terrain waits for nonblocking cache
  publication. The saved-city streaming replay now advances with power-plant previews on both one
  worker and the default pool. All 1,532 Rust tests and both Godot bridges pass. See [`terrain.md`](terrain.md).

- `ROAD-11`: terrain clipping resolves numeric-dust gaps using only their connected source anchors,
  so a distant curb step cannot block road/terrain publication. The iso save now produces all five
  engineered terrain patches and six road chunks on load, save/reload, and live road rebuilding.
  All 1,530 Rust tests and both Godot bridge regressions pass. See [`roads.md`](roads.md).

- `SAVE-01`: city saves include the active camera's position, orbit angles, zoom, and projection.
  Native load restores the view before renderer refresh; older saves remain loadable.
  See [`ui.md`](ui.md).

- `ROAD-10`: road/walkway mode shows parcel constraints, and every preview path checks the same
  local parcel corridor as commit. Span framing stays continuous across closely spaced profile
  samples while retaining exact node-mouth geometry. See [`roads.md`](roads.md) and [`zoning.md`](zoning.md).

- `BUILD-01`: explicit farms and service/industry buildings restore their saved fractional road
  frontage instead of moving to grid cell zero. Placement and load share the transform calculation,
  preserving the saved support plane and rebuilding site footprints at the correct position.
  See [`building_allocator.md`](building_allocator.md).

- `TRAFFIC-01`: saves preserve directed sidewalk lanes and junction connector endpoints through
  lane rebuilds and graph compaction. Load restores lane heights and entrance caches before agent
  references; version 57 saves recover junction routes from saved topology and positions.
  See [`traffic.md`](traffic.md) and [`entrance_and_exit.md`](entrance_and_exit.md).

- `ROAD-09` / `ZONE-02`: road/terrain clipping preserves exact tile boundaries and short canonical
  edges; save/load preserves road grades instead of resnapping roads to terrain. Curved parcel-run
  spacing validates its final attachment, and existing invalid endpoint parcels use the save
  quarantine lifecycle. See [`roads.md`](roads.md) and [`zoning.md`](zoning.md).

- Road placement no longer applies the Godot-only 90-degree spline guard or exposes the dead
  `Too steep` rejection; standard-road grade targets now shape and diagnose the generated profile
  without acting as player limits. Exact surface compilation remains a transactional integrity gate.
  Two logged ordinary continuation/T-junction regressions now compile reliably because canonical
  same-material band fragments retain profile authority across their ownership seams, while
  genuinely unanchored bands remain rejected. Bounded candidate validation also retains successful
  required artifacts when an unchanged excerpt-frontier node fails its cold context compile, rather
  than misreporting that unrelated failure as a road rejection. Terrain CDT source recovery now
  also preserves ownership for junction boundary edges shorter than `1 mm` when their endpoints
  occupy distinct canonical cells, preventing a successful commit from being hidden by the atomic
  terrain/road upload guard. See [`roads.md`](roads.md).
- Road benchmark schema 3 separates unprofiled release measurement from CPU/GPU diagnostics.
  Add `--gpu-profile` to a windowed road workload to retain Godot GPU stage timings in its log;
  runtime metadata marks the capture as profiled, excluding it from acceptance comparisons.
  The RX 7900 XTX / 1080p repeat with grass mipmaps and the window left stationary passes all
  48 fixtures in each run: median periodic GPU time is 0.630 ms; unprofiled road-operation frame
  intervals average 16.360 ms and reach 40.742 ms. Longer intervals therefore recur without
  reported desktop switching, but their cause remains unresolved: these are CPU callback
  intervals, not GPU presentation timings. This is an empty, paused road fixture. See
  [`roads.md`](roads.md).
  `./run.sh --benchmark-gameplay-roads[-headless]` defaults to 48 paired flat-terrain fixtures,
  resetting the world per case. Select `scaling` for fixed local edits with larger remote grids,
  `interaction` for prepared/dragged/immediate-click paths, or `saved` for pinned edits in a city.
  Authored Kuopio matrices remain diagnostic options. Metrics separate readiness, atomic render
  acknowledgement, idle settlement, frame intervals, and generation-matched command stages.
  Summaries no longer pool initial roads with junction edits or report tiny-sample maxima as p95.
  Strict paired reports reject mismatched inputs/settings and profiled captures. All Criterion
  targets compile again, real mutations complement no-op controls, and normal tests check benchmark
  API drift. Flat-world setup also fixed dirty flags with empty patch ledgers that prevented render
  settlement. The side-32 stress fixture now completes: local span footprint exports fix the
  `ROAD-14` integer-overlay overflow, preserving precision, authored endpoints, and full
  render/earthwork outlines. All 1,024 grid degrees and 1,984 edges validate before local edits.
  Contracts: [`roads.md`](roads.md).
  The first measured optimization reuses existing triangle grids for visible road-height queries.
  Across three matched release/headless process pairs, local T click-to-ready medians fell from
  `252 → 65 ms` with 112 background edges and `1140 → 139 ms` with 480; empty-network results
  remain within noise. Strict output checks also exposed and fixed dropped guides on lock
  contention (`ROAD-15`) and nondeterministic split-created edge IDs (`ROAD-16`). Both comparison
  binaries include those correctness fixes; guide counts and graph/lane cardinalities match.
  All 1,544 Rust tests, eight report checks, three Godot bridge suites, 48 paired layout fixtures,
  and interaction/authored-terrain replays passed for that first optimization.
  The next pass retains per-edge guide geometry against exact mesh/terrain dependencies and
  incrementally maintains CCH contraction scores. It also fixes discarded routing alternatives
  and premature query termination (`ROAD-17`). Three matched process triplets at side 32 show
  local T click-to-first-idle `3731 → 202 ms`, routing `3437 → 21 ms`, and an additional
  `359 → 202 ms` response reduction from guide reuse with the router held fixed.
  All 1,553 Rust tests and bridge/interaction/authored-terrain replays pass. These are headless
  road-edit measurements, not whole-game FPS. That pass's warmed empty-network first stroke
  regressed `36.2 → 41.2 ms`; subsequent T latency was unchanged. `ROAD-18` now removes the
  uninterruptible command sleep and publishes completed edit snapshots between fixed movement
  ticks. Three new matched warmed pairs improve first-road response `43.7 → 30.8 ms`
  (`16.1–36.3%` across pairs), with queue wait `7.33 → 0.079 ms`. Side-32 controls retain the
  large-grid gains with matching guide/graph/lane output; their additional speedup is inconclusive.
  All 1,557 Rust tests, eight report checks, three Godot bridge suites, 48 paired fixtures,
  interaction and saved-city replays pass. See [`roads.md`](roads.md) for measurement limits.
  Earlier schema-2 optimization passes removed duplicate exact preview/commit validation, cached
  target-group geometry and quantized ownership predicates, spatially indexed rail/seam coverage,
  eliminated repeated contour and source scans, and handed exact preview-produced junction
  rail/ownership/arrangement topology to the matching commit. On the same headless baseline
  three-repetition workload, total runtime fell from `14.28 s` to `9.84 s`; fixture p95 fell from
  `687/832/988 ms` to `422/442/442 ms` for bend/T/four-way, while commit p95 fell from
  `229/328/463 ms` to `96/83/83 ms`. Instrumented four-way commit compilation replays the
  junction rail stage in about `2.8 ms` and skips the prior height/arrangement/triangulation block;
  road-triggered terrain regeneration, road-mesh precompute, and terrain visual refresh were the
  dominant measured boundary in those fixtures. The matching windowed workload also passed: total runtime fell
  from `24.72 s` to `21.36 s`, four-way fixture p95 from `1,134 ms` to `823 ms`, and four-way commit
  p95 from `517 ms` to `244 ms`. The controlled matrix then exposed that the fixed `100 ms` exact
  preview debounce dominated every successful fixture and that an exact rejection did not replace
  the cheap synchronous verdict. Initially reducing the idle gate to `25 ms` (since removed by
  `ROAD-21`) and making the exact result
  authoritative cut controlled headless preview p50 by `52.7–60.5%`, fixture p50 by `29.1–40.2%`,
  and total measured runtime from `29.96 s` to `24.72 s`. The exact ROAD-08 curve then rejected in
  `41–75 ms` instead of appearing pending beyond `90 s`; current geometry accepts and commits it,
  and schema 3 verifies that successful outcome at the same site. A follow-up
  profile found that each exact preview cold-compiled every node in its bounded validation graph.
  Exact validation now seeds the incremental compiler with only required edges/nodes and their local
  incidence closure: sampled preview node-compiler CPU fell `18.1%`, the hardest double-T local
  compile fell from about `41 ms` to `27 ms`, and the full headless capture fell to `24.28 s`.
  Profiling then found that the double-T commit's bulk topology finalizer re-solved a wider split-edge
  ledger than its exact preview, invalidating three of five span artifacts and two of four node
  artifacts. Preview now replays that exact bulk ledger through shared scope helpers: the third
  commit reuses `5/5` spans and `4/4` nodes, its surface compiler fell from about `37.3 ms` to
  `0.69 ms`, and an isolated repeated third-commit p50 fell from `92.0 ms` to `83.0 ms`. The next
  downstream profile identified refined-terrain CDT as the commit bottleneck. Contributor-only CDT
  planning halves the first double-T patch from `16` windows / about `2,768` source samples to `8` /
  about `1,318`, and reusing its exact adaptive grading margin removes the duplicate terrain-probe
  pass. Per-contributor margin/guide construction and per-window clipping/sampling now execute as
  ordered Rayon jobs, with fingerprint and manifest aggregation remaining serial and canonical.
  Direct dense-layout instrumentation lowers refined input p50 from `10.677 ms` to `4.886 ms` and
  complete refined-worker p50 from `13.758 ms` to `12.428 ms`; the matching full controlled capture
  lowers measured commit-phase CPU samples from `1,842` to `1,545` and commit wall-time sum from
  `4,118 ms` to `3,858 ms`, with all 32 fixtures passing. A 12-repetition focused release run
  lowers aggregate double-T commit p50 from `82.875 ms` to `76.436 ms`; a clean release run also
  passes all 32 controlled fixtures. See
  [`roads.md`](roads.md) and [`reference.md`](reference.md).
- Committed roads now render as deterministic terrain-span-sized meshes instead of one
  full-network `ArrayMesh`. Rust rebuilds only the changed surface/earthwork chunk union, emits its
  unique owners once, assigns each triangle to one centroid-owned chunk, retains distant chunk
  buffers by immutable `Arc`, and publishes accumulated upserts/removal tombstones until an exact
  generation acknowledgement. Godot preflights the complete dirty terrain batch, stages changed
  road chunks as detached instances, and commits the matching terrain/road pair back-to-back; a
  failed engineered CDT retains the complete previous pair without acknowledging the road revision.
  World replacement clears old road chunks before rebuilding terrain, then hydrates an explicit
  full road snapshot. The road grid now starts at the terrain world minimum instead of
  making world zero a four-chunk corner, reducing a representative central edit to one chunk while
  retaining O(1) key lookup. `./run.sh --benchmark-road-chunks` replays fixed Rust generation and
  Godot upload workloads across increasing resident-chunk counts. See [`roads.md`](roads.md).
- Explicit grain farms now follow the coal-mine style placement flow: the player places the farm
  building, draws a nearby field polygon, and the saved field site gates renewable `grain`
  production without consuming a map resource deposit. `ECON-06` corrects grain-farm staffing from
  eight workers per hectare to one per ten hectares, rounded up with a two-worker minimum. Output stays at
  290 grain/day/hectare; staffing, demand, startup payroll, and export reserves share the new
  authored worker density, and the inspector shows total field capacity. The weaker `OWA` export bid
  remains a real external market when a connected outside freight gateway exists, so starter farms
  can advertise their area-scaled jobs even before local processing demand is large enough to absorb
  the crop.
- WorldEditor now has authored coal-deposit painting as a sparse terrain-aligned resource layer.
  `WorldDefinition` persists coal richness chunks separately from terrain and water, and the editor
  visualizes richer deposits as darker terrain-shader overlay data instead of mesh decals. See
  [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Explicit industry extractors can now bind to authored deposits: coal-mine assets use the
  `coal_mine_basic` extractor profile, place through the Industry toolbar, and attach a player-drawn
  extraction polygon within 10 m of the building footprint. The committed extraction area scales
  hourly output and physical worker capacity against the same 10,000 m2 baseline, while local input
  holds and lower `OWA` pricing keep local buyers preferred without suppressing gateway-backed
  export staffing. See
  [`economy.md`](economy.md) and [`ui.md`](ui.md).
- Release launches now default to a low-overhead crash-diagnostics recorder: `run.sh --release`
  sets `METRUM_CRASH_DIAGNOSTICS=1`, Rust installs a panic hook plus hang watchdog, and the sim
  thread records a fixed-size flight recorder of command, phase, and frame summaries that dumps to
  `logs/` on panic or watchdog-detected progress stalls. `METRUM_HANG_WATCHDOG_MS=0` disables only
  hang detection, and `METRUM_HANG_ABORT=1` aborts after the first hang dump.
  Foreground debug modes such as `--debug road`, `--debug perf`, and `--debug traffic` remain
  opt-in. See [`reference.md`](reference.md).
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
  targeting (`building` before `road`, then a plant when neither covers the cursor). Bulldoze and
  undo are queued onto the simulation thread so
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
  building-site CDT seams now prefer road-owned heights at shared XZ vertices. Repeated failed
  live network/access replans now trigger a watchdog recovery: housed citizens are returned inside
  `home_building`, while freight and immigrant carriers recover to a connected border node with
  traffic-debug evidence preserved. See
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
  detail fade. Runtime grass albedo/height imports now include full mipmap chains (`TERRAIN-02`),
  verified through the shared material loader. The stationary-window RX 7900 XTX repeat reports
  0.630 ms median periodic GPU time; both 48-fixture runs pass. Earlier before/after timings are
  qualified by reported desktop switching; see [`terrain.md`](terrain.md).
  Water uses a dark Baltic-blue depth palette with
  less terrain bleed and restrained
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
  now also dry-run the local post-split surface compile, including interior crossings against nearby
  road edges, so degenerate tight bends are rejected with a visible reason instead of landing as
  missing roadbed. Grounded `Standard` roadbeds that touch authored
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
  previews exact child/adult/elder candidate composition for move-in jobs, unemployment, pension,
  child support, and essential-cost viability rather than using a full-household or average-worker
  proxy. Starter household sizing now uses a deterministic admission mix, including one-person
  households, capped by `flat_size_m2` with adult and lighter child area weights; larger
  single-family homes permit larger families without forcing every arrival to fill the home. See
  [`economy.md`](economy.md) and
  [`demand.md`](demand.md).
- Baseline city fiscal policy is live: gross wage payments withhold income tax, household store
  pickup splits buyer-paid VAT from seller revenue, positive
  daily commercial/industrial budget growth pays business profit tax, occupied residential homes
  and active private commercial/industrial buildings pay daily property tax, and households may
  receive treasury-backed unemployment, pension, and child-support transfers. All tax and transfer
  call sites route through live
  `CityFiscalPolicy`; accepted policy changes are logged to the economy debug stream;
  `get_economy_overview()` exposes bounded Policy tab controls plus selected-control
  revenue/transfer detail; save/load persists policy state and separate tax/transfer ledger
  buckets. See
  [`economy.md`](economy.md).
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
  changes. Production follows the staffed workers and fuel availability. `CIV-01` is now marked
  done after a 39-day gameplay run validated coal power placement, coal freight, funding-sensitive
  production, local utility revenue, OWA fallback on under-coverage, and recovery to full power
  coverage after increasing funding. See
  [`economy.md`](economy.md).
- Freight shipments now dispatch physical truck carrier agents through the existing lane movement
  system. Local freight, `OWA` imports, and `OWA` exports settle only when the carrier reaches the
  destination building or border terminal; empty carriers then return to their source building or
  border base before being removed. In-transit freight now expires after a bounded timeout, refunds
  reserved buyer payments, restores dispatched source inventory when possible, and clears stale
  request failures after route-topology changes. The Godot renderer maps the freight vehicle type
  to `assets/models/vehicles/freight/delivery.glb`. See [`economy.md`](economy.md).
- Industrial exports now hold affordable local business and utility input demand before selling to `OWA`,
  repeated same-resource exports saturate to a lower outside bid, and commercial store jobs/input
  targets scale from the larger of recent household sales and local household demand/stock
  recovery instead of immediately using full authored capacity.
- Added `service_store` economy profiles for aggregate standalone commercial services, with starter
  barber and pharmacy assets bound to `personal_service_small` and `health_essentials_small`;
  personal-service demand now also creates capped representative visible visits without changing
  aggregate revenue accounting.
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
- Moved content-pack management into the shared Options window, available from MainMenu and gameplay `File -> Options...`; the gameplay construction toolbar no longer carries a `Mods` action. The Options window also now exposes `Graphics -> Fullscreen` and `Accessibility -> UI Scale`, persisted through `user://settings.cfg`; UI Scale applies immediately to scale-aware procedural UI, including inspector/economy detail text and eligible floating-window sizes. `settings.cfg` also stores restored `layout/<id>` window sizes/positions and Economy Overview split offsets. See [`ui.md`](ui.md).
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
  and `res://bootstrap/mods/` without overwriting user-owned files. Fresh profiles also seed
  `user://settings.cfg` for general options state and `user://active_packs.cfg` with the bundled
  `kenney` pack enabled; later saved pack selections, including an empty list, remain
  player-owned. See [`ui.md`](ui.md), [`terrain.md`](terrain.md), and
  [`asset_editor.md`](asset_editor.md).
- Added a dedicated `MainMenu` front-door scene and `LaunchState` startup handoff so normal launch no longer boots an empty fallback gameplay map. `New Game` now begins from `user://worlds/`, `Load Game` begins from `user://saves/`, and gameplay only opens after one of those selections. See [`ui.md`](ui.md).
- Gameplay `File -> New Game` now opens a `user://worlds/` picker and loads the selected `WorldDefinition` into the live gameplay scene, pausing immediately after the refresh. See [`terrain.md`](terrain.md) and [`ui.md`](ui.md).
- Gameplay `Save` and `Load` now open file pickers rooted at `user://saves/` instead of using one fixed `savegame.sqlite` path. See [`ui.md`](ui.md).
- Added a compact city-status HUD panel between the clock and R/C/I meter for treasury balance and live agent count, backed by continuously refreshed snapshot values. See [`ui.md`](ui.md).
- **Pioneer demand floor removed**: the static 0.70 floor on `ResidentialGrowth`, `CommercialGrowth`, and admission pressure has been removed from `demand.rs`. Real household transfers now provide early-city bootstrap solvency through normal household budgets and spending.
- **Demand formula changes**: `ResidentialGrowth` no longer gates on `job_availability` (people can settle before jobs exist), and household pull now includes explicit regional migration pressure in addition to open jobs. `IndustrialGrowth` now uses the local industrial input-capacity deficit for active business and utility inputs, with resource-specific output absorption. `NonResidentialSpawnLimit` changed from `resident_presence` to `1.0` to break the commercial/industrial bootstrap deadlock.
- **Household transfer and starter tuning live**: `pay_household_transfers` implemented in `households.rs`; unemployment, pension, child support, household starting budget/stock, household utility cost, and OWA utility costs are authored in `economy/profiles.toml`, initialized into live fiscal policy, and validated by the runtime loader.
- **Building bankruptcy live**: two-day `budget_distress` check implemented in `households.rs`, `budget_distress: bool` persisted in SQLite schema.
- **Household economy cleanup**: deserted buildings are excluded from household supplier flows, forced OWA liquidation sells only unreserved inventory, utility providers must be staffed before providing local service revenue, and unemployment timers advance even when the treasury is empty.

## Reference

- Stable technical lookup data: [`reference.md`](reference.md)
- Live work tracker: [`roadmap.md`](roadmap.md)
- Historical numbered ledger: [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md)
