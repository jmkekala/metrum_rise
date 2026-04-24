# Improved Roads

## Purpose

This document owns the shipped roadbed runtime for surface roads and the remaining road-specific
work tracked under [`ROAD-01`](roadmap.md).

It answers these questions:

- what the authoritative road surface model is
- how road geometry interacts with shared earthworks, rendering, and editor preview
- which subsystem owns each piece of data
- what deterministic rules the roadbed runtime must satisfy
- which legacy ownership patterns must not be reintroduced

It does not own:

- lane routing policy or turn legality
- frontage-side building entrance semantics
- the shared engineered-ground / earthworks contract used by roads, pads, and future foundations
- terrain storage internals or large-world terrain streaming policy

Those remain owned by [`entrance_and_exit.md`](entrance_and_exit.md),
[`earthworks.md`](earthworks.md), and [`terrain.md`](terrain.md).

## Document Conventions

Interpretation rules:

- `current runtime` means the shipped implementation in the repository today
- `remaining work` means the not-yet-shipped follow-up tracked under [`ROAD-01`](roadmap.md)
- `must` means required for the owning contract
- `should` means intended unless a better measured implementation replaces it
- `may` means optional

Terminology:

- `logical graph`: the existing network topology used for routing and editing
- `road plan polyline`: an edge's world-space XZ path before any surface-width expansion
- `roadbed`: the authoritative drivable / walkable 2.5-D surface generated from the logical graph
- `visual carrier`: the deterministic geometry layer used for rendering, visible-surface queries,
  and road-driven earthworks
- `piece`: one compiled visual road element such as `Span`, `Bend`, `Terminal`, or `JunctionN`
- `profile`: the ordered side-aware road boundary stack carried by one visual piece
- `section`: one sampled cross-section of the roadbed along an edge
- `band`: one lateral surface band such as carriageway, curb, sidewalk, shoulder, or median
- `throat`: the edge-local boundary where an edge corridor hands off into a visual piece
- `legacy node patch`: the retired loop-based junction / terminal carrier that previously tried to
  connect incident edge corridors
- `earthworks`: the derived cut / fill imprint that the roadbed applies to visual terrain
- `surface chunk`: one cached local render / query unit for the roadbed

## Problem Statement

The original surface-road runtime had three architectural problems that made elevated DEM worlds
expose road failures immediately:

- the visible road surface is derived from centerline points whose `y` value is reused across the
  full width, so lateral offsets on cross-slopes float on one side and clip into terrain on the
  other
- terrain carving, editor preview, and final road mesh are not driven from one shared solved
  surface model, so they can disagree about the same road's height
- the active standard-road path must stop treating the road as "centerline plus patched top
  surface" and instead own one authoritative roadbed with explicit width, bands, and visual pieces

The roadbed runtime must continue to solve those problems at the ownership level, not by
reintroducing local patches to the retired renderer.

## Current Runtime Summary

Phases 1-10 now establish the shipped roadbed replacement:

- preview, committed surface rendering, world-surface queries, and terrain earthworks consume one
  compiled roadbed cache
- grounded `Standard` roads stamp both the paved footprint and a deterministic outer shoulder /
  cut / fill transition margin into visual terrain
- grounded `Standard` roads keep a bounded design crossfall instead of rolling the carriageway to
  match the full hillside cross-slope, and the remaining mismatch is absorbed by terrain
  earthworks
- the old widened-ribbon top-surface renderer and centerline-only flattening path are retired

This closes the rough-terrain ownership gap that previously let nearby terrain visually crowd or
overlap the road edge after phases 1-8. Remaining visual limitations on very rough terrain now come
primarily from terrain-cell resolution rather than from missing earthwork ownership or a
terrain-following carriageway profile.

## Current Runtime Gap

The core roadbed ownership and bounded grounded-road crossfall are now live, but authored `10 m`
terrain worlds can still visually smear terrain back across the road corridor on moderate hills.

That remaining gap is no longer caused by:

- centerline-only road ownership
- a terrain-following carriageway crossfall solve
- missing outer cut / fill earthwork margins

But one important ownership gap still remains in the current runtime:

- later terrain authoring still resynchronizes placed grounded `Standard` roads to edited source
  terrain instead of treating the placed roadbed as fixed and reshaping terrain / earthworks
  around it

The earlier terrain-side render-boundary blocker is already removed by [`TERRAIN-01`](roadmap.md),
so this is now primarily a coarse-grid terrain representation problem plus the still-open fixed-road
ownership gap above. Phase 11 exists to quantify that gap on the shipped `10 m` terrain grid,
compare it against a candidate `5 m` grid on the same world-space scenarios, and measure that on
the split terrain path owned by [`terrain.md`](terrain.md) before any default density move is
attempted.

