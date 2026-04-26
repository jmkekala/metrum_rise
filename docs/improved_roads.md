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
- `earthworks`: explicit structural cut / fill geometry or visual terrain adjustment for road
  classes that intentionally expose bridge, tunnel, retaining, or portal support
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

## Current Status

The roadbed rewrite has already made the important ownership split:

- the logical graph owns connectivity, IDs, lane semantics, and authored plan curves
- the visual road carrier owns deterministic `Span`, `Bend`, `Terminal`, and `JunctionN` pieces
- road pieces own explicit top-surface footprints for asphalt, curb / shoulder, and sidewalk
- Godot uploads cached buffers and must not rebuild road or terrain topology from graph guesses
- visible-world queries prefer road-owned top surfaces before terrain

The road-touched terrain path has moved to the Spade CDT hardcut.

Live behavior:

- grounded `Standard` roads send piece-owned footprint clip polygons into road-touched terrain
  patches
- Rust returns baked terrain patch mesh payloads for those road-touched patches
- terrain under the road-owned footprint is no longer intended to be a visible carrier
- Spade is now the runtime terrain-patch CDT backend under `simulation::terrain::cdt`
- the live road-touched terrain patch bridge feeds piece-owned road loops into the CDT module,
  emits accepted terrain faces, rejects road-footprint faces, and reports CDT counters through
  `--debug road-geometry`
- `Terminal`, `Bend`, and `JunctionN` visual node pieces now resolve asphalt and sidewalk
  ownership through `i_overlay` before Spade CDT triangulation; sector-built geometry is only a
  deterministic candidate source, not a final visual carrier or material classifier

Accepted ownership backend:

- `i_overlay` is the Rust-side polygon boolean / ownership backend for road-piece region cleanup
- Spade remains the Rust-side CDT backend for triangulating already-owned regions
- `robust` is intentionally out of scope for now; the road runtime should not add a standalone
  predicate crate unless `i_overlay` plus Spade leaves one measured, specific predicate gap

Not accepted as the final target:

- seam-strip emission from road outer-loop segments to nearby terrain
- conservative terrain-cell omission based on fully-owned cell triangles
- terrain density experiments as the main answer to road / terrain gaps
- any shader mask, water plane, zoning overlay, or background-color coincidence that hides missing
  topology

The old seam-strip / cell-triangle hybrid is retired from the live road-touched terrain path.

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
- render mesh vertices, preview mesh vertices, lane-marking anchors, stitched terrain seams, and
  structural earthwork stamps all derive from that same roadbed

The codebase must not maintain separate height-conditioning implementations for:

- preview
- committed road mesh
- terrain flattening
- road picking

### 1A. Terrain Under The Owned Footprint Is Road Support, Not A Visible Carrier

The terrain directly under a committed road-owned footprint is not allowed to behave as an
independent visible surface.

Deterministic rule:

- the owned footprint is the full committed top-surface width of the piece:
  - carriageway
  - curb / shoulder
  - sidewalk when present
- for every world position `(x, z)` inside that footprint, grounded `Standard` roads replace
  visual terrain with the authoritative solved road surface itself
- that support surface follows the road's solved profile, not world-horizontal terrain:
  - if the road has longitudinal grade, the support follows that grade
  - if the road has crown or crossfall, the support follows that crossfall
- inside the grounded-road footprint, that support surface replaces visual terrain locally instead
  of being blended against whatever source terrain previously existed there
- terrain under the owned footprint is therefore no longer an independent visible carrier; it must
  not remain the final visible owner of the road center or sidewalk band
- the terrain renderer must not emit terrain fragments below the owned footprint; height-matching
  alone is not sufficient because coplanar or near-coplanar terrain can still render through the
  road surface
- the seam outside the owned footprint must be explicit:
  - Rust generates the road-touched terrain mesh so its inner edge follows the road-owned outer
    boundary
  - terrain vertices created on that inner edge reuse the road-owned edge height; no ordinary
    closure strip, seam carpet, or shader mask is allowed for grounded `Standard` roads
  - structural / retaining variants may still render explicit engineered materials where the
    deterministic road class calls for exposed structure
- road-footprint suppression must be produced by omitted terrain topology, not by a shader mask or
  broad visual band-aid that leaves holes between road and terrain
- terrain patches intersecting the compiled road-owned footprint must not drop to a coarser mesh
  LOD that can reintroduce overlap after the road seam was already solved
