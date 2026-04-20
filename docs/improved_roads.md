# Improved Roads

## Purpose

This document owns the deterministic long-term replacement of the current surface-road
implementation.

Tracked work for this document lives under [`ROAD-01`](roadmap.md).

It answers these questions:

- what the authoritative road surface model is
- how road geometry interacts with terrain, rendering, and editor preview
- which subsystem owns each piece of data
- what deterministic rules the replacement must satisfy
- which old code paths are compatibility-only and must eventually be removed

It does not own:

- lane routing policy or turn legality
- frontage-side building entrance semantics
- terrain storage internals or large-world terrain streaming policy

Those remain owned by [`entrance_and_exit.md`](entrance_and_exit.md) and
[`terrain.md`](terrain.md).

## Document Conventions

Interpretation rules:

- `current runtime` means the shipped implementation in the repository today
- `replacement target` means the required long-term implementation this document defines
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

The current runtime still has three architectural problems that make elevated DEM worlds expose
road failures immediately:

- the visible road surface is derived from centerline points whose `y` value is reused across the
  full width, so lateral offsets on cross-slopes float on one side and clip into terrain on the
  other
- terrain carving, editor preview, and final road mesh are not driven from one shared solved
  surface model, so they can disagree about the same road's height
- the active standard-road path still treats the road as "centerline plus patched top surface"
  instead of as one authoritative roadbed with explicit width, bands, and node patches

The replacement must remove those problems at the ownership level, not by adding more local
patches to the existing renderer.

## Replacement Goals

The replacement target must guarantee:

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

### New `RoadSurfaceSystem`

The replacement target introduces one authoritative road-surface layer, referred to here as
`RoadSurfaceSystem`.

It must own:

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

## Authoritative Replacement Model

### 1. One Roadbed Model For All Surface Consumers

The replacement target is a 2.5-D roadbed model shared by all road-surface consumers.

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

The replacement target may later add medians, parking lanes, tram reservation bands, or cycle
tracks, but the ownership model must already support arbitrary ordered lateral bands.

Deterministic rule:

- lateral roadbed height comes from the section's cross-section profile, not from terrain samples
  under the left or right edge

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

The replacement target does not rely on implicit overdraw alone to define junction ownership.

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

The replacement target may use deterministic triangulation such as ear clipping or another stable
polygon triangulation strategy, but it must satisfy:

- the same boundary loop always triangulates to the same index order
- the patch owns the junction center explicitly
- no sidewalk triangles appear inside the carriageway-owned area
- no carriageway seam appears between incident edges and the node patch

### 8. Sidewalk Ownership Is Side-Aware

The long-term replacement must stop treating sidewalks as a symmetric visual halo around the road
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

The replacement target must stop treating all road-surface normals as `Vector3::UP`.

Normals for:

- carriageway
- sidewalk
- bridge deck
- transition ramps

must be derived from the actual triangles or reconstructed cross-section planes so sloped and
banked geometry shades correctly.

## Terrain Interaction Contract

### 1. Source Terrain Remains Authoritative Ground

The road system must not rewrite authored source terrain.

`TerrainSystem` continues to own:

- source terrain
- visual terrain

Roads only contribute derived earthworks to the visual terrain buffer.

### 2. Earthworks Derive From The Roadbed, Not From Edge Centerlines

Earthworks must be generated from the compiled roadbed sections and node patches.

They must not be generated by:

- sampling only the edge centerline height
- reusing the old "flat ribbon around the centerline" assumption
- running a second independent smoothing pass after the road mesh is already decided

Inside the paved / sidewalk footprint:

- visual terrain must be cut or filled to the roadbed support surface defined by the road system

Outside the footprint but inside the earthwork margin:

- visual terrain transitions back to source terrain using deterministic shoulder / cut / fill rules

### 3. Earthworks Must Be Chunk-Local

The replacement target must retire full-world dense terrain flattening for local road edits.

Required rule:

- a road edit marks only touched terrain chunks dirty
- earthwork recomputation runs only for those touched chunks
- terrain visual upload stays bounded to those chunks or to a bounded compatibility window until
  chunked terrain rendering replaces the current full-heightmap upload path

The road replacement may temporarily ship through a compatibility boundary, but the final contract
is chunk-local.

### 4. Bridges And Tunnels Have Separate Earthwork Rules