The next representation step is now fixed in [`earthworks.md`](earthworks.md): if denser terrain is
still not sufficient, roads move to a closed road-owned local earthwork mesh near the roadbed
footprint instead of asking visual terrain to remain the final visible owner of the cut / fill
shape.

## Roadbed Contract

The shipped roadbed runtime must guarantee:

- one authoritative roadbed model drives preview, committed render mesh, terrain earthworks, and
  world-surface picking
- lateral road width is part of the solved geometry, not a render-only offset from a 1-D centerline
- node surface ownership is explicit and robust for arbitrary angles, width transitions, dead ends,
  T-junctions, and 4-way intersections
- grounded roads, bridges, and tunnels use one shared surface pipeline with class-specific rules,
  not unrelated mesh code paths
- local edits rebuild only touched edges, nodes, and terrain chunks
- no hot-path allocations or per-frame geometry rebuilds occur outside editing / load boundaries
- output is deterministic for the same world definition, edit sequence, and tick sequence

## Ownership Boundaries

### `RegionGraph`

The logical graph remains authoritative for:

- node identity and edge identity
- route connectivity
- lane counts and modal permissions
- authored edge class (`Standard`, `Bridge`, `Tunnel`)
- plan polyline control points in world XZ

The logical graph must not be treated as:

- the final visible road surface
- the final earthwork boundary carrier
- a generic node polygon carrier for bends or junctions

### `RoadSurfaceSystem`

The current runtime uses one authoritative road-surface layer, `RoadSurfaceSystem`.

It owns:

- edge longitudinal grade solutions
- edge cross-sections and lateral bands
- the deterministic visual carrier compiled from the graph and the solved roadbed
- visual piece classification (`Span`, `Bend`, `Terminal`, `JunctionN`)
- piece-local profile boundaries and mouth / throat handoff data
- the surface mesh cache used by the renderer
- the surface query cache used by editor preview and picking
- the earthwork stamps that terrain applies to the visual terrain buffer

### `TerrainSystem`

`TerrainSystem` remains authoritative for:

- authored source terrain
- derived visual terrain storage
- terrain chunk residency and upload boundaries

`TerrainSystem` must not decide road heights. It only consumes the road system's derived earthwork
contract when building the visual terrain surface.

### Godot Renderers

Godot-side renderers must only upload cached buffers and bind materials.

They must not:

- resample terrain to invent road heights
- apply ad hoc lateral offsets to centerline vertices
- rebuild road surface topology from scratch

### Editor Preview

Editor preview must call the same road-surface solve code as committed placement.

The preview path and the committed path may differ in cache lifetime, but not in geometric rules.

## Authoritative Roadbed Model

### 1. One Roadbed Model For All Surface Consumers

The shipped roadbed runtime is a 2.5-D roadbed model shared by all road-surface consumers.

For one world position `(x, z)` inside a surface road footprint:

- there is exactly one authoritative roadbed height query owned by the road system
- render mesh vertices, preview mesh vertices, lane-marking anchors, and terrain earthwork stamps
  all derive from that same roadbed

The codebase must not maintain separate height-conditioning implementations for:

- preview
- committed road mesh
- terrain flattening
- road picking

### 2. Edge Surface Is Sampled As Ordered Sections

Each non-deleted surface edge must compile into an ordered list of sections.

Each section must store enough data to reconstruct the roadbed locally:

- longitudinal distance `s` from the edge start
- center position in world metres
- unit tangent in XZ
- unit lateral axis in XZ
- solved center height in world metres
- the ordered lateral bands and their offsets from the centerline
- the band-edge heights implied by the edge's lateral profile

Section generation must follow these rules:

- every original edge polyline knot must appear as a section
- every node throat must appear as a section
- long straight spans must be subdivided by fixed world-metre refinement constants owned in code
- refinement must be deterministic and camera-independent
- bridges and tunnels may use different refinement constants than grounded roads, but those
  constants must also be fixed and deterministic

### 3. Longitudinal Grade Is Solved Once Per Edge

The road system must solve longitudinal grade once and cache the result.

For grounded `Standard` edges:

- the grade solve samples authoritative source terrain along the edge plan polyline
- node endpoint heights are constrained by the chosen node solution
- intermediate section heights are derived by the edge grade solver
- the solver may smooth or clamp grade, but the final solved profile becomes authoritative

For `Bridge` and `Tunnel` edges:

- the grade solve is constrained by authored bridge / tunnel intent and clearance rules, not by
  copying source terrain at every section
- abutment / portal transitions must still be represented by the same section list format