- road-locked terrain patch selection must not use the wider earthwork envelope; otherwise one
  raised or lowered road can force unrelated far-field terrain into the clipped mesh path
- when authored terrain is coarser than the required close-up road-support fidelity, road-locked
  terrain render patches must use a denser baked mesh step so the rendered terrain follows the
  road-owned seam instead of cutting across it with coarse triangles
- grounded `Standard` roads apply that footprint-replacement rule along the whole grounded footprint
- `Bridge` edges do not create midspan terrain support; only abutment-owned grounded footprint
  regions may stamp terrain support below the road
- `Tunnel` edges do not create buried midspan terrain support; only visible portal-owned grounded
  footprint regions may stamp terrain support below the road

Forbidden outcomes:

- terrain visibly rising through asphalt or sidewalk inside the owned footprint
- terrain renderer LOD reintroducing visible overlap after the stitched road hole already removed
  terrain inside the owned footprint
- terrain forming a canyon or trench under a grounded road because outer tie-in faces were stamped
  into terrain as if they were the terrain carrier
- terrain under a road being flattened as one global world-horizontal plane instead of following
  the solved road profile

For grounded `Standard` roads, asphalt, shoulder / curb, and sidewalk geometry are the visible
ground inside the owned footprint. Explicit road-owned earthwork geometry remains part of the
deterministic ownership model for terrain integration, structural cases, seam tie-ins, and future
retaining variants, but ordinary grounded roads must not render that carrier as a separate visible
mesh layer below asphalt or sidewalk. Ordinary grounded roads must instead receive a Rust-generated
terrain patch mesh whose hole boundary exactly matches the road-owned footprint edge, so the road /
terrain border has no terrain below it and no visible strip beside it.

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
- the `Bend` piece emits candidate asphalt, sidewalk / curb, and outer-footprint polygons from its
  two incident mouths
- `i_overlay` resolves those candidate polygons into disjoint ownership regions before any bend
  triangles are emitted
- Spade triangulates the resolved bend regions and preserves their constrained boundaries
- a sharp bend may shrink, split, or locally remove sidewalk area where there is no valid room, but
  sidewalk must never overlap carriageway asphalt
- the `Bend` outer footprint is the boolean-union result of the resolved road-owned regions, not a
  second-pass extraction from already-emitted triangles

### 9. `Terminal` And `JunctionN` Own Their Geometry Explicitly

`Terminal` and `JunctionN` must remain explicit visual pieces with deterministic ownership.

Required rule:

- `Terminal` owns its cap / closure as a one-mouth road-piece region problem
- `JunctionN` owns its surface as an `n >= 3` road-piece region problem
- `Terminal`, `Bend`, and `JunctionN` must use the same region pipeline:
  - compile mouth-derived candidate polygons
  - resolve ownership with `i_overlay`
  - triangulate resolved regions with Spade CDT
  - export the resolved outer footprint to terrain seam generation
- `JunctionN` must not be built from one generic angle-sorted cloud of throat endpoints as final
  geometry
- `JunctionN` must not be rendered as one global sidewalk annulus between one outer loop and one
  inner loop
- `JunctionN` must not let adjacent-mouth sector strips decide final visible ownership after CDT
- `JunctionN` must not rely on nearest-material classification, render order, or centroid voting to
  decide whether a triangle is asphalt or sidewalk
- `JunctionN` may use sorted adjacent mouths only to generate deterministic candidate polygons and
  candidate split lines before boolean ownership resolution
- no sidewalk triangles may own the carriageway center
- no sidewalk triangle may overlap any asphalt triangle, including acute-angle corners
- no carriageway seam may appear between an incident `Span` throat and the node-owned continuation

### 10. Mouths And Boolean Regions Are The General Connector Model

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

- `Terminal` receives exactly one mouth profile
- `Bend` receives exactly two mouth profiles
- `JunctionN` receives three or more mouth profiles
- `JunctionN` sorts mouths by centerline angle around the node with a stable tie-breaker only to
  build canonical adjacent-mouth candidate regions
- each candidate region is assembled band-by-band from paired mouth boundaries, then passed to
  `i_overlay`; candidate regions are not final render geometry
- connector sampling must use a fixed deterministic step no larger than `1 m`
- the runtime must not generate the full node from a single generic fill polygon and hope later
  triangulation recovers the intended ownership
- the runtime must not keep overlapping road / sidewalk candidate polygons after the boolean
  ownership stage

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