For `Bridge` edges:

- the deck surface is owned by the roadbed like any other surface edge
- terrain earthworks are limited to abutments or explicitly authored supports
- the terrain directly under the span is not flattened to deck height

For `Tunnel` edges:

- the tunnel roadbed still exists for routing and underground rendering
- surface terrain does not carve down to the tunnel deck except at portals

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

The replacement may preserve the current Godot material contract where useful:

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

The long-term replacement must add a combined world-surface query that can return:

- roadbed height if a roadbed owns the point
- otherwise terrain height

Editor tools that place or inspect roads must use that combined query rather than raycasting
against terrain alone.

## Performance Contract

The replacement must respect the project's scale target.

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

## Migration Rules

The old renderer and flattening code are compatibility code only. They are not the long-term
architecture.

The replacement should land in deterministic slices:

1. Introduce `RoadSurfaceSystem` and compile cached edge sections from the existing logical graph.
2. Move editor preview to that shared compilation path.
3. Replace top-surface road and sidewalk mesh generation with cached corridor and node-patch
   surfaces.
4. Replace lane-marking generation so it anchors to throat / section data.
5. Replace centerline-based terrain flattening with chunk-local earthwork stamping from the same
   roadbed caches.
6. Add combined world-surface picking for editor tools.
7. Remove the old contour / patch / widened-ribbon renderer path and any dead cached junction
   polygon state left behind.

During migration:

- the logical graph may remain the same
- the road-surface caches may coexist with old render paths only long enough to validate parity
- once one surface class is switched to the new compiler, that class should stop accumulating new
  special-case patches in the old renderer

## Implementation Plan

The implementation plan for [`ROAD-01`](roadmap.md) is phase-based. Each phase must leave the
runtime in a deterministic, testable state rather than introducing a large partially-owned rewrite.

### Phase 1 - Lock the new road-surface data model

- Add the `RoadSurfaceSystem` shell and make it the only future owner of compiled roadbed data.
- Introduce explicit compiled-surface types for:
  - edge sections
  - lateral bands
  - node patch classes
  - node patch boundary loops
  - per-chunk surface cache entries
  - per-chunk earthwork cache entries
- Keep `RegionGraph` authoritative only for logical topology, modal permissions, edge class, and
  plan polyline control points.
- Keep all roadbed heights in world metres inside the road surface compiler, even while
  `TerrainSystem` still uses the current scaled storage internally.
- Add deterministic dirty tracking for:
  - touched edges
  - touched nodes
  - touched road surface chunks
  - touched terrain chunks
- Do not change the visible renderer yet.

Goal: create one explicit place in the runtime where the replacement can accumulate authoritative
surface state without immediately entangling it with the old renderer.

### Phase 2 - Compile edge sections and node patch inputs

- Implement edge-section compilation from the existing logical graph:
  - fixed section refinement
  - longitudinal grade solve
  - lateral-axis solve
  - lateral band layout
  - piecewise-linear cross-section profile
- Implement node classification into `Terminal`, `PassThrough`, `WidthTransition`, and `Junction`.
- Implement deterministic throat generation and ordered node-boundary loop assembly.
- Keep the old road mesh path active, but compile and cache the new surface data in parallel.
- Add Rust-side tests for:
  - grade determinism
  - section refinement determinism
  - node classification
  - throat ordering and stable boundary-loop generation

Goal: make the core compiler real and testable before any Godot-visible mesh changes depend on it.

### Phase 3 - Move preview onto the shared compiler

- Replace the current preview-only road conditioning path with temporary `RoadSurfaceSystem`
  compilation using the exact same:
  - grade solve
  - section refinement
  - lateral profile
  - node patch generation inputs
- Keep preview cache lifetime temporary, but do not allow any separate "looks similar enough"
  geometry path.
- Add preview-versus-commit parity tests for:
  - flat terrain
  - cross-slope terrain
  - bridge preview
  - tunnel preview
- Keep committed rendering on the old path for one phase longer so preview parity can be validated
  separately.

Goal: eliminate the first source of geometry drift by making preview an early consumer of the
authoritative compiler.

### Phase 4 - Replace standard-road top-surface rendering

- Replace the standard-road carriageway and sidewalk top-surface renderer with mesh generation from:
  - compiled edge sections
  - compiled lateral bands
  - compiled node patches