The renderer must never resample terrain on the fly after the grade solve.

### 4. Lateral Profile Is Explicit

Every section must expose explicit lateral bands rather than inferring the full width from a single
edge `width` offset.

At minimum, the model must support:

- left sidewalk band
- left curb / shoulder band
- carriageway band
- right curb / shoulder band
- right sidewalk band

The runtime may later add medians, parking lanes, tram reservation bands, or cycle
tracks, but the ownership model must already support arbitrary ordered lateral bands.

Deterministic rule:

- lateral roadbed height comes from the section's cross-section profile, not from terrain samples
  under the left or right edge
- grounded `Standard` roads may use terrain only to choose a bounded design crossfall direction and
  magnitude; they must not copy the full hillside left/right terrain delta into carriageway roll

This is the key rule that removes the "half buried, half floating" failure mode.

### 5. Cross-Section Profile Is Piecewise Linear In Width

For any section, the surface profile across width must be reconstructible as a piecewise-linear
function of lateral offset.

That function must support at least:

- flat carriageway
- crowned carriageway
- fixed one-sided crossfall
- curb step between carriageway and sidewalk
- outward sidewalk slope

The exact authored defaults may be simple at first, but the representation must not hard-code
"single flat ribbon" as the only possible shape.

### 6. Visual Presentation Is A Separate Piece/Profile Carrier

The roadbed runtime must split simulation topology from visible geometry.

Required rule:

- the logical graph continues to own connectivity, IDs, lane semantics, and authored plan curves
- the visual carrier becomes a separate deterministic geometry layer built from that graph data
- the minimum required visual piece set is:
  1. `Span`
  2. `Bend`
  3. `Terminal`
  4. `JunctionN`
- the renderer, visible-surface queries, and road-driven earthworks must consume that same visual
  carrier instead of rebuilding independent geometry from raw graph clips

### 7. Every Piece Owns One Ordered Side-Aware Profile

The runtime must not treat a road as one anonymous width around a centerline.

Each visual piece must own one ordered side-aware profile.

For the default standard-road slice, the minimum required profile stack is:

1. left outer sidewalk edge
2. left curb / shoulder edge
3. left carriageway edge
4. right carriageway edge
5. right curb / shoulder edge
6. right outer sidewalk edge

Later profile variants may add more band boundaries, but the runtime must not collapse back to one
anonymous throat endpoint pair.

### 8. `Span` Owns Ordinary Edge Corridors And `Bend` Owns Every Two-Way Corner

Ordinary edge corridors and two-edge corners must not be represented by a generic node polygon.

Required rule:

- every non-terminal edge run compiles to one or more `Span` pieces using the solved longitudinal
  grade plus the ordered section profile
- every degree-`2` non-pass-through corner compiles to a dedicated `Bend` piece
- a `Bend` is a normal visual road piece, not a fallback or special-case patch
- triangle networks therefore compile as `3 spans + 3 bends`; they do not require a generic node
  polygon carrier
- the `Bend` piece owns both carriageway and sidewalk continuation through the corner, so the
  runtime must not leave that continuation to overlapping edge strips or to terrain-side fill
- the `Bend` piece must not synthesize a shared junction-style asphalt core polygon; it connects
  its two mouths directly through piece-owned sector geometry
- the `Bend` piece outer boundary loops must come directly from those compiled bend sectors rather
  than from a later generic polygon-boundary extraction pass

### 9. `Terminal` And `JunctionN` Own Their Geometry Explicitly

`Terminal` and `JunctionN` must remain explicit visual pieces with deterministic ownership.

Required rule:

- `Terminal` owns its cap / closure geometry directly from the incident piece profile
- `Terminal` outer boundary loops must come directly from that compiled cap geometry rather than
  from a later generic polygon-boundary extraction pass
- `JunctionN` owns one carriageway center plus explicit perimeter sidewalk sectors between adjacent
  mouths
- `JunctionN` must not be built from one generic angle-sorted cloud of throat endpoints
- `JunctionN` must not be rendered as one global sidewalk annulus between one outer loop and one
  inner loop
- `JunctionN` must not synthesize one generic center asphalt polygon from all mouth inner points;
  the carriageway core is assembled from explicit adjacent-mouth wedges around the node
- `JunctionN` must not run a second-pass center/core polygon builder after sector compilation; the
  explicit adjacent-gap sectors are themselves the live carriageway carrier
- `JunctionN` must not reuse the `Bend` side-sector builder as its final geometry carrier; it
  owns a dedicated adjacent-gap sector builder for multi-arm node geometry