## Spade CDT Terrain-Patch Hardcut

The next accepted representation is a Spade-backed terrain patch generator. It replaces the current
road-touched terrain mesh builder; it does not sit beside it as a fallback.

### Target Backends

Required rules:

- `i_overlay` is the production polygon ownership backend for boolean cleanup before triangulation
- `spade::ConstrainedDelaunayTriangulation` is the only planned CDT backend for the production
  terrain-road seam
- `i_overlay` owns polygon-region operations such as union, intersection, difference, holes,
  self-overlap cleanup, and asphalt / sidewalk / terrain ownership subtraction
- Spade owns constrained triangulation only; it must not be asked to infer material ownership from
  overlapping hint polygons
- `try_bulk_load_cdt` is the accepted insertion path for live code, so malformed or overlapping
  constraints are counted and skipped instead of panicking the simulation thread
- Spade refinement helpers are not used until this project has a pinned deterministic refinement
  contract
- `robust` is not a planned dependency in this path; if a future implementation needs a standalone
  exact-predicate crate, that requires a narrow, measured spec update naming the missing predicate
- any future replacement of `i_overlay` or Spade requires a new explicit benchmarked spec change

### Target Owner And API

The CDT implementation belongs in Rust simulation code, not in Godot and not in the giant Godot
bridge layer.

The target API is one deterministic terrain-patch function:

- input:
  - terrain patch world rectangle
  - source-terrain sample vertices for that patch
  - piece-owned road footprint loops intersecting that patch
  - road seam heights carried by those footprint-loop vertices
- output:
  - baked terrain vertices / normals / UVs for the patch
  - accepted face count
  - rejected road-footprint face count
  - constraint count
  - invalid / skipped constraint count
  - hard error details when triangulation itself fails

The API must not expose:

- road graph internals
- Godot mesh types
- shader-mask assumptions
- water or zoning overlay state

### CDT Input Rules

The CDT input is canonical and deterministic. Before CDT input is built, road and terrain ownership
polygons are resolved with `i_overlay`:

- asphalt polygons are unioned into one non-overlapping asphalt region per affected piece / patch
- sidewalk candidate polygons are subtracted by asphalt before they can become visible sidewalk
- terrain patch polygons are subtracted by the full road-owned footprint before terrain faces can
  be emitted
- overlap cleanup must happen in Rust before Spade sees constraints; CDT is not a replacement for
  polygon boolean ownership
- all boolean-operation inputs are quantized, ordered, and keyed by stable piece IDs before being
  passed to `i_overlay`
- all boolean-operation outputs are canonicalized before CDT:
  - rings use the project epsilon and contain no duplicate consecutive vertices
  - outer rings and holes use deterministic winding
  - rings are sorted by stable owner, area, centroid, then vertex order
  - degenerate rings below the minimum area threshold are rejected with debug counters

After region ownership is resolved, CDT input obeys these rules:

- the patch rectangle is inserted as the outer constrained boundary
- every grounded road-owned outer footprint loop is clipped to the patch before triangulation
- footprint segments crossing the patch boundary are split at the boundary intersection point
- all constraint segments are deduplicated after quantization to the project epsilon
- constraint segments may meet at shared endpoints only; they must not cross through each other
- source-terrain sample points are inserted only if they are outside all road-owned footprints
- road seam vertices are inserted exactly from the compiled road piece and use road-owned seam
  height
- source-terrain vertices use source / visual terrain height according to the terrain patch
  contract in [`terrain.md`](terrain.md)
- input vertex and constraint order must be canonical:
  - patch corners first
  - road loops sorted by stable piece ID and local loop index
  - loop vertices in the compiled piece order after clipping / splitting
  - source-terrain sample vertices sorted by patch-local grid coordinate

No camera position, render LOD choice, debug flag, or thread scheduling order may affect CDT input.

### CDT Output Rules

After `try_bulk_load_cdt`, terrain faces are classified in Rust:

- a face is accepted if its centroid is inside the patch domain and outside every road-owned
  footprint
- a face is rejected if its centroid is inside any road-owned footprint
- accepted faces must preserve road seam constraint edges as terrain boundary edges
- accepted faces must not cross a road-owned footprint loop
- accepted faces must not expose world background between the road top surface and the terrain
  patch
- rejected faces are not emitted, not hidden by shader discard, and not replaced by water or zoning
  overlays