- Generate normals from the actual compiled geometry instead of forcing `Vector3::UP`.
- Keep the current Godot material contract where it does not block the new geometry ownership.
- Remove any old standard-road surface code that still:
  - offsets laterally from centerline Y only
  - infers junction center ownership from overdraw alone
  - depends on widened ribbons or node disks as the authoritative surface model
- Add black-box mesh tests for:
  - cross-slope roads
  - arbitrary-angle bends
  - T-junctions
  - 4-way junctions
  - width transitions
  - car-only roads without sidewalks

Goal: move visible surface-road ownership from the old renderer to the compiled roadbed for normal
surface roads.

### Phase 5 - Replace bridge, tunnel, and marking paths

- Move bridge deck top surfaces to the compiled roadbed path using the same section and band model.
- Move tunnel portal surface handling to the compiled roadbed path while still suppressing buried
  surface sections where required by the spec.
- Replace lane-marking generation so it derives from compiled throat / section data instead of
  centerline offsets.
- Ensure footpath joins use the same side-aware surface ownership model as the new standard-road
  path.
- Add tests for:
  - bridge deck continuity
  - tunnel portal ownership
  - marking termination at throats
  - footpath joins on one sidewalk side only

Goal: remove the remaining top-surface exceptions so one surface compiler owns all visible road
classes.

### Phase 6 - Replace terrain earthworks with roadbed-derived stamping

- Replace centerline-based terrain flattening with earthwork stamping from compiled sections and
  node patches.
- Stamp only touched terrain chunks rather than rebuilding full-world dense road flatten output for
  each local edit.
- Keep `TerrainSystem` authoritative for source and visual terrain storage; the road system only
  supplies derived earthwork inputs.
- For bridges:
  - limit earthworks to abutments and explicitly authored support regions
- For tunnels:
  - limit earthworks to portals
- Add tests for:
  - paved-footprint agreement between visual terrain and roadbed support surface
  - no under-span bridge flattening
  - no along-span tunnel carving
  - local invalidation bounded to touched terrain chunks

Goal: remove the second source of geometry disagreement by making terrain earthworks consume the
same compiled roadbed as the renderer.

### Phase 7 - Add combined world-surface queries and tool migration

- Add new combined world-surface queries that return:
  - roadbed height when roadbed owns the point
  - otherwise terrain height
- Update road placement, move, inspect, and selection tools to use the combined query where road
  ownership matters.
- Keep existing terrain-only queries as terrain-only APIs unless and until the owning docs are
  updated together.
- Update debug and editor helpers so they can visualize:
  - compiled sections
  - lateral bands
  - node patch loops
  - earthwork chunk bounds

Goal: make the editor and debug paths consume the same world-surface ownership model as the
runtime renderer.

### Phase 8 - Remove legacy paths and compatibility state

- Delete the remaining old standard-road surface renderer path.
- Delete the remaining centerline-only terrain flattening path.
- Delete dead contour / patch / widened-ribbon compatibility code and any dead cached junction
  polygon state left behind by the old renderer.
- Delete preview fallbacks that still bypass the compiled roadbed path.
- Rewrite or remove tests that assert old implementation shapes instead of the new black-box
  contracts.
- Update [`terrain.md`](terrain.md), [`reference.md`](reference.md), and any affected tool/API docs
  so the shipped runtime contract matches the new implementation.

Goal: leave one coherent road surface architecture in the repository instead of a permanent
dual-path system.

## Test Contract

Tests for the replacement must be black-box contract tests, not shape snapshots of one internal
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
- terrain earthwork agreement with the roadbed inside the paved footprint
- deterministic rebuild: same input graph produces the same section data and mesh indices
- local invalidation: editing one edge does not force unrelated chunks to rebuild

## Explicit Replacement Rule

The current active top-surface road path is not to be patched into the long-term solution.

The code to remove or retire from the active path includes:

- centerline-only lateral lift logic for road surface vertices
- terrain flattening that samples only the centerline height and then separately resyncs the road
- any renderer code that treats node disks or widened ribbons as the sole ownership model
- dead cached junction polygon state that no longer serves the replacement pipeline

The long-term system is:

- one logical graph
- one authoritative roadbed compiler
- one terrain earthwork contract derived from that roadbed
- one render cache derived from that same roadbed