- no sidewalk triangles may own the carriageway center
- no carriageway seam may appear between an incident `Span` throat and the node-owned continuation

### 10. Mouths And Adjacent Sectors Are The General Connector Model

The shipped roadbed runtime does not treat sidewalks as a symmetric visual halo around the road
centerline.

For every incident surface edge at one piece boundary:

- compute a throat section at the deterministic handoff distance
- compute that handoff distance from the exact clip carried on the graph edge, and for acute
  bends / junction sectors derive that clip from adjacent outer roadbed boundaries rather than
  from a fixed half-width constant
- derive one canonical mouth profile from that throat section
- preserve the ordered semantic profile stack through that mouth

General connector rule:

- `Bend` connects exactly two mouth profiles directly
- `JunctionN` sorts mouths by centerline angle around the node with a stable tie-breaker, then
  builds one adjacent-mouth sector for each consecutive pair
- each adjacent sector is assembled band-by-band from paired mouth boundaries
- connector sampling must use a fixed deterministic step no larger than `1 m`
- the runtime must not generate the full node from a single generic fill polygon and hope later
  triangulation recovers the intended ownership

Sidewalk ownership remains explicit per side:

- left and right sidewalk bands are separate authored / derived bands
- footpath connections attach to one side or the other, not to an abstract road-center sidewalk
- crosswalk and frontage-side semantics must stay aligned with [`entrance_and_exit.md`](entrance_and_exit.md)
- pedestrian crosswalk mouths must use the same deterministic throat clip / mouth positions as the
  visible road surface; they must not invent a separate shallower node boundary

This document does not define the pedestrian-routing rules, but it does require the road surface
representation to preserve side-aware geometry so those rules can use it later.

### 11. Lane Markings Derive From The Same Surface Model

Lane markings remain a separate render layer, but their geometry must derive from the same section
and throat data as the top surface.

Required rules:

- markings follow solved section heights, not independent centerline offsets
- markings attach to mouth profiles and terminate or continue according to the chosen node builder,
  not according to one generic node-fill polygon
- markings never extend into carriageway areas not owned by the current edge corridor

### 12. Normals Are Derived From Real Geometry

The roadbed runtime does not treat all road-surface normals as `Vector3::UP`.

Normals for:

- carriageway
- sidewalk
- bridge deck
- transition ramps

must be derived from the actual triangles or reconstructed cross-section planes so sloped and
banked geometry shades correctly.

## Road Earthworks Boundary

The shared engineered-ground contract now lives in [`earthworks.md`](earthworks.md). This document
only owns the road-specific side of that boundary.

Road-specific requirements:

- grounded `Standard` roads use compiled roadbed sections and lateral bands as the support surface
  that the shared earthworks system must honor
- road-specific footprint ownership comes from carriageway, curb / shoulder, and sidewalk bands,
  not from a centerline-only approximation
- grounded roads require an outer deterministic cut / fill margin derived from those same compiled
  sections
- bounded grounded-road crossfall remains a roadbed rule owned here, while the shared tie-in back
  to terrain is owned by [`earthworks.md`](earthworks.md)

Road-class-specific earthwork rules:

- `Bridge` edges keep the deck surface on the roadbed path, but earthworks stay limited to
  abutments or authored supports
- `Tunnel` edges keep the underground roadbed for routing and underground rendering, but surface
  terrain carving stays limited to portals

## Rendering Contract

### 1. Surface Meshes Are Cache Outputs

The renderer consumes cached surface chunks for:

- carriageway
- sidewalk / curb
- lane markings
- bridge concrete / structural meshes

The mesh generator may share low-level helpers, but it must not share the old contour-patching
ownership model.

### 2. Material Contract Can Stay Stable

The current runtime preserves the Godot material contract where useful:

- carriageway vertices use the carriageway material path
- sidewalk vertices use the sidewalk material path
- lane markings stay a separate top layer

But the geometry contract changes:

- `UV` values are produced by the road-surface compiler, not by assumptions baked into the old
  widened-strip renderer
- `UV.y` must not be treated as a hidden "road edge" contract outside the compiler that owns it

### 3. Overdraw Remains Allowed As A Render Detail, Not As Ownership

Render-layer ordering may still be used for final compositing, but it is no longer the primary
definition of who owns the junction center.

Ownership comes from:

- edge corridor boundaries
- visual piece boundaries
- explicit band surfaces

Overdraw is only a draw-order convenience after that ownership is already known.

## Editor And Query Contract

### 1. Preview And Commit Use The Same Solve

The preview road shown during drawing must be produced by the same compilation steps as the
committed road:

- edge plan generation
- class selection or validation
- grade solve
- section generation
- visual piece generation