- normals and UVs are derived from the emitted accepted triangles
- conflicting constraints are reported through `terrain_cdt_invalid_constraints` and
  `terrain_cdt_status=conflicted`; this is a geometry bug signal, but it must not panic Godot,
  poison the simulation mutex, or re-enable an older clipping path

CDT failures are hard failures in debug output. The implementation must not fall back to:

- seam strips
- conservative cell-triangle omission
- old subtractive terrain clipping
- ordinary earthwork closure strips
- water planes
- terrain shader masks
- Godot-side polygon clipping

### Code Replacement Target

The Spade hardcut replaces the current road-touched terrain builder in
`rust/src/nodes/simulation_node.rs`.

The removed live-path concepts are:

- `emit_constrained_terrain_seams`
- terrain-cell binning used only for conservative road ownership omission
- fully-owned-cell triangle suppression as the ordinary road seam mechanism
- ear-clipped road footprint diagnostics as a substitute for constrained triangulation

The Godot terrain renderer remains a thin upload bridge. It continues to choose between a normal
rectangular terrain patch mesh and a Rust-baked road-touched terrain patch mesh, but it must not
clip road holes itself.

### Implementation Sequence

Implement the hardcut in this order:

1. Done: move `spade` from `dev-dependencies` to runtime `dependencies`.
2. Done: add a small Rust terrain CDT module with no Godot dependencies.
3. Done: port the current Spade spike into module-local tests.
4. Done: add constrained patch-boundary clipping for road footprint loops.
5. Done: add tests for roads crossing one patch edge, two patch edges, and a patch corner.
6. Done: add tests for multiple footprint loops in one patch.
7. Done: add tests for `Bend`, `Terminal`, and `JunctionN` footprint loops.
8. Done: replace the live road-touched terrain patch builder with the CDT module.
9. Done: remove the retired seam-strip / cell-triangle live path.
10. Done: update `--debug road-geometry` output to report CDT constraints, accepted faces, rejected
    faces, invalid constraints, and preserved seam-edge counts.
11. Done: make live terrain CDT use `try_bulk_load_cdt` so malformed constraints are debug-counted
    instead of panicking the backend.
12. Done: hard-cut `Terminal`, `Bend`, and `JunctionN` visual fills to local Spade CDT
    triangulation fed by resolved road-piece regions.
13. Done: replace node-piece material hints with `i_overlay` boolean ownership for `Terminal`,
    `Bend`, and `JunctionN`.
14. Done: remove hint-only / nearest-material / centroid-vote node material fallbacks after
    boolean ownership emits disjoint asphalt and sidewalk regions.
15. Later: adapt `Span` output to the same resolved-region contract once node pieces are validated.

Acceptance requires `cargo check`, the CDT contract tests, and Godot headless load to pass.

### Road-Piece CDT Rules

Road-piece CDT is now part of the accepted visual carrier for node pieces:

- `Span`, `Bend`, `Terminal`, and `JunctionN` still own semantic road shape
- `Terminal`, `Bend`, and `JunctionN` generate deterministic asphalt and sidewalk candidate
  polygons from incident mouth profiles, then resolve those candidates with `i_overlay`
- asphalt owns carriageway first; sidewalk is computed as sidewalk-candidate area minus asphalt;
  terrain is computed as patch area minus the full road-owned footprint
- CDT only triangulates already-decided polygons and material boundaries
- CDT must not decide lane, sidewalk, crosswalk, mouth, or frontage semantics
- invalid Spade constraints are counted as geometry bugs; the live node-piece path must not fall
  back to nearest-material classification, centroid voting, render order, or legacy sector strips

### Road-Piece Boolean Ownership Rules

The current node-piece hardcut is `i_overlay` plus Spade:

- `Terminal`, `Bend`, and `JunctionN` are compiled through one shared road-piece region builder
  where the node class only changes the number and shape of incident mouths
- the builder emits candidate polygons for asphalt, sidewalk / curb, crosswalk decals, and the
  outer footprint
- `i_overlay` resolves those candidates into disjoint ownership regions before any triangulation:
  - asphalt is the highest-priority visible road region
  - sidewalk / curb may shrink, taper, split, or disappear locally when a sharp angle leaves no
    valid area, but it must never overlap asphalt
  - crosswalks are decals anchored to asphalt / sidewalk edges and must not create topology holes
  - terrain starts outside the outer footprint only
- Spade triangulates each resolved region and preserves the constrained boundaries produced by
  `i_overlay`
