# Metrum Rise — Project Dashboard

This file is the live dashboard for current state, priorities, and links to the owning docs. It is intentionally summary-first.

The old monolithic ledger and numbered backlog are archived in [`archive/project_legacy_2026-04-09.md`](archive/project_legacy_2026-04-09.md). That archive is historical reference only, not the current planning source.

## Snapshot

Graded terrain now stays visible with FSR 2 on macOS: complete cell-buffer publication fixes
`TERRAIN-10`, reproduced and verified under Metal and Vulkan/MoltenVK
([terrain renderer contract](terrain.md#godot-tile-renderer)).

The sky now has weather: volumetric clouds that drift on simulation time and are saved with the city, cloud shadows on every lit surface (terrain, water, trees, roads, buildings, cars and pedestrians) that darken the sun as fully as a tree's shadow down to sunset, and cloud reflections in the water ([RENDER-16](terrain.md)). At night, stars and a Milky Way turn with the clock behind the clouds ([RENDER-17](terrain.md)). The weather changes by itself on a seeded, saved schedule, and Tools > Weather holds a chosen scenario or Tools > Looks sets a time and weather in one click ([RENDER-20](roadmap.md)). Trees and water now share one wind that follows the weather and holds still while paused ([RENDER-18](terrain.md)), and on calm mornings mist gathers over the lakes and burns off by ten ([RENDER-19](terrain.md)). Storms come in gusts that bend whole stands of trees, roughen the lakes and roll wind waves across meadows and grain fields ([RENDER-22](terrain.md)); the first GPU grass draws near a low camera ([VEG-23](terrain.md#gpu-grass-prototype--veg-23)), and lakes reflect the stars and the Milky Way at night. The wind now wanders with each weather change, turning and changing speed, and a cirrus layer drifts above the cumulus on its own faster wind ([RENDER-25](roadmap.md)); the moon keeps a month, drifting east among the stars and waxing and waning ([RENDER-24](terrain.md)). Storms now rain: the deck darkens and the ground, roads and roofs go dark and wet; falling streaks and water rings were removed as fake until rain is done properly ([RENDER-23](terrain.md)).

The asset editor's **Preview** button now shows a building's yard as the game would plant it: real hedge modules and the trees, shrubs and flowers its planting areas grow, from the same Rust planners as a spawned house; **Reroll** shows another layout and **Exit preview** returns to editing, all without touching the asset ([TOOLS-14](asset_editor.md#yard-planting)).

Cars now turn around only where the lanes allow it: at dead ends, or by looping, for example
around a roundabout. Routes, commute quotes and same-junction trips follow that rule at both
trip ends, and routes saved under the old rules are replanned on load. The dumped city that
looped cars through impossible U-turns now replays with no trip failures
([NET-07/NET-08](entrance_and_exit.md#vehicle-u-turn-routing-contract-net-07-net-08)).

Lane acquisition now shares one mode/ownership contract across movement, road edits and debug
spawning; save/load use the same checks ([TRAFFIC-02](traffic.md#shared-lane-occupancy-contract-traffic-02)).
The saved-city pedestrian/vehicle-lane mismatch remains under investigation; the original
transition has not been reproduced.

Roads can again be attached beside an existing sloped three-way junction after loading a save.
Splitting an incident edge now retains its independently solved physical support knots, and the
topology-only junction finalizer adjusts those stations without rebuilding them from sparse control
geometry. All 11 refused strokes in the reported dump build independently against its original
network and installed asset packs (`ROAD-57`; see [`roads.md`](roads.md)).

Building assets can now line their yards with clipped hedges that a spawned house lays itself: rows cut at driveways and the entrance, corners closed and adjoining yards sharing one hedge, editable with the brushes and removed with the house unless the player changed them ([VEG-15](asset_editor.md#yard-hedges)). On sloping streets, hedges and yard plants now stand on the yard's graded lawn instead of sinking into it ([VEG-21](terrain.md#authored-trees-silhouette-shadows-yard-plants-and-rocks-2026-09-27)), and front hedges meet a neighbour's corner straight and without a gap ([VEG-22](asset_editor.md#yard-hedges)). Assets can also mark lawn planting areas that each spawned house fills with its own trees, shrubs or flowers, where players can plant trees too ([VEG-17](asset_editor.md#yard-planting)); six Finnish yard flowers can also be planted with the brush ([VEG-20](terrain.md#authored-trees-silhouette-shadows-yard-plants-and-rocks-2026-09-27)), and grow a back yard without moving the street side ([TOOLS-12](asset_editor.md#back-yards)). Hedge rows drawn with the brush now join at corners and snap to hedges already standing ([VEG-13](roadmap.md)), and hedges stay drawn out to the trees' far range as one box per straight run ([VEG-14](terrain.md)).

Trees, yard shrubs, clipped hedges and rocks are now Blender models; every tree casts its own baked silhouette in place of the old per-species lathe blob, and hedges are laid as rows with a line tool ([VEG-09 and VEG-10 details](terrain.md#authored-trees-silhouette-shadows-yard-plants-and-rocks-2026-09-27)). The brush now thickens gradually: each click or held tenth of a second adds 10 trees/ha until the preset's density ([VEG-11](terrain.md#gradual-brush--veg-11-2026-09-27)). From a distance, lone trees no longer sit on blurred forest-floor blots, far crowns grow up to 1.6x past the shadow range, and the far ground stand-in matches the authored trees' colour and crown-to-crown texture ([details](terrain.md#forest-from-a-distance-2026-09-28)). A far tree now renders within 12% of its near level at every tested pose: small pine crowns no longer vanish from its baked mips, and it catches the near crown's sun sheen ([VEG-12](terrain.md#impostor-refit-to-the-authored-trees--veg-12-2026-09-29)). Each impostor quad is now cropped to what its baked views draw, which takes a dense painted stand from `39.9` to `28.8 ms` of GPU at eye level ([details](terrain.md#impostor-quads-cropped-to-their-views-2026-09-30)).

Vegetation painting now uses live, clumped darts with separate tree, ground-cover and rock spacing, repeat-stamp stability, and single-plant bulldozing ([VEG-05 details and verification](terrain.md#natural-brush-proposals-and-independent-occupancy--veg-05-2026-09-26)).

- **Vegetation grid and near canopy (in progress)**: the vegetation scatter no longer rides the
  terrain grid. A two-tier grid subdivides only ground inside the near canopy and understory, and
  the per-frame staleness sweep caches its generation reads per owner. In a generator-density
  forest that took resident patches from 2518 to 415 and frame time from `27.66 ms` to `13.01 ms`
  with the GPU unchanged. The near band is back at `800 m` after `200 m` proved to be tuned on a
  brush-painted worst case. Foliage cards are cropped to their own alpha bounds, worth about
  `1.1 ms` of GPU from inside a dense stand. Past `SHADOW_PROXY_M` trees cast from the lathe
  crown rather than from the branched tree, worth `19.6 ms` of a close dense forest frame;
  inside it the branched tree still casts, because a proxy shadows the crown it stands in.
  Subdividing the near grid to 8 is worth `10 ms` of GPU and was rejected: it costs more than
  that in CPU spikes while the camera moves, because the residency sweep scales with the square
  of the subdivision. A third canopy level sits between the branched tree and the lathe: it
  keeps every foliage card and drops the interior wood, which is `41%` to `50%` of the near
  triangles and `25.8 ms` of a close painted-stand frame. Each tree dissolves from the full
  into the reduced level by its own distance over `45-55 m`, dithered like the impostor
  handover; a patch the band crosses draws both levels from one mesh. It replaced a whole-patch
  swap on the patch-centre distance, which popped trees `15 m` from the camera during an orbit. Past `250 m` every tree is now a
  hemi-octahedral impostor baked offline from the near tree, and the lathe only casts shadows.
  The painted stand drops from `40.0` to `23.6 ms` of GPU in the stand and from `52.0` to
  `16.5 ms` from the air. Trees stop receiving cast shadows past `120 m`, where every caster
  is a solid proxy that shadowed the trees inside it. Each tree now hands over to its own baked
  impostor by its own distance, dithered over `150-200 m`, instead of a patch at a time. A tree
  now keeps its form when its terrain patch changes between the fine and the coarse patch grid,
  and changes its shadow caster by its own distance rather than its patch's. The impostor is
  now lit by each tree's own yaw; see
  [the impostor lighting](terrain.md#each-impostor-is-lit-by-its-own-yaw-2026-09-25). Past
  `4.5 km` the ground stands in for the trees at their seen density; see
  [the far range](terrain.md#the-ground-stands-in-for-the-trees-past-the-far-range-2026-09-25).
  The forest floor now loses the sky its crowns hide, so a stand's floor in shade is no longer
  lighter than the meadow; see
  [the floor sky](terrain.md#the-forest-floor-loses-the-sky-its-crowns-hide-2026-09-26).
  Past the shadow range the floor now keeps the sun the open crowns pass and takes its shade
  down the sun ray, so a stand edge no longer turns black at `420 m`; see
  [the far floor](terrain.md#the-far-forest-floor-matches-the-shadows-it-replaces-2026-10-01).
  Pine, birch and aspen leaves are calibrated against forest photos, so a stand from the air
  is no longer nearly as bright as the meadow; see
  [the leaf albedo](terrain.md#leaf-albedo-of-pine-birch-and-aspen--calibrated-against-photos-2026-10-01).
  Crowns inside a stand now lose sun to their neighbours by a per-tree stand closure, which
  brings a stand from the air to the photographs' range and leaves lone trees unchanged; see
  [the stand sun](terrain.md#crowns-in-a-stand-lose-sun-to-their-neighbours-2026-10-01).
  Leaves no longer take a sun highlight, so crowns facing the sun stay green instead of turning
  white and flat; see [the leaf highlight](terrain.md#leaves-take-no-sun-highlight-2026-10-02).
  The pine is rebuilt to the Scots pine photo set, with longer crowns and real pine bark; see
  [the pine rebuild](terrain.md#pine-rebuilt-to-the-photo-set--veg-18-2026-10-03).
  Past the handover a tree's trunk zone is shaded, so a far stand no longer floats on lit
  trunks; see [the far stand](terrain.md#a-far-stand-is-dark-under-its-crowns-2026-09-26).
  Terrain residency now covers the cull distance in every direction, so a quick turn no longer
  builds the distance in view for about 2.5 s; terrain and water patches cull by their own
  footprint, which keeps the draw count under its old value (terrain.md, streaming list).
  See [the grid fix](terrain.md#a-tree-keeps-its-form-across-the-patch-grids-2026-09-25),
  [the per-tree handover](terrain.md#each-tree-hands-over-to-its-impostor-on-its-own-distance-2026-09-24),
  [the shadow fade](terrain.md#trees-stop-receiving-the-shadows-of-the-proxies-they-stand-in-2026-09-23),
  [the impostors](terrain.md#distant-trees-are-impostors-of-the-near-tree-2026-09-22),
  [the intermediate canopy measurements](terrain.md#the-branched-tree-at-half-the-wood-is-worth-26-ms-2026-09-22) and
  [the shadow proxy and the measurements behind it](terrain.md#trees-cast-from-the-lathe-crown-not-from-the-tree-2026-09-22).
- **Road preview modes (`ROAD-29`, done)**: Options → Gameplay selects Road only
  (faster, default) or Road and terrain. Changes apply immediately; full preview uses reversible
  terrain rendering and the same placement rules. Tests and rendered checks pass; release deployed. [Contract](archive/roads_legacy_surface_2026-10-23.md#player-selectable-road-preview-modes-road-29).

- **Junction preview refactor (`ROAD-28`, done)**: hover builds road/junction geometry;
  terrain reconstruction waits for click. The locality correction preserves connected roads
  outside junction/profile transitions, removes the blanket preview lift and reuses exact
  committed/previous-cursor geometry. A matched native moving-preview fixture improves flat
  median 20.2 → 17.1 ms and sloped 34.4 → 24.6 ms; genuinely changed junctions still need solving.
  Repeated identical-input worker cost stays near 1 ms through 100,000 background buildings;
  snapshot cost is measured separately. All 2,012 active Rust tests, four headless suites,
  rendered junction checks and Rustdoc pass. Release extension deployed. New geometry may
  intersect unchanged terrain during hover. See [scope, measurements and limits](archive/roads_legacy_surface_2026-10-23.md#junction-preview-without-terrain-reconstruction-road-28).

- **Road-generated cell zoning (`ZONE-04`, done)**: 10 m square cells extend six rows on
  both sides of eligible roads. Compatible orthogonal junctions share a grid; curves and
  competing roads retain non-overlapping cells. Existing paint/buildings remain fixed when
  incompatible grids meet. Cells/Parcels controls expose Cell, Marquee, Fill and Brush with
  separate Erase; fill follows connected same-profile cells on one local grid.
  Manual parcels coexist through shared reservations. Asset-sized lots use ordinary demand,
  growth, redevelopment and demolition/undo. Save v67 preserves both workflows and loads older
  parcel saves. The release extension is deployed locally. Final verification: 2,004 Rust tests,
  four headless bridge suites, eight rendered layouts and warning-free Rustdoc.
  Post-handoff testing removed whole-grid dominance: shallower rows retain priority so both
  roads keep their frontage, while paint/erase preserves the displayed layout. Empty cells
  cut off from rectangular road-frontage support are removed after overlap resolution.
  Adding a junction now preserves the uninterrupted backside grid of the original straight
  road; endpoint coverage and split/restore phase handling are corrected. Fresh release
  verification passes 2,008 Rust tests and both zoning bridge suites. The earlier rear-cell
  support check added 25–27 µs to the small straight fixture.
  Road options now include a left-side Tool mode / Snapping panel. `Snap to zoning grid`
  aligns straight-road placement with nearby curb frames and road widths through the checkbox
  only. Grid directions attract the cursor nearby while other angles remain free. The Shift
  shortcut is removed. Road connections remain enabled.
  Fresh validation: 2,013 release Rust tests and the road-tool bridge suite pass; the updated
  extension is deployed. Locality measurements are recorded in the zoning document.
  Straight endpoint extensions now inherit both curb phases, closing gaps when the original
  length is not a cell multiple. All 2,016 Rust tests and both zoning bridge suites pass;
  the updated extension is deployed.
  See [feature handoff](zoning.md#feature-handoff--zone-04).

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

- **Local experiment V04**: the vegetation branch adds a decorative tree preview to Kuopio. Tree patches now regenerate from the terrain surface generation, so a road or building edit clears the trees it covered without a world reload. An art-direction pass adds aerial perspective, dark water, bushes and rocks, and rebuilt scatter meshes. A fresh E13 capture on the GTX 1060 prices the scatter at 30-41% of GPU frame time from an eye-level 300 m horizon view, superseding the earlier 22% figure; the 9% laptop figure no longer applies and the laptop is still unmeasured. The scatter is vertex- and draw-bound rather than fill-bound: at 1.5x render scale its cost does not move while the rest of the frame nearly doubles. S1 adds deterministic near mesh variants (12/12/6/6) and a per-instance tint, sharing one material and leaving placement and LOD ranges unchanged. Patch upload now depends on patch distance: a patch past the near band skips the variant split, the tints and 36 of its 38 nodes, and the two crown levels share one instance whose mesh swaps at the mid boundary rather than a second uploaded copy. On a fixed headless fixture a near patch costs 18.0 ms and a far patch 2.9 ms, against 20.5 ms and 6.1 ms when every patch built every level. A near patch still exceeds a 60 fps frame and is not accepted; what is left is the placement loop, which rescans the payload once per species. Matched E13 runs across that change give identical draw calls, so the gating is an upload win only: Godot already culled the skipped instances by visibility range. Instance colour multiplies vertex colour, so no shader was needed; the tint has not been checked on screen. S2a replaces the near conifer and broadleaf crowns with a two-level branch skeleton and opaque foliage tufts, leaving the mid, far, bush and rock meshes alone. It costs 565 and 525 triangles against caps of 600 and 550, and 51 ms of startup against 31 ms. A matched E13 pair prices it: draw calls unchanged, primitives 4.34 M to 7.01 M at yaw 180, and the scatter from 30-41% to 42-51% of GPU frame time. The silhouette now reads as a branched tree rather than a cone, but opaque tufts hold less volume than the domes they replaced, so canopy mass and trunk occlusion both dropped and scattered trees show bare leader. S2b then adds three crossed alpha-scissor cards per branch tip as a second surface on the same mesh, sharing the uploaded transforms and adding no instances, with one procedural 128x128 mask and crown-outward normals. A third matched E13 run prices it at +0.6 ms: primitives rise only 4% while draw calls rise by 407, because a second surface is a second draw per near instance. Cut-outs are the cheaper way to add apparent foliage, and the crowns are visibly fuller for it. The scatter now costs 45-55% of GPU frame time. S3 then makes near trees and bushes sway, as a vertex displacement in two lean shaders sharing one include, with the per-plant phase read from the instance origin so no per-instance data is added and the sway weight authored into vertex colour alpha. Distant crowns and rocks stay rigid because their sway would be below a pixel. A fourth matched E13 run returns 0.27 to 0.60 ms with draw calls and primitives identical, because the two shaders replace StandardMaterial3D's general-purpose one on every near plant: the first change in this experiment that made the frame faster. Motion is verified by pixel-identical repeat captures with the scatter off against a 6% frame difference with it on. Still open: the conifer keeps a long bare trunk with foliage only in its upper half, and the wind's flutter term is per-plant rather than per-branch, so a crown translates rigidly instead of its tips moving independently. The palette is not yet calibrated and canopy density is unchanged. A baked 2x2 foliage atlas now supplies distinct broadleaf clusters and conifer sprays with offline coverage-corrected mips, preserving all geometry and sway weights; fresh headless catalogue medians are 65.026 ms before and 56.281 ms after. A forest-floor ground mask was tried and then removed: `woodland_field()` mirrored the scatter's `woodland()` in GLSL, but nothing linked the two at runtime, so the ground was coloured by a sine wave rather than by trees and the mask survived disabling the vegetation renderer entirely. `RENDER-05` now derives forest-floor land cover from the accepted generated and authored canopy: fixed 8 m R8 crown coverage replaces grass hue with litter, moss and duff at unchanged albedo luminance, while actual tree shadows supply occlusion. Coverage includes neighboring crowns and follows the existing terrain and vegetation revisions through reusable terrain texture stages. Understory density is quadrupled and bushes no longer cast shadows, measured together at `+1.89 ms` of scatter on `yaw180`. Quadrupling the canopy instead cost `+19.8 ms` and was rejected: scatter cost follows the square of range, and the canopy reaches `4500 m` against the understory's `420 m`, so the far band and not the near one is what blocks canopy density (`RENDER-04`). Birch now exists as eight of the twelve broadleaf variants with banded pale bark and its own atlas cell. Reference photographs then showed the land palette was systematically wrong in hue, not value: sunlit vegetation in the photographs measures hue `59-79` degrees against `98-115` for every authored green, and a fixed `3.00` chroma gain on the grass photo drove it to full saturation, which together produced the fluorescent open ground. The palette is rotated into the measured band, the gain is now `1.15`, topographic contour lines no longer draw outside the analysis overlays. The foliage palette is rotated with it, so trees no longer read bluer than the ground. The understory's six variants stop being one 3 m faceted cone and become boreal ground layer - blueberry and lingonberry mats, a grass tuft, a fern, prostrate juniper and one spruce sapling - at `36-64` triangles against a flat `64`, measured at `+7.26 ms` of scatter on `yaw180` against `+7.48 ms`. The generator's four hard-coded constants are now `VegetationConfig` - enabled, seed, stand coverage and canopy stems per hectare - chosen per world and persisted in the save at format version 60, with player edits now stored as a sparse cell-keyed delta in save format 61 (`VEG-01`). A tree is now addressable: a Terrain-submenu tool plants one species or clears plants, with a ground-ring cursor and drag strokes, and Rust's independent per-patch vegetation revision rebuilds only the touched patch without advancing any terrain generation. An untouched patch skips the edit store entirely through a coarse cell-block index, measured at 226.4 against 247.5 us for a canopy patch and 2.294 against 2.476 ms for its understory grid, with 100000 background edits elsewhere costing an unedited patch nothing. A road, building, terraform or committed farm field placed later now clears an authored plant as well as a generated one, re-derived on every fetch rather than stored, so removing the surface restores the plant. The tool is two controls, a species choice and a clear, because ctrl and the mouse wheel size the brush and the radius also selects the dispatch: at its minimum a plant lands under the cursor, and above it the disc fills a deterministic 4 m planting lattice (625 points/ha) plus generator candidates (`VEG-05`), bounded to a 256 m planting radius. The ground ring reports both, by size and by colour. Coverage only moves a threshold and is free; density sizes the grid cell and is clamped to four times the shipped grid, a renderer limit rather than a generator one (`RENDER-04`). The two-sine stand field is replaced by three octaves of value noise at a metric `500 m` stand size, because its wavelengths were fixed in metres and put the whole `500 m` sandbox world inside one stand. Scaling that size to the world instead was tried and measured wrong: global coverage stayed correct at `0.565` while the `2 km` window at spawn fell to `0.002` and an E13 probe lost half the resident trees, so local coverage is now pinned by a test. The new-game dialog opens after a world is picked and offers one Rust-owned Forest choice, Less | More, with no readout: Less is the shipped 0.565 coverage on an 11 m canopy grid (about 64 trees/ha) and More is 0.8 on the 8 m ceiling (about 122); it opens on Less with the unchanged seed; `start_new_game` carries them in, while `load_world_definition` keeps the shipped forest for the editor, the benchmarks and the probes. Every brush action is now reversible with `Ctrl+Z`: each edit records a bounded inverse journal of the cells it is about to change, sized O(changed cells) and allocating nothing for a cell the generator still owned, and a held drag mints a stroke id that merges its stamps into one history entry rather than filling the 30-entry stack with fragments of one gesture; the id rather than a first-stamp flag is what keeps a clear from attaching itself to the planting underneath it. An authored plant can now name the tree it is, not only its species: `AuthoredPlant` carries a renderer variant pinned over the appearance seed's own choice, biased so that zero is the unpinned case the generator and every earlier save write, persisted in save format 63 and packed into the existing lane five rather than a seventh float in the stride. Spruce and aspen were already in the world and unreachable, because there was nowhere to record which conifer or broadleaf the player meant. The brush is then driven by an eleven-entry preset table in Rust - four unpinned species, pine, spruce, birch and aspen, and three numbered mixes - each carrying the fraction of its 4 m lattice it keeps, its instance scale band and a weighted species mix, with `BRUSH_OPTIONS` only naming them. The brush offers nine of the eleven: the unpinned conifer and broadleaf stay in the table as what the generator plants, but are not offered, because once pine and spruce are both nameable a "conifer" is only an unnamed two-to-one mix of them. The first four ordinals plant exactly what they planted before. Density presets thin rather than densify, because the lattice is already above a real stand, and a stroke still clears nothing. Bulldoze now answers a vegetation target after its building and road queries, over a fixed 4 m cursor disc, and deletes that one plant through the brush's own clear rather than a new mutator, so it writes the same tombstone and takes one undo entry per click (`VEG-02`). See [the local experiment contract](terrain.md#local-vegetation-experiment-v01).

- **Day/night cycle (`RENDER-01`)**: scene lighting is now a pure function of the simulation's operational clock. A real solar-position solve drives the sun arc; the palette is keyed to solar elevation rather than to the clock, so sunrise and sunset are symmetric and golden hour, blue hour and a moonlit night all fall out of one curve. One key light carries the sun by day and the moon at night; the moon keeps a 29.5-day month, drifting east among the stars, rising later each day and lighting the ground by its phase (`RENDER-24`). Terrain, water and site ground follow through shader globals, so one write per frame relights every resident patch. New games now start at `07:30` instead of midnight. The cycle is cosmetic, so `Tools > Time of Day` now holds the light at `Morning`, `Midday`, `Afternoon`, `Evening` or `Midnight` while the clock keeps running, and `Follow Clock` returns to the cycle. Not yet done: no seasonal sweep, no street or window lighting at night, the pinned hour is not saved with the game, and the laptop cost is unmeasured. See [the lighting contract](terrain.md#14-terrain-site-ground-water-and-lighting-materials-are-runtime-presentation).

- **Scale target**: at least 1,000,000 total population across simulation tiers in one large world.
- **Simulation model**: full-FSM simulation stays inside the active area of interest; distant world regions are expected to degrade to coarser flow-field or aggregate simulation.
- **Current focus**: keep the playable small-to-medium city slice correct, deterministic, and scalable while the docs/planning cleanup continues and baseline water/render plus local road-build performance are hardened.

## Shipped Foundations

- **Road guide removal (`ROAD-27`)**: removed guiding lines, guide snapping, their global
  cache and the G shortcut. Zoning-grid snapping, road connections and active placement
  measurements remain. All 2,010 active Rust tests, both Godot cursor/preview suites and eight
  report tests pass; the release extension is deployed. See [road guide removal](archive/roads_legacy_surface_2026-10-23.md#road-guide-removal-road-27).

- **Road preview crash (`ROAD-26`)**: generated-carrier height lookup no longer indexes an
  empty candidate list during preview or commit validation. Missing support follows existing
  validation. All 2,014 Rust tests and both road-tool/continuous-preview Godot suites pass;
  updated extension deployed. See [`roads.md`](archive/roads_legacy_surface_2026-10-23.md#empty-generated-carrier-height-lookup-road-26).

- **Third-road placement (`ROAD-25`)**: node boundary export preserves distinct submillimetre
  segments, closing the reproduced Kuopio terrain ownership gaps. Rejected exact previews retain
  a visible red fallback and explanation. Road drawing waits for simulation acceptance; rejection
  keeps the stroke editable. Road logging no longer enables the cyan surface overlay implicitly.
  See [`roads.md`](archive/roads_legacy_surface_2026-10-23.md#third-road-boundary-and-placement-feedback-road-25).

- **Machinery upkeep (`ECON-09`)**: grain farms use 0.1 kg and coal mines 4 kg Machinery per hectare/day; food processors and utilities use building-level inputs. Paid OWA imports, small-consumer stock batches, startup funding and inspector stock/rates are integrated. Business/utility customers drive matching industrial demand. The audit unified restock/supplier logic, preserved small-shop shipment buffers, corrected net production headroom and city payroll/refund reporting, removed duplicate ledger/resource helpers and production constants, and rejected duplicate recipe ports. Depleted mines now stop creating jobs and Machinery demand; distress sales protect upkeep stock, and inventory fill counts shared input/output stock once. `machinery_factory_basic` is ready for the incoming four-worker industrial asset (Steel + Metals → 40 net Machinery/day); the production model itself is still user-created content. See [`economy.md`](economy.md#machinery-upkeep-econ-09) and [`asset_editor.md`](asset_editor.md).

- **Terrain reload and yard seams**: CDT cleanup now preserves distinct near-endpoint
  intersections instead of merging them by a dimensionless tolerance. This fixes the reproduced
  missing terrain patch without disabling mesh validation; terrain debugging also reports rejected
  patch publication. Shared-side sample welding now precedes containment in both adjacent tiles,
  closing the reproduced narrow commercial-yard gap without changing assets or flattening the
  graded apron. Contract revision 14 also fixes the `farms.sqlite` rejection: site and road guides
  share source heights, and building aprons grade both sides of tile boundaries.
  CDT export now also measures geometry in retained `f64` world coordinates before converting
  patch-local values to Godot buffers. This fixes the benchmark-city patch `(27, 0)`, where early
  `f32` conversion collapsed a valid sub-millimetre site edge and falsely rejected the patch as
  pathologically steep.
  See [`terrain.md`](terrain.md).
- **Level building yards (`EARTH-02`)**: buildings and usable yard interiors share a level pad; 2 m lot-edge strips provide graded road/neighbor/terrain tie-ins. Zoned, service and explicit industry placement now share support preparation, installation and site-terrain invalidation. Mixed 2D/3D frontage projection and the explicit-mode raw-height fallback are removed. Paired zoned/service tests compare final site and terrain geometry; `farms.sqlite` now reproduces and verifies the corrected terrain-rejection path; the original unsaved view remains unavailable. See [`earthworks.md`](earthworks.md).
- **Road network and routing**: modular `RegionGraph`, lane system, CCH pathfinding, road rendering, border nodes, and roadway editing are all live.
- **Zoning and building allocation**: Rust-owned road-aligned parcels, parcel occupancy, roadside building placement, vacancy indexing, and no-build edge flags are live. Road hover previews both sides using the selected zoning options; one press creates the valid lots (`ZONE-03`). Manual drags and automatic fill share a layout anchored to existing parcels, respecting mixed lot widths and the selected gap. Slopes, bends and endpoint rounding retain the shared spacing corrections. Every density uses the same terrain check for the selected lot dimensions, independent of assets. Missing compatible assets permit terrain-valid zoning while growth waits for suitable content. Actual site failures use red preview feedback without cursor warning text. See [`zoning.md`](zoning.md) and [`building_allocator.md`](building_allocator.md).
- **Entrance-aware movement**: the building entrance/exit rewrite is implemented through the exact-plan system described in [`entrance_and_exit.md`](entrance_and_exit.md), including the Phase 1–6 and Phase 8 slices already verified against the live code. Cars and pedestrians share road/pad/graded-terrain height ownership in both access directions; source terrain no longer overrides cut or filled yards. Off-lane cars now use mesh-sized rigid footprint support, including after interpolation, so grade breaks cannot bury the sampled bottom contacts despite a correctly grounded centre. The shared Rust solver preserves horizontal heading and remains level on wholly flat pads; lane/access handoffs do not blend across height owners.
- **Benchmark coverage**: Criterion covers live agent access and isolated road kernels; the chunk suite measures generation and fixed-payload Godot upload. Release gameplay measurements separate paired flat layouts, remote-grid scaling, pointer interaction, and pinned saved-city edits; authored Kuopio replays and Samply runs provide diagnostics. Comparisons require matched inputs, complete generation-matched output, and independent process pairs. See [`roads.md`](roads.md).
- **Economy foundation**: household records, building-centric daily economy, physical truck freight jobs, `OWA` fallback, exact entrance-side freight routing/ETA, household transfer disbursement (unemployment, pension, and child support), two-day building bankruptcy, short private-building construction timers, baseline fiscal revenue (income tax, household VAT, business profit tax, and daily property tax), live `CityFiscalPolicy` controls/save state, first city-owned service-building placement/funding, explicit field-backed grain farms, aggregate `service_store` commercial services, and the starter `grain -> packaged_food -> household_supplies` chain are all live. See [`economy.md`](economy.md).
- **Demand foundation**: Rust owns hourly admission and private development, with daily household
  departures. Admission checks bounded actual applicants against ready homes and live,
  revenue-backed jobs; construction jobs drive housing planning, not arrivals. Independent
  businesses reserve customers and workers, including households that can be housed before the
  workplace finishes. Queued placement revalidates those claims before spending owner equity.
  Housing counts actual asset capacity; citywide pressure no longer gates growth or decline.
  Failed-site clearance preserves healthy vacant buildings; departures use individual eligibility.
  The HUD shows actual project counts and concrete constraints. `DEM-02`'s three
  [exit criteria](demand.md#exit-criteria) (second-mix year, 10k time budget, no housing
  overbuild) pass on one build as of 2026-10-11.

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

- **Constructive roadbed (`ROAD-68`, in progress, branch `road`; `ROAD-68-H`, the last gate, done
  on branch `road68-H` 2026-10-29/30, rounds 6-7, its misses carried or accepted by the user's decisions, question 66 (a) on 2026-10-30; on `road` since 2026-10-30)**: the legacy road-surface
  compiler (about 145k lines) is retired. Its rendered baseline missed 16.7 ms on every sampled
  moving pose, and `ROAD-67`, which ported its semantics, is superseded. Junctions will be built
  directly from their arms (corners, round joins, mouth stations, heights `plane + level`) and feed
  the existing terrain CDT through oriented outer chains. Preview runs synchronously; commit adopts
  the preview's pieces. The targets are an exact road-and-terrain preview within 16.7 ms and a
  commit within one frame. The game may break between gates `ROAD-68-A`–`H`. `ROAD-68-A`,
  `ROAD-68-B` and `ROAD-68-C` are done: roads render as span pieces between node pieces built from
  each junction's layout, queries answer from the rendered triangles and buildings place again.
  `ROAD-68-D` (done on its branch) clips terrain to the roads' outer chains per 64 m CDT tile,
  with one grading envelope for roads and sites, and keeps terrain, road chunks and building pads
  watertight in shared frames. Pre-D work makes profiles, clips and
  pieces independent of drawing order on flat ground (open questions 15 and 16), tapers
  straight width changes (D4) and joins footpaths to road junctions at sidewalk height (D5), with
  zebras kept at the corner beside footpaths (open question 17). `ROAD-68-G` rebuilds routing
  off the commit (D7): a pure job adopted one frame later while the frame holds. Its part 2
  makes the commit's publication, entrances and agent pass follow the edit, parts 3 and 4 its
  cell lots, lane lifts and agent passes (median 21.8 ms at 100k residents); part 7 defers the
  commit's lots one frame and part 10 (done, 2026-10-29) brings every populated commit within a
  frame: first commits 9.6-13.8 ms at 10k-100k
  ([commit stages, part 10](roads.md#commit-stages-part-10-the-gate-road-68-g-2026-10-29)). `ROAD-68-H` round 1
  (2026-10-29, in progress) removed the dead paths gates D-G and `TERRAIN-07-G` left, made
  `./run.sh --test` run every rendered test under gamescope and passed the headless rows except
  the commit's locality at 100k remote sites, which the user accepted as measured (later tracks
  `ROAD-71`, `ROAD-72`, `TERRAIN-11`; [record](roads.md#round-1-cleanup-and-the-headless-rows-road-68-h-2026-10-29)).
  Round 2 (2026-10-29): the preview rows' late poses on the user's display run the same work at
  the P-cores' idle clock (70 % of sampled poses on time with the 60 FPS cap, 85-90 % without),
  not presentation pacing; the new populated row (simulation at 1x on the 100k city) misses at
  0 of 590, each pose re-emitting whole 512 m road chunks (11.5 MB); the routing adoption frame's
  125 ms agent tick is the movement scratch re-prepared for the new revision plus waiting replans
  (`PERF-11`). The user decided the next round (questions 62 and 63): no FPS cap in the Method
  with Wayland reported beside Xwayland, the pose's main-thread work cut to fit the idle clock,
  sub-chunk preview payloads; targets unchanged
  ([record](roads.md#round-2-the-pacing-check-the-active-rendered-row-the-carried-items-road-68-h-2026-10-29)).
  Round 3 (2026-10-29): a preview pose sends its rebuilt pieces and the published runs they
  replace, which Godot collapses in the resident chunk (11.5 -> 2.8 MB a pose at 100k, call and
  install 14-15 -> 7-10 ms; committed payloads, composed chunks and 43 preview captures bitwise
  the control's), and the road tool's view holds every published chunk (a latent bug left it the
  unacknowledged upserts); `ROAD-72` makes a split's building migration local (topology 0.48 ->
  0.06 ms at 100,000 remote sites). The rows still miss: paused 78.7 % Xwayland / 90.4 % Wayland
  without the FPS cap (main-thread work at the idle clock unchanged), populated 27-29 % active,
  57-62 % paused, its T's rest of frame the graded renderer's streaming after the load (question
  64, open); the commit row passes (at most 13.0 ms at 100k); the user accepted carried item 1;
  the base terrain variant no longer reads a tangent basis it never uses (Godot's per-pose
  tangent warning, 0 after). The gate is blocked by `TERRAIN-12`, found by the user in play:
  tile-shaped holes where the previous preview pose lay, for 1-2 frames
  ([record](roads.md#round-3-sub-chunk-preview-payloads-the-poses-main-thread-work-the-city-scan-road-68-h-2026-10-29)).
  Round 4 (2026-10-29): `TERRAIN-12` reproduced from the user's dump and fixed in the Rust
  staging (T60 and T61 (1), the user's rules): the renderer is handed a pose's whole graded
  selection, each tile its exact bake (unmoved ones kept through a second selection of the
  move), or none, and no preview terrain for a pose that draws no road; the drag probe swings its
  strokes and checks every frame (the renderer's invariant, no preview tile without its road, no
  background pixel). T61 (2), the undo's one-frame hole, and T62 (a), holding a pose's replaced
  road runs until its terrain displays, are decided and not yet built. By question 65 (a) with
  (d) the rendered preview rows are carried to `TERRAIN-13` (the active row also to
  `RENDER-30`). The populated
  row now measures the steady state (question 64 (c)); the user widened `TERRAIN-13` (the graded
  renderer's GDScript bookkeeping to Rust) and carried the GDScript share of the pose's
  main-thread work there; the rest of the rows' misses is question 65
  ([record](roads.md#round-4-terrain-12-fixed-in-the-rust-staging-the-poses-main-thread-work-attributed-the-runs-road-68-h-2026-10-29)). Round 5
  (2026-10-29): an undo, a road deletion or any other road chunk install made while a graded
  terrain edit has not drawn keeps the replaced chunks drawn until it does (T61 (2), the journal's
  serial against the renderer's); a preview pose keeps its replaced road runs drawn until its
  terrain displays (T62 (a)). The drag probe shows the background in 1 of 6,739 frames (a fast
  click's batch frame), but the held runs double and z-fight with the new junction pieces: back to
  the user as T64, with one populated drag pose frame of 9.67 ms under background adoption; the
  commit row (at most 12.01 ms at 100k) and every bitwise row pass, and `TERRAIN-14`, `ROAD-73`
  and `UI-09` are new rows
  ([record](roads.md#round-5-the-undo-hold-the-junction-hold-the-runs-on-the-final-tip-road-68-h-2026-10-29)).
  Round 6 (2026-10-29): by the user's T64 decision (about 21:45) the junction hold is removed and
  the preview is round 4's united junction again (43 captures identical); the undo hold stays;
  `TERRAIN-12` is done for the hole classes fixed (dump 0 of 416, 100k swing 0 of 282, the undo
  frames 0), with the junction frames and the fast click's crack (17 of Kuopio's 6,038 frames)
  carried to the new root-cause question T65 (one rule for when the drawn road and the graded
  terrain change), and the 9.67 ms drag frame class to the new `PERF-12`
  ([record](roads.md#round-6-the-users-t64-change-t65-the-runs-on-the-final-tip-road-68-h-2026-10-29)).
  Round 7 (2026-10-29): the plants round 6's clip showed on the road after the undo are the
  vegetation's own lag, not the undo hold (the same with the hold off): it updates an edit's
  patches up to 42 frames after the click, so plants stand on a new road meanwhile (612 of
  15,310 Kuopio frames, all converged); carried to T65 as its third layer, no code change
  ([record](roads.md#round-7-plants-on-the-road-after-the-undo-the-vegetations-own-lag-road-68-h-2026-10-29)). Before `ROAD-68-E`, `roadbed/` was
  compacted for quality to 7,229 production lines with every product bitwise unchanged.
  `ROAD-68-E` (done; its open items closed by `TERRAIN-07` and `ROAD-68-G`, confirmed at `ROAD-68-H`) previews each road pose synchronously in the frame that samples it,
  with the commit's own plan, pieces and road chunks, and the commit installs them; one preview
  mode remains (D3). The preview builds the touched terrain patches with the commit's functions
  and the commit installs them when its own sources match; undo puts the pre-edit terrain back.
  Edits rebuild only the terrain near changed chain segments, so a split trunk's far half keeps
  its tiles; since `TERRAIN-07` the preview's terrain is the graded surface's, its stage within
  3.0 ms but for the user's recorded exceptions.
  Part 2 makes strokes ignore the pieces their own edit rebuilds (sloped T
  junctions no longer depend on drawing order), refuses commits whose pieces newly cover fields
  or painted cells, and refuses node moves, crosswalk overrides and edge class changes whose
  pieces would fail or cover reserved land; the road tool keeps an exact copy of the zoning
  cells, updated after each edit from a record of what changed. Still open: the terrain stage
  (`TERRAIN-07`) and building a road's lots in its preview (paused by the user). `ROAD-68-F` part 1 builds zebra paint, street lights
  and road signs from the pieces in the one emission the preview and the commit share, so
  previewed chunks with crossings or signs are the committed chunks bitwise. Part 2 covers the
  road families without terrain: roads without sidewalks, mixed widths and profiles (a straight
  width or sidewalk change now gets a node piece with a taper, and the old span taper is gone),
  bridge decks and tunnel road pieces, and one triangle walk for the zoning and field overlap
  checks. Bridge abutments, tunnel portals and building sites moved to `TERRAIN-07`; since
  `TERRAIN-07-C` round 3 a bridge or tunnel can be built from a road again. Tests that need later gates are disabled until
  then. A road's grade is at most 1:3 (refused as `road_too_steep`, warned from 16%), and
  profiles, junction blends and terrain-synced roads are built within it inside the 5 m
  source-delta band where their pins allow; the Kuopio reference save is re-recorded with today's
  code as an interim reference until the user's new save ([open questions 37-39](roads.md#road-68-open-questions),
  `ROAD-68-K14` to `ROAD-68-K16`). A save is byte-deterministic: the same city saves the same
  file twice and after a load ([`simulation.md`](simulation.md#save-bytes-road-68-k16)).
  See the [design and plan](roads.md#constructive-roadbed-road-68).

- **Graded terrain surface (`TERRAIN-07`, spec adopted 2026-10-25)**: the user adopted
  approach B for the terrain surface rewrite after the spike: the 10 m base heightmap plus a fine
  grading and footprint distance layer per 64 m tile near roads and sites, an exact seam mask in
  the shader and road skirts, at 0.5 m texels, replacing the exact CDT cut that misses the 3.0 ms
  terrain stage beside built-up blocks. The draft specifies a pointwise grading function (1:2
  cones, a 64 m cap, the overlap rule, coverage from the roadbed's top triangles), tiles as a GPU
  residency cache in four levels, pads as terrain, the consumers' query API, budgets and gates
  `TERRAIN-07-A`-`G`. The user adopted the recommendations of open questions T1-T17 and allows
  the game to break while the rewrite is under way. The legacy terrain surface sections moved to
  the archive. Gate `TERRAIN-07-B` (2026-10-25) built the road skirts: every grounded road piece
  hangs a 0.5 m strip from its outer edges, drawn in the curb stone and deepened with distance,
  and bridges close their deck sides down to the concrete slab. While the exact cut still meets
  the roads, the skirts stay hidden in the ground. Gate `TERRAIN-07-A` (done 2026-10-25) built
  the grading core and its queries for roads beside the legacy path, read by no consumer yet: a populated commit's graded tiles take 1.02-2.77 ms
  from 0 to 100k residents, every value bitwise equal however the tiles are batched. On the
  re-recorded Kuopio reference (`roads.md` question 38) 33 of 44 whole-edit bakes fit the 3.0 ms
  stage (worst 6.70 ms); under T20 (a), decided by the coordinator under the user's delegation for
  review, no frame bakes a whole edit (undo keeps the replaced tiles, about 1.4 MB per entry;
  previews bake only what changed since the last pose; the commit adopts), and gate
  `TERRAIN-07-E` accepts those frame paths against 3.0 ms. Gate `TERRAIN-07-C` round 1 (gate done
  2026-10-26 in round 5) makes building sites terrain clients: the terrain is each pad at its support height,
  the yard paving is a material class equal to the legacy partition's, site changes select only
  the tiles within their reach, and retaining walls close the step where a road's embankment
  stands over a lower road; bridge and tunnel ends follow in round 2. Open: with the pads grading
  the ground, three of the populated city's road commits beside built-up blocks take 3.3-4.3 ms
  of terrain against the 3.0 ms stage; exact pruning levers remain. Round 2 (2026-10-26) takes two
  exact levers (2.86-3.86 ms on the edits beside built-up blocks, still a miss at 2k, 10k and
  100k residents: `terrain.md` T21, open) and starts the structure ends
  (T22, decided under the user's delegation for review): a structure's free end is capped and a
  node piece where only structures meet stands off the terrain. Round 3 lets a bridge or tunnel be
  built from a road in play: the road closes its edge across the structure's mouth with a wall
  run, so the commit no longer rejects it, and the graded terrain walls the step there (an
  abutment under a bridge's end, a headwall over a tunnel's); the bridge's own abutment face and
  the opening over the tunnel's bore are round 4's, with T21. Round 4 hangs the abutment face
  below a bridge's slab where it meets a road, so the bridge's road chunks close there; the
  opening over a tunnel's bore is designed for round 5, and the site-dense full-edit budget is
  carried to gate E's frame paths for review (T21). Round 5 opens the ground over a tunnel's
  deck near its mouth wherever it stands less than 5 m above it, with walls along the tunnel's
  sides and none across the opening; with it gate C is done, its whole acceptance re-run, and the
  site-dense full-edit budget stays carried to gate E for review. Gate `TERRAIN-07-D` (landed 2026-10-27, rounds 1-11; the Load miss accepted by the user, 2026-10-27) builds the graded tiles' residency and the Godot tile renderer: the payload workers bake requested tiles off the simulation lock, Godot uploads them in place into prewarmed slot pools and draws each patch's 64 m cells as instances whose borders stitch exactly. Since round 2 the game renders the graded surface (no refined CDT patch is requested) and Godot's surface height and ray queries answer from it; the exact cut stays reachable from the reference test for comparison. Round 3: a commit rebakes and installs only its dirty resident tiles, together in one frame. Round 4 fixes the defect that kept graded terrain from drawing (Rust refused the plain payload of every patch under a road, so those patches never installed), draws the wall faces, and renders the reference fixtures with no pinhole. Round 5: the GPU idle-frame matrix missed (graded GPU p50 0.70-0.94 ms above exact, the frame 4-5 ms longer); the renderer now works on changes instead of every frame (4.6 ms of graded work per idle frame on Kuopio, now 0.05 ms), the Kuopio poses' GPU difference is attributed to frame pacing, the bridge mouth's dark face is fixed at its cause, and the plot lighting suites pass inside `./run.sh --test`. Round 6: graded terrain settles on the populated save (it never did: a capacity of its level-2 tiles was set below what dense blocks need, and refused requests were re-asked every frame), an edit creates no resources and installs within 1.0 ms (0.73 ms on the populated save). Round 7: the GPU matrix's graded poses turned out to draw without parts of the terrain (Godot ran out of room for per-instance shader values, so cells fell back to defaults); the room is now sized and the matrix must run again. The plan-view pinholes are gone (terrain at a road's edge is drawn behind the road), the terrain under a bridge's end now meets the bridge's own face instead of overlapping it, and the extra 4 ms a frame of a fast-moving high camera over a town is attributed, its fix planned (T26). Round 8: the matrix re-run was valid and missed where graded cells draw (0.56-0.84 ms of GPU on the populated town and the saved Kuopio save); the cost was found in the depth write behind the road (withdrawn), the edge mask, and the finest cells reaching too far (levels now end at 64 and 128 m), and the water reflection's extra triangles in over-fine road patch meshes (fixed). The rule for the hidden terrain under roads now always runs down from the road's edge (T34, decided by the user on 2026-10-27: it replaces the user's T4 decision), which closes the background sliver at the Kuopio road edge; the moving camera's work now follows the changes (T26). The matrix on the fix passes every pose but the saved Kuopio one, still 0.20 ms over (T35). Round 9: tiles that need no edge mask draw with a variant without it (exact, but it takes only 0.014 ms of that pose's 0.19 ms; cheaper cell vertices cannot help, measured; reverted in round 11); pooled cell sets no longer hold Godot's per-instance shader room; the dashed line under a bridge's end and the sky pixels beside it are fixed at their cause (the mask's cut edge fell short of the bridge's mouth). Round 10: the saved Kuopio pose's extra GPU time is in the colour pass, where the finer cells within 1 km are shaded (two thirds the 64-128 m ring, a third beyond), with the edge mask about 40% of it; no exact lever closes it (T35). An undo is now tested to rebake exactly the tiles its commit changed, back to their earlier bytes; streaming installs check their 1.0 ms budget before each piece of finished work, the tile lists included (the worst frames were a dozen lists arriving together); the water reflection keeps showing the ungraded ground (T38, decided under the delegation for review). Round 11: the user decided T34 (the hidden terrain under every road runs down from its edge) and accepted the saved Kuopio pose's +0.17 ms of GPU over the exact cut, every other pose at or below (T35); the variant without the edge mask is reverted (0.014 ms for 51 MiB of video memory and twice the cell sets' prewarm on Kuopio), and an arriving tile list now counts its tiles one at a time under the streaming budget (the populated save's worst streaming frame 1.11 -> 1.04 ms, the budget plus one result's work; that last result is named for `TERRAIN-07-G`). The GPU idle-frame matrix on the gate tip: graded at or below the exact cut on every pose but the saved Kuopio one (+0.21 ms, T35's residual with the edge-mask variant reverted; the +0.17 ms the user accepted was measured with it), the populated town 0.40-0.56 ms below; 0 overflow errors. The Load budget misses (Kuopio about +250 ms, the saved Kuopio game about +200 ms over the exact cut; the populated town 1.07 s faster), about half of it the graded renderer's setup, mostly the prewarmed cell sets (T25): accepted by the user, 2026-10-27, the budget unchanged. For the user's review: T25 (a)'s memory side, T38 (a) and `WATER-05`'s buffer setting.
  Gate `TERRAIN-07-E` round 1
  (the Rust half, 2026-10-26, work in progress): each preview pose, the commit and undo stage
  their graded tiles by one rule, reusing a held tile only when its inputs are unchanged, and
  every staged tile equals a fresh bake bitwise on the reference junctions, after save and load
  too. The commit adopts the preview's tiles and undo the tiles the commit replaced (baked off the
  frame after the commit), each within 1.4 ms on Kuopio. No terrain failure can refuse a road any
  more, and a terrain edit that would leave a road steeper than 1:3 is refused with an undo that
  restores the source terrain. Open, not relaxed: a dragged road rebuilds its whole span every
  pose, so a pose reuses almost nothing and 31 of 299 Kuopio drag poses take more than 3.0 ms
  (worst 6.6 ms; `terrain.md` T27, exact levers next). Round 2 (2026-10-27) splits each tile's
  work across the workers and trims the coverage and selection, with every result unchanged:
  Kuopio's worst pose falls from 7.1 to 4.7 ms, but 19-22 of 299 poses and the first poses of
  the populated drags still exceed 3.0 ms, so the user chooses between spreading a large pose
  over frames (recommended) and anchoring a growing road's stations. The game's commit at 100k
  takes 27 ms on its first sample, 9-15 ms after. Round 3 (2026-10-27) gathers each tile's
  inputs once and moves the routing rebuild and every other background job to a pool of its
  own, which removes the populated drags' 23-47 ms stalls; 13-16 Kuopio poses still exceed
  3.0 ms. Round 4 (2026-10-27, rebased onto gate D): the user decided that the preview's
  terrain may lag while a road is dragged (`terrain.md` T27 (d)); each frame now finishes only
  a bounded share of the pose's terrain tiles, nearest the dragged end first, the earlier poses'
  tiles shown until then, and once the drag stops the tiles become exactly the committed ones
  within a few frames. Every drag frame on Kuopio and in the populated city now stays within
  3.0 ms (worst 2.9 ms), every committed tile unchanged. Open for the user: a click right after
  a fast drag finishes the remaining tiles in its own frame, up to 4.3 ms. Round 5 builds the
  Godot half: the preview's tiles show in the game at the camera's levels and the commit's
  tiles install without a rebake, bit for bit the preview's, a fresh bake's and the loaded
  save's; the click right after a fast drag still misses (bound by its tiles' total work), and
  the user accepted it (`terrain.md` T27 (f), 2026-10-27): that click may take its terrain
  past 3.0 ms, exact in its frame, within the 16.7 ms commit. Gate E is done (2026-10-27,
  rounds 6-7 on the real display): every resident patch's cell set is built in the
  background and the commit takes over the preview's tiles; the holes after a world load
  (on `road` since gate D) are fixed; a commit with no preview may exceed 3.0 ms like the
  click after a fast drag (`terrain.md` T43, the user); the idle-frame matrix measures the
  steady state after the background builds, whose seconds after a load the user accepted;
  four close results go to `ROAD-68-H`'s final verification (`roads.md`).
  Gate `TERRAIN-07-F` done 2026-10-28 (rounds 1-14; the real-display idle-frame matrix passes
  every pose): all Rust and Godot
  consumers use the graded surface;
  water suppression is an exact cell-area query, site feasibility reads the base and 64 m cap,
  inspection uses the visible-surface ray, and grass exclusion reads the graded cells' `sdf` and
  class. T37's render-only vehicle rule keeps exact rigid support through 250 m, then uses one
  prepared query for the centre normal and contacts, lifting to 0.020 m clearance. Round 13
  restores the 4.462 MB shared-owner query index and fixes its stale-piece cause: one canonical
  roadbed publisher bypassed the local index/journal updater, leaving the generation unchanged
  for thread-local scratch. Every publisher now drains the same old/new `TerrainChanges` through
  one atomic boundary. The regression fails before that call and passes after it; lifecycle
  products equal a fresh rebuild after every mutation. Fresh matched 100k culled/full means are
  0.846/5.159 ms control, 1.168/5.605 shared, 1.443/6.085 valid no index and 2.145/7.060 exact
  at every distance. Every one of 1,101 products matches; shared's +0.322 ms culled gap is inside
  the unchanged +0.42 ms bar. Fresh full-day snapshot/frame means are 0.963/6.311 ms control,
  1.227/6.609 shared and 1.569/7.181 valid no index; shared's +0.264 ms snapshot gap is inside the
  unchanged +0.40 ms bar. T44 therefore keeps shared under the user's speed-for-marginal-memory
  criterion; the user agreed T44, T46 (b) and T47 (a) on 2026-10-28, validated at `ROAD-68-H`.
  Round 11's invalid empty-index number remains
  marked invalid; no target is relaxed. Round 11 also fixes the T-junction stem's grass
  at the cause: an in-place tile upload rewrote the grading payload without invalidating grass's
  private mask. The fresh 2x crop has clear asphalt, crosswalk and both sidewalks; the final
  opposite-way T is also clear, and its dumped mask contains the complete differently oriented
  stem across the tile/patch split. The first round-13 GPU matrix passed every pose except
  `city_low`, whose graded frames varied between the 3.3 and 5.4 ms modes. Round 14's instrumented
  gamescope attribution disproves the suspected upload loop: the 300-frame windows at `city` and
  `city_low` each contain zero tile uploads, same-level uploads, mask-revision bumps and grass-mask
  renders on gate E, the round-13 tip and the renderer-revert diagnostic. The cause was steady-frame
  state submission: grass rebound all five mirrored cell buffers and their instance uniforms and
  grading textures even when the patch revision was unchanged. Mirror synchronization is now
  revision-driven; unchanged slots do one integer comparison, while a real payload change still
  refreshes only the patches meeting that tile. Two fixed gamescope rounds put graded `city_low`
  below exact (3.20 against 3.46 ms and 3.05 against 3.87 ms). Fresh captures keep both T stems
  clear and all three load views stable. The final full gates are recorded in the owning gate
  record; the control regression stays `PERF-09`. T25 (a), T38 (a), T40, T42 and
  `WATER-05`'s buffer setting remain pending the user's review. See the
  [specification](terrain.md#graded-terrain-surface-terrain-07) and the gate records
  ([A](terrain.md#terrain-07-a-record), [C](terrain.md#terrain-07-c-record),
  [D](terrain.md#terrain-07-d-record), [E](terrain.md#terrain-07-e-record),
  [F](terrain.md#terrain-07-f-record)).

- **`TERRAIN-07-G` done (2026-10-29).** The exact/CDT terrain surface and its
  frontend selector, refined jobs, baked meshes and water clip are deleted; gameplay, references
  and benchmarks are graded-only. The stage, locality and bitwise rows passed before deletion.
  Round 3 found that the earlier bend fix hid its cause: at the plan view's 1 m lattice the edge
  mask can miss a road-edge segment shorter than the lattice spacing, leaving single sky pixels at
  the bend and the road end (`terrain.md` T48); the Kuopio pixel dates from gate D's withdrawal of
  the depth push behind roads (T49). The user decided both on 2026-10-28 (per-square extra mask
  candidates; a depth-writing variant only for conflict tiles); they are the next round's to
  build. Round 3 also restored what the deletion had broken: the first road commit after a load
  or bulk build again stages only its own terrain, and the water reflection reads the ground the
  water is drawn on again, which removes the retained-memory rise the first valid measurement
  showed (the game's settled memory is now at or below road's). Round 4 built both decisions:
  every reference fixture, the bend, the road end and the Kuopio wall included, renders without
  a sky pixel. Their cost is open for the user (`terrain.md` T50, T51): far from the camera the
  extra mask candidates run to hundreds per lattice square and the slot pools would grow from
  133 to 366 MB of GPU memory, and on hills the depth-writing variant covers about half of the
  near tiles. The new bake work first tripled the preview's terrain stage (a miss); made parallel
  it is back under 3.0 ms on the reference fixtures. Round 5 measured the tip: the reference
  preview, the converged commits and undos, locality and bitwise pass, but a populated drag's
  frame, which now bakes a tile in 2-3.5 ms, exceeds 3.0 ms in 2-7 of 150 frames, one undo took
  7 ms, and the game holds 1.3 GB more memory at 100k (0.28 GB on Kuopio; 0.14 GB more on the
  GPU). On the user's display the depth-writing variant costs nothing measurable; the extra
  candidates cost 0.2-0.35 ms on the saved city's high camera paths. These are open for the user
  (`terrain.md` T50-T52); the archive moves are done. The user decided T50, T51 and T52 on
  2026-10-28 (keep both as built; attribute first). Round 6: the reference views that flipped
  between runs were captured before the terrain renderer had finished building its cell sets (a
  patch then draws its ungraded ground over a road in a cut); the test now waits for the
  renderer to settle, and the same window in the game, after a load and while streaming, is open
  for the user (`terrain.md` T53). The memory rise was mostly a defect, fixed: finished terrain
  tiles queued without bound behind the renderer's install budget; the 100k city now settles
  below road's memory, and what remains is the larger slot pools at setup and on the GPU (the
  user's planned list atlas, T50). A populated drag's frame still bakes a tile in 2-3.7 ms (0-2 of 150 frames
  over 3.0 ms), the cost of the two decided mask features (T52). Round 7 built the user's T53 (a):
  a terrain patch whose graded tiles are not all ready draws nothing instead of its ungraded
  ground, and a world load hands the game over (input, camera, clock) only once the terrain has
  settled (since `TERRAIN-15`, once the terrain near the camera is ready), its cell sets built meanwhile at full speed (the 100k city ready in 33 s instead of
  never settling; Kuopio 6.3 s against 5.7 s plus 90 s of background builds). There is no
  loading screen yet (`UI-08`). A drag's catch-up frame now stages two tiles instead of three:
  every populated drag frame fits 3.0 ms, but on Kuopio's steepest strokes one tile alone takes
  up to 3.6 ms (for the user, T27). The round found that the 100k city wants three times more
  level-2 and level-3 terrain slots than the pools hold, so its far graded terrain never
  installed (T54, for the user; now counted instead of re-requested every frame). T50's list
  atlas is decided and not built yet. Round 8 built the user's two decisions together: each level's
  graded tiles now live in pages, one per block of patches (a tile on a block border is held in
  both), and a tile's seam, edge and candidate rows live in its page's atlas sized by use instead
  of fixed rows per slot; a page that fills moves its block to a larger one without a visible gap.
  The 100k city now installs every one of its 10,500 wanted tiles (none left as ungraded ground),
  every reference render is unchanged, and Kuopio holds 190 MB less video memory than road; the
  100k city's video memory and load time grew with the tiles it now holds (details and the
  user's rows in the record). A whole-map view is analysed for later (`RENDER-29`). Round 9 found
  why one undo in about fifty stalls: the terrain stage freed spare buffers in the frame and the
  allocator handed the memory back there (fixed), and the frame's worker threads wait for CPUs
  and allocator locks while background routing rebuilds run (on `road` too; options for the user,
  `terrain.md` T57). An exact speed-up of the steepest Kuopio tiles (the road coverage pass in
  parallel, no product changed) leaves 2-3 of 299 drag frames over 3.0 ms instead of up to 4.3 ms
  (for the user, T27); the terrain renderer's page allocator moved into its own script. The user
  accepted T56, T57 and T27 on 2026-10-29 and asked for the startup memory and a leak to be fixed
  first. Round 10 fixed both: each world load leaked one node per terrain cell set, and every set
  was prewarmed with the meshes of the depth-writing variant that only conflict tiles draw; a set
  now gets them with its first such tile, and the game's memory before a save loads is at road's
  (6 MiB below). Renders, install counts and products are unchanged. The user accepted the last two
  rows: Kuopio's drag frames in the gate session over the numbers accepted before, on the same
  binary (T58), and the settled memory above road's, attributed with `TERRAIN-09` (T59). No target is relaxed; details are
  in the [Gate G record](terrain.md#terrain-07-g-record).

- **Simulation scheduler refactor (`SIM-02`, done 2026-10-22)**: hourly and midnight work moves from
  calendar boundaries to per-entity phases, timed events, rolling snapshots and request queues.
  Old saves break; continuous income and per-entity settlement windows are accepted gameplay
  changes; GPU compute is deferred. See the [engine contract and migration](simulation.md#sim-02-scheduler-refactor)
  and the [`PERF-07` frame budget](simulation.md#frame-budget-perf-07). `SIM-02-A` (2026-10-15) added
  the guards every step must pass (`tools/sim02_gate.py`: behaviour envelope, burst shares,
  money conservation, reference invariants, determinism, worst sim frame) and recorded the
  pre-refactor baseline: the worst frame is 637 ms at 30k and 2.5 s at 100k. `SIM-02-B`
  (2026-10-16) moved saves to format 87: **saves from earlier builds no longer load.** Each
  step since bumps the format again (90 after `SIM-02-D`). `SIM-02-D` (2026-10-16): a
  construction site now completes exactly its authored duration after it starts, at its own
  slot, instead of on the next hour (up to an hour later than before). `SIM-02-E`
  (2026-10-16): trucks, shoppers, service visitors and arriving families no longer all leave on
  the hour. Each kind leaves at slots spread over the hour, and deliveries, store pickups, returns
  home and move-ins settle when the agent arrives instead of at the next hour (save format 91).
  `SIM-02-F` (2026-10-16): each building produces, each household eats and uses its utilities,
  and each resident feels hunger at its own minute of the hour, not all at :00. City-wide
  store sales and electricity dispatch still settle on the hour, and electricity bills the
  activity of the hour before (save format 92). `SIM-02-G` (2026-10-16): midnight no longer
  settles the city. Each building, household and worker settles its own 24-hour window at its
  own minute of the day (wages, taxes, transfers, bills, bankruptcy, rezoning grace), the
  budget report sums payments as they happen, and 00:00 keeps only the calendar close and,
  until `SIM-02-H`, the pollution, noise and desirability update (save format 93). A business
  that cannot pay its workers two days running goes bankrupt, and an adult not paid a wage that
  day draws unemployment benefit. `SIM-02-H` (2026-10-17): pollution, noise and desirability are rebuilt by a
  sweep that runs through each day and publishes at its end, instead of at midnight, and dirty
  building lots regenerate one map region per slot instead of all at once in the hourly demand
  pass. Loading a game no longer re-derives every lot when the asset catalog is unchanged, which
  was most of the stall in the first hour after a load (save format 94). `SIM-02-I` part 1 (2026-10-17):
  each household recounts its members at its own minute of the hour, and electricity, service
  shopping and the shops' activity floor run per building and household, reading the city's
  power coverage and shop prices of the hour before, so power and shop takings lag by an hour.
  Freight finds suppliers listed an hour earlier, and demand plans on a snapshot swept over the
  previous hour. A game saved mid-hour now continues exactly as one that was not saved (save
  format 95). `SIM-02-I` part 2 (2026-10-18): the hourly jobs-funding census no longer runs in one
  frame at the top of the hour. It copies the city over the first half of each hour, works
  through the copy over the second half and takes effect at the next hour, so funded jobs reflect
  the city 31–60 minutes earlier. Tax and service-funding changes reach funded jobs at the next
  hour instead of at once, though workers a funding cut no longer pays still leave at once, and
  demand planning counts unemployed adults and open jobs on the live
  city, which stops the growing starter town admitting more adults than it has jobs (save
  format 97). `SIM-02-I` part 3 (2026-10-18): new shops and factories are checked once an hour,
  when demand is planned, instead of each running its own jobs census as construction starts;
  each keeps a permit until it starts, and one whose door moved in the meantime is rejected. A
  permit voided by a home that failed to place waits for the next hour instead of cancelling the
  building. Every home now draws its new residents from a seed of its own, so households
  differ from earlier versions, and a game saved mid-hour, even right after a road edit,
  continues exactly as before (save format 99). `SIM-02-I` part 4 (2026-10-19): demand
  planning no longer runs in one frame at the top of the hour; homes, shops and industry are
  planned in the hour's first three slots and the hour's buildings are queued, permitted and
  cleared in the fourth, in the same order as before. Newly admitted households set off three
  slots later, and route guidance now follows new and removed buildings at once instead of up to
  a day later (save format 101). `SIM-02-J` parts 1 and 2 (2026-10-19): freight orders, housing
  moves, shopping, service visits and job searches now happen a few at a time through every
  minute instead of all at the top of the hour: each building, household and resident asks at
  its own minute. A family that loses its home looks for a new one at its own minute, so the
  hour's newcomers may take the vacancy first, and a worker whose better job filled before it
  applied waits for its next job review instead of trying again each hour. Shops run out less
  often in larger cities (save format 102). `SIM-02-J` part 3 (2026-10-19): old or failing
  buildings are cleared, families leave and newcomers arrive a few at a time through the hour
  instead of all at its start, and building upgrades and downgrades take effect a slot after
  they are decided (save format 103). The legacy hour and day code is gone: no game code runs because
  the clock crossed an hour or a day. The render snapshot skips agents outside the camera before
  posing them, with identical instances, which halves the 100k mean frame at 1× (`SIM-02-K` part 2).
  Loading now generates the zoning cells near every road and prepares the first frame's routing,
  so the first lot regions and frames after a load no longer stall for 50–130 ms; a 100k load
  takes about 1.6 s longer (`SIM-02-K` part 3). The funding census now runs in bounded slices
  through the hour's second half, so no census slot holds the core for more than about 4 ms at
  100k (`SIM-02-K` part 3). The agent tick no longer rebuilds its lane occupancy from every
  agent when agents spawn or leave, and moves agents in balanced shards: the 100k mean frame
  falls by about 1.5 ms at 1× and 2.6 ms at 32× (`SIM-02-K` part 4). The first hours after
  loading no longer stall in demand planning: loading prepares what the first planning slot used
  to build, and the census storage gets a prepared copy of the live books at load, with spare row
  capacity so appended buildings no longer reallocate it in a planning slot (`SIM-02-K` part 5). Loading also prepares the route scratch the first trips and job
  searches after it used to build, so the first minutes after a load no longer stall for 8–15 ms
  frames at 100k (`SIM-02-K` part 5). The hour's planning now runs a game minute apart,
  residential, commercial, industrial, then admissions and actions (minutes 0 to 3), and
  removals, departures and level changes start at minute 4, so at 32× no frame carries two of
  them (`SIM-02-K` part 5, save format 104). A building leaving a lot it keeps no longer
  regenerates the lot's cells, so the 30k 32× minute-4 frames fall to at most 5.7 ms; the 1×
  render-snapshot spikes are scheduler preemption, not snapshot work. Each census's planning books are now prepared at minute 59 of the hour
  before they serve, so the hour's residential planning slot no longer rebuilds their site rows:
  about 4 ms less at 100k and 1.2 ms less at 30k (`SIM-02-K` part 5). The seventh commit keeps each
  book's pass rows and admission's border sweeps by what changed, splits the minute-59 preparation
  and moves the removal slot's scans onto the pool: the 100k actions slot falls from 6.5 to 4.2 ms
  and the removal slot from 4.6 to 3.5 ms (`SIM-02-K` part 5). The eighth commit refreshes each
  census book's business finance masks at minute 59 instead of in the residential slot's first
  claim and fills the zone index's chunk lists without a sort: the 100k residential slot falls from
  4.2 to 3.4 ms and the zone-index rebuild from 2.1 to 1.5 ms (`SIM-02-K` part 5). The request
  queues pause in the hour's planning, preparation and removal slots and catch up in the next slot,
  and trips due then leave one slot later (`SIM-02-K` part 5). Each hour's planning now counts and
  scans half a game minute before it decides, and the actions run at minute 3's second half
  (`SIM-02-K` part 5, save format 106). Commute departures now leave over
  their minute's slots instead of all at its first, so the 100k departure waves no longer stall
  the agent tick, and the unused per-edge congestion value is gone (`SIM-02-K` part 4, save
  format 105). The 100k city's busy streets are not jammed: shoppers walk several kilometres
  because only the oldest stores get customers (`ECON-22`; `SIM-02-K` part 6).
  The census's business claims now count their scan work, so no census slot holds more than about
  2.2 ms at 100k, and the removal slot keeps the zone index across its swap-removes (slot 64 3.5
  to 2.6 ms at 100k) (`SIM-02-K` part 5). Each removal now scans the agents and households once and
  defers its terrain boxes, and the census book's preparation runs in four parts, so the removal
  slot and the minute-59 preparation no longer set 100k 1× frames over 8 ms; 9 such frames remain
  in `PERF-07` (31 before), mostly preemption (`SIM-02-K` part 5). The planning setups now scan
  beside their workforce count and a workforce reset passes over the buildings once, so no planning
  setup sets a 100k 1× work frame over 8 ms; the actions slot (2.8 / 3.4 ms) is the largest steady
  slot left (`SIM-02-K` part 5). The frontage delay update, which every minute-first frame
  carried, now runs by lane range on the pool, and the planning slots scan the free places beside
  their site preparation; the actions slot (2.7 / 2.9 ms) and slot 65 (2.6 / 3.2 ms) are the
  largest steady slots left (`SIM-02-K` part 5).
  `SIM-02-K` part 7 (acceptance, 2026-10-22) closes the refactor: `PERF-07` accepts on the frames
  no other task preempted, with the raw p99.9 and the preempted frames less their wait as guards,
  and 30k at both speeds and 100k at 1× meet 8 ms. 100k at 32× (13.6 ms mean, 22–24 ms worst) is
  deferred until players' cities approach 100k agents, tracked as `PERF-08`
  ([open questions 6 and 7](simulation.md#open-questions)).

- **Starting treasury**: new games begin with 500,000, increased from 100,000. See
  [money source](economy.md#money-source); earlier balance runs retain their original starting capital.

- **Electricity accounting (implemented)**: profile-authored building demand,
  kWh meters, demand-limited coal generation and unpaid household bills are implemented.
  The [consumption table and 1/2/3-worker ledger](economy.md#utility-service-layer)
  target an affordable 170-resident starter plant. Fresh validation passes 362 economy tests,
  all three village growth tests and six staffed plant/mine scenarios. A labor/housing guard fixes
  an unstaffed industrial startup after housing saturation; fuel buffers now follow staffed capacity.
  The small town supports one power worker, or two with a working 1 ha mine; a third loses money.
  Final matched 10k timing adds 0.18 ms/hour (+8.4%); the earlier revision measured +0.35 ms.
  Neither comparison detects a significant daily change.
  Mid-day save continuation, removal/undo accounting and the Godot display regression pass.
  Pre-73 saves retain cash and convert generation, but lack the new meters for their first partial day;
  see [validation and limitations](archive/economy_history_2026-10-10.md#electricity-validation-econ-14).

- **Healthy city growth (`ECON-11`–`ECON-13` and `DEM-02` done)**:
  The original saved-town replay stayed at seven residents through day 32 despite empty homes
  and high industrial pressure. Independent entry now removes the global startup-cash recovery
  dependency; the same save grows from 7 to 85 residents, with 69 employed, over 30 days.
  See the [saved-town diagnosis and redesign](demand.md#growth-redesign-dem-02).
  The [critical demand review](archive/demand_history_2026-10-10.md#critical-demand-review-2026-10-07) also identifies category
  dilution, repeated distress penalties and size-independent admission pacing. Fresh checks:
  the pre-change build passed 93 demand-related tests and all three growth scenarios; the two
  tracked startup tests failed and the saved-town stall reproduced. The first admission edit now
  evaluates bounded actual applicants without a preview veto and excludes prospective marginal
  jobs from their income. Completion-time workforce reservations now pair needed homes with
  workplaces and reject projects that would reuse workers or depend on homes finishing too late.
  Project operating costs now include authored electricity and real freight batch sizes, and
  grocery entry also requires operating income. Queued projects now reserve parcels, homes,
  workforce and output customers using their delayed completion dates. Employment and admission
  now share revenue-backed hiring with stock/freight working capital; nonworker arrivals preserve
  homes reserved for workers. Placement revalidates labour and customers before owner funding,
  including failed or bulldozed supporting homes. Commerce uses recurring unmet sales value,
  and spawn percentages no longer veto viable construction need. Rejected businesses now expose
  viable alternatives on the same lot, and shared reservations replace the single-export-project
  quota. The mass-zoning regression passes with five additional staffed, profitable factories and
  34,600 owner equity within the observed labour bound. Selection validation passes 2,165 library
  tests and all three growth scenarios; that run retained the utility and cash-continuation failures.
  Its matched 10k timing comparison detects no significant change. The admission preview/runway,
  marginal shop job forecast, regional cutoff and repeated distress penalties are now deleted,
  including their unused config and saved failure memory (format 74). Cleanup verification passes
  2,161 library tests and all three growth scenarios; the existing utility quantity failure remains.
  The matched incremental 10k benchmark shows no significant timing change. Queued save/reload
  and developed-town replays now pass 30 days with sub-cent cash tolerance and exact discrete
  outcomes. Format 75 preserves traffic smoothing and live agent decisions, while freight quotes
  reevaluate current frontage delays over stable free-flow network costs. All 35 focused save
  checks pass, and the final run passes 2,169 library tests and all three growth scenarios, with
  only the pre-existing utility quantity failure. The matched 10k reload-fix comparison detects
  no significant hourly/daily timing change. The cash-continuation issue is resolved.
  Housing selection now counts actual household capacity, removes averaged building quotas and
  accumulated spawn credits (format 76), and retains optional homes for viable workplaces.
  Fresh validation passes 2,165 library tests and all three growth scenarios, including both
  reload continuations; the sole failure remains the existing utility quantity assertion.
  The matched 10k capacity-cleanup comparison detects no performance regression.
  The HUD now reads actual hourly project counts and housing, site, asset, income and labour
  constraints; unused spawn thresholds and saved hysteresis are removed (format 77).
  This revision passes 2,167 library tests, all three growth scenarios and isolated headless/rendered
  Godot meter checks; only the pre-existing utility quantity assertion fails.
  The reversed 10k performance comparison records +0.126 ms per hourly tick (about 5.3%);
  daily timing is unchanged statistically, and performance acceptance remains open.
  Clearance now removes only deserted empty sites, preserving healthy vacant homes and workplaces;
  its pressure thresholds and saved hysteresis are removed (format 78). Worker reservations now
  follow actual employment: losing funding cannot release a still-assigned employee to a new project.
  The grocery-failure regression now replaces the shop without rezoning and retains positive
  operating cash and 96% lifetime staffing over 180 days. Fresh verification passes 2,171 library
  tests and all three growth scenarios, including both cash continuations; only the existing utility
  assertion fails. The latest combined HUD/clearance 10k comparison detects no significant change,
  but does not close full-redesign performance acceptance or the earlier HUD timing concern.
  Household departures now use one eligibility policy in planning and execution; recently displaced
  and recovered households are protected, and unused departure slots cannot remove housed families.
  Format 79 removes the citywide crisis-removal credit. Departure and housing regressions pass;
  all three growth scenarios and both cash continuations pass, with the existing utility failure
  still tracked. A fresh year of the original stalled save grows from 7 to 80 residents, with all
  15 firms earning positive operating cash. Matched departure execution improves at every measured
  size (65,536 households: 3.688 to 0.862 ms); full-redesign performance remains open.
  Instant business upgrades now share workforce/customer reservations with new projects, credit
  incumbent jobs once, and cannot use unfinished homes. Target recurring costs include retained
  employees and target-level property tax. Fresh validation passes 2,173 library tests and all three
  growth scenarios, including both cash continuations; only the existing utility assertion fails.
  The saved-town year still retains 80 residents and all 15 firms with positive operating cash.
  The combined exit/upgrade 10k comparison matches city outcomes; hourly +4.8% (p=0.10) and daily
  timing differences are not significant. Active-project scaling and performance acceptance remain open.
  Upgrades now proceed independently of citywide pressure; format 80 removes their saved
  hysteresis and authored thresholds. Shared target working-capital valuation counts cash,
  usable inventory and paid cargo, replacing the duplicate four/eight-day cash-only reserve.
  Fresh verification passes 2,174 library tests and all three growth scenarios, including both
  cash continuations; only the existing utility quantity assertion fails. The saved-town year
  retains 80 residents / 66 employed and all 15 firms with positive operating cash.
  Business downgrades now preserve supportable incumbents and require a viable replacement with
  shared worker/customer claims. Fresh verification passes the eight-case planner regression,
  2,175 library tests and all three growth scenarios; only the existing utility assertion fails.
  Matched 10k hourly timing rises 16.3% initially and 4.4% in reversed order (the latter not
  significant); the timing concern and active-level-change scaling remain open.
  Format 81 removes downgrade thresholds, saved hysteresis and zoning growth-profile references.
  Downsizing now protects ready homes reserved for workers, vacancy space and unhoused households;
  lower tiers cannot hide viable upgrades. Fresh verification passes 90 demand tests, 2,178 library
  tests and all three growth scenarios, including both cash continuations; only the existing utility
  assertion fails. The incremental matched 10k comparison detects no significant timing change;
  active level-change planning takes 0.577 / 4.173 / 30.727 ms at 2k / 16k / 131k buildings.
  This does not establish active-queue scaling. Business proposals and queued execution now use
  actual outside-route freight prices, including the proposed asset's door. Fresh verification
  passes 2,181 library tests and all three growth scenarios, including both cash continuations;
  only the existing utility assertion fails. The matched 10k comparison detects no significant
  incremental timing change. Consumer forecasts now use each household's recurring disposable
  income, shared by hiring and project reservations; format 82 preserves that census on reload.
  Fresh verification passes 2,185 library tests and all three growth scenarios, including both cash
  continuations; only the existing utility assertion fails. The 10k hourly estimate rises 12%,
  with different admissions, so matched performance acceptance remains open. Full net capacity now
  stays reserved across construction completion and staffing searches, matching live hiring;
  replacement credits and maintenance demand use the same net accounting. The adapted saved-town
  year avoids the unstaffed second health shop and its 3,465 owner grant while retaining 80 residents
  and all 15 operating-cash-positive firms. Fresh verification passes 2,188 library tests and all
  three growth scenarios, with only the existing utility failure; the matched 10k comparison
  detects no significant incremental timing change. Format 83 removes the obsolete R/C/I pressure
  values, their normalization work and saved columns. Daily departures now reuse only the parallel
  household census; the saved-town year remains byte-for-byte identical to the previous report.
  Fresh verification passes 2,181 library tests and all three growth scenarios, including both
  cash continuations, with only the existing utility assertion failing. Matched 10k hourly cost
  decreases 9.95%; daily timing shows no significant change. New isolated queue measurements
  reserve 65,536 projects in 11.161 ms and make 32,768 new claims in 2.791 ms. Repeated project
  validation stays near 0.32 ms per 1,024 checks as background buildings grow to 131,072;
  88 demand and 11 queue regressions pass. The shared operating calculator now accepts per-resource
  supply quotes and matching stock replacement prices. Fresh verification passes 2,184 library
  tests and all three growth scenarios, including both cash continuations; only the existing
  utility assertion fails. All growth and saved-town reports are unchanged; the matched 10k
  comparison detects no incremental regression. Live callers retain import quotes until finite
  local supplier reservations are connected. Housing construction now includes existing unhoused
  households in its capacity target, even without new jobs or an outside connection; ready and
  committed homes offset that need once. Both new regressions reproduce the omission before the
  fix and pass afterward; the full run passes 2,186 library tests and all three growth scenarios,
  with only the existing utility assertion failing and unchanged long-run reports. The initial
  10k timing increase does not reproduce in three balanced-order repeats; all city records match.
  Existing residents now claim affordable ready homes before hourly job and arrival planning,
  using level ordering in the existing vacancy index and the census's member links. Daily eviction
  clocks are unchanged, and moving out of a broken home now releases its old occupancy. Fresh
  verification passes 2,189 library tests and all three growth scenarios, including both cash
  continuations; only the existing utility assertion fails. Long-run and saved-town reports are
  unchanged. Matched 10k timing shows no significant change; 128 rehousing claims stay near
  0.02 ms through 65,536 unrelated vacant homes. Future-home reservations now match displaced
  households by completion time and individual affordability, using actual unemployed adults and
  preserving physical places for nonworkers. Post-employment counts exclude unhoused employees
  from housed labour totals. Fresh verification passes 2,195 library tests and all three growth
  scenarios, including both continuations; only the existing utility assertion fails, and all
  growth/saved-town reports remain identical. At 65,536 homes, displaced-household matching takes
  2.42 ms; 512 prepared worker claims stay near 0.02 ms. The initial matched 10k hourly increase
  does not reproduce in reversed order; all city records match. Live hiring now stages finite local
  production claims using actual supplier prices and directed routes; rejected operations release
  their claims exactly, and dispatch can choose a cheaper executable import. The construction-stage
  full release run passes 2,204 library tests, all three growth scenarios and both cash continuations, with
  only the existing utility assertion failing. The saved-town year retains 80 residents and all
  15 operating-cash-positive firms; optimization preserves all growth and saved-town reports.
  The earlier matched 10k hourly increase from 2.69 → 5.79 ms remains unresolved.
  Construction integration adds no detected regression (5.672 → 5.655 ms hourly), with identical
  city records. Successful placement of 1,024 factories takes 113.36 ms, excluding setup/first
  check, and verifies 13,414,400 owner equity. Zoned construction shares residual inputs across queued/new
  projects, requotes the selected alternative, and commits only after physical placement. Ordered
  replay retains full output reservations without understating unfinished shops' inputs.
  Live services also reserve finite inputs while retaining fiscal payroll limits; the service
  follow-up passes 129 household tests, 99 demand tests, both continuations, all growth scenarios,
  expansion and recovery, with reports unchanged. Upgrades/downgrades now release only the
  incumbent's finite input claims and value retained stock at quoted prices. The subsequent full
  run passes 2,207 library tests, all growth scenarios and both continuations, with the same known
  utility failure and unchanged reports. Matched hourly timing is unchanged (5.547 → 5.581 ms).
  Constructing farms/extractors and services now reserve finite inputs using actual work areas,
  private capital and separate future fiscal ceilings, including placement, undo and remapping.
  The final release run passes 2,211 library tests, all growth scenarios and both continuations;
  only the known utility assertion fails, with all five reports unchanged. The small initial
  hourly increase does not reproduce in reversed order (5.667 ms baseline / 5.661 ms current).
  Private snapshot input demand and supplier quantities now use exact funded recipe activity,
  capped by current staff; rounded jobs cannot inflate production. Local repricing retains a
  valid import alternative when needed to preserve funded staffing and releases rejected claims.
  Fresh verification passes 2,213 library tests, all growth scenarios and both continuations;
  only the known utility assertion fails, and all five reports match. The first 4% hourly increase
  does not reproduce in reversed order (5.608 ms baseline / 5.503 ms current). Fresh physical
  placement verifies 1,024 factories and exact owner equity in 115.45 ms, excluding preparation.
  Equal-route recurring quotes now prefer the cheaper supplier before building index; cached
  search, rollback and proposed-door coverage pass with all 419 economy tests.
  Matched hourly timing detects no incremental change (5.710 → 5.669 ms), with identical city records.
  Recurring quotes now include receiving-window goods prices, with boundary revalidation and
  planning snapshots built from the matching refreshed funding view. Fresh verification passes
  2,216 library tests, all growth integrations and both continuations; only the known utility
  assertion fails, with all five long-run reports unchanged.
  The initial 8% hourly timing increase does not reproduce in reverse order (5.628 / 5.657 ms);
  discrete benchmark counts match, while current treasury ends 54 currency lower. Physical
  execution verifies 1,024 placements in 110.62 ms; warm queue checks remain flat with background size.
  Accepted local funding now publishes additional staffed output into the shared supply book;
  compiled dependency phases place suppliers before downstream buyers. Fresh verification passes
  424 economy tests, 2,219 library tests, all growth integrations and both continuations. Only the
  known utility assertion fails; all five long-run reports remain unchanged.
  The initial hourly increase does not reproduce in reverse order (5.663 / 5.565 ms, p = 0.23);
  discrete benchmark counts match, with current treasury 54 currency higher.
  Live consumer firms now reserve individual recurring family budgets and desired quantities at
  seller prices, with directed shopping round trips and exact rejected-quote rollback. Household
  car access shares the existing census and is reconstructed on load. Commercial construction now
  forks that customer book for constructing/queued projects and replacement rollback; quotes carry
  supported workers through selection, labour calendars and execution. Input-free services are
  included. Fresh focused checks pass 431 economy and 24 pathing tests before the final bounded
  route-cache follow-up. The subsequent current-source release run passes 2,227 library tests,
  all growth integrations and both reload continuations; only the known utility assertion fails.
  The saved-town year passes with an unchanged report and 15 operating-cash-positive firms.
  Fresh matched hourly timing is 6.508 → 6.462 ms against the pre-consumer build (p = 0.88),
  but 2.545 → 6.556 ms against ready-rehousing: the full regression remains. The daily workload
  also changes final employment, so it does not establish equivalent outcomes.
  Cost-only CCH queries and directed pedestrian traversal now reuse prepared search storage.
  A subsequent seller-price correction and reusable commute-query storage pass 2,229 library
  tests and all growth integrations, with only the known utility assertion failing; growth and
  saved-town reports match the preceding build. Matched hourly/daily changes are not significant
  (6.599 → 6.553 ms / 630.34 → 563.60 ms), and the full regression remains. Business financing
  and replacement dependency work are still active. The business ledger/router now has focused
  coverage for shared financing, directed delivery prices and exact replacement rollback.
  Live funding now validates provisional business purchases jointly: both required financing
  regressions pass, including local recipe cycles and unfunded-buyer withdrawal. Household
  output cannot be promised again to business inputs (442 economy tests pass). Subsequent work
  links business deliveries into owned input receipts, reserves current staff before expansion,
  and forks financed business customers into construction, queues and execution. The first
  construction integration passes 445 optimized economy tests. The preceding release passes 2,241
  library tests, all growth integrations, both continuations and the unchanged saved-town year;
  only the known utility assertion fails. Matched hourly timing regresses about 51%; reversing
  run order still shows about 16% (7.0712 → 8.2182 ms). Daily differences are not significant.
  The 10k benchmark ends with six fewer employed residents; each executable repeats its own
  records exactly. Performance and equal-outcome acceptance remain open. Replacement output/dependency
  revalidation, mixed-market closure and scaling remain open. Subsequent bounded complete freight caches,
  combined household/business activity and customer reassignment after financing rejection pass
  448 optimized economy tests. Fresh release checks pass 2,244 library tests, all three growth
  integrations, both reload continuations and the saved-town year; only the known utility
  assertion fails. All five long-run reports match the preceding build. The first matched
  hourly increase (7.123 → 8.001 ms) does not reproduce in reversed order (7.497 / 7.584 ms,
  p = 0.71); complete economic records match. Earlier regressions and outstanding runtime
  requirements remain open.
  Replacement probes now support exact enclosing rollback across several input/customer owners.
  Output publication and affected-buyer traversal remain unfinished. A focused fragmentation
  regression exposes receipt growth beyond the previous bound. Live matching now retries with
  prepared storage (452 optimized economy tests pass); construction/replacement growth remains open.
  Matched 10k timing detects no additional regression (7.165 → 6.815 ms hourly), with identical
  economic records. Earlier regressions remain open.
  The preceding rollback/numeric revision passes 2,247 release library tests, all growth integrations,
  both reload continuations and the unchanged saved-town year, with only the known utility failure.
  Prepared replacement sources now retain actual recipes, prices and doors through CCH metric
  changes; 453 release economy tests pass. Their output transactions still need connection to
  replacement admission and affected-buyer financing revalidation.
  The small initial hourly increase is not significant in reversed order (6.641 / 6.742 ms,
  p = 0.30); economic records match. Earlier performance regressions remain open.
  Operation records now retain supported plans through construction; replacement payroll is
  capped at target slots. Buyer-local business receipt withdrawal preserves exact enclosing
  rollback, and retained prices include receiving-window charges (455 release economy tests pass).
  Matched 10k timing detects no additional regression (6.882 → 6.962 ms hourly, p = 0.38),
  with identical economic records. Runtime affected-operation integration and full performance
  acceptance remain unfinished; details and evidence stay in `demand.md`.
  Customer lookup now retains buyer/recipe-port geometry across financing changes. Fresh checks
  pass 2,251 library tests, all growth integrations, both continuations and the unchanged saved-town
  year; only the known utility assertion fails. Matched hourly time improves in both orders
  (8.116 → 6.930 ms initially), with unchanged economic records. A subsequent exact stop after
  the last active bucket entry passes 456 release economy tests and both locality benchmarks.
  Its small initial hourly increase does not reproduce in reversed order (7.011 ms baseline /
  6.940 ms current, p = 0.51), with unchanged economic records. Full runtime/performance
  acceptance remains open.
  Buyer input reseeding now reuses withdrawn account slots and restores appended/overwritten
  rows and routing availability exactly on rejection (457 release economy tests pass).
  The initial hourly timing increase does not reproduce in reversed order (6.955 ms baseline /
  6.869 ms current, p = 0.29), with identical economic records. Runtime replacement integration
  and full performance acceptance remain pending.
  Household reservations now share the enclosing runtime replacement transaction, restoring
  several accepted owner changes together and recycling old receipts. Fresh verification passes
  2,254 release library tests, all growth integrations, both reload continuations and the unchanged
  saved-town year; only the known utility assertion fails. The initial apparent timing gain does
  not reproduce in reversed order (6.947 ms baseline / 6.951 ms current, p = 0.95), with identical
  economic records. Output publication, affected-operation financing and full performance
  acceptance remain open.
  New, queued and constructing projects now publish input purchases backed by their own
  supported operation. Placed construction uses actual cash/cargo/stock, and failed placement
  proofs roll back future demand. Buyer doors use prepared existing resource buckets and
  parcel identities. Selective downstream input repair now preserves unaffected supplier
  tranches and exact rollback. Replacement selection now withdraws/publishes output and
  revalidates affected buyers and suppliers in one transaction; 463 focused release economy
  tests pass, including buyer/supplier insolvency rejection and storage replay. Broad checks
  pass 2,259 library tests, all growth integrations, both reload continuations and the unchanged
  saved-town year; only the known utility assertion fails. Matched hourly timing regresses
  6.9024 → 7.1106 ms (+3.0%, p = 0.01). The physical execution guard subsequently passes
  the same broad correctness checks and unchanged reports; hourly timing increases
  6.9887 → 7.2055 ms (+3.1%, p = 0.01). Receipt-capacity preflight and full-proposal replay
  pass 465 focused release economy tests, including forced arena growth and exact runtime
  replacement rollback. Matched hourly timing shows no significant additional change
  (7.2829 → 7.3068 ms, p = 0.78), with identical economic records. Subsequent queue execution
  retains financed commitments backed by placed housing; 14 targeted queue tests pass,
  including pending-customer reload. The final queue source passes 2,263 library tests, all
  growth/reload checks and the unchanged saved-town year, with only the known utility assertion
  failing. Its initial hourly increase does not reproduce in reversed order (7.2386 ms baseline /
  7.2114 ms current, p = 0.74), with identical economic records. Profiling identifies CCH query
  work as a major cost; direct road-end label indexing passes 25 pathing tests and the final build passes 2,264 library tests, all growth/reload checks and the unchanged saved-town year (only the known utility assertion fails). Matched hourly timing improves 7.1613 → 6.9396 ms; daily timing shows no significant change. The full regression remains. Prepared household bucket positions subsequently pass 466 economy tests, including four-regime populated-market isolation. The first timing run regresses; reversed and repeated comparisons improve, the latter 7.1301 → 6.7020 ms hourly, with identical economic records. That runtime build passes 2,265 library tests, all growth/reload checks and the unchanged saved-town year, with only the known utility assertion failing. Connected-market repricing now passes after correcting premature customer ceilings; 2,266 library tests, all three growth integrations and the saved-town year pass apart from the known utility assertion. Affected input repair passes 468 focused economy tests and restores mixed-market closure from five to three full passes (53.262 → 34.494 ms at 102,400 residents); matched whole-economy timing shows no significant change and identical economic records. Its broad run passes 2,267 library tests, all three growth integrations and the saved-town year apart from the known utility assertion, with all five reports unchanged. The failed-expansion receipt-loss fix passes 469 focused economy tests and 2,268 library tests plus all three growth integrations and the saved-town year, apart from the known utility assertion; all five reports match. Repeated mixed-market timings are closer, with middle-size increases still recorded. Exact household-quote prefix reuse passes 470 focused economy tests, including an uncached oracle and changed-prefix reconstruction, and improves the largest mixed-market diagnostic from 35.408 to 28.114 ms. Whole-economy timing has no significant change and identical records. Broad validation passes 2,269 library tests, all three growth integrations and the saved-town year apart from the known utility assertion; all five reports match. A fresh whole-workload profile points to daily commute routing and hourly entrance geometry; sample loss and incomplete stacks limit the diagnostic. Lazy freight geometry and indexed business-customer route reuse pass 472 focused economy tests. Matched whole-economy timing has no significant change and identical economic records; after temporary-quota recovery, 2,271 library tests, all three growth integrations and the saved-town year pass apart from the known utility assertion; all five reports match. Subsequent worker-local commute-leg reuse and job-query scratch retention pass 473 focused economy tests. Matched timing improves daily settlement 514.89 → 103.30 ms (−79.94%) with identical economic records; hourly 7.0818 → 7.0013 ms has no significant change. Broad validation passes 2,272 library tests, all three growth integrations and the saved-town year apart from the known utility assertion; all five reports match. Exact cross-census household quotation reuse now validates budget/access inputs and seller requests before retaining claims; changed prefixes reset prepared accounts without recalculating the census. Fresh validation passes 475 focused economy tests, 2,275 library tests, all three growth integrations and the saved-town year, with only the known utility assertion and all five reports unchanged. Matched hourly timing improves 6.4418 → 5.2694 ms (−18.2%) with identical economic records; daily timing shows no significant change. Mixed-market warm medians decrease at all five measured sizes in both run orders. Full-route selection now also reuses prepared scratch, eliminating discarded candidate paths and connector allocations. Fresh validation passes 476 focused economy tests, 2,276 library tests, all three growth integrations and the saved-town year, with only the known utility assertion; all five reports match. Exact route-product comparisons pass and warm route medians improve at all four sizes in both run orders. Whole-economy timing detects no significant change (5.3495 → 5.3587 ms hourly; 99.079 → 97.666 ms daily), with matching economic records. The replacement matrix now also passes five queued-buyer variants, with exact all-owner financing/input/household rollback, 15 supported workers and subsequent physical grocery placement. Its final build passes 476 focused economy tests, 2,276 library tests, all three growth integrations and the saved-town year, with only the known utility assertion and all five reports unchanged. Matched timing detects no significant change. Corrected retry attribution puts local funding at 3.352 ms versus import funding at 0.283 ms in the diagnostic profile. Globally absent-customer propagation now reduces the 64-link withdrawal case from about 30 ms to 1.1 ms and from 64 censuses to two. Buyer counting shares the parallel market census; retries scan staffed input buyers. Its final build passes 478 economy tests, 2,278 library tests, all three growth integrations and the saved-town year, with only the known utility assertion and all five reports unchanged. Whole-economy timing detects no significant change; the initial largest mixed-market penalty no longer appears. A subsequent stale projected-supplier handle fix validates current membership before owner lookup; its fresh broad checks retain these counts and reports, with no significant timing change. The per-buyer search-cache trial was removed without a whole-economy gain. Disconnected chains now use component-local buyer tallies: the 64-link/16,384-background case falls from 29.975 to 1.261 ms (64 censuses to two). Final checks pass 2,280 library tests, all three growth integrations and the saved-town year with only the known utility assertion and all five reports unchanged; matched 10k timing detects no significant change (5.4304 → 5.4566 ms hourly, p = 0.84). The remaining unused-method warnings are resolved: superseded probes/harnesses become test-only. The long-standing utility assertion (540 versus 96) was a stale fixture from the retired MJ power recipe; it now expects 540 kg, so later runs have no known failure. The 2026-10-09 correctness follow-up propagates recounted purchase bounds through partly active supplier chains (32 censuses to at most three, exact against replay), preflights census receipt fragmentation exactly, and covers queued supplier-replacement and cyclic-replacement reloads; 2,283 library tests, all three growth integrations and all five reports pass unchanged, while one of four 10k hourly comparisons detected +12.9% that the others did not reproduce. The 2026-10-09 performance follow-up attributes the hourly gap to serial joint financing (supplier search rejected 90% of visited candidates), skips those regions with exact door bounds, hashes registry lookups faster and proves cashless firms unfundable up front: 10k hourly time falls 5.9985 → 5.0895 ms (−15.15%, p = 0.0014) with identical records, the Step-1 +12.9% flag is not reproduced (p = 0.25), the queue slowdown is explained as new required work, and cashless-rival cascade combinations need two censuses instead of one per link; profitability-failing chains still need one census per link, and the payroll/hiring mismatch (`ECON-17`) is fixed: payroll and worker shedding read the funded census capacity, removing the 10k fixture's 355 daily unpaid quits and 611 re-hires with all five reports unchanged. Broader populated execution/scaling stays open; `demand.md` owns the evidence.
  Broader dependency and performance acceptance remain open; `demand.md`
  owns the evidence and validation boundaries.
  Preceding broad checks pass 2,238 library tests, all growth integrations, both continuations and
  the unchanged saved-town year, with only the known utility failure. The joint funding pass
  still adds 12.199% hourly cost in matched runs (7.0255 → 7.8825 ms); performance is not accepted.
  The preceding build passed
  2,224 library tests and all three growth integrations, with only the known utility assertion failing.
  After owner receipts and cached pedestrian proofs, final checks pass 430 economy, 23 pathing,
  25 save tests, both continuations and the saved-town year; its report is unchanged.
  Matched 10k hourly timing regresses 5.7515 → 6.4500 ms (+12.1%); discrete counts match,
  treasury differs. The [fresh AI handoff](archive/demand_history_2026-10-10.md#ai-continuation-handoff--2026-10-08)
  records historical next steps, validation boundaries and artifacts; [continuation progress](archive/demand_history_2026-10-10.md#continuation-implementation--2026-10-08)
  records the current changes and verification boundary.
  Funded-buyer closure, cyclic local chains, replacement output publication, populated
  execution with changing frontage prices and full growth/performance acceptance remain open. See the
  [implementation contract](archive/demand_history_2026-10-10.md#implementation-contract-and-progress).
  The preceding independent-entry full run passes 2,164 library tests and all three growth scenarios,
  including two years with all five business profiles operating. The scarce-labour expansion
  test then passes separately after correcting its capital bound to count actual arriving adults:
  46 added industrial lots produce three factories, each above 95% staffing and operating cash
  positive, spending 26,940 in owner equity. The pre-existing utility-machinery expectation still
  fails. The saved-town replay retains its population after a year, and all 14 firms have positive
  cumulative operating cash with no bankruptcy/removal. Candidate-site freight, local buyer
  affordability/connectivity and full-redesign performance acceptance remain open. This revision is not accepted
  or deployed.
  Failed-business recovery, grocery viability and regional industrial markets repaired 2026-10-06;
  existing village services, area-based producer costs and explicit resource units extend that foundation.
  Production/freight economics now replace fixed outside buying caps; validation is in progress. Next resolve varied-town affordability,
  define funded activities and simplify demand around unmet capacity.
  - The private local-consumption comparison gives 144.50 jobs per 1,000 residents with 20 ha
    reference grain farms, mostly
    food retail/processing. Additional meshes do not create customers; new activities need
    affordable spending or explicit external/public funding.
  - The [regional-market repair](archive/economy_history_2026-10-10.md#regional-industrial-markets-econ-14) replaces export
    price decay with finite shared customer volumes at 1.0× prices; imports remain 1.25×.
    New factories must cover full operating costs, and household admission requires recurring
    income rather than temporary benefits or pantry-refill jobs. Shopping and rehousing fixes
    keep supply tasks viable. The two-year village now sustains 58 residents and 45 jobs, with no
    bankruptcies or replacement capital, no second-year stockouts, and all eligible benefits paid.
    The [service and producer audit](archive/economy_history_2026-10-10.md#village-services-and-producer-audit-econ-14)
    opens barber/pharmacy at 60 residents, each with one worker and positive year-two operating
    cash (+5,719 / +2,685); city recurring balance is +199,536 after transfers and road upkeep.
    Farms/mines now charge 20,000 / 25,000 per added hectare and consume area-scaled utilities.
    Placement/edit previews quote both costs. Corrected freight clock conversion restores input
    deliveries. One-hectare producers cover ongoing costs; oversized sites fail without fresh
    capital. Initial private capital and diagnostic infrastructure endowments are explicit.
    Varied-town pantry shortages and pensioner affordability remain open; the owner doc records
    fresh correctness and matched 10k performance evidence.
  - [Resource units and payload](economy.md#resource-units-and-freight-payload): physical goods
    use kg, electricity kWh (MWh display), water/sewage m³ and services completed-service equivalents.
    Processing and retail quantities conserve mass while price conversions preserve recipe
    monetary values. A carrier holds at most 20,000 kg; per-resource delivery batches remain
    40/50/100 kg. The payload check adds O(1) arithmetic, with no extra routing or trip expansion.
    Save format 72 preserves arrival composition and carrier quotes and still converts pre-70 quantities; oversized
    legacy shipment cargo is rejected. [Production/freight economics](archive/economy_history_2026-10-10.md#production-and-freight-economics-econ-14)
    remove outside purchase quotas. Quotes use cached route time (4 dispatch + 4 per round-trip
    vehicle-hour), with no additional routing for pricing. Producer reorder lots and daily-output
    export batches replace tiny hourly loads. Grain farms produce 20 kg/ha/day (7.3 t/ha/year)
    with two to six workers; placement and resizing enforce a 4 ha minimum. Area
    construction and upkeep remain uncapped. Expansion waits for operating industry to recover startup cash. Unemployed adults
    now accept reachable funded work even with household savings, matching demand’s labor pool.
    Earlier farm validation: 395 Rust checks and both Godot economy/field checks pass; 4/10/20/100 ha
    farms survive two years with positive final-quarter cash and no recapitalization. Matched 10k
    timing detects no significant change. The subsequent shopping repair retains task ownership
    until hourly delivery acknowledgement; work departures no longer discard paid cargo. Admission
    now reserves incoming adults and transfer claims, checks each family's recurring income, and
    preserves approved composition through arrival/save. Applicants advance with simulation hours
    even after rejection; bulk-input purchase days no longer stall recovered factories. Fresh
    validation: 392 targeted Rust tests and all three growth integration tests pass. The two-year
    village grows from 114 residents / 88 employed at year one to 156 / 119, with no replacement
    business capital, no second-year empty-pantry days, and positive recurring city finances.
    All four farm towns now stay supplied through year two. A 20 ha mine's depletion still leaves
    unemployment and household shortages (182 empty-household-days in year two); recovery after
    a large finite-resource closure remains open. The 1/4 ha mine towns stay supplied after depletion.
    Fresh 10k timing on the same eight performance cores detects no significant change
    (hour 2.086 → 2.050 ms; daily intervals remain broad). The fixture has one extra employed
    resident after the admission fix. Unpinned mixed-core comparisons and full limits are in the owner doc.
    The following results belong to the earlier unit-only build.
    [Unit-only validation](archive/economy_history_2026-10-10.md#unit-and-payload-validation-econ-14): 350 Rust checks, three
    growth scenarios and the Godot quantity display pass; matched 10k timings show no significant
    change. Two residential selection tests fail identically on the pre-change build and remain
    separate `DEM-02` startup/selection work.
  - The 2026-10-05 review at `21100889` passed targeted unit tests and concurrent-year
    determinism; both enabled economic targets failed. The starter scenario ends with 12
    households, no jobs and treasury −42,465. The 600-household regional-pull term is not a hard
    immigration cap and does not explain this early failure.
  - The 10k timing fixture runs a live economy since 2026-10-09 (`ECON-18`): agents move and
    `OWA` freight is uncapped, so only road traffic limits trade. Stock checks are spread evenly
    over the hours (`ECON-19`). Since 2026-10-10 each grocery produces, staffs and restocks at
    its own funded census activity instead of a citywide ratio (`DEM-02`), so output matches
    what households buy and empty pantries fall from about 3,750 to 169; service stores follow
    the same rule. Grocery route checks reuse the cached walking proofs with identical results.
    The new baseline is 34.18 ms hourly and 8.60 ms daily. The first live 100k run completes
    after the `ECON-20` crash fix, but its daily workplace search grows quadratically and its
    single border road starves industry. Job search now shares exact network legs across
    workers and hours: 100k option building falls 28% with identical records, and the 100k
    household hour is the main hourly cost. See the
    [fixture record](archive/economy_history_2026-10-10.md#live-economy-fixture-econ-18-2026-10-09).
  - Use matched **10k** hourly/daily performance checks. **The 1M run is deferred** and larger
    tiers do not block this work. Existing scaling fixes (`ECON-15`, `ALLOC-02`–`ALLOC-04`,
    `ROAD-44`–`ROAD-49`, `ROAD-52`, `ECON-16`) and their measurements remain recorded in the
    owner docs; road span earthwork records remain separate work.
  - Healthy-growth acceptance covers household affordability, business survival beyond startup
    funds, recurring fiscal balance and growth where opportunity exists. Numerical ranges and
    the first added activities are still design work. See the
    [plan and evidence](economy.md#growth-redesign-econ-11econ-12-dem-02).
- **Road preview display performance (`ROAD-30`–`ROAD-35`, `ROAD-37`–`ROAD-40` and `ROAD-42` done;
  `ROAD-36`, `ROAD-41` and `ROAD-43` parked)**: prioritize **Road and terrain**. With input-to-display baselines captured, tackle native buffer validation, rendering
  resource reuse, retained geometry locality, packed payload deltas, worker scheduling and
  compatible mesh updates one at a time. Each step needs matched performance and correctness
  evidence. Keep Road only as a comparison; decide whether to retain both modes after measuring
  the improvements. `ROAD-30` now has request-correlated timings, two matched rendered release
  baselines and a diagnostic matrix. Road validation and terrain-resource creation are the
  larger measured frontend costs. `ROAD-31` moves road-buffer validation into a Rust seal certificate.
  Main-thread validation drops from 1.9–5.0 ms to ≤0.11 ms, and matched stationary medians improve 1.6–4.7 ms
  in full mode. `ROAD-32` recycles preview nodes, meshes, materials and textures through a bounded
  display/staging pair. Per-request terrain resource creation drops to zero and full-mode stationary
  medians improve 3.0–4.2 ms. `ROAD-33` keeps unaffected existing road geometry resident while
  junction bounds move. Only the edited approaches are re-clipped and sent, which cuts dense moving
  payloads by 43–66%. `ROAD-34` gives full-mode terrain products display revisions. Unchanged
  road-free patches are sent as metadata and keep their displayed slots, so each pose restages 4–6
  of 16 patches and full-mode stage-and-install falls 0.3–0.5 ms. `ROAD-35` keeps the worker fed with
  the latest pose: idle gaps vanish and 15–70% more previews show while moving. Displayed age falls
  about 11 ms where compiles fit a frame but each result can be a frame older where they take 1.5–2
  frames; that trade-off is accepted. `ROAD-36` measured in-place mesh buffer updates and parked
  them: only 5–30% of moving road-layer bytes are compatible and all submission costs under 0.5 ms
  per result. The 2026-09-29 bottleneck analysis of the 10-case matrix shows the Rust junction
  compile dominates: `road_ms` is 75–90% of worker time, and a four-way `JunctionN` takes 30–38 ms on
  one thread. Two cheaper defects add to it. The previous preview is freed under the result lock,
  costing 1–6 ms per result plus main-thread waits. Junction tasks also starve a small shared Rayon
  pool, causing 40–103 hitch frames in the crossing fixtures with four workers. `ROAD-37` now frees the
  previous preview on a bounded reclaim thread after an O(1) swap, and the lock wait is gone. `ROAD-38`
  gives the preview worker its own Rayon pool, sized to the machine's logical CPUs. The four-worker
  crossing hitches fall to zero with identical products. `ROAD-39` indexes explicit-step lookups in
  node export: junction `export_ms` roughly halves in crossings, with identical products. `ROAD-40`
  inlines rounding, uses fixed-seed hashing and removes re-sorts. A single-thread junction compile is
  9–11% faster, and moving worker time falls 4–11%, with identical products. mimalloc was measured and
  rejected for now: worker time −6%, but peak RSS +31% and a +23% 1M-agent idle tick. The idle-tick regression is `CODE-15`. Three items were measured and dropped:
  - `ROAD-41` (parallelism inside one junction) is parked: the compile is a chain of 0.1–2 ms stages, and the one candidate was neutral in Godot.
  - `ROAD-42` is a no-go: offering the previous pose's topology cache found almost no reusable keys and slowed moving compiles by up to 2%.
  - `ROAD-43` is parked: the roughly 90,000 allocations per junction compile are small stage products, not reusable scratch, and removing 10% of them changed nothing measurable.

  Next: the Road only review
  ([findings and tasks](archive/roads_legacy_surface_2026-10-23.md#preview-compile-bottlenecks-road-37road-42)).
  [Ordered plan and acceptance criteria](archive/roads_legacy_surface_2026-10-23.md#preview-display-performance-plan-road-30road-36).

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
  [`roads.md`](archive/roads_legacy_surface_2026-10-23.md#kuopio-terrain-regression-replay-road-24).
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
| Debug channels / log / in-game debug console / `run.sh` debug flags | [`debug.md`](debug.md) |
| Simulation clock / scheduler / sim-frame budget | [`simulation.md`](simulation.md) |

## Recent Structural Changes

- `TERRAIN-15` done (branch `terrain15`, 2026-10-30): the game is playable while the map streams. A load or a new game hands the game over (input, camera, clock) once the terrain within 2 km of the camera is ready, not once every patch out to 8 km has settled; the far terrain streams on within the frame budget, its unready patches showing the background. Playable after 2.5-3.1 s instead of 6.0 s on a Kuopio save, 2.4-2.7 s instead of 5.9 s on a new game, 38.9-39.4 s instead of 48.1 s at 100k, whose 20 s load call and GDScript set builds (`TERRAIN-13`) remain. See [`terrain.md`](terrain.md#godot-tile-renderer) and T53.
- `ROAD-68-G` done (branch `road68-G10`, part 10, 2026-10-29): a road commit is acknowledged within one frame at every populated tier. Rebased onto `TERRAIN-07-G` (the legacy terrain state is gone), the commit plans its lots in a job the next frame adopts, starts its routing rebuild after that plan, repairs only the attachments its edit reaches, and now queues its undo record's kept terrain tiles after its last stage. Every populated first commit takes 9.6-13.8 ms at 10k-100k residents (control 21.1-27.5 ms); products, locality products and agent records are bitwise unchanged ([`roads.md`](roads.md#commit-stages-part-10-the-gate-road-68-g-2026-10-29)).
- `TERRAIN-07-F` done (branch `terrain07-F`, rounds 1-14, 2026-10-28): every consumer reads the graded surface (grass exclusion, overlays, water suppression, site feasibility, inspection, vehicle ground). T37's 250 m vehicle-ground LOD with T45's 0.020 m contact lift; T44's shared-owner query index (4.46 MB at 100k), now published with the canonical roadbed piece map. Matched 100k gaps +0.322 ms culled pose and +0.264 ms snapshot against +0.42/+0.40 bars. Round 14 fixed grass rebinding unchanged mask mirrors every frame, the real-display matrix's `city_low` miss. T44, T46 (b) and T47 (a) agreed by the user, validated at `ROAD-68-H`.
- `TERRAIN-07-E` done (branch `terrain07-E`, rounds 1-7, 2026-10-27): round 7 found that round 6's hole check could never fail (it compared a 32-bit float with a 64-bit constant); with it fixed, the renderer before round 6's fix fails after the second load and the tip passes 13 loads, and renders show `road`'s gate D tip with the same hole after a reload, and the grass on the asphalt and the zoning cells the same on gate D (gate F's). A built cell set now waits hidden under its patch's node, so starting to draw moves no node. The real-display probe passes (0 failures, 0 holes; pose 1.55 ms, commit 0.49, undo 0.85). The first idle-frame matrix read Kuopio's close and overview 0.6-0.8 ms over the exact cut: they were sampled inside the background cell-set builds after the load (about 3 s uncapped, 82 s at 60 Hz); once those finish the graded terrain costs what the exact cut does. The user accepted that window; the matrix now samples the steady state (`5972bef8`): Kuopio's poses at the exact cut (`close` +0.01, `overview` -0.00, `ground` -0.02 ms GPU p50), `saved` +0.21 (T35's), `city` -0.41, `city_low` -0.13, 0 overflows. The user carried four close results to `ROAD-68-H` (`roads.md`).
- `TERRAIN-07-E` round 6 (branch `terrain07-E`, work in progress): the user accepted the click right after a fast drag taking its terrain past 3.0 ms (`terrain.md` T27 (f)). A drag probe on the real display found pose frames of up to 17.7 ms where a road first reached a patch: its cell set was built whole in that frame. Every resident patch now builds its set in the background, hidden and holding no instance-uniform slot until it draws, with its data kept on the CPU so a tile's move no longer reads the GPU buffer back; the commit takes over the preview's tiles without uploading them. Under gamescope every catch-up, commit and undo frame now fits 1.0 ms and pose frames take at most 1.8 ms (the sets starting to draw); the display run on these fixes is pending. A world load no longer leaks the previous renderer's cell sets.
- `TERRAIN-07-E` round 5 (branch `terrain07-E`, work in progress): the road tool's preview now shows its graded terrain tiles in Godot: each frame hands the terrain renderer the tiles it changed, which draw from slots of their own over the installed tiles at the camera's levels (`terrain.md` T42, for review), and a commit's tiles install through the edit journal in one frame with the preview's released, never rebaked; the exact cut's preview patches and `road_terrain_preview.gd` are gone. Preview, committed tiles, a fresh bake and the bake after save and load are equal bit for bit in `road_junction_preview_test.gd`. Finer sampling bands take the worst tile from 2.67 to 2.30 ms; the click right after a fast drag still misses 3.0 ms because it is bound by its tiles' total work, so `terrain.md` T27 goes to the user (recommended: hold such a click until the preview has caught up, at most a few frames). A crossing committed across a trunk on a 10 m world panics in the exact cut's legacy road loops (named for `TERRAIN-07-G`).
- `TERRAIN-07-E` round 4 (branch `terrain07-E`, work in progress, rebased onto gate D): the road tool's preview now finishes a large drag pose's terrain over the following frames (`terrain.md` T27 (d), decided by the user): one tile in the pose's frame, nearest the dragged end, then three a frame, each shown tile an exact bake of some pose, converging to the committed tiles bit for bit; every Kuopio and populated drag frame stays within the 3.0 ms terrain stage (worst 2.87 ms; the populated first poses fell from 3.9-4.4 to 1.4-1.9 ms). A click right after a fast drag bakes what the preview had not, up to 4.3 ms (open for the user). The road tool's per-frame call `advance_road_preview_terrain` is in place for the Godot half.
- `TERRAIN-07-E` round 3 (branch `terrain07-E`, work in progress): each terrain tile of a preview pose, commit or undo now gathers its inputs once, and a tile whose inputs are unchanged stops before sampling; every tile is bit for bit as before, and Kuopio's drag poses are about 0.1 ms faster (worst 4.3-4.6 ms against 5.2 ms, 13-16 of 299 still over the 3.0 ms stage). The occasional 23-47 ms stall of an undo or a drag's first pose in the populated city was the commit's routing rebuild: it ran on the same thread pool as the frame's work, which then waited for it. All background work (the routing rebuild, the undo record's terrain tiles, deferred frees and payload jobs) now runs on a background pool of its own, and no stall remains in repeated runs. Spreading a large pose's terrain over the following frames, recommended in round 2, would show stale tiles and is withdrawn; anchoring a growing road's stations at its start, estimated to bring every Kuopio pose but one within 3.0 ms if the road's heights behind its end stay put, is the user's decision (`terrain.md` T27). See [`terrain.md`](terrain.md#terrain-07-e-record).
- `TERRAIN-07-E` round 2 (branch `terrain07-E`, work in progress): a preview pose's terrain tiles are now split across the worker threads row by row, the road coverage test skips the points a triangle cannot reach, and the tile selection decides most tiles before reading their surroundings; every tile is bit for bit as before. On the Kuopio drags the worst pose falls from 7.1 to 4.7 ms and the median from 1.40 to 1.24 ms, but 19-22 of 299 poses still exceed the 3.0 ms terrain stage, as do the first poses of the populated city's drags (3.8-5.2 ms at 10k-100k residents): the choice between finishing a large pose's terrain over the following frames and anchoring a growing road's stations is the user's (`terrain.md` T27, the first recommended). Measured as the game commits, with the preview's tiles adopted, the populated city's first road commit takes 25-30 ms against the 16.7 ms frame, 8-11 ms of it lot regeneration; the later commits 6-16 ms. See [`terrain.md`](terrain.md#terrain-07-e-record).
- `TERRAIN-07-E` round 1 (branch `terrain07-E`, work in progress, the Rust half): the road tool's preview, the road commit and undo prepare the graded terrain tiles, still read by no consumer. A preview pose bakes only tiles whose inputs changed since the last pose, the commit takes the preview's tiles when their inputs match, and undo puts back the tiles the commit replaced, which a background job bakes after the commit; a tile is reused only when its inputs are unchanged, so every tile equals a fresh bake bit for bit. The commit's legacy terrain validation is gone: no terrain failure refuses a road. Terrain undo now restores the source terrain the roads and lots read, and a terrain edit that would leave a road steeper than 1:3 is refused, so a sculpt can no longer leave a city whose save fails to load (`roads.md` question 48, closed). On Kuopio the commit's terrain takes at most 1.35 ms and undo's 0.99 ms; a dragged road rebuilds its whole span every pose, so its poses reuse almost nothing and 31 of 299 drag poses miss the 3.0 ms stage (`terrain.md` T27, open). See [`terrain.md`](terrain.md#terrain-07-e-record).
- `ROAD-68-K24` (branch `road68-K24`): the road commit benchmark now runs without the test build's extra checks, as the economy benchmarks already did, so the times it reports hold only game work (the user's decision on [`roads.md`](roads.md#road-68-open-questions) question 50). Measured once against the same code with the checks: every result is bit for bit the same, the time agents take per frame drops by up to 3 ms in the 100k city, and the road commit itself does not change; the 100k commit still misses its 16.7 ms budget in two of three samples. The benchmark city could never be built for 1,300 or 13,000 residents: its street grid rounds its width down to whole blocks, which for those sizes leaves too few homes. Such a city is now built one block wider, and every size that already worked is built exactly as before (question 55, decided by the coordinator under the user's delegation, for review). The wider 13,000-resident city showed that the benchmark city moved its first households in without taking them out of the road occupancy lists; it now does. A saved and reloaded benchmark city keeps a few fewer cached empty map areas than a freshly built one, which a later request rebuilds; the benchmark's comparison now counts everything else in that store. See [`roads.md`](roads.md#commit-breakdown-without-test-only-checks-road-68-k24-2026-10-26).
- `ROAD-68-K19` (branch `road68-K19`): the test that decides whether a point lies on the road surface, which the terrain's road coverage, road heights and lane heights read, is now exact: it no longer counts points up to 0.1% of a triangle's height past its edges, and a point on an edge counts for every triangle that meets there, so the answer is the same however a junction is turned (confirmed by the user, 2026-10-26). Roads, junctions and saves are bit for bit the same; coverage and road heights change only at points a hair beyond an edge that the old test still counted ([`roads.md`](roads.md#road-68-open-questions) question 37). The exact test made the graded terrain's road coverage slower. `ROAD-68-K23` found that the whole rise in the 10k city's commit is that coverage pass and made it faster with the same answers, but the graded stage still ran 0.12-0.22 ms over `road` there. `ROAD-68-K25`'s landing session measures the graded stage under its 3.0 ms budget at every tier and sample from 0 to 100k (at most 2.72 ms, where `road` itself takes up to 3.11 ms), with every product as recorded, so the branch is ready to land: the 3.0 ms budget, not `road`'s speed, is the target (decided by the coordinator under the user's delegation, 2026-10-26, for review). The same branch carries the crossing ends of foot lanes on the carriageway rather than on the curb by rounding (`ROAD-68-K20` part 2; the surface choice is pending the user's review) and a fix that measures the angle between a junction's roads the same however the junction is turned: the reference 25° Y was refused in two of its eight orientations, on `road` too.
- `ROAD-68-K22` (branch `road68-K22`): the road commit benchmark no longer builds its 10k, 30k and 100k cities in every timing run. Each test binary saves them once before a session, through the economy benchmark's saved-city code and its check that a loaded city continues exactly as the saved one, and every timed run loads them; the measured commits give bitwise the same products on a loaded city as on a built one. That check found a save bug: a game saved while households were on their way into the city forgot them, so the loaded game planned the rest of the hour differently. Saves now keep them (save format 108; older saves do not load, and the Kuopio reference save is re-recorded with every road point unchanged). See [`roads.md`](roads.md#saved-cities-for-the-commit-breakdown-road-68-k22-2026-10-26) and [`simulation.md`](simulation.md#save-bytes-road-68-k16).
- `ROAD-68-K21` (branch `road68-K21`): the user's decisions of 2026-10-26 on [`roads.md`](roads.md#road-68-open-questions) question 37. The road tool's cursor snap and the bulldozer pick the nearer road, and between roads at exactly the same distance always the one with the lower id, whatever order the edge index holds them in; the bulldozer's near-tie tolerance, which could pick the farther road, is gone. A save holding a road steeper than 1:3 no longer loads: the load fails with `road_too_steep`, the rule an edit meets, through the existing load failure path (an error message, no crash, the live city kept). Loads repair nothing any more: the junction repair for saves from before `ROAD-68-C` and two dead branches for saves without the budget and service tables are removed. Every road, junction and save product is unchanged. Open for review: until `TERRAIN-07-E` refuses them, a terrain edit can leave a road steeper than 1:3, and that city's save would not load (question 48).
- `TERRAIN-07-C` (branch `terrain07-C`, five rounds): building sites and structure ends on the graded terrain, still read by no consumer. The terrain is each building's pad at its support height, bitwise; yard and frontage paving are material classes equal to the legacy paving partition's; a placement or removal rebakes only the tiles within its reach (0.7-2.8 ms, flat from 0 to 100k background sites); retaining walls close the step where one road's embankment stands over a lower road or a pad over a road. Bridges and tunnels can be built from a road in play again: a road closes its edge across a structure's mouth, a bridge's free end and slab ends are closed, the terrain walls the step under a bridge's mouth, and near a tunnel's mouth the ground over the deck opens wherever it stands less than 5 m above it, with walls along the tunnel's sides. Every value stays bitwise equal to the evaluation over every seam. Decided under the user's delegation for review: the structure-end shapes (`terrain.md` T22) and one narrowed rule of the opening. Open for review: with the city's sites as grading inputs the first road commit beside built-up blocks takes 3.1-4.1 ms of terrain against 3.0 ms, carried to gate E's frame paths under T20 (a) (T21). See [`terrain.md`](terrain.md#terrain-07-c-record).
- `ROAD-68-K18` (branch `road68-K18`): the check that every junction the road tool touches still lays out and sits on its plane, which runs on every preview pose, no longer allocates working memory: the junction's arms, layout and mouth stations are built in buffers that stay with each worker thread, and the check no longer builds the mouth sections it threw away. In the preview benchmark its allocations fall from 222,520 to 69 and the plan stage's from 450,320 to 227,693; the roadbed's own plans allocate only what they keep. Every road, junction and save is bit for bit the same. The R-tree queries whose results depend on the tree's internal order are recorded (only two pickers' ties, which no fixture exercises), with the recommendation, open for review, to sort those two by edge id when the tree's re-index is narrowed ([`roads.md`](roads.md#road-68-open-questions) question 37).
- `ROAD-68-K17` (branch `road68-K17`): the solve that fits each junction's plane and eases its roads onto it, which runs twice on every preview pose and on every commit, no longer allocates working memory: its buffers stay with each worker thread, as the road tool's height solve's do since `ROAD-68-K16`. In the preview benchmark its allocations fall from 163,224 to 11,619 (the R-tree's own bookkeeping and the profiles it writes), and every road, junction and save is bit for bit the same ([`roads.md`](roads.md#road-68-open-questions) question 37).
- `ROAD-68-K16` (branch `road68-K16`): `ROAD-68-K15` lands on `road` after gate E and `TERRAIN-07-A`. The road tool's height solve, which runs on every preview pose, no longer allocates working memory per stroke: its buffers stay with each worker thread, and every road it builds is bit for bit the same. Saves are now byte-for-byte reproducible: lane connections were written in hash-map order, so two saves of the same city differed; they are written in key order, and the save time is the save's only outside input. The Kuopio reference save is re-recorded (only that row order changes) and re-pinned. The commit's `surface` stage rise that `ROAD-68-K15` had put down to code layout without a profile is now attributed by profile: in `ROAD-68-K15`'s build the site paving query's envelope test was compiled out of line (the same queries, more instructions), and the rebased build has no rise. What the grade rule still costs the preview's plan, about 0.016 ms a pose, is its own work, measured by profile (`roads.md` question 37).
- `ROAD-68-K15` (branch `road68-K15`): the blends that ease a road onto a junction's plane stay within 5 m of the ground, as the road itself does, so all of the recorded Kuopio session's roads meet the replay's quality limits again; where a junction's plane and the ground leave no such blend within 1:3, the edit is refused as `road_too_steep`. A terrain edit re-grades the roads on it by the same rule (refusing one that would leave a road steeper than 1:3 is still open). Old saves with steeper roads load unchanged; only new edits are refused. The Kuopio reference save is re-recorded (five roads move by at most 0.36 m). See [`roads.md`](roads.md#road-68-open-questions) questions 37-39.
- `ROAD-68-K14` (branch `road68-K14`): roads are at most 1:3 steep (the user's decision: just under the world's steepest streets). The road tool refuses a steeper road as `road_too_steep` with the grade and the limit before the click, and shows an amber warning from 16%; node moves and other node edits refuse it too. Profiles are built within 33% where their ends and the 5 m cut-or-fill limit allow, including the blends that ease a road onto a junction's plane, so the recorded Kuopio session's 36 strokes all build again with no road over 1:3 (21 before). The Kuopio reference save is re-recorded from those strokes with today's code as an interim reference until the user's new save, which is pinned the same way. Four re-recorded junction blends leave the ground by up to 0.36 m more than 5 m ([open question 39](roads.md#road-68-open-questions)). See [`roads.md`](roads.md#road-68-open-questions) questions 37 and 38.
- `TERRAIN-07-B` (branch `terrain07-B`): road skirts and bridge fascias. Road pieces now hang strips from their outer edges, a layer of the piece mesh beside the top surface: 0.5 m skirts on every grounded piece, so the graded terrain can sag below a road's edge without a visible gap, and fascias on bridges, which close the slit between the deck and its concrete slab (the slab now shares their vertices). The skirts travel in the road chunks' former earthwork layer and the curb shader deepens their bottoms with the camera distance. Every top-surface product of the roadbed probe is unchanged; the road chunk watertightness checks are stricter, since every outer edge of a road must now be closed by a skirt or fascia. See [`terrain.md`](terrain.md#terrain-07-b--road-skirts-in-the-piece-mesh).
- `TERRAIN-07-A` (branch `terrain07-A`): the graded terrain's grading core and queries for roads, beside the legacy exact cut in `simulation/terrain/graded/`. Seams are the roadbed's grounded outer-chain segments in key order, coverage is the roadbed's own top-triangle predicate, the pointwise grading function is pruned exactly per tile, and tiles bake on four level lattices into texels and seam rows, independent of batch, order and worker count. Dirty tiles follow each changed segment's own reach and, for changed pieces, only where the covering triangles differ, so a populated commit bakes 6-12 tiles in 1.55-2.48 ms from 0 to 100k residents. The spec gained corrections (the inside rule's depth limit, coverage from grounded pieces, the selection rule) and open questions T18 (hilly terrain cost) and T19 (point-query cost). The user decided T2: where two roads' tie-in cones conflict the ground takes the upper road's embankment and a retaining wall closes the step at the lower road (walls built in `TERRAIN-07-C`). Rounds 2 and 3 made the pruning cheap on hills while keeping every value bitwise: two passes per 4 m bin, a per-sample stop once no later seam can change the value, and four points evaluated together. Round 4 (gate done) re-measured on the Kuopio reference re-recorded under the 1:3 grade limit: a populated edit takes 1.02-2.77 ms, the tallest retaining wall falls from 43.9 m to 0.37 m, and 33 of 44 whole-edit bakes fit 3.0 ms (worst 6.70 ms). Open question T20 is decided (a) by the coordinator under the user's delegation, for review: undo keeps the tiles it replaced (about 1.4 MB per entry, 3.7 MB at most) and previews bake only what changed since the last pose, so no frame bakes a whole edit, and gate `TERRAIN-07-E` accepts those frame paths against the unchanged 3.0 ms. See [`terrain.md`](terrain.md#terrain-07-a-record).
- `TERRAIN-07` specification drafted for review (branch `road68-T07spec`, docs only): the graded terrain surface (approach B, 0.5 m texels, user decision 2026-10-25) with its invariants, grading function, levels, renderer, queries, budgets, gates and open questions in [`terrain.md`](terrain.md#graded-terrain-surface-terrain-07); the legacy terrain surface sections of `terrain.md` and the CDT parts of `earthworks.md` are archived in [`archive/terrain_legacy_surface_2026-10-25.md`](archive/terrain_legacy_surface_2026-10-25.md).
- `ROAD-68-F` part 2 (branch `road68-F2`): the remaining road families that do not touch terrain, tested flat and sloped through the node-piece checks, the product probe and the road chunk watertightness check. Roads without sidewalks lay out, cap and cross as specified (only roads with sidewalks cross); a mouth now keeps every run beyond a corner at least 1 m long, which removed a sliver where an arm without a crossing has two corners ending a hair apart. A straight degree-two node between two profiles (a width change, or sidewalks against shoulders) is laid out with D4 tapers, since such spans share no mouth row; the span model's own width taper is removed. Bridges and tunnels are pieces like any road and meet roads at node pieces or span mouths watertight; their abutments and portals (the terrain side of `ROAD-61`) are `TERRAIN-07`'s, and a structure stroke that joins a road is rejected until then. Zoning cells and fields are tested against road pieces through one triangle walk on the pieces' own grids. Open question 17's footpaths beside roads without sidewalks stay open. Round 4 (branch `road68-F4`) closes gate F: a 0.06 ms slower preview payload after part 2 was the road chunk router's inlining and the triangle clip's copies following the crate's codegen-unit partition (same chunks bitwise); the router is now compiled beside its caller and the clip works in place. See [`roads.md`](roads.md#remaining-families-road-68-f-part-2-2026-10-25).
- `ROAD-68-F` part 1 (branch `road68-F1`): crosswalk paint, street lights and road signs are built from the roadbed's pieces, never from lanes, in the one chunk emission the road tool's preview and the commit share, so a previewed stroke with crossings or signs publishes exactly its previewed chunks ([open question 26](roads.md#road-68-open-questions), met). Crossings come from the node pieces' layouts, or from the span row a pass-through crossing keeps, oriented by the lanes' own rule, so the painted zebra and its pedestrian connectors share one kerb segment bitwise. A road end keeps its street lights 6 m clear of its round cap. The faded bars noted on the sloped Y are the designed studded-tyre wear of old crossings, not z-fighting ([open question 35](roads.md#road-68-open-questions)). See [`roads.md`](roads.md#markings-lights-and-signs-from-the-layout-road-68-f-part-1-2026-10-25).
- `ROAD-68-E` part 2, round 7 (branch `road68-E7`): nothing visible changes. A road commit hands the road tool its copy of the zoning cells while it copies the road graph for routing, so the hand-over costs no extra time, and preparing many new lots at once (a populated city, a load, a large paint) no longer slows down with their number, which round 5 had made quadratic; every product is bitwise unchanged. Gate E lands with this round; building a road's lots in its preview stays paused ([question 33](roads.md#road-68-open-questions)) and the terrain stage is `TERRAIN-07`'s.
- `ROAD-68-E` part 2, round 6 (branch `road68-E6`, in progress): nothing visible changes. The game's zoning cell store is fast again (plain maps), and the road tool keeps its own exactly maintained copy, updated after each edit from a record of what changed ([question 32](roads.md#road-68-open-questions)). Building a road's lots in its preview is paused by the user until the commit is measured with `TERRAIN-07`'s terrain ([question 33](roads.md#road-68-open-questions)). A rare false "invalid" in the road tool's hover test was the test misreading a "try again" answer.
- `ROAD-68-E` part 2, round 5 (branch `road68-E5`, in progress): nothing visible changes. The slower road commits beside painted cells come from the shareable cell store itself, not from sharing it; three fixes were measured and none removes it, so how the road tool gets a cell store it can plan lots on is [question 32](roads.md#road-68-open-questions) (the user decided: find the cost first, otherwise keep the game's store fast and give the road tool its own exactly maintained copy). Lots are now planned by one planner and installed by one installer, the step the preview will reuse.
- `ROAD-68-E` part 2, round 4 (branch `road68-E4`, in progress): round 3 on top of `ROAD-68-F` part 2. The junction rule of open question 31 also applies to part 2's narrow rural Y and its width and sidewalk-to-shoulder changes on slopes and at bridges, which changes six of their probe entries; nothing else changes. Its second part keeps the zoning cell store in a form the road tool can share without copying (the first step towards building a road's new lots in its preview; the preview will not show them, user decision); nothing visible changes, but road commits beside painted cells are about 1 ms slower until that is fixed.
- `ROAD-68-E` part 2, round 3 (branch `road68-E3`, in progress): the road tool's preview now shows, before the click, when a road or its junction corners would cover a farm field or painted zoning cells, with the same test the commit uses; painting cells refreshes what the tool sees. A road whose junction lies next to another junction outside the stroke's immediate neighbourhood no longer fails at the commit with a broken far junction (a recorded Kuopio stroke and a street-grid T on an 8% slope failed this way). Junctions on slopes whose mouths reach beyond 12 m (narrow Y junctions, wide roads at an angle) now take their tilt from the ground across their own extent, which removed three 5-6 m embankments beside steep recorded Kuopio junctions ([open question 31](roads.md#road-68-open-questions), for review).
- `ROAD-68-E` part 2 (branch `road68-E2`, in progress): a stroke no longer pins its profile to the road pieces its own edit rebuilds; it meets a road it crosses at that road's profile and joins a road end at the node's height, and a road drawn from another road's end joins it like any junction instead of re-solving the old road's height profile, so sloped T and Y junctions come out the same whichever road is drawn first; at a crossing the road drawn second meets the first exactly at the first road's height, so the first-drawn road sets it ([open question 30](roads.md#road-68-open-questions)). Terrain edits no longer leave sloped junctions broken (every junction's profile is re-solved, and an edit that would still break one is rolled back), and bulldozing a road is refused when it would break a junction. A commit is refused when its road pieces would newly cover a farm field or painted zoning cells, junction corners and road ends included, instead of testing straight corridors. Moving a road node, toggling a crosswalk or changing an edge's class is tried first and refused, with nothing changed, when a junction would fail or reserved land would be covered. See [`roads.md`](roads.md#road-68-open-questions).
- `ROAD-68-E` part 1 (branch `road68-E1`, in progress): the road tool previews synchronously. Each changed pose calls `preview_road` in the tool's early process callback, which plans the stroke with the commit's own plan on a versioned context (the published view and a private graph replica that follows commits by records, with no per-command copy of the city) and returns the changed road chunks as the commit publishes them; Godot shows them in that frame. A click installs the previewed pieces when nothing changed since, and the preview stays shown until the committed chunks are installed. The request mailbox, late publisher, ribbon and "Road only" mode are gone (D3). Undo swaps the replaced pieces back and restores the graph and lanes locally. Crosswalk paint and road signs are missing from the preview until `ROAD-68-F` builds them from the layout. The preview also builds the terrain patches its pieces touch (the commit's tiles, sources and build functions, on a published per-patch terrain context), Godot stages them with the road chunks, and the commit installs the preview's entries when its own sources have the same signatures; undo puts the pre-edit refined terrain entries back. Since round 4 the terrain CDT input names chain runs by geometry inside the tile instead of graph pieces, and an edit stamps only tiles within the envelope cap of changed chain segments (or of changed pieces where a building site grades), so a T on a long trunk rebuilds the same tiles as on a short one; replaced road-tool views are freed on the shared pool. Round 5 is on `road` (work in progress, rebased onto `ROAD-68-G` parts 3-5): a lane-local undo now derives the entrances on its rebuilt lanes again, and the terrain staging is tested on the real terrain renderer; the terrain stage (with `ROAD-68-D` part 6) still misses its budget, and at the CPU's idle clock 38 of 864 streamed frames exceed 16.7 ms. See [`roads.md`](roads.md#preview-commit-undo-and-load).
- `ROAD-68-D` (branch `road68-D`, done 2026-10-25): terrain is clipped to the roads again. Each 64 m CDT tile builds its road loops from the roadbed's outer chains inside its halo (`roadbed/terrain.rs`), closed blocks are holes by orientation, the CDT sees one `RoadBoundary` source per chain segment, roads and building sites share one grading envelope (`terrain/grading.rs`), and an edit re-assembles only the tiles its old and new pieces reach. A street on a tile line keeps one exact seam in both tiles, and each tile probes its grading rays once. Road ownership refreshes only patches a compile or terrain stroke changed; building-site changes keep it. Bridges and tunnels leave their patches unrefined until `TERRAIN-07` (abutments and portals moved there from `ROAD-68-F`). The sloped T's terrain stage takes 1.85–1.88 ms against its 2 ms target ([open question 22](roads.md#road-68-open-questions), resolved), and the economy perf contract is back within its control's range. Span rows follow one constructive station rule; terminal cap apexes are exact. Patch terrain is watertight: the regular filler conforms to its tiles' sides and small faces are no longer dropped. The road surface is watertight too: zebras paint whole asphalt triangles instead of cutting them, road chunks clip at chunk lines and take the frame of the terrain patch under them, and building pads are composed into the terrain patch mesh (no separate site foundation mesh), so the Kuopio yard test runs again ([open questions 23 and 24](roads.md#road-68-open-questions)). Reference-screenshot pinholes: 33, then 9, now 0. Straight-edge seam T-vertices at CDT tile lines stay open (question 23). Part 6 (branch `road68-D6`): commits work at city scale; a load or bulk road build refreshes the world's terrain once instead of leaving its changes to the first commit, and the CDT tile clip leaves no zero-width rails along tile sides, which rejected the first commit of the 100k-resident city. The commit's terrain stage still misses its 3.0 ms budget at city scale (10-73 ms; [open question 29](roads.md#road-68-open-questions)). See [the archive](archive/terrain_legacy_surface_2026-10-25.md#from-roadsmd-footprint-and-terrain-the-road-side-of-the-exact-cut) (deleted by `TERRAIN-07-G`).
- `ROAD-68` compaction before E (branch `road68-compact`): `roadbed/` drops from 7,545 to 7,229 production lines with no behaviour change. Shared grid, bounds, mesh-assembly and layout helpers replace near-duplicates, dead code and needless parameters go, and a product-hash probe over every roadbed fixture and the rendered reference junctions guards later gates. See [`roads.md`](roads.md#roadbed-compaction-before-e).
- `ROAD-68-G` (branch `road68-G`): a road commit, undo or road bulldoze no longer builds the CCH. It runs as a pure job on the road-tool snapshot's graph copy and is adopted six frames later; until then `step_frame` holds, so no route query pairs the new graph with the old hierarchy, and a late job blocks only the adoption frame. Paused frames count; a second edit restarts the hold; a save installs the result and ends the hold. Results are bitwise identical across worker counts, a 400 ms late rebuild and a save in the hold. The commit loses 312 ms at 100k residents. Part 2 (branch `road68-G2`): the 2.4 s left was the breakdown harness never acknowledging an upload; the publication now re-lifts only rebuilt pieces' lanes and splices only rebuilt pieces into the changed chunks, the commit derives only changed entrances and runs the agents' route test in parallel, the adoption offset is one frame with a late build awaited outside the core lock, and the remaining editor rebuilds are deferred too. Outcomes are bit for bit unchanged; the commit takes 41 ms at 100k (78 ms before), its cell-lot regeneration about 20 ms of it ([open question 21](roads.md#road-68-open-questions)). See [`roads.md`](roads.md#local-commit-publication-road-68-g-part-2-2026-10-24) and [`simulation.md`](simulation.md#pure-jobs). Part 3 (branch `road68-G3`): a road edit queues cell-lot regions and invalidates cells only for the roads whose geometry or curb frames changed, not for every arm of a re-solved junction and its ring, and cell generation rounds, hashes and looks up legality more cheaply; the lots, parcels and generated cells are bit for bit the control's. The commit takes 28.6 ms at 100k (41.7 ms control), lots 7.8 ms of it, and still misses a frame ([`roads.md`](roads.md#commit-cell-lots-road-68-g-part-3-2026-10-24)). Part 4: undo and the other queued edits mark the whole curb-frame ring again (part 3's narrower marks queued fewer regions after an undo; a digest test now compares every edit kind with the whole-ring marks), each new lane is lifted once, the publication splices into reserved buffers, cells are invalidated without the lots' frontage reach and the agent passes probe their sets cheaply; products and determinism digests are bit for bit unchanged and the commit takes a median 21.8 ms at 100k (41.8 ms control), still missing a frame, mostly in the snapshot publish and copy `ROAD-68-E` owns ([`roads.md`](roads.md#commit-stages-part-4-road-68-g-2026-10-24)). Part 5: no pool worker waits for a pure job any more (the adoption frame's wait deadlocked the debug suite once joins ran on the pool; the frame's caller now finishes the rebuild before entering the pool, with a regression test), the new-lane lift and chunk seal run on the pool and the cell road-overlap test is exact without rounding; median 19.3 ms at 100k before the `ROAD-68-D` rebase. On `road` the 100k commit is rejected and the commit's terrain validation costs 15–69 ms, which `ROAD-68-D` part 6 fixes ([`roads.md`](roads.md#commit-stages-part-5-road-68-g-2026-10-25), [`simulation.md`](simulation.md#pure-jobs)). Part 6: the first, slowest sample is the larger edit beside the painted grid, not a cold start; its reattachment search, street lights and signs and retained frontage lines run on the pool and crossing signs no longer copy sections, bit for bit (20.5 / 17.9 / 16.0 ms per sample at 100k before the rebase, control 23.4 / 19.2 / 13.7); still WIP until the 100k commit can be measured on `road` ([`roads.md`](roads.md#commit-stages-part-6-road-68-g-2026-10-25)). Part 7 (branch `road68-G7` on `terrain07-E`, WIP): a commit plans its cell lots in a pure job that the next frame adopts with the attachment repair and entrance update, bitwise as planning in the commit (question 33 (c)); the first commits take 16.0-17.3 ms at 10k-100k (control 25.8-29.7) and only the first 100k sample still misses 16.7 ms, its largest stage the legacy refined terrain ownership that `TERRAIN-07` removes ([`roads.md`](roads.md#commit-stages-part-7-road-68-g-2026-10-27)). Part 8 (WIP): the routing rebuild starts after the lot plan and the sim thread ticks a frame after a commit that deferred its lots (question 58, taken pending the user's review), so the adoption frame takes 0.3-2.3 ms; three of twelve populated first commits still take 17.0-17.9 ms; a new locality fixture keeps the commit's products bitwise from 0 to 100k remote sites, with about 3 ms more commit and 3-6 ms more adoption repair at 100k ([`roads.md`](roads.md#commit-stages-part-8-road-68-g-2026-10-27)). Part 9 (branch `road68-G9` on `road`, WIP): the adoption's attachment repair tests only the buildings in the 512 m chunks the edit reaches (0.07-0.17 ms at 100k remote sites, 2.8-8.1 before; checked against the whole city's test in test builds); first commits 14.4-18.2 ms, three of twelve populated first samples still miss; the agent passes and the graph copy are open questions 59 and 60 ([`roads.md`](roads.md#commit-stages-part-9-road-68-g-2026-10-28)). Part 10 (branch `road68-G10` on `TERRAIN-07-G`, the gate, 2026-10-29): the rebase drops the legacy terrain state with `TERRAIN-07-G`, and the commit queues its undo record's kept terrain tiles after its last stage, so their bake no longer takes the commit's cores; every populated first commit takes 9.6-13.8 ms at 10k-100k (control 21.1-27.5), every later sample at most 7.5 ms, products and agent records bitwise. Questions 58 (b), 59 (b) and 60 (a) are the user's decisions (2026-10-28), carried to `ROAD-68-H` with the locality growth at 100k remote sites and the economy contract's fiscal means ([`roads.md`](roads.md#commit-stages-part-10-the-gate-road-68-g-2026-10-29)).
- `ROAD-68` pre-D review fixes (branch `road68-fix`): a footpath beside a crossable arm moves only its mouth, and its zebra stays right outside the corner (user decision, [open question 17](roads.md#road-68-open-questions)); footpaths join the sidewalk where their centreline meets it, also between the crossing and the mouth. Footpaths on both sides of a road end lay out, junction profiles rank their corridors by a documented geometric rule in `f64` with the layout's arm directions, node moves and road removals re-solve the profiles they reach, and the remaining silent skips reject placements. See [`roads.md`](roads.md#pre-d-review-fixes).
- `ROAD-68` pre-D (branch `road68-pre1`): footpaths join road junctions (D5, [open question 14](roads.md#road-68-open-questions)). A footpath at a road node is attached to the sector it lies in: its ribbon meets the sidewalk's outer boundary on shared vertices at sidewalk height, and the profile solve pins its end there. Footpath-only nodes lay out as ordinary arms, footpath lanes end at their layout mouths and join the corner walks, dead ends keep a turn-back connector, and road arms keep their crossings. A bend whose crossing does not fit on its first arm crosses on the other ([open question 13](roads.md#road-68-open-questions)). See [`roads.md`](roads.md#footpath-junctions-and-the-bend-crossing-fallback-pre-d).
- `ROAD-68` pre-D (branch `road68-pre2`): the junction profile solve derives every physical profile from its control polyline and the final topology, never rewriting control or ranking roads by build order; junction layouts read control geometry; strokes pin only to carriageways. Flat reference junctions are bitwise independent of drawing order; slopes remain [open question 16](roads.md#road-68-open-questions). A straight width change now tapers linearly along both approaches (D4) instead of stepping at the node, and nearly straight joins miter without sliver strips. See [`roads.md`](roads.md#edit-order-independence-pre-d).
- `ROAD-68-C` follow-up 2 (branch `road`): every degree-two node crosses once again, as the legacy rule did (user decision 2026-10-24, [open question 12](roads.md#road-68-open-questions)): a bend on its node piece right outside the curve, a pass-through node on its arm's span beside the node, with kerb points from the span's section row there. Walkers can change sides at every node, so the walking planner's whole-network proof holds again, on the street-grid benchmark city too. See [`roads.md`](roads.md#one-crossing-per-degree-two-node-follow-up-2).
- `ROAD-68-C` follow-up (branch `road`): curb returns are 3.5 m (D1), and pedestrian crossings come from the junction layout: each zebra fills the 3 m straight approach right outside the curve on the node piece, sidewalk lanes end on its crossing line, corner walks follow the layout's sidewalk, and spans carry no zebras. Bends cross only where forced, so walkers change sides only on crossings ([open question 12](roads.md#road-68-open-questions)). See [`roads.md`](roads.md#tighter-corners-and-junction-crosswalks-follow-up).
- `ROAD-68-C` (branch `road`): junctions, bends and road ends render again as node pieces built from their junction layouts: carriageway core, curbs, sidewalks and curb faces sharing exact mouth sections with the spans, heights on each node's solved plane. Graph clips and the profile solve's hard zone come from the layout mouths, obtuse corners between two- and four-lane arms fit by extending the wider arm backwards, and layout failures reject placements with reasons. A four-way node piece builds in 0.071 ms. See [`roads.md`](roads.md#road-68-c--junction-layout-and-node-pieces).
- `ROAD-68-B` (branch `road`): roads render again as constructive span pieces (carriageway, curbs, sidewalks, footpaths and curb faces) between the graph clips, with lane markings, street lights and signs on their sections. Height, band, ray, overlap and lane queries answer from the rendered triangles through a 32 m query-cell index, so lanes snap and buildings place on their frontage again. Junction, bend and terminal areas stay empty until `ROAD-68-C`. A 100 m span builds in 0.017 ms. See [`roads.md`](roads.md#road-68-b--profiles-and-spans).
- `ROAD-68-A` (branch `road`): the legacy road-surface compiler (`surface/`, 432 files) and the asynchronous road preview worker are removed. Consumers read the empty `roadbed/` façade, so roads render nothing and buildings cannot be placed until `ROAD-68-B`; the road preview validates synchronously. Tests that need road geometry are disabled until their gate. See [`roads.md`](roads.md#road-68-a--remove-legacy-stand-up-the-façade).
- `VEG-25`: trees no longer turn darker as the camera comes within 60 m. The near trees' detailed shadow casters shaded their own crowns; they now stand back from the sun as the far casters do, and the ground shadows are unchanged. See [the shadows paragraph](terrain.md#authored-trees-silhouette-shadows-yard-plants-and-rocks-2026-09-27).
- `RENDER-16`: an overcast or stormy sky no longer shows a grey stripe on the horizon at night or a lilac one at dusk. The far cloud layer now fades into the sky's horizon haze. See [`terrain.md`](terrain.md#14-terrain-site-ground-water-and-lighting-materials-are-runtime-presentation).
- `VEG-26`: forest and brushed trees keep `3 m` off footpaths and sidewalks and `4 m` off lamp poles; a path drawn later clears the trees beside it. Yard plantings keep their layout. See [`terrain.md`](terrain.md#local-vegetation-experiment-v01).
- `VEG-25`: tree shadows now start at the trunk and move smoothly with the sun. Trees near the
  camera cast from their own model, at a frame cost of about 2-3 ms. See
  [the shadows paragraph](terrain.md#authored-trees-silhouette-shadows-yard-plants-and-rocks-2026-09-27).
- `WATER-04`: lake reflections no longer stair-step at the shore, trees in them keep their size as you move, two lakes no longer flip between high and low quality, and the camera stays above the water. The Rainy afternoon look is hidden until rain ships. See [`terrain.md`](terrain.md#14-terrain-site-ground-water-and-lighting-materials-are-runtime-presentation).
- `WATER-04`: buildings now reflect in lakes. The planar reflection pass mirrors each building
  batch through one shared proxy, within 1 km, at no measurable frame cost; bridges, hedges and
  ground roads are not reflected yet. See
  [the reflection buildings contract](terrain.md#14-terrain-site-ground-water-and-lighting-materials-are-runtime-presentation).
- `ROAD-64`: the inside of a curve through a junction no longer gets a second sidewalk, and zebra crossings sit square to the road with even kerb margins and are worn paint in the asphalt itself, painted over patches and drains. See [`roads.md`](roads.md#pedestrian-junction-crossings).
- `TRAFFIC-03`: cars no longer turn around where a footpath joins a road; footpaths no longer count as junction arms for cars' connectors. A road ending in a footpath is still not a dead end, so cars do not turn around there (upstream's `NET-07` U-turn rule). See [`traffic.md`](traffic.md#junction-connectors).
- `TRAFFIC-04`: walkers no longer cut across the lawn where curved roads meet; bends and Y junctions get sensible clips. Saved bends keep their old clips until redrawn. See [`roads.md`](roads.md#pedestrian-junction-crossings).
- `ZONE-07`: on curved streets, driveways reach the road and neighbouring yards share one hedge across the wedge between their lots. See [`earthworks.md`](earthworks.md) and [`asset_editor.md`](asset_editor.md#yard-hedges).
- `ROAD-54`: road placement now refuses hairpins tighter than the road, junction arms closer than 25° and junctions with more than six arms, and says why. Steep grades wait for the hill policy. See [`roads.md`](roads.md#road-geometry-validation-road-54).
- `ROAD-60`: Finnish crossing signs stand at every zebra, at the near edge of the stripes, speed-limit signs where the limit changes or after the junction of a long street, and warning signs (steep hill with its percentage, bends, uneven road) on the awkward roads placement still accepts. Cosmetic for now. See [`roads.md`](roads.md#road-signs-road-60).
- `TRAFFIC-05`: walking is priced at walking pace instead of the road's speed limit, so trips longer than about 280 m now drive. See [`entrance_and_exit.md`](entrance_and_exit.md#mode-policy). Lane heights now follow the road surface in the simulation itself, not only where a chunk is drawn, so a session continues identically after save and reload; see [`traffic.md`](traffic.md#current-scope).
- `RENDER-26`: cars keep the Kenney models (generic Finnish models were withdrawn); at night their lamps glow and the nearest cars' headlights light the street. See [`traffic.md`](traffic.md#vehicle-models-and-lamps-render-26).
- `ROAD-59`: street lights along every road and footpath, Finnish models placed in Rust along whole streets, poles of meeting streets kept 12 m apart; at night lit roads and lamps read from across the map, lamps past the 32 real lights are lit inside the materials with the real light's maths, so far streets keep their contrast and walls light up at any distance; past 500-700 m a coarse lamp light map keeps streets lit over 8 km around the view. See [`roads.md`](roads.md#street-lights-road-59).
- `ROAD-56`: roads look Finnish: worn grey stone-mastic asphalt that fades light with age, long trench patches, manholes and drain grates, ruts only on busy roads, granite kerbs, sealed cracks and patches, and white lane lines with Traficom dash lengths (1 m / 3 m up to 50 km/h, 3 m / 9 m above) instead of the yellow centre line; two-lane residential streets are unmarked. See [`roads.md`](roads.md#finnish-road-surface-and-markings-road-56).

- `ROAD-55`: footpaths now connect to sidewalks and to each other at junctions, so walkers can route along them. Before, the junction builder skipped them and no walker could ever reach one. See [`roads.md`](roads.md#pedestrian-junction-crossings).

- `ROAD-65`: saves no longer break after a terrain-brush stroke or an undo while agents cross junctions. Both rebuild every lane, and agents now keep their lanes by identity instead of by the old lane number. See [`roads.md`](roads.md).
- Trip failure log (2026-10-13, `DEBUG-06`): every trip failure in the movement tick logs a
  line on `events`. A line names the agent by render id with its row, kind and mode, where it
  was, its destination, the reason from the failure site's own data (missing turns and roads,
  the planner's rejection of each exit candidate, stale access plans, departures that cannot be
  planned) and what happens next: a replan, a retry with its failure count, or the watchdog's
  recovery home or to a border. Records are collected during the parallel pass under a lock
  taken only on failure and logged after it in agent order. Repeats are quiet for a simulated
  minute. Past 20 per place or 200 per minute, failures fold into one summary per reason. The
  agent inspector's `Last failure` row shows the inspected agent's latest line. See
  [`debug.md`](debug.md#trip-failures).
- Road rejection reports (2026-10-13, `DEBUG-03`): every refused road stroke, whether refused by
  the road tool's click check or the simulation's commit, logs on `events` why (the check's
  reason and numbers), where, and the stroke itself, then a `road_attempt` line with the exact
  points, lanes, snapping and road generation. Plans refused for a failed check now name that
  check instead of a plan state. The console dump writes its save first and names it.
  `./run.sh --replay-road-attempts <dump>` loads that save in a throwaway profile and replays
  each attempt, reporting whether the refusal reproduces. See
  [`debug.md`](debug.md#road-rejections).
- Agent inspector (2026-10-13, `DEBUG-05`): with no tool active, a click on a car, bike or
  pedestrian on a road opens a window beside the cursor (draggable and resizable, like the
  Building Inspector) with its mode, state, speed, activity, lane, route progress and its home,
  work and destination as links that frame the building and open its inspector. The route is
  an outlined line of constant screen width, readable from far away; panning stops the follow
  but keeps the inspection until Esc, and Follow returns to the agent. Its route is drawn from the agent to the
  destination door, walked with the movement tick's own lane and connector rules. The camera
  follows the agent's drawn position, with zoom and orbit still working, until Esc, the close
  button, a miss, a world load or the agent's removal (explained on `events`). The selection
  lives in the agent system and is repaired in O(1) at swap-remove; picking reads the lane
  occupancy snapshot near the click. There is no source building: agents do not store where a
  trip began. See [`debug.md`](debug.md#agent-inspector).
- Junction connector overlay (2026-10-13, `DEBUG-04`): the `junction_connectors` visual mode
  draws the connector arcs cars drive through junctions near the camera, straight from the lane
  geometry. Its views color them by whitelist (`arcs`), turn speed cap (`speed`) or cars on them
  (`occupancy`), or add the road lane lines (`lanes`). With no tool active, a click focuses one
  junction and logs its connectors on `events`. While the view is on, a stall detector logs each
  car stopped on a connector for 10 simulated seconds, with the blocking car or the cause. The
  mesh rebuilds only on network, view, focus or camera-chunk changes. With the view off it does
  no work and nothing runs on the simulation tick. See
  [`debug.md`](debug.md#junction-connectors).
- Debug visual modes (2026-10-12, `DEBUG-02`): the terrain, grass, building-site, pedestrian VAT
  and terrain residency (force LOD1, full world) debug views now switch at runtime from the
  console's Visual modes pickers or the `visual` command. Launch flags set only the starting
  value. Terrain and grass views are global shader uniforms, so a switch touches no patch
  material. Residency switches go through normal streaming, with no teardown. Mode names and
  parsing moved to Rust (`debug::visual`), and the `pedestrian_vat` offsets view is now
  `offsets`, since `off` means no view. See [`debug.md`](debug.md#visual-modes).
- Debug system (2026-10-12, `DEBUG-01`): Rust and GDScript debug output now share one fixed
  channel registry in a single atomic, so a disabled call is one relaxed load with no
  allocation. Enabled lines keep their `[DEBUG:<tag>]` stdout text and also join a 4096-line log.
  Traffic lines now print to stdout as `[DEBUG:traffic]`. An always-on, buffer-only `events`
  channel keeps engine warnings and errors and road-tool rejections. The `§` key opens the
  in-game console on every scene: Log, Perf and Channels tabs, commands, Copy, and Dump to
  `user://debug/`. GDScript no longer parses `METRUM_DEBUG*` variables. Under a bare
  `--debug`, `terrain`, `buildings`, `perf`, `traffic`, `sim` and `lighting` are opt-in. Rust,
  Godot bridge and asset suites pass. The matched agent-tick run is within spread. Access ingress
  measured +5.3% in the default build and −0.6% with code alignment forced, which confirms code
  placement. See [`debug.md`](debug.md). `agent_benchmark`
  compiles again: the `Building` electricity fields are public. `PERF-06` tracks a shorter
  agent-tick check. The Kuopio replay fixture hash in `placements.json` is repinned to the
  file `51cc4ca0` re-saved for save format 83, so `test_road_terrain_replay.py` passes again.

- Routing performance follow-up (2026-10-11): `NET-02`–`NET-05` track exact CCH hot-path
  improvements, nested dissection/elimination-tree queries, bucket many-to-many commutes and
  restricted CPU sweeps. `NET-02` is done against `03c1cf45`: query time falls 35–38% at
  10k/100k; four matched 10k economy pairs improve the hour 35.50 → 25.13 ms and the day
  9.22 → 9.01 ms, with identical paths, records and five growth reports. All 2,298 library
  tests and four growth tests pass. `ECON-13-B` stays open; the full 100k economy benchmark
  was not rerun for NET-02. `NET-03` now gives a further 21–29× point-query speedup at
  10k/100k and fixes `NET-06`; all 2,303 library and four growth tests pass. Its changed
  route ties/f32 sums alter populated-city records. The 10k hour improves 24.99 → 11.14 ms,
  but the day regresses 9.09 → 14.15 ms. `NET-04` is complete: exact bucket commutes preserve
  records and reduce the 100k open-job query matrix 61×; the full daily mean falls 6.2% in one
  pair. `NET-05` is a measured production no-go; its scalar/eight-source prototype stays test-only;
  the user deferred 1M runs. See the
  [routing plan and measurements](entrance_and_exit.md#cch-routing-performance-track-net-02net-05).
  `NET-06` now has an independent regression: the previous hierarchy priced a required
  5-second loop as zero. The replacement charges it correctly and also preserves required
  return detours. All five independent growth reports remain byte-identical.

- Economy performance contract (2026-10-12): `ECON-13` is closed and replaced by a standing
  [contract](economy.md#performance-contract-perf-01perf-05). It uses saved 10k and 30k cities,
  one simulated day per tier, deterministic work-count scaling (≤1.2× the household ratio) and
  fixed time budgets (10k: 40 ms per hour, 10 ms per day; 30k: 3×), in at most 5 minutes. The 100k
  tier is deferred. `PERF-01` builds the runner, `PERF-02` per-phase checks, and `PERF-03`–`PERF-05`
  track the known superlinear costs.
  Implementation started 2026-10-13: snapshot-backed runner, phase attribution, conservative
  diff selection and short locality gates are wired. The short agent and locality contracts
  pass initial release checks. Completing bulk lane-height initialization fixes exact 10k
  save/load continuation. The 30k probe found omitted retained commute quotes; save version 86
  now persists them without changing their daily pricing cadence. Preserving pending network
  retries fixes another continuation mismatch; both tiers now pass at day 12. A complete cached
  run with physical-core affinity finishes in 55.20 seconds; locality and short agent gates pass.
  `PERF-02` and `PERF-06` are complete. The 30k daily budget and census/owner/staffing/supplier/job
  work bounds fail. Stable workplace and movement query ownership now makes all work counters
  match across three fresh runs per tier, with unchanged next-day records and 125 targeted
  regressions passing. All five growth reports remain byte-identical; busy-city and queued
  save continuation pass. Active-window coverage and full economy acceptance remain pending.
  A short staffing experiment removed only four daily trials at 30k and was reverted;
  fresh five-report references are retained for the remaining census and search changes.
  Opt-in funding-input traces complete in 3.15/10.06 seconds at 10k/30k. They identify one/four
  unchanged execution inputs; reusing only those would still fail census scaling (3.5395×).
  The remaining census fix must address changed inputs or eliminate additional full passes.
  Exact import-quote reuse within closure retries now cuts staffing trials to 60,333/291,769
  at 10k/30k, preserving next-day records and passing 118 targeted regressions. The 4.8360×
  ratio still fails the 3.4176× limit. All five growth reports remain byte-identical and
  recovery/expansion/save-load checks pass; full acceptance remains pending.
  Hourly placement/removal counters now enforce fixture coverage: fresh short day runs show
  churn in only 8/24 intervals at 10k and 20/24 at 30k. Both snapshots fail `PERF-01`
  coverage; next-day records and previous work counts remain unchanged.
  Early and later continuation scans rule out every day-4–18 midnight boundary. The historical
  every-hour observation was for 100k; choosing a different boundary in this window cannot
  satisfy the unchanged 10k/30k fixture requirement.
  Snapshot revisions now include runtime economy and demand TOML inputs. Fresh regeneration
  preserves exact continuation and previous records/counts; the cached full contract takes
  56.05 seconds, with the same nine failures. Configuration invalidation and 14 runner tests pass.
  Local-input funding now skips an unused import-priced inventory valuation. All 122 targeted
  tests pass; short day checks preserve both records and all work counts. The 30k daily sample
  remains over budget at 34.57 ms; no additional contract gate is closed.
  A normal fiscal-policy stress case now demonstrates hourly removals throughout days 16–25
  at both tiers. Loading now rebuilds vacancy/admission indices after restoring the asset
  registry, fixing the extra unhoused household. Twenty focused regressions pass; both stress
  continuations match every hourly record and churn count in 11.26/31.82 seconds including
  preparation. The integrated two-workload runner passes 19 unit regressions. Loading also now
  preserves valid immediate network replans after demolition; 33 save regressions pass and both
  fiscal snapshots pass exact agent continuation and 24/24 churn coverage. The expanded contract
  finishes in 67.16 seconds, with 31.72 seconds of initial 30k fiscal generation reported separately.
  Thirteen limits fail: the original seven, plus four fiscal time budgets and fiscal supplier/CCH
  scaling. Original records/counts remain identical; phase, locality and agent checks pass.
  Demand now reuses the existing cell-generation completeness cache while deriving every dirty
  lot region. A fresh full contract takes 64.91 seconds: fiscal hourly samples pass at
  30.17/85.62 ms; eleven limits remain, including daily settlement and work scaling. All four
  next-day records, work counts and churn sequences match the retained pre-cache build.
  Recovery, expansion and the corrected four-test growth run pass; all five reports are
  byte-identical. A fixed-cost profit bound was tested and removed because it reduced no work
  counts. Household claims now defer downward eligibility-index updates until acceptance,
  preserving immediate release and rollback semantics. All 179 targeted regressions pass;
  the full contract takes 65.36 seconds with the same eleven failures and unchanged records
  and work counts. Fiscal daily samples are 28.01/91.48 ms. Four growth tests plus recovery and
  expansion pass; all five reports remain byte-identical.
  Recurring supplier quotes now bypass spatial search for empty resources using existing indices.
  All 238 targeted tests pass; records, churn and other counters are unchanged. The full contract
  takes 64.50 seconds with ten failures: fiscal supplier searches are now zero, while baseline
  searches fall to 20,978/105,798 and still fail scaling. Four growth tests plus recovery and
  expansion pass; all five reports remain byte-identical.
  Both workloads retain independent time/work gates because bankruptcy sharply reduces employment.
  Fresh opt-in staffing attribution preserves both baseline records and all work counts:
  30k successful trials alone exceed the scaling allowance. Failed-worker pruning cannot close
  `PERF-04`; the next target is repeated successful funding evaluations/passes (see economy.md).
  `PERF-03` now has allocator-owned, allocation-free worker-change tracking across employment,
  agent removal and bulk recounts. All 362 targeted tests and four unchanged cached days pass.
  Census counts and acceptance failures remain unchanged.
  The journal now also covers productive area and operational eligibility. All 375 targeted
  tests and four unchanged cached days pass; an opt-in 30k assertion validates nine complete
  execution-pair journals and six conservative layout-change fallbacks. Funding now reuses unchanged
  production-market chunks; 379 targeted tests and four unchanged cached days pass. Its full
  cached contract takes 63.95 seconds; phase, locality and agent checks pass, while ten limits
  remain open. Fresh import-plan attribution rules out a broader capital guard alone as the
  staffing fix. Fresh growth integration, recovery and scarce-labor expansion regressions pass;
  all five reports remain byte-identical. Full funding reuse remains pending.
  Exact import-plan handoff now passes 131 targeted tests and preserves all four cached-day
  records/churn. Baseline staffing trials fall to 56,488/251,537, still above the scaling limit.
  The full cached contract takes 64.15 seconds: phase, locality and agent checks pass, with the
  same ten failed limits. Fresh growth integration, recovery and scarce-labor expansion tests
  pass on the handoff build; all five reports are byte-identical. A short fiscal trace identifies
  household reservations for rejected plans as the next daily-cost investigation. Exact-key
  diagnostics now find reusable rejected queries covering 82.50%/84.12% of daily visited
  candidates at 10k/30k. Production reuse now retains exact rejected single-output quantities
  and restages receipts before any later acceptance. It passes 147 targeted tests and preserves
  all four cached-day records, churn and work counts. The full contract takes 64.20 seconds;
  phase, locality and agent checks pass, but the same ten limits remain open. Fiscal daily
  samples are 22.21/72.67 ms, still over budget; no matched speedup is claimed. Fresh growth
  integration, recovery and scarce-labor expansion tests pass; all five reports are byte-identical.
  **Scope clarified 2026-10-14:** the fast benchmark suite is the deliverable and is complete
  (`PERF-01`, `PERF-02`, `PERF-06`). Scaling fixes (`PERF-03`–`PERF-05`) are parked.
  The suite continues to report the ten known limits without requiring their repair. Use
  `./run.sh --perf-contract`, `--phase census`, or `--agents`; see economy.md for routine commands.

- `ECON-13` (2026-10-12): test-only work counters (`debug::work`,
  `benchmark_populated_work_counts`) show the 100k scaling gap is algorithmic: proposed site rows
  grow 67×, execution-epoch censuses 6.4× and market copies 12.8× for a 10.2× city. Proposed
  site rows now update only the buildings and parcels named by new allocator and zoning change
  logs, behind a separate exact proposed-site index. The hourly pass plans on the live economic
  books inside per-book pass journals and restores them exactly instead of copying them. All
  library tests, five growth reports and populated records match; test builds fingerprint the
  live books after every pass. 10k: 8.17 ms/hour, 8.31 ms/day (from 8.72/8.79, p = 0.0286);
  100k: 111.38/174.93 ms (from 115.46/181.52). `ECON-13-B` is rewritten as per-phase work-count scaling (≤1.2× the
  household ratio) plus a fixed 100k budget (400 ms/hour, 100 ms/day). The old same-build ratio rose
  whenever linear work got faster. Execution epochs borrow their census books too, so copied
  rows are 0. B still fails on execution-epoch censuses, staffing and search fan-out; paused. See the
  [record](economy.md#econ-13-work-counts-and-live-book-planning-2026-10-12).

- `ECON-13` (2026-10-11): economy prices read a per-lane frontage delay published once per
  operational hour (`TransitNetwork::publish_frontage_prices`); agent trips keep the per-second
  live delay; save format 85 stores both. The funding census pins operations that cannot profit
  even at their best possible prices, and customer propagation prepares the business layout
  itself, so a loss-making supply cycle resolves in 2 censuses at any length. Latest routing
  measurements fail `A/B` (10k day 14.33 ms; 100k hour/day 20.37× / 84.18×) and freshly pass
  `C/D`. The 10k spike includes 55.97 ms of dirty lot regeneration; funding preparation remains
  expensive at 100k. Exact follow-up candidates separate explicit-site cell invalidation from
  terrain publication, index road-surface clearance, skip authored-parcel scans when none exist,
  and combine existing border quote minima. Cached entrance geometry and exact shopping
  shortlist/proof reuse, narrower walking-proof invalidation and bounded freight replacement
  plus exact lattice overlap, repeated-clearance and pedestrian-prefix reuse bring two
  10k runs to 9.22 ms/hour and 10.45 ms/day. One complete 100k run averages 134.95 / 287.73 ms
  with the same final record; A/B still fail (14.64× / 27.54× scaling). All five growth reports
  are byte-identical. Complete freight-quote replacement and customer-tree traversal preserve
  records but do not establish a full-run gain. Exact customer/price dependency checks pass;
  compact connectors, a turn-completeness proof, equivalent query keys and shared entrance
  projection pass. Entrance-input/allocation retention also passes correctness checks, but its
  10k 10.03/10.53 ms and 100k 143.99/305.38 ms measurements establish no further improvement.
  Car-access-only updates and parallel entrance/removal maintenance pass 2,317 library tests
  and preserve all reports/records. Their fresh 10k means are 9.59/10.10 ms and one 100k run
  measures 140.87/264.65 ms. Household remaps, retained car membership, constant-time ring
  bounds and exact freight affordability bounds pass 2,322 library tests and preserve all
  reports/records: 10k 9.38/10.34 ms, one 100k run 143.02/252.15 ms. A/B remain open at
  15.24×/24.38×. Exact freight-door retention and shared household visit labels now pass
  2,324 library tests and all growth reports. Latest 10k means: 9.46/10.02 ms; one 100k run:
  136.34/256.70 ms (14.41×/25.62×), with unchanged records. A/B remain open. Exact-price
  affordability masks pass 2,327 library tests and all unchanged reports/records, but
  10k 9.36/10.05 ms and 100k 138.15/255.01 ms establish no overall gain. The trial remains
  under performance review. Borrowed cell footprints and shopping-shortlist pruning pass
  2,329 library tests and all reports; 10k 9.48/10.10 ms and isolated cell timings establish
  no gain. Component-wide eligibility counts pass 2,330 library tests and all reports:
  10k 9.28/10.03 ms, one 100k 135.22/215.71 ms (14.57×/21.51×), records unchanged.
  Clipped chunk scans, same-frame cell exclusion and cached shopping distances pass 2,334
  library tests and all reports. Their two 10k runs meet A's budget at 9.23/9.67 ms, with
  unchanged records. One 100k run measures 129.43/205.53 ms (14.01×/21.25×), preserving
  records; B still fails and final repeat acceptance remains pending.
  The workplace-cache hasher also passes 2,334 library tests and all reports: 10k
  9.30/9.66 ms, one 100k 129.00/203.78 ms (13.87×/21.09×), with identical records.
  Its isolated lookup gain does not establish a significant whole-city improvement.
  Shared commute-cache stripes pass 2,336 library tests and all reports: 10k 9.26/9.62 ms,
  one 100k 122.96/201.52 ms (13.28×/20.95×), preserving records. B remains open. A fresh
  daily trace identifies 32–34 ms of proposed-site preparation. Exact entrance reuse passes
  2,337 library tests and all reports: 10k 9.24/9.91 ms and one 100k 121.72/200.62 ms
  (13.18×/20.25×), preserving records without a clear overall gain. Tied-radius maintenance
  and household walking-proof reuse are committed as `5a268e64`. The unsuccessful recurring-
  supplier bound trial was removed.
  A diagnostic initialization stack overflow was
  fixed by allocating the fixed crash-recording ring once on the heap.
  Continued exact fixes are authorized, preserving economic decisions and
  job-lock timing. See the [status](economy.md#routing-follow-up-2026-10-11).

- `DEM-02` done (2026-10-11): residential need and the workforce calendar count
  arriving households and their adults from admission's own applicant stream, not one home or one
  worker per job. The calendar credits those adults net of the store workers their shopping
  needs, and a store short only of workers opens with the crew the calendar can supply.
  `GrowthScenario::producer_town` adds a farm, mine and power-plant town. All three exit criteria
  pass on one build; all library and growth tests pass; 10k timing is unchanged. See the
  [exit-criteria status](demand.md#exit-criteria).

- `DEM-02` level changes (2026-10-10): the upgrade family (`asset_set`) is optional. Zoned
  buildings without one change level to an unfamilied asset with the same zone, density and lot
  footprint at the next level, chosen by a stable per-site hash; family members stay within their
  family. A pre-zoned player save with four unfamilied level-2 houses now records 85 residential
  upgrades in 60 days (none before). Level changes now refresh the yard (hedge/planting) of the
  new asset and play a 2.5 s render-only rise of the new model (visually accepted in game); the
  simulation swap stays instant. See [level changes](demand.md#level-changes).

- Docs (2026-10-10): [`economy.md`](economy.md) and [`demand.md`](demand.md) are rewritten as
  current-state specifications. Their previous contents, including the `ECON-14`/`DEM-02`
  diagnoses, implementation logs and dated measurements, moved verbatim to
  [`archive/economy_history_2026-10-10.md`](archive/economy_history_2026-10-10.md) and
  [`archive/demand_history_2026-10-10.md`](archive/demand_history_2026-10-10.md); links to dated
  evidence now point there.

- `ECON-10`: the developer economy editor is removed: its scene, scripts, Tools menu entry,
  `--economy-editor` mode, sandbox playback, scenarios, controllers and JSON export. Economy data
  is hand-edited in `economy/profiles.toml` and validated when the game loads it. See
  [economy profiles file](economy.md#economy-profiles-file).
- Idle frames against upstream `7e471302`, whole branch (M2 Pro, 1080p, idle p50): Kuopio
  overview `16.0 -> 11.6 ms`, close `13.3 -> 9.5 ms`, forest `13.3 -> 9.7 ms`, cliff
  `12.5 -> 8.8 ms`. Upstream skips the Kuopio pack's hedged houses, so town poses and load
  times do not compare. See [branch acceptance](terrain.md).

- `UI-05`: the Building Inspector shows one building at a time; clicking another building
  replaces the open window instead of stacking a new one. See [`ui.md`](ui.md).

- `UI-06`: Max FPS now holds in game after a restart. Starting a world no longer resets the cap
  to 60. See [`ui.md`](ui.md).

- `UI-07`: asset editor planting labels are near-white (or near-black) with a wider halo instead
  of green on the green lawn, and the yard hedge preview is translucent so labels draw over it. See [`asset_editor.md`](asset_editor.md).

- `WATER-03`: water reflects the shore and trees, the lake bed fades with the water the view ray
  crosses, and wind ripples move the surface. The lake had been lit as a downward-facing surface
  (flipped back-face normal), which pinned Fresnel at its maximum; it now uses water's physical
  Fresnel, so it is darker looking down. About `+0.5-0.9 ms` at water-filled 1080p views on the
  M2 Pro. See [`terrain.md`](terrain.md).

- `WATER-02`: lake seam slivers, hairline seam cracks and water tilting up steep banks were
  rendering faults, not the Kuopio DEM import: the water shader read terrain height with the
  watermap's layout, water borders repeated edge depth instead of the neighbour's, and shoreline
  vertices took dry terrain height. Water now stays flat at its level and terrain forms the
  shoreline. See [`terrain.md`](terrain.md).

- Options → Graphics gains V-Sync, Show FPS (top-right readout), Max FPS, Render scale (FSR 2),
  Shadows and View distance, applied live and persisted in `user://settings.cfg`. They replace
  the `METRUM_GFX` preset environment variable and its F8-F11 hotkeys. Low shadows, reduced view
  distance and 67% render scale together take the Kuopio overview from `11.8` to `8.8 ms` on an
  M2 Pro. See [`ui.md`](ui.md).

- `TERRAIN-06`: Rust bakes per-patch relief and cliff-reach masks on the payload worker; the
  terrain shader reads them from one shared texture array and runs the cliff test only near cliffs.
  Image unchanged, idle p50 `-0.13-0.28 ms` on M2 Pro. New `cliff` / `cliff_low` benchmark poses.
  See [`terrain.md`](terrain.md).

- Idle-frame matrix: `forest` / `forest_low` poses and `METRUM_IDLE_BENCH_CANOPY_DENSITY` measure
  denser canopy; at the `121.9/ha` ceiling vegetation costs up to `7.35 ms`, mostly tree vertex
  work. Benchmark runs no longer pick up the saved fullscreen state, building detail or HiDPI UI
  scale. See [`terrain.md`](terrain.md).

- `TERRAIN-03` / `TERRAIN-04`: idle frames and startup. Eleven road/site textures now import
  VRAM-compressed with mipmaps (startup `3.7 -> 1.9 s`), and the terrain shader reads four
  position-only noise fields from a texture baked once per world (`terrain_world_noise.gd`)
  instead of evaluating them per pixel. Idle p50 on an M2 Pro: overview `15.91 -> 15.01 ms`,
  town `11.17 -> 10.43 ms`, visually unchanged. A new idle-frame benchmark matrix
  (`METRUM_GAMEPLAY_BENCHMARK_MATRIX=idle`) and `tools/xctrace_report.py` support the work. See
  [`terrain.md`](terrain.md).

- `TOOLS-11`: pack rows in Options → Mods gain `Verify` (imported packs, against the kept
  `checksums.sha256`), `Show folder` and `Remove…` (to Trash; drops the enabled entry; refused for
  bundled packs and packs a running city uses). See
  [installed pack actions](asset_editor.md#installed-pack-actions--tools-11).
- `TOOLS-10`: Options → Mods gains `Import pack…`. It picks a `.metrum.zip`, refuses it unless the
  SHA-256 entered separately matches, and has Rust re-check every export rule, size limits and
  `checksums.sha256` off the main thread before staging and renaming the pack into `user://mods/`.
  Identical reinstalls change nothing; replacements go to Trash first; new packs install disabled.
  The pack scanner now skips hidden folders. See
  [share archive import](asset_editor.md#share-archive-import--tools-10).
- `TOOLS-09`: the library pack menu gains `Pack settings…` (validated `pack.toml` edits; versions
  must now be semantic versions) and `Export pack as zip…`, which writes a deterministic
  `<pack_id>-<version>.metrum.zip` plus `.sha256` sidecar off the main thread. The archive holds
  exactly the files the manifests reference, from `assets/archive.rs`, which publication now
  checks against too. See [the archive format](asset_editor.md#share-archive-format--tools-09).
- `TOOLS-08`: asset publication now writes a fresh folder containing only the files the document
  references, instead of merging into the previous one; textures, LODs and thumbnails dropped by an
  edit no longer pile up in packs. Published `colours/` names come from the current schemes, not
  editing history. Existing packs are cleaned by republishing each asset. See
  [`asset_editor.md`](asset_editor.md).
- `ZONE-05`: road-side cells now appear with the committed road instead of 9–15 frames later.
  Terrain keeps per-512 m height revisions from exactly changed samples, replacing the global
  generations that re-uploaded every visible cell chunk. While the overlay is shown, the sim tick
  regenerates edited warm chunks before publishing the edit. Chunk state polling returns four
  values per chunk. See [`zoning.md`](zoning.md#road-commit-cell-overlay-latency--2026-09-30).
- `RENDER-14`: car render interpolation moved from GDScript to Rust (`smooth_car_render_data`,
  `nodes/sim/render/car_visual.rs`). Behaviour is unchanged: same rates, 80 m snap, height-owner snap
  and busy-support fallback. `agents.gd` now only uploads buffers. Main-thread cost for lane cars fell
  from about 1.2 µs to 0.06 µs per car (20,000 cars: 25 → 1.2 ms per frame, windowed matched A/B).
  Off-lane cars keep the native support solve. See [`traffic.md`](traffic.md).
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
  hang detection. Interactive developer release runs default `METRUM_HANG_ABORT=1`, producing an
  all-thread core after the first hang report; automation, headless runs and explicit
  `METRUM_HANG_ABORT=0` remain dump-only. Road commands retain allocation-free endpoints and a
  stable all-point hash, and pre-transaction markers distinguish shared-pool wait, preview-plan
  validation, cold road compilation, terrain completion and transaction opening (`DEBUG-09`). The
  watchdog, the threads `SimulationNode` starts and the extension's shared Rayon pool, which
  replaces Rayon's global pool, are joined before Godot unloads the library
  ([`CODE-17`](debug.md#shutdown)).
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
  physical Fresnel from the level surface: dark looking down, reflective at grazing angles. It
  reflects the shore and trees through a screen-space march, the engine reflects the sky and
  sun, and analytic wind ripples, faded by pixel footprint, move the surface (`WATER-03`). Scene lighting / shadow policy is
  centralized through the Godot rendering bridge. Gameplay and WorldEditor now
  share a continuous procedural hemisphere sky with no literal horizon seam and a sun driven by
  that same directional light. Volumetric clouds drift with a saved Rust weather state, cast
  shadows on every lit surface and reflect in the water, and stars and the Milky Way turn with the
  clock at night (`RENDER-16`, `RENDER-17`). Gameplay now exposes the renderer's full
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
- Road-preview cell-reservation validation now queries each affected compiled span/junction owner
  once through the existing cell and surface-triangle indices, then caches the exact verdict by
  cell revision. This removes the simulation-lock freeze when a dense four-way preview contained
  thousands of valid surface polygons, without weakening commit validation or making the work
  depend on remote zoning (`ROAD-66`). See [`roads.md`](roads.md).
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
  and repeated same-resource exports saturate to a lower outside bid. Grocery jobs/input targets
  now follow resident consumption and pantry recovery net of shelves and shopper reservations;
  whole worker rounding and bulk sales do not inflate production demand.
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
  heights or raw heights within 0.1 mm, so elevated multi-arm nodes with contradictory same-XZ
  carriageway owners reject deterministically until ownership selects one carrier before height
  sampling. The 0.1 mm allowance (2026-09-29) stops f32 rounding splitting two band fields
  across a half-millimetre key boundary; a sloped 4-way junction was rejected this way. Source-band height
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