The preview may reuse temporary IDs and temporary caches, but it must not use a separate
"looks similar enough" geometry path.

### 2. Combined World-Surface Picking Must Exist

The runtime exposes a combined world-surface query that can return:

- roadbed height if a roadbed owns the point
- otherwise terrain height

Editor tools that place or inspect roads must use that combined query rather than raycasting
against terrain alone.

## Performance Contract

The shipped roadbed runtime must respect the project's scale target.

Required bounds:

- local edge edits rebuild only touched edges, touched nodes, and touched surface / terrain chunks
- no full-network mesh rebuild is allowed as the steady-state response to one local road edit
- section compilation for independent touched edges should parallelize with Rayon
- terrain earthwork stamping for independent chunks should parallelize with Rayon
- no allocation is allowed inside per-frame render upload loops beyond unavoidable Godot boundary
  copies

Preferred cache structure:

- use the existing road chunking / terrain chunking conventions where they already fit
- avoid inventing a second unrelated spatial ownership grid when one of the existing chunk systems
  can index the same work

## Remaining Work

Phases 1-10 of [`ROAD-01`](roadmap.md) are shipped. They are intentionally not repeated here as
rollout history; this document now focuses on the live roadbed contract plus the remaining active
follow-up work.

### Phase 11 - Coarse-Grid Fidelity Decision Gate

Phase 11 exists because the roadbed runtime is now coherent, but authored `10 m` worlds can still
visually smear terrain back across a grounded road corridor on moderate hills.

Required Phase 11 work:

- keep deterministic characterization tests for the same grounded hillside-road case on both the
  shipped `10 m` terrain grid and a candidate `5 m` grid
- measure visible terrain overlap back onto the compiled roadbed footprint instead of relying on
  screenshots or manual editor inspection
- keep the world-space road path, terrain function, and grading contract identical between the
  `10 m` and `5 m` variants so the comparison isolates terrain-cell density
- treat those tests as terrain-side evidence, not as permission to extend the current whole-map
  dense terrain renderer upload path
- use the characterization as a decision gate:
  - if `5 m` materially reduces overlap, it remains a viable path
  - if `5 m` still leaves unacceptable overlap, the next fix must be a different representation
    such as road-owned closed local earthwork geometry rather than more heightfield density alone

Phase 11 is a measurement and decision phase, not a claim that denser terrain alone is already the
solution. Any default density move now depends on the chunk-local terrain / water render split
defined in [`terrain.md`](terrain.md).

### Phase 12 - Keep Placed Roadbeds Fixed Under Later Terrain Edits

Placed-road ownership must stay explicit after the road is committed.

Required Phase 12 work:

- terrain-authoring edits must no longer resynchronize placed grounded `Standard` roads to edited
  source terrain as a side effect
- once a road is committed, later terrain edits must write source terrain first and then rebuild
  visual terrain, local earthwork geometry, and earthworks around that fixed roadbed instead
- explicit road-edit operations remain the only way to move or regrade the committed road surface
- placement-time grounding may still choose the initial roadbed, but that placement solve must not
  remain a live dependency of later terrain brushes
- the same fixed-roadbed rule must stay consistent across committed render mesh, world-surface
  queries, local earthwork geometry caches, and terrain earthworks
- if a later terrain edit would create an extreme surrounding cut / fill case, the road still stays
  fixed; later earthwork geometry variants may change representation, but terrain brushes must not
  silently move the committed roadbed

Phase 12 is separate from Phase 11 because it changes ownership semantics first; denser terrain or
local earthwork geometry still remain possible follow-ups after that rule is in place.

### Phase 13 - Closed Road-Owned Earthwork Mesh Rewrite

The next representation fix is now deterministic, and it replaces the current corridor-sheet
prototype instead of polishing it further.

Required Phase 13 work:

- grounded roads must own closed local earthwork geometry on both sides of the committed roadbed
  footprint
- each road edge span must derive left and right outer tie-in polylines from terrain sampled at a
  maximum `2 m` longitudinal spacing in world space
- each side mesh must begin at the road-owned top-surface boundary and end at the sampled tie-in
  boundary for that same side
- the visible tie-in boundary must not be inferred only from the authored terrain cell grid or only
  from the current compiled road section spacing
- the first shipped rewrite variant must include deterministic slope faces plus closure /
  underside geometry wherever otherwise-visible voids would appear
- flat-ground cases must collapse toward a minimal shoulder / verge join instead of emitting a wide
  apron or visible side sheet
- terminal road ends must emit deterministic cap geometry from the road-owned top surface to the
  tie-in boundary
- the visual road carrier must split from the logical graph:
  - the graph remains connectivity and routing authority
  - the visual carrier becomes explicit geometry pieces