- region ownership must be deterministic for arbitrary `n >= 1`, including terminals, 2-arm bends,
  oblique T-junctions, 4-way junctions, and sharp-angle multi-arm junctions
- Voronoi or angle-bisector logic may be used only as a deterministic candidate-line generator for
  splitting sidewalk regions before boolean cleanup; it must not replace `i_overlay` ownership
  subtraction
- no legacy sector strip, hint-only classifier, nearest-material fallback, or render-order trick may
  decide final asphalt / sidewalk ownership after this hardcut

### Junction-First `i_overlay` Refactor

The implemented hardcut focuses on node pieces first because they are the historically broken
case. `Span` pieces keep their current corridor generation until the node-piece ownership pipeline is
validated, then spans are adapted to emit the same resolved-region format.

Scope:

- first target: `Terminal`, `Bend`, and `JunctionN`
- deferred target: `Span` integration into the same region-output contract
- out of scope for this slice: lane-routing policy, pedestrian legality, building frontage rules,
  and retaining-wall variants

Required node-piece input:

- stable piece ID
- node class: `Terminal`, `Bend`, or `JunctionN`
- node world position
- incident mouths sorted by deterministic class rule:
  - terminal: the only mouth
  - bend: the two mouths ordered by edge ID after geometric normalization
  - junction: counter-clockwise centerline angle, then edge ID, then side index
- for each mouth:
  - centerline direction away from the node
  - throat position and throat width profile
  - asphalt left / right edge points
  - curb / shoulder left / right edge points
  - outer sidewalk / shoulder left / right edge points
  - road class and profile flags
- project epsilon and minimum-area thresholds

Candidate-region generation:

- asphalt candidates are built from mouth carriageway edges plus deterministic adjacent-mouth
  connector lines
- sidewalk candidates are built per side from curb / shoulder edges to the outer sidewalk / shoulder
  edge
- outer-footprint candidates are built from the full road-owned top-surface boundary
- candidate lines may use angle bisectors or local Voronoi-style split lines only to propose
  deterministic boundaries between adjacent sidewalk candidates
- candidate generation must not emit final render triangles
- candidate generation must not discard an acute-angle sidewalk because it looks visually awkward;
  only boolean ownership and minimum-area cleanup may remove it

Boolean ownership order:

1. Quantize and canonicalize all candidate rings.
2. `i_overlay` unions all asphalt candidates into one or more disjoint asphalt regions.
3. `i_overlay` unions all sidewalk / curb candidates into sidewalk candidate regions.
4. `i_overlay` subtracts asphalt regions from sidewalk candidates.
5. `i_overlay` unions asphalt plus final sidewalk / curb regions into the road-owned outer
   footprint.
6. Degenerate output rings below the minimum area threshold are dropped and counted.
7. The resulting disjoint material regions are triangulated with Spade CDT.

Acute-angle rule:

- asphalt has priority over every sidewalk / curb candidate
- sidewalk / curb regions may shrink, split into multiple islands, or collapse to nothing at acute
  angles
- a collapsed sidewalk is valid only when the remaining boolean area is below the documented
  minimum area threshold
- no final sidewalk / curb triangle may overlap final asphalt
- no final sidewalk / curb triangle may cross the asphalt boundary to reach another sidewalk island
- no final asphalt triangle may be omitted merely to preserve sidewalk continuity
- all acute-angle behavior must be symmetric under reversed road direction and stable mouth order

Output contract:

- the region builder returns disjoint material regions, not render strips:
  - asphalt regions
  - sidewalk / curb regions
  - optional decal anchor regions
  - one road-owned outer footprint
- every output region includes canonical rings, holes, area, material, and source piece ID
- Spade CDT emits triangles only from resolved regions
- the renderer consumes Spade triangles grouped by material
- terrain patch generation consumes the same resolved outer footprint, not a re-extracted render
  boundary
- visible-world surface queries consume the same resolved triangles as the renderer

Debug contract:

- `--debug road-geometry` must report, per node piece:
  - node class
  - incident mouth count
  - candidate asphalt / sidewalk / footprint ring counts
  - `i_overlay` output region counts by material
  - dropped degenerate ring count
  - final Spade constraint count
  - skipped / invalid Spade constraint count
  - final triangle count by material
- debug output must make acute-angle collapses visible as explicit dropped or split sidewalk
  regions, not as silent missing geometry

Legacy removal target:

