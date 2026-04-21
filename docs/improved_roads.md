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
- `section`: one sampled cross-section of the roadbed along an edge
- `band`: one lateral surface band such as carriageway, curb, sidewalk, shoulder, or median
- `throat`: the edge-local boundary where an edge corridor hands off into a node patch
- `node patch`: the junction or terminal surface patch connecting incident edge corridors
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
- the active standard-road path still treats the road as "centerline plus patched top surface"
  instead of as one authoritative roadbed with explicit width, bands, and node patches

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

It is now primarily a coarse-grid terrain representation problem. Phase 11 exists to quantify that
gap on the shipped `10 m` terrain grid, compare it against a candidate `5 m` grid on the same
world-space scenarios, and feed that result into the terrain-side render-boundary work owned by
[`terrain.md`](terrain.md) before any default density move is attempted.

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

The logical graph must not be treated as the final visible road surface.

### `RoadSurfaceSystem`

The current runtime uses one authoritative road-surface layer, `RoadSurfaceSystem`.

It owns:

- edge longitudinal grade solutions
- edge cross-sections and lateral bands
- node patch classification and triangulation inputs
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

### 6. Node Surface Uses Explicit Patches

The roadbed runtime does not rely on implicit overdraw alone to define junction ownership.

Each relevant node must compile to one of these classes:

- `Terminal`
- `PassThrough`
- `WidthTransition`
- `Junction`

Classification must be deterministic from:

- incident edge classes
- incident edge directions in XZ
- lateral-profile compatibility
- modal surface presence on each side

Required behavior:

- `Terminal` nodes emit a terminal patch
- `PassThrough` nodes emit no node patch and hand corridor ownership directly edge-to-edge
- `WidthTransition` nodes emit a transition patch even when the node is nearly straight
- `Junction` nodes emit a full node patch

### 7. Node Patch Boundaries Are Built From Throats

Node patches must be bounded by incident edge throats, not by ad hoc circular disks that ignore the
real corridor width.

For every incident surface edge at one node:

- compute a throat section at the node handoff distance
- derive left and right boundary points for each relevant lateral band
- sort incidents by angle around the node center with a stable tie-breaker
- assemble ordered boundary loops from those throat points

The runtime may use deterministic triangulation such as ear clipping or another stable
polygon triangulation strategy, but it must satisfy:

- the same boundary loop always triangulates to the same index order
- the patch owns the junction center explicitly
- no sidewalk triangles appear inside the carriageway-owned area
- no carriageway seam appears between incident edges and the node patch

### 8. Sidewalk Ownership Is Side-Aware

The shipped roadbed runtime does not treat sidewalks as a symmetric visual halo around the road
centerline.

Sidewalk ownership must be explicit per side:

- left and right sidewalk bands are separate authored / derived bands
- footpath connections attach to one side or the other, not to an abstract road-center sidewalk
- crosswalk and frontage-side semantics must stay aligned with [`entrance_and_exit.md`](entrance_and_exit.md)

This document does not define the pedestrian-routing rules, but it does require the road surface
representation to preserve side-aware geometry so those rules can use it later.

### 9. Lane Markings Derive From The Same Surface Model

Lane markings remain a separate render layer, but their geometry must derive from the same section
and throat data as the top surface.

Required rules:

- markings follow solved section heights, not independent centerline offsets
- markings terminate at node throats unless one specific node patch rule extends them
- markings never extend into carriageway areas not owned by the current edge corridor

### 10. Normals Are Derived From Real Geometry

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
- node patch boundaries
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
- node patch generation

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
    such as road-owned corridor / skirt geometry rather than more heightfield density alone

Phase 11 is a measurement and decision phase, not a claim that denser terrain alone is already the
solution. Any default density move now depends on the chunk-local terrain / water render split
defined in [`terrain.md`](terrain.md).

### Phase 12 - Keep Placed Roadbeds Fixed Under Later Terrain Edits

Placed-road ownership must stay explicit after the road is committed.

Required Phase 12 work:

- terrain-authoring edits must no longer resynchronize placed grounded `Standard` roads to edited
  source terrain as a side effect
- once a road is committed, later terrain edits must rebuild visual terrain and earthworks around
  that fixed roadbed instead
- explicit road-edit operations remain the only way to move or regrade the committed road surface
- placement-time grounding may still choose the initial roadbed, but that placement solve must not
  remain a live dependency of later terrain brushes
- the same fixed-roadbed rule must stay consistent across committed render mesh, world-surface
  queries, and terrain earthworks

Phase 12 is separate from Phase 11 because it changes ownership semantics first; denser terrain or
local corridor geometry still remain possible follow-ups after that rule is in place.

## Test Contract

Tests for the roadbed runtime must be black-box contract tests, not shape snapshots of one internal
implementation.

Must cover:

- straight grounded road on flat terrain
- straight grounded road on strong cross-slope
- arbitrary-angle bend with sidewalks
- obtuse bend with sidewalks
- pass-through split with no center bubble
- width transition on a nearly straight corridor
- T-junction center owned by carriageway
- 4-way junction center owned by carriageway
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
- dead cached junction polygon state that no longer serves the roadbed pipeline

The shipped system is:

- one logical graph
- one authoritative roadbed compiler
- one terrain earthwork contract derived from that roadbed
- one render cache derived from that same roadbed