- the minimum required visual piece set is `Span`, `Bend`, `Terminal`, and `JunctionN`
- two-edge non-pass-through nodes must be built as `Bend` pieces, not by a generic junction fill
- multi-arm nodes must be built as `JunctionN` pieces with one carriageway-owned center plus
  explicit adjacent-mouth sidewalk sectors
- future node-piece perimeter tie-ins must follow the same ownership model so edge ownership and
  node ownership still meet cleanly at the throat boundary
- local earthwork geometry must be partitioned into touched terrain-chunk-aligned caches instead of
  one monolithic world mesh
- road visible-surface queries must resolve as road top surface first, then road-owned local
  earthwork geometry, then visual terrain
- terrain suppression may only hide true overlap beneath client-owned geometry; it must not act as
  a substitute for missing tie-in faces, and road-edge visibility must remain correct without a
  terrain texture-mask carrier
- retaining or wall variants are later geometry extensions, not a separate earthworks system

Phase 13 is now explicitly a rewrite of the failed corridor-sheet representation, not an extension
of it.

The still-open building-pad client work and later retaining / wall variants tracked in
[`earthworks.md`](earthworks.md) are later additions to the same subsystem, not blockers for the
first roads-first closed-mesh rewrite.

Current status:

- the earlier corridor-sheet prototype is not accepted as the Phase 13 endpoint and should not be
  polished further
- the useful conclusions from that prototype remain in force: fixed-roadbed ownership, chunk-local
  rebuilds, and explicit visible-surface precedence stay part of the target contract
- the Phase 13 carrier rewrite is now live in code; the remaining work is refinement on top of
  that shipped carrier, not another representation reset
- the graph/visual split is now live in code:
  - committed spans now own explicit road / sidewalk polygons plus explicit earthwork polygons and
    outer earthwork boundaries
  - committed spans now compile their outer boundary loops directly from section ranges instead of
    extracting them back out of emitted polygons
  - committed terminals compile explicit cap polygons from the mouth profile
  - committed two-way bends now compile through a dedicated `Bend` builder instead of sharing one
    generic connected-node path with `JunctionN`
  - committed two-way bends compile explicit band-by-band bend polygons from paired mouth profiles
  - multi-arm nodes now compile through a dedicated `JunctionN` builder fed by ordered incident
    mouths and adjacent gap sectors rather than by the old generic connected-node path
  - multi-arm nodes compile explicit `JunctionN` road polygons plus sidewalk sector polygons
  - adjacent mouth-side sectors no longer fall back to one emergency quad when side profiles do
    not have matching band counts; they use one deterministic merged depth-break connector solve
  - node incident ordering and bend / junction classification no longer recover throat directions
    from compiled edge sections; they read deterministic inward directions from compiled span
    mouth profiles
  - width changes are no longer treated as a separate visual node-piece class; ordinary spans own
    that handoff directly
  - compiled visual polygons now cache deterministic triangles so rendering, surface queries, and
    earthwork stamping no longer re-fan concave node sectors differently at each call site
  - `JunctionN` adjacent-mouth sectors now use the ordered-mouth ownership rule directly: for each
    adjacent CCW gap, the sector is built from `current.left` and `next.right` instead of from a
    heuristic gap-facing side selector
  - adjacent mouth-side sectors now insert fixed-step connector samples at `<= 1 m` in addition to
    profile breakpoints, and `Bend` no longer borrows the generic junction-style center asphalt
    polygon
  - `Terminal` outer boundary loops are now compiled directly from explicit cap geometry instead of
    being reconstructed from emitted polygons afterward
  - `Bend` outer boundary loops are now compiled directly from explicit bend sectors instead of
    being reconstructed from emitted polygons afterward
  - footpath mouths now compile directly inside the incident piece-mouth path instead of through a
    separate fallback helper
  - `JunctionN` no longer borrows one global angle-sorted center asphalt polygon; its center
    carriageway is assembled from adjacent-mouth wedges around the node
  - `JunctionN` outer boundary loops are now compiled directly from explicit adjacent-gap sectors
    instead of being reconstructed in a second pass from the ordered mouth list
  - `Bend` and `JunctionN` no longer share one connector-strip polygon builder; each piece class
    now owns its own connector-strip sampling path
  - renderer output, visible-surface queries, debug overlays, and road-driven earthwork stamping
    now consume explicit visual pieces instead of a node-patch carrier
  - terrain earthwork chunk coverage and stamping no longer walk compiled edge sections after
    piece compilation; they consume span-owned earthwork polygons and span-owned outer earthwork
    boundaries
  - span-piece earthwork stamping no longer regenerates tie-in faces from boundary loops at stamp
    time; spans now compile explicit tie-in side polygons plus explicit tie-in outer loops during
    piece compile
  - node pieces now also compile explicit earthwork polygons and earthwork outer boundaries, so
    node earthwork chunking, bounds, and terrain stamping no longer borrow the visible polygon
    carrier at runtime
  - `Terminal`, `Bend`, and `JunctionN` now pass explicit earthwork polygons and explicit
    earthwork outer loops directly from their own builders instead of relying on a shared
    node-piece assembler to infer earthwork ownership from visible geometry
  - node-piece earthwork stamping no longer regenerates tie-in faces from boundary loops at stamp
    time; `Terminal`, `Bend`, and `JunctionN` now compile explicit tie-in side polygons plus
    explicit tie-in outer loops during piece compile
  - the visible road mesh now includes a dedicated earthwork layer fed directly from compiled span
    and node earthwork ownership instead of stopping that carrier at chunking, terrain stamping,
    and world-surface queries
  - render no longer draws the full support carrier directly: spans and node pieces now compile a
    separate render-only earthwork face set, so hidden support polygons stay available for
    stamping / queries without leaking into the visible road mesh
  - render-only earthwork faces are now classified deterministically as either `Slope` or
    `RetainingWall`, and the renderer routes those two face classes to different materials instead
    of painting every tie-in face as generic exposed earthwork
  - visible-world height/raycast queries can now hit compiled span and node earthwork geometry
    before falling through to terrain