- removed: hint-only node material classification once boolean regions own material output
- removed: nearest-material and centroid-vote fallbacks from node-piece material ownership
- removed: sector strips as final visual carriers for `Terminal`, `Bend`, and `JunctionN`
- removed: any `JunctionN` code that synthesizes a center asphalt core after sector assembly
- removed: any `Bend` or `JunctionN` path that depends on second-pass outer-boundary extraction from
  already-emitted render triangles
- keep mouth/profile calculation, throat clipping, grade/crossfall sampling, chunk invalidation,
  and Spade CDT triangulation because those remain part of the target pipeline

Acceptance tests for the junction-first hardcut:

- terminal with sidewalks emits asphalt, sidewalk / curb, and one outer footprint without overlap
- 2-arm bend at 30, 60, 90, 120, and 150 degrees emits no sidewalk-over-asphalt overlap
- triangle network emits three independent bends with closed outer footprints
- T-junction at 30, 60, 90, 120, and 150 degrees emits no sidewalk-over-asphalt overlap
- 4-way junction with exact 90-degree angles emits symmetric asphalt and sidewalk ownership
- 4-way junction with one acute arm emits split or collapsed sidewalks only where boolean area
  requires it
- `N > 4` junction emits deterministic ownership for stable mouth ordering
- reversed edge direction and equivalent edit order produce the same canonical regions
- terrain seam footprint equals the node-piece resolved outer footprint exactly
- `--debug road-geometry` reports the same region counts for repeated builds of the same save

### Later Extensions

Retaining walls, richer structural tie-in materials, and building-pad engineered-ground clients are
later extensions of the shared earthworks system in [`earthworks.md`](earthworks.md). They do not
block the Spade terrain-patch hardcut.

## Legacy Retirement Rules

The hardcut does not retire the whole roadbed runtime. It retires the remaining terrain-seam and
generic-ownership patterns that conflict with the piece/profile model.

Still valid and must be extended:

- logical graph ownership in `rust/src/simulation/network/mod.rs` and graph modules
- solved edge grades, section sampling, and lateral band definitions
- explicit `Span`, `Bend`, `Terminal`, and `JunctionN` visual pieces
- chunk-local dirtying and rebuild boundaries
- preview / commit parity
- Godot mesh upload as a thin presentation bridge

Retired patterns that must not return:

- centerline-only road lifting or terrain flattening
- generic node patches as final visual authority
- annulus-style sidewalk ownership around one global node center
- second-pass outer-boundary extraction from already-emitted polygons as the ownership source
- generic junction fill polygons as the source of bend or multi-arm node semantics
- road-touched terrain generated from seam strips plus conservative terrain-cell omission
- Godot-side road-hole polygon clipping
- shader, water, zoning, or background-color masking as a road / terrain seam carrier
- a runtime fallback from Spade CDT failure into any older terrain clipping path

## Test Contract

Tests for the roadbed runtime must be black-box contract tests, not shape snapshots of one internal
implementation.

Must cover:

- straight grounded road on flat terrain
- straight grounded road on strong cross-slope
- arbitrary-angle bend with sidewalks
- obtuse bend with sidewalks
- shallow-angle bend with sidewalks
- acute 2-arm bend with sidewalks where sidewalk may split or collapse but must not overlap asphalt
- triangle network composed of three independent bends
- pass-through split with no center bubble
- width transition on a nearly straight corridor
- T-junction center owned by carriageway
- acute T-junction where sidewalk may split or collapse but must not overlap asphalt
- 4-way junction center owned by carriageway
- 4-way junction with one acute arm and deterministic sidewalk ownership
- `N > 4` multi-arm node center owned by carriageway
- `N > 4` multi-arm node with acute neighboring arms and no sidewalk-over-asphalt overlap
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
- Spade CDT terrain patch with a straight diagonal road footprint fully inside one patch
- Spade CDT terrain patch with a road footprint crossing one patch edge
- Spade CDT terrain patch with a road footprint crossing two patch edges
- Spade CDT terrain patch with a road footprint crossing a patch corner
- Spade CDT terrain patch with multiple road footprint loops in the same patch
- Spade CDT terrain patch with `Bend`, `Terminal`, and `JunctionN` footprint loops
- Spade CDT accepted faces preserve every road seam constraint edge
- Spade CDT rejected faces cover road-owned footprints without emitting terrain inside them
- road-touched terrain debug counters report constraints, accepted faces, rejected faces, and hard
  constraint failures
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