- the remaining Phase 13 work is no longer the carrier rewrite itself; the main remaining gap is
  refinement work inside that live carrier: richer retaining / wall variants, better material
  treatment, and carrying the same explicit piece-owned geometry model through those variants
  without regressing the shared piece/profile ownership model

## Legacy Retirement Map

The piece/profile rewrite does not retire the entire roadbed runtime. It retires the parts of the
previous carrier that synthesized visible road ownership from generic node patches and loop
triangulation.

The following current code remains architecturally useful and should be extended rather than
thrown away:

- logical graph ownership in `rust/src/simulation/network/mod.rs` and `graph/`
- solved edge grades, section sampling, and lateral band definitions in
  `rust/src/simulation/network/surface/mod.rs`
- chunk-local surface and earthwork cache boundaries in
  `rust/src/simulation/network/surface/mod.rs`
- Godot-side refresh timing and mesh-upload orchestration in
  `godot/scripts/renderers/network_renderer.gd`

The following code is legacy under the new target and is already removed or explicitly retired.

### Rust Legacy Carrier To Retire

- `RoadSurfaceNodePatchClass` and `RoadSurfaceNodePatch` in
  `rust/src/simulation/network/surface/mod.rs`
  - this is still the old node-patch carrier
  - it collapses arbitrary non-pass-through two-edge cases into a generic `Junction` path instead
    of a real `Bend` piece
- `RoadSurfaceNodeBoundaryPoint` and `RoadSurfaceNodeBoundaryLoop` in
  `rust/src/simulation/network/surface/mod.rs`
  - these exist only to support the current loop-synthesis carrier
- `compile_node_patch`, `classify_node_patch`, `build_node_patch_boundary_loops`,
  `build_throat_boundary_points`, `build_terminal_boundary_loop`, and `finalize_boundary_loop` in
  `rust/src/simulation/network/surface/mod.rs`
  - these are the current generic node-fill builders that the rewrite is replacing
- any generic node-piece outer-boundary extraction path that tries to infer `Terminal`, `Bend`, or
  `JunctionN` ownership from already-emitted polygons
  - all live node-piece classes now compile explicit outer boundary loops directly from their own
    piece geometry
- any shared node-piece earthwork inference path that tries to clone visible polygons or visible
  outer loops into node earthwork ownership
  - all live node-piece classes now compile and pass explicit earthwork polygons and explicit
    earthwork outer loops directly from their own builders
- any shared connector-strip polygon builder used as the live geometry carrier for both `Bend` and
  `JunctionN`
  - those piece classes now own separate connector-strip builders even though they still share
    lower-level geometry utilities like point sampling
- the separate `build_footpath_piece_mouth(...)` fallback helper
  - footpath mouths now compile directly inside the main incident-mouth builder
- node-patch earthwork helpers in `rust/src/simulation/network/surface/mod.rs`
  - `node_patch_overlaps_chunk`
  - `visit_visible_node_triangles`
  - `stamp_node_patch_earthwork_margins_for_chunk`
  - `stamp_node_patch_earthworks_for_chunk`
  - `node_patch_bounds`
  - any other helper whose job is specifically to turn a node patch loop into render or earthwork
    ownership
- section-window edge earthwork stamping helpers in `rust/src/simulation/network/surface/mod.rs`
  - `stamp_edge_earthworks_for_chunk`
  - `stamp_standard_edge_earthwork_margins_for_chunk`
  - `stamp_edge_earthwork_margin_side`
  - once compiled span pieces exist, earthwork chunk coverage and terrain stamping must not fall
    back to raw edge-section windows as the steady-state authority
- tests that lock in loop-based node-patch behavior in
  `rust/src/simulation/network/surface/mod.rs`
  - for example `throat_boundary_loops_are_angle_sorted_and_stable`
  - these must be replaced by black-box piece/profile tests once the new carrier lands

### Rust Renderer Paths To Retire

- loop-fan node rendering in `rust/src/simulation/network/render/road/standard_surface.rs`
  - this renderer path is now retired
  - `emit_compiled_surface_mesh(...)` now consumes explicit road polygons and sidewalk polygons
    from the visual carrier
- any renderer path that assumes node ownership comes from one outer loop plus one carriageway loop
  rather than from explicit `Span` / `Bend` / `Terminal` / `JunctionN` piece geometry

### Rust Debug / Bridge Paths To Retire Or Rename

- `piece_boundary_lines` in `rust/src/nodes/sim/render/network.rs`
  - this is the piece-oriented replacement debug channel
  - it must not regress back to exposing raw node patches as the visual authority
- any future geometry dumps that still serialize `compiled_node_patches`, `boundary_loops`, or
  `carriageway_boundary_loops` as if those were the final visual authority

### Godot-Side Legacy Bridges To Retire Or Rename

- `piece_boundary_lines` debug overlay consumption in
  `godot/scripts/tools/network_tool.gd`
  - this is the first hard-cut replacement for the old `node_patch_lines` bridge
  - the overlay must keep visualizing explicit road pieces rather than regressing to node-patch
    terminology
- any editor/debug terminology that still tells the user the visual road authority is a node patch
  or a generic junction fill

### Explicit Non-Legacy Boundaries

The following are not legacy by themselves and should not be thrown away just because they are old:

- edge section sampling
- lateral band definitions
- chunk-local dirtying and rebuild boundaries
- preview/commit parity as a rule
- the graph as simulation authority
- Godot mesh upload as a thin presentation bridge

## Test Contract

Tests for the roadbed runtime must be black-box contract tests, not shape snapshots of one internal
implementation.

Must cover:

- straight grounded road on flat terrain
- straight grounded road on strong cross-slope
- arbitrary-angle bend with sidewalks
- obtuse bend with sidewalks
- shallow-angle bend with sidewalks
- triangle network composed of three independent bends
- pass-through split with no center bubble
- width transition on a nearly straight corridor
- T-junction center owned by carriageway
- 4-way junction center owned by carriageway
- `N > 4` multi-arm node center owned by carriageway
- car-only road with no sidewalk bands
- footpath joining only one sidewalk side
- bridge span above terrain without terrain flatten under the span
- tunnel portal behavior without surface carving along the buried segment
- preview / commit parity for the same input path
- terrain earthwork agreement with the roadbed inside the paved footprint on supportive terrain
  densities
- deterministic cut / fill transition outside the paved footprint inside the earthwork margin
- grounded hillside roads keep a bounded carriageway crossfall instead of following the full raw
  terrain cross-slope
- deterministic characterization of the same grounded hillside road at `10 m` and `5 m` terrain
  resolution, including explicit measured overlap comparison
- deterministic rebuild: same input graph produces the same section data and mesh indices
- local invalidation: editing one edge does not force unrelated chunks to rebuild

## Explicit Runtime Rule

The shipped roadbed runtime is the only valid path for surface roads. New work must extend it
instead of reintroducing retired centerline ownership.

The following legacy patterns must not be reintroduced:

- centerline-only lateral lift logic for road surface vertices
- terrain flattening that samples only the centerline height and then separately resyncs the road
- paved-footprint-only earthwork stamping as the final grounded-road terrain contract
- any renderer code that treats node disks or widened ribbons as the sole ownership model
- any visual road carrier that tries to derive final bends or multi-arm junctions from one generic
  graph-driven fill polygon
- any annulus-style node sidewalk model built from one outer loop and one inner loop
- dead cached junction polygon state that no longer serves the roadbed pipeline

The shipped system is:

- one logical graph
- one authoritative roadbed compiler
- one deterministic visual carrier built from explicit road pieces
- one terrain earthwork contract derived from that roadbed
- one render cache derived from that same roadbed
