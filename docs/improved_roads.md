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
- `node grade carrier`: the deterministic node-local ownership model that carries each rendered
  node top-surface vertex from an explicit band owner, seam constraint, and height surface
- `canonical node arrangement`: the final node-local 2D ownership graph built before
  triangulation; it contains the exact asphalt, curb / shoulder, sidewalk, and footprint seam
  vertices that the renderer, terrain clipper, and debug output will consume
- `canonical node-owned mesh`: the final node-local top-surface graph for `Terminal`, `Bend`, and
  `JunctionN`; render polygons, terrain clip loops, query footprints, and local earthwork / skirt
  roots are all derived from this one graph
- `geometry backend`: a reviewed Rust crate or existing project subsystem that owns a low-level
  geometry operation such as offsetting, boolean clipping, triangulation, spatial lookup, spline
  evaluation, or shape validation
- `seam constraint`: a required node-local edge such as road / curb, curb / sidewalk, sidewalk
  outer edge, span / node handoff, or final road / terrain footprint boundary
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

This section is the implementation-status source of truth. The lower sections define the target
contract and implementation plans; when a lower section describes a stricter requirement, this
section says whether the runtime has reached it yet.

### Implemented Baseline

- the logical graph owns connectivity, IDs, lane semantics, and authored plan curves
- `RoadSurfaceSystem` owns deterministic `Span`, `Bend`, `Terminal`, and `JunctionN` visual pieces
- road pieces own explicit top-surface footprints for asphalt, curb / shoulder, and sidewalk
- Godot uploads cached buffers and must not rebuild road or terrain topology from graph guesses
- visible-world queries prefer road-owned top surfaces before terrain
- grounded `Standard` road footprints are consumed by the road-touched terrain path
- Spade is the runtime terrain-patch CDT backend under `simulation::terrain::cdt`
- road-touched terrain patches emit CDT terrain faces, reject road-footprint faces, and report
  constraint / face counters through `--debug road-geometry`
- terrain clip input uses an `i_overlay` union of grounded `Standard` span and node footprints
  intersecting the patch query, not raw overlapping per-piece loops
- the terrain CDT bridge deterministically nodes T-touching, crossing, and collinear-overlap
  roadbed constraints before Spade input; inserted cutter heights use the final road-owned
  top-envelope policy only after road ownership and provenance are solved
- road-geometry debug output includes compiled node-piece topology, mouth seam, outer-boundary, and
  earthwork-face / top-surface matching diagnostics for dumped edges
- asphalt, curb / shoulder, sidewalk, and raised-step faces are separate render payloads
- `Terminal`, `Bend`, and `JunctionN` route through the canonical
  input -> rail -> boolean ownership -> height -> Spade triangulation path
- render triangles, terrain clip loops, query support, earthwork roots, and chunk coverage are
  derived from the same compiled visual-piece ownership instead of from Godot-side guesses
- node earthwork faces carry final owner provenance; grounded `Standard` node boundary portions are
  terrain/CDT seams, while structural visibility remains class-owned
- terminal cap ownership is generated through `surface::terminal`; the retired input-side terminal
  end-band helper is not the runtime path
- post-overlay boundary snapping, seam welding, shared grade sampling, source-vector height
  plumbing, and broad hidden repair paths are no longer accepted runtime mechanisms
- `Span` output uses the resolved-region staging shape used by node output: role-tagged asphalt,
  curb / shoulder, and non-road regions feed render polygons, query support, terrain clip loops,
  earthwork roots, and chunk coverage
- surface and road-touched terrain chunk ownership is indexed by compiled visual-piece coverage;
  dirty rebuilds use `old_coverage union new_coverage` and sorted contributor sets

### Partially Implemented

- logged `Terminal` and 2-arm `Bend` regressions compile through canonical curb / sidewalk
  ownership, but broader `JunctionN` and arbitrary-node coverage is still incomplete
- `Bend` / `JunctionN` conflict throats use pairwise material-conflict distances and emit raw
  full-roadbed / carriageway corridor authority before per-band owner clipping. The generated flat
  Bend and 3-way `JunctionN` angle matrix now covers acute, right-angle, obtuse, and near-parallel
  cases, with representative reversed-edge-direction and equivalent-edit-order compile coverage.
  Generated elevated Bend and 3-way `JunctionN` variants now cover the same matrix; 4-way /
  arbitrary `N > 4` and exact raw-polygon identity checks remain open.
- post-boolean `node_non_road` subdivision requires explicit profile seam-rail evidence for final
  curb / shoulder and sidewalk ownership, but diagnostics for missing evidence are not yet complete
  enough to replace every downstream height failure
- source-authorized post-boolean support materialization is shipped for the current exact
  source-rail, owner-pair / opposite-owner, raised-step, noded constraint interpolation,
  source-edge endpoint dust, same-owner interpolation clusters, same-millimetre duplicate-source
  clusters, final-footprint raised-step boundary pairs, JunctionN curb / shoulder mouth-band
  edge contacts, same-material endpoint paths backed by explicit material-step provenance, and
  final owned-region seam endpoints backed by explicit source-rail interpolation
- a pre-height-evaluation height-field completeness gate now resolves every final owned-region
  vertex through its owner-scoped `NodeBandHeightFieldId` before heighted region construction; a
  missing final carrier reports structured missing-carrier diagnostics instead of leaking through
  the old unscoped height evaluator
- road-geometry diagnostic dumps now serialize stage, backend, owner, height-field, point / edge,
  residual, seam, and constraint metadata as queryable JSON fields instead of an opaque Rust debug
  blob; remaining work is filling any missing-artifact coverage gaps before extending the
  generated conflict matrix into 4-way and arbitrary-`N` cases
- road-touched terrain CDT reports widened near-road samples and retaining-wall classifications, but
  authored / extreme DEM coverage and any required closure variants remain open

### Open ROAD-01 Work

- finish missing-artifact diagnostic coverage for missing source rails, missing carrier support,
  rejected residuals, open boundaries, duplicate exposed edges, and non-explicit boundary vertices
- extend the conflict-first Bend / JunctionN test matrix from generated flat and elevated Bend /
  3-way cases to 4-way, arbitrary `N > 4`, and exact canonical raw-polygon identity checks
- complete terrain / road agreement tests for authored and extreme DEM cases, including retaining
  wall and widened tie-in combinations
- keep rejecting real ownership, seam, and carrier residuals; do not reintroduce miter patches,
  adjacent-mouth connector patches, nearest-height fallback, min/max repair, averaging, or
  render-order priority

### Implementation Plan Map

- terrain CDT status and rules live under
  [`Spade CDT Terrain-Patch Hardcut`](#spade-cdt-terrain-patch-hardcut)
- the node pipeline phase plan lives under
  [`Library-Backed Node Rework Plan`](#library-backed-node-rework-plan)
- Bend / JunctionN candidate ownership rules live under
  [`Conflict-First Node Candidate Hardcut`](#conflict-first-node-candidate-hardcut)
- black-box coverage requirements live under [`Test Contract`](#test-contract)

## Accepted Geometry Backends

- `i_overlay` is the Rust-side polygon boolean / ownership backend for road-piece region cleanup.
  It owns union, intersection, difference, hole handling, and overlap removal.
- Spade remains the Rust-side CDT backend for triangulating already-owned regions. It must receive
  canonical regions with explicit constraints, not overlapping material hints.
- `rstar` remains the spatial lookup backend for broadphase queries, dirty-region lookup, and
  indexed road / terrain ownership searches. Road code must not add another spatial index when the
  existing R-tree answers the query.
- `glam` is the preferred internal vector math backend for road geometry. Use `DVec2` / `DVec3`
  for canonical arrangement construction, intersections, offsets, grade evaluation, and validation
  where precision matters; use `Vec2` / `Vec3` only for clearly non-critical float math. Godot
  `Vector2` / `Vector3` are boundary types for GDExtension input, render upload, debug output, and
  save/load interoperability, not the long-lived internal geometry representation.
- `cavalier_contours` is the preferred backend for polyline offsetting, joins, caps, parallel
  curves, and contour cleanup in the node rework. Road code must not continue hand-rolling offset
  rails or corner joins once this backend is adopted.
- `splines` is the preferred backend for explicit longitudinal grade profiles and height
  evaluation at canonical vertices. It may evaluate heights only after ownership and seam vertices
  are known; it must not decide material ownership.
- Parry may be used for shape intersection, containment, distance, and validation diagnostics when
  those checks would otherwise be reimplemented locally. Use the 2D crate for XZ road arrangement
  validation unless a later 3D earthwork check explicitly needs the 3D crate.
- `lyon_geom` may be used for path math and deterministic curve flattening. `lyon_tessellation`
  may only be adopted if it replaces a specific tessellation responsibility with tests; it must not
  run beside Spade as a second triangulation truth for the same surface.
- A new hand-written computational geometry routine is accepted only when no existing project
  subsystem or reviewed Rust crate covers the operation. The spec or code comment must name the
  missing backend capability, the deterministic ordering rule, and the complexity bound.
- `robust` is intentionally out of scope for now; the road runtime should not add a standalone
  predicate crate unless the accepted geometry backends leave one measured, specific predicate gap.

## Not Accepted As Final Target

- seam-strip emission from road outer-loop segments to nearby terrain
- conservative terrain-cell omission based on fully-owned cell triangles
- terrain density experiments as the main answer to road / terrain gaps
- any shader mask, water plane, zoning overlay, or background-color coincidence that hides missing
  topology
- paired adjacent-mouth strip candidates as the authoritative node footprint or sidewalk source
- mitered cap / guard / connector patches as the primary ownership model for `Terminal`, `Bend`,
  or `JunctionN`
- additional sliver cleanup as the primary answer to malformed node candidates
- nearest-polygon, nearest-segment, terrain, asphalt, or full-roadbed fallback as a rendered node
  sidewalk / curb / shoulder height field
- using float vector equality as canonical topology identity. Canonical node arrangement identity
  must use explicit quantized keys / stable IDs; `glam` values are numeric working values only.
- carrying Godot vector types through core road-surface geometry after the data has crossed into
  the Rust simulation boundary, except as a temporary adapter while migrating existing code
- hand-written offsetting, polygon cleanup, point-in-polygon, intersection, or triangulation logic
  when an accepted backend already owns that operation
- any compatibility path that keeps the old post-overlay shared grade sampler as the source of
  rendered elevated `Bend` or `JunctionN` top-surface heights
- any second-pass outer-boundary reconstruction that creates terrain clip or mouth seam vertices
  outside the final canonical boolean footprint
- a world-flat junction slab as the answer to elevated or sloped junctions
- merging same-material elevated node domains before preserving their internal height seams
- convex hull, concave hull, or corner-rounding output as the authoritative sidewalk ownership
  source before asphalt / footprint ownership has already been resolved

The old seam-strip / cell-triangle hybrid is retired from the live road-touched terrain path.

## Roadbed Contract

The shipped roadbed runtime must guarantee:

- one authoritative roadbed model drives preview, committed render mesh, terrain earthworks, and
  world-surface picking
- lateral road width is part of the solved geometry, not a render-only offset from a 1-D centerline
- node surface ownership is explicit and robust for arbitrary angles, width transitions, dead ends,
  T-junctions, and 4-way intersections
- node top-surface height provenance is explicit and material-owned; every rendered node vertex has
  one deterministic grade-field owner
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
  - the carriageway has no crown, banking, or crossfall
  - sidewalks use the same longitudinal road height plus the explicit curb step
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
mesh layer below asphalt, curb, or sidewalk. Ordinary grounded roads must instead receive a Rust-generated
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

- lateral roadbed height comes from the section's centerline grade plus explicit band offsets, not
  from terrain samples under the left or right edge
- grounded `Standard` roads do not use drainage crown, banking, or terrain-derived crossfall;
  carriageway edges at the same station share the same road height

This is the key rule that removes the "half buried, half floating" failure mode.

### 5. Cross-Section Profile Is Piecewise Linear In Width

For any section, the surface profile across width must be reconstructible as a piecewise-linear
function of lateral offset.

That function must support at least:

- flat carriageway
- curb step between carriageway and sidewalk
- flat sidewalk plateau at the solved curb-top height

The exact authored defaults may be simple at first, but the representation must not collapse the
curb step into a single flat ribbon. Sidewalks are intentionally flat in the current city-building
profile; the road intentionally has no carriageway crossfall or crown.

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
- the `Bend` piece uses the same conflict-first corridor candidate model as every other node
  piece: conflict-bounded full-roadbed corridors define the bend footprint, conflict-bounded
  carriageway corridors define bend asphalt, and the remaining footprint becomes sidewalk / curb
  after boolean subtraction
- `i_overlay` resolves those candidates into disjoint ownership regions before any bend triangles
  are emitted
- Spade triangulates the resolved bend regions and preserves their constrained boundaries
- a sharp bend may shrink, split, or locally remove sidewalk area where there is no valid room, but
  sidewalk must never overlap carriageway asphalt
- the `Bend` outer footprint is the boolean-union result of the resolved road-owned regions, not a
  second-pass extraction from already-emitted triangles

### 9. `Terminal`, `Bend`, And `JunctionN` Own Their Geometry Explicitly

`Terminal`, `Bend`, and `JunctionN` must remain explicit visual pieces with deterministic
ownership.

Required rule:

- `Terminal` owns its dead-end closure as explicit band topology from one incident mouth
- `JunctionN` owns its surface as an `n >= 3` road-piece region problem
- `Bend` and `JunctionN` must use the hardcut band-owned region pipeline:
  - compile conflict-bounded mouth-derived footprint and asphalt candidates
  - extract required mouth, material, curb, sidewalk, and outer-footprint seam rails
  - resolve XZ ownership with `i_overlay` while preserving each region's band owner
  - build explicit height surfaces from the owning rails before triangulation
  - triangulate owned regions with Spade CDT using all required seam constraints
  - insert canonical final-outline vertices into the top mesh before terrain seam generation
- `Terminal` must export the canonical final outline of its explicit carriageway, side non-road,
  and end-band top-surface polygons to terrain seam generation only after those outline vertices
  are present in the rendered top mesh
- `JunctionN` must not be built from one generic angle-sorted cloud of throat endpoints as final
  geometry
- `JunctionN` must not be rendered as one global sidewalk annulus between one outer loop and one
  inner loop
- `JunctionN` must not let adjacent-mouth sector strips decide final visible ownership after CDT
- `JunctionN` must not rely on nearest-material classification, render order, or centroid voting to
  decide whether a triangle is asphalt or sidewalk
- `JunctionN` must not rely on a shared nearest-constraint grade field to decide the height of a
  rendered sidewalk, curb, or asphalt vertex after ownership has already been clipped
- `JunctionN` may use sorted mouths only for deterministic ordering, diagnostics, and optional
  crosswalk / side-label anchoring; sorted adjacent-mouth strips must not define the final
  footprint, asphalt, or sidewalk candidates
- no sidewalk triangles may own the carriageway center
- no sidewalk triangle may overlap any asphalt triangle, including acute-angle corners
- no carriageway seam may appear between an incident `Span` throat and the node-owned continuation

#### Node-Owned Canonical Topology Contract

`Terminal`, `Bend`, and `JunctionN` are the sole topological owners of their node-local surface.
Incident spans provide solved mouth constraints; they do not own or patch the node interior. This
rule is independent of edit order: a road that happened to exist first has no priority over a road
added later once both are incident to the same logical node.

Required ownership model:

- the stable logical node ID owns exactly one active canonical node topology at a time
- the active topology kind is derived from the current incident surface mouths:
  - one mouth becomes `Terminal`
  - two non-pass-through mouths become `Bend`
  - three or more mouths become `JunctionN`
- changing the incident mouth set replaces the node topology deterministically under the same node
  ID; it must not keep hidden geometry from the previous `Terminal`, `Bend`, or `JunctionN`
- spans export mouth profiles and source constraint IDs only:
  - boundary vertex XZ
  - solved height
  - band kind
  - side / profile role
  - edge ID and incident side
  - boundary / rail index
- the node arrangement inserts those mouth vertices as canonical constraints before boolean
  ownership, height evaluation, or CDT triangulation
- any node-local vertex that has the same quantized XZ as a mouth vertex must reuse the same
  canonical arrangement vertex when the material owner and solved height agree
- if two candidate sources produce the same quantized XZ but incompatible material ownership or
  height, the node compiler must report a deterministic geometry error; it must not choose the
  first-created road, average heights, min/max heights, nearest owner, or render-order priority
- every material seam is an explicit canonical arrangement edge, not an accidental adjacency
  between independently triangulated polygons
- asphalt / curb, curb / sidewalk, sidewalk outer edge, span handoff, and final footprint boundary
  edges must be represented in the same canonical graph before material triangles are exported

Deterministic construction order:

1. Collect every current incident surface mouth from the graph and sort by stable node-local angle,
   then edge ID, side, band index, and source constraint index.
2. Convert each mouth profile into canonical source constraints. These constraints are immutable
   inputs for the node solve; they are not rendered directly by the span once the node owns that
   area.
3. Build full-roadbed, carriageway, curb / shoulder, sidewalk, and footprint candidates from those
   canonical constraints.
4. Resolve material ownership with `i_overlay` into disjoint owned regions while preserving every
   source constraint ID that contributed to each exposed boundary.
5. Build one `NodeArrangement` containing all accepted owned-region rings, required material seams,
   span handoff edges, and footprint edges.
6. Assign exactly one material / band height field to every arrangement vertex and edge.
7. Triangulate from the arrangement constraints and label emitted faces by their owning
   `NodeOwnedRegion`.
8. Export road render triangles, sidewalk / curb render triangles, visible-surface query triangles,
   terrain clip loops, earthwork roots, and chunk coverage from that same arrangement.

Required reuse semantics:

- reuse means "same canonical arrangement vertex", not "copy coordinates from whichever road was
  older"
- spans and nodes share vertices only at explicit handoff constraints
- same-material internal seams may share vertices when their height field agrees exactly after
  quantization
- different-material seams share endpoints but keep explicit seam-edge ownership on both sides
- terrain, water, zoning overlays, and Godot renderers consume exported node topology; they never
  rebuild or simplify node ownership from raw road centerlines

Forbidden outcomes:

- placement order changes the node footprint, material regions, or rendered heights
- a previous terminal cap remains as hidden ownership after the node becomes a bend or junction
- two incident roads independently triangulate overlapping sidewalks or curbs inside the node
- a material gap is accepted because the union of all top triangles covers the footprint in XZ
- terrain CDT, renderer z-bias, or material ordering hides a missing node-owned seam
- debug output reports only a visual background gap without identifying the missing canonical
  vertex, seam edge, owned region, or height owner

### 10. Correct Non-Road Ownership Is Material-First

The shipped roadbed runtime does not treat sidewalks as a symmetric visual halo around the road
centerline, and it does not treat sidewalks as independent strips that are later hidden under
asphalt.

For `Bend` and `JunctionN`, correctness is defined by this ownership order:

```text
node_asphalt   = union(carriageway_corridors) intersect node_footprint
node_non_road  = node_footprint - node_asphalt
node_curb      = explicit curb/shoulder transition cells clipped to node_non_road
node_sidewalk  = node_non_road - node_curb
terrain        = terrain_patch - node_footprint
```

`node_non_road` is not a rendered material by itself. It must be split into band-owned curb /
shoulder and sidewalk regions before height sampling and triangulation. For bend and junction
pieces, those band-owned regions are the only visible non-road sources inside the node-owned
conflict region.

For `Terminal`, correctness is defined by explicit band topology instead of a rendered generic
remainder:

```text
terminal_asphalt  = carriageway bands from handoff to graph endpoint
terminal_sidewalk = side non-road bands to endpoint + U-shaped non-road end band
terrain           = terrain_patch - union(terminal top-surface polygons)
```

Required rules:

- asphalt owns first; non-road surfaces never reserve space that asphalt later has to dodge
- for `Bend` and `JunctionN`, sidewalks may shrink, split into islands, or collapse to nothing when
  asphalt leaves no legal area
- for `Terminal`, side and end non-road bands are emitted from solved endpoint bands and must not
  expand into asphalt or into a full-roadbed slab
- for `Bend` and `JunctionN`, a missing sidewalk is valid only when the resolved `node_sidewalk`
  area is below the documented minimum area threshold
- for `Terminal`, a missing sidewalk cap is valid only when the solved endpoint profile has no
  sidewalk / curb bands on either side
- no final sidewalk / curb / shoulder triangle may overlap final asphalt at any angle
- no final sidewalk / curb / shoulder triangle may cross an asphalt boundary to connect two
  sidewalk islands
- no final asphalt triangle may be omitted merely to preserve sidewalk continuity
- terrain is clipped by the same final road-owned top-surface footprint, not by a separate hull or
  render outline. For `Terminal`, `Bend`, and `JunctionN` this is the canonical boolean
  `node_footprint`.

This material-priority rule applies to every node class:

- `Terminal` is a one-mouth ownership problem
- `Bend` is a two-mouth ownership problem
- `JunctionN` is an `n >= 3` ownership problem

The node class changes the number of incident mouths. It does not change asphalt-first ownership.

#### Node Band Grade Carrier Hardcut

`Terminal`, `Bend`, and `JunctionN` geometry must be built from one deterministic node-local
arrangement before triangulation. Accepted geometry backends may own low-level operations such as
offsetting, polygon booleans, spatial lookup, spline evaluation, validation, and CDT
triangulation, but they must not erase the band owner, required seam rails, or height surface that
will grade each rendered vertex.

This is the long-term replacement for nearest-polygon, nearest-segment, full-roadbed, or
domain-index height sampling inside node pieces. It is a hardcut replacement, not a compatibility
shim around the current post-overlay grade sampler. A sloped or elevated junction is valid when
every material seam is constrained and every rendered vertex is owned by the correct band-local
height surface.

Implementation ownership:

- keep `node.rs` responsible for incident mouth collection, endpoint-profile collection, piece
  classification, and topology
- move offset rails, caps, contour joins, and node-outline construction to accepted geometry
  backends where they exist; project code adapts backend output into road-owned data structures
  instead of reimplementing those algorithms
- keep `overlay.rs` or its replacement responsible for wrapping `i_overlay` boolean ownership,
  owner-preserving clipping, and canonical ordering; it must not be the place that invents missing
  heights after ownership is already damaged
- keep height evaluation in an explicit height-carrier layer. Carrier triangles may evaluate known
  canonical vertices, while source handoff and contour-support vertices must arrive as explicit
  precomputed support heights. Height evaluation must not create vertices, look along source edges
  for missing support, choose material ownership, repair contradictory seams, or fall back to
  parametric band sampling
- source-band height carriers must arrive with both side rails already materialized to matching
  canonical path vertices; height evaluation must reject one-sided explicit paths instead of
  synthesizing an opposite chord from support points
- noded split vertices on an explicit carrier contour may inherit height from that carrier contour;
  arbitrary vertices outside the carrier must still fail loudly
- keep `Terminal` on explicit side-band / end-band topology, but record the same owned-region
  metadata so terminal, bend, and junction seam checks share one contract

Minimum private data model:

- `NodeBandHeightField`: one owner-preserving source surface for a node piece band, built from the
  visible throat profile and the graph-endpoint profile
- `NodeOwnedRegion`: one resolved material region with band kind, deterministic owner index,
  canonical rings, seam constraints, and final triangulated source-sampled polygon
- `NodeArrangementVertex`: one canonical XZ key, one material-owner context, one
  `NodeBandHeightFieldId`, and one solved height. A vertex may be shared by several final regions
  only when those regions agree on the same height field and solved height exactly after project
  quantization.
- `NodeArrangementEdge`: one explicit seam segment with owner(s), material transition kind, and
  source constraint IDs plus the height-field owner context for each side where height continuity
  is required
- numeric epsilon / quantization keys are representation tools only. They may deduplicate identical
  coordinates produced by a backend, but they must not average heights, choose the nearest owner,
  weld contradictory seams, or hide a missing band owner.

Inputs:

- stable node ID and visual piece kind
- stable sorted incident-mouth list used by the ownership solver
- each incident edge's solved section bands at the mouth / throat handoff
- final XZ material ownership polygons after asphalt, curb / shoulder, and sidewalk regions are
  resolved
- solved mouth rails for carriageway edges, curb inner edges, curb outer edges, sidewalk inner
  edges, sidewalk outer edges, footpath edges, and final road-owned footprint edges
- project overlay quantization, minimum-area threshold, and CDT triangle tolerance

Output:

- one height value for every rendered node top-surface vertex, produced by deterministic spline /
  ruled-surface evaluation from that vertex's owning band height field
- one material / band owner for every rendered node triangle
- one canonical outer footprint loop set extracted from the final boolean `node_footprint` before
  terrain seam generation, chunk coverage, and local earthwork / skirt roots
- optional debug records for missing constraints, dropped regions, seam height mismatches, and
  grade-limit violations

Core invariants:

- every span / node handoff vertex must match the incident span section height exactly
- every material seam must be represented by owner-preserving clipped region boundaries before CDT
  emits triangles
- no rendered node triangle may cross an asphalt / curb, curb / sidewalk, sidewalk outer, or
  span-handoff seam
- no material may sample from an unrelated material's height field
- no shared curb / shoulder vertex may be canonicalized to an averaged half-step height; flat
  road nodes may use only asphalt-height and sidewalk-height curb rail vertices
- no rendered node top-surface vertex may sample terrain, full-roadbed interpolation, or zero height
- a missing band-local grade owner is a geometry error and must be reported through
  `--debug road-geometry`; it must not silently fall back to a nearby unrelated domain
- every required mouth material-change vertex and outer-footprint vertex must have a matching final
  rendered top-surface vertex at the same overlay coordinate unless asphalt-priority ownership
  deliberately removes that non-road island
- every node outer-boundary vertex used by terrain clipping or local earthwork generation must come
  from the canonical boolean `node_footprint`. If that vertex is also part of a rendered owned
  region, it must be present in the heighted arrangement before CDT input is built. Vertices
  introduced by an unrelated height resampling pass, hull, terrain-clip reconstruction, or
  "already covered so sample it" export path are invalid.
- every node outer-boundary edge exported to terrain clipping or local earthwork / skirt generation
  must be an edge of the canonical boolean `node_footprint`, not a reconstructed edge-count walk
  over emitted render triangles. Render CDT edges may be split differently from the footprint as
  long as every emitted top-surface triangle centroid remains inside that footprint.
- a closed exterior edge cycle below the deterministic overlay numeric area budget may be discarded
  as backend dust. The budget is not one fixed global area: it is computed from the canonical
  boundary length, canonical vertex count, the 0.1 mm world-point deduplication width, and a hard
  10 cm^2 / 0.001 m^2 cap. A larger exterior cycle that cannot be polygonized is a geometry error;
  it must not be replaced by a hull, all-triangle fallback, or sampled terrain clip loop.

Material height fields:

- `Carriageway` uses only carriageway owner carriers
- `CurbOrShoulder` uses only curb / shoulder owner carriers; it must not derive a curb strip by
  sampling carriageway on one side and an arbitrary walkable domain on the other side after overlay
- `Sidewalk` uses only sidewalk owner carriers
- `Footpath` uses only footpath owner carriers
- later bands such as cycle track, parking, median, or tram reservation must add explicit material
  fields or documented mappings before they render inside nodes

Height provenance rules:

- `Carriageway` vertices use only carriageway carriers
- `CurbOrShoulder` vertices use only curb / shoulder carriers
- `Sidewalk` vertices use only sidewalk carriers
- `Footpath` vertices use only footpath carriers
- terrain clip / earthwork boundary vertices must be canonical footprint vertices with solved
  footprint heights. A sub-budget boundary-only vertex may interpolate along the ordered footprint
  contour between adjacent solved heights; it must not sample terrain or an unrelated material.
- constants such as curb step height or shoulder width may exist only in the section/profile
  solver; node builders consume solved profile output instead of duplicating those constants

Elevated-junction policy:

- a `Bend` or `JunctionN` may be sloped or warped; it must not be forced to one world-flat plate
- the node band grade carrier may interpolate between incident mouth constraints using node-local
  CDT barycentric interpolation, ruled band interpolation, or another deterministic interpolation
  with the same explicit seam constraints
- if the grade delta across a node exceeds the documented design limit, the compiler must choose a
  deterministic response:
  - extend the conflict handoff / transition length when there is room
  - subdivide the local band grade carrier while preserving seams
  - or reject / flag the geometry with a debug reason
- the compiler must not hide an excessive grade delta by lifting sidewalks, burying asphalt, adding
  a render-only skirt, or flattening the node without updating incident mouth constraints

Hardcut region solve sequence for `Terminal`, `Bend`, and `JunctionN`:

1. Build incident mouth rails, endpoint rails, and conflict handoff distances from solved profiles.
   Use accepted geometry backends for offset rails, caps, joins, and contour cleanup instead of
   local ad hoc line-intersection or corner-special-case code.
2. Build conflict-bounded full-roadbed and carriageway candidates from those canonical rails.
3. Use `i_overlay` to produce primary XZ ownership:

```text
node_footprint = union(full_roadbed_corridors)
node_asphalt   = union(carriageway_corridors) intersect node_footprint
node_non_road  = node_footprint - node_asphalt
```

4. Extract required seam rails from incident mouth profiles and primary XZ ownership: span
   handoff, asphalt boundary, asphalt / curb contact, curb / sidewalk contact, sidewalk outer edge,
   and final footprint boundary.
5. Split `node_non_road` into explicit `CurbOrShoulder` and `Sidewalk` owned regions. A residual
   non-road area without a deterministic band owner is a geometry error, not a fallback sidewalk.
6. Clip solved mouth and endpoint band carrier surfaces to their owned regions while preserving
   owner metadata: incident edge ordering, band kind, deterministic owner index, and
   `NodeBandHeightFieldId`.
7. Discard owned material regions whose area is inside the deterministic overlay numeric area
   budget before height evaluation and CDT, while still counting their shapes as claimed for
   residual accounting. These regions are backend dust, not renderable road or sidewalk polygons.
8. Reject unowned residuals. A full-roadbed closure carrier is not a rendered fallback for missing
   curb, sidewalk, or shoulder ownership.
9. Build the canonical node arrangement from every accepted region. Every seam vertex and outer
   footprint vertex is inserted before triangulation and receives one owner and one
   `NodeBandHeightFieldId`.
10. Triangulate owned regions with Spade CDT using the arrangement vertices and seam constraints as
   input.
11. Sample each emitted triangle vertex from that triangle's owning `NodeOwnedRegion`. Cross-region
   nearest sampling, terrain sampling, and full-footprint grade fallback are forbidden.
12. Reject or debug-count any triangle whose centroid or vertices cross a material seam after
   triangulation.
13. Verify final shared seam edges. All regions sharing a seam vertex must reference the same
    canonical arrangement vertex and solved height. Any cross-owner height disagreement after
    quantization is a geometry error; do not weld, average, min/max, choose by owner priority, or
    move either side.
14. Export the canonical outer footprint directly from the final boolean `node_footprint`. Open
    chains, unmatched endpoints, duplicate exposed edges, or vertices outside the canonical
    footprint are geometry errors.
15. Export those canonical footprint loops to terrain clipping. Terrain seam height and local
    earthwork / skirt roots come from solved boundary heights or deterministic contour
    interpolation for sub-budget boundary-only vertices, not from a terrain sample or raw render
    triangle walk.

Curb and sidewalk continuity:

- every resolved overlay boundary where asphalt touches walkable non-road must either be same-height
  or have a deterministic `CurbOrShoulder` transition region on the non-road side
- asphalt-adjacent transition strips are top-surface geometry, not vertical patch faces
- adjacent curb transition strips must share seam vertices at common asphalt-boundary points
- millimetre-scale closure slivers produced by the deterministic overlay grid are valid geometry
  and must remain renderable unless they are below the shared minimum area threshold
- broad `Sidewalk` regions may only claim area after `CurbOrShoulder` has claimed the explicit
  asphalt-adjacent transition
- a sidewalk may shrink, split, or collapse when asphalt leaves no valid area, but it must not cross
  asphalt to preserve visual continuity

Deterministic tie-breaks:

When multiple same-material constraints can own a sample, choose by:

```text
material ownership -> explicit seam containment -> distance to owning rail -> incident mouth angle
-> edge id -> side -> band index -> constraint index
```

This tie-break is only allowed inside the same material field. It must never choose between asphalt
and sidewalk, or between terrain and road, for a rendered node top-surface vertex.

Debug contract:

`--debug road-geometry` must include node band-grade diagnostics for every compiled `Bend` and
`JunctionN`:

- node ID and piece kind
- incident mouth count and mouth order
- final footprint / asphalt / curb / sidewalk / rejected residual area
- owned region count by material and band
- source carrier count by material and band owner
- top-surface height-owner ranges by material and band owner
- CDT triangle count by material
- emitted triangle count by material
- maximum span-handoff seam height error
- maximum mouth material-seam XZ error
- maximum outer-footprint top-boundary XZ error
- maximum cross-node road grade and sidewalk grade
- dropped / rejected triangle count and reason

Debug output for road logs must include enough compiled node-piece data to diagnose lifted
sidewalks without inferring them from edge section dumps alone.

Acceptance:

- a flat span connected to any `Terminal`, `Bend`, or `JunctionN` keeps identical seam heights for
  asphalt, curb / shoulder, sidewalk, and footpath rails
- an elevated 4-way `JunctionN` with different incident mouth heights keeps every sidewalk mouth
  touching its incident span and emits no background-visible gap at the throat
- node sidewalks may slope through the node, but may not lift away from their own span mouth or sag
  toward asphalt / terrain merely because another approach has a different height
- along a continuous sidewalk rail, height changes only because of the solved longitudinal grade or
  the band-local height surface constrained by that rail
- no emitted node triangle bridges two unrelated approach height planes unless a material seam and
  grade constraint explicitly allow that interpolation
- acute-angle and arbitrary `n`-way nodes still obey material ownership first: sidewalks never cross
  final asphalt to preserve height continuity
- the same graph built in a different edit order produces the same canonical node regions, grade
  constraints, and sampled heights
- `cargo test simulation::network::surface` includes regression cases for elevated `Bend`,
  elevated `JunctionN`, seam-height equality, same-material seam preservation, and deterministic
  repeated compile output

Non-goals:

- this hardcut does not add rounded corners, Spade refinement, or cosmetic sidewalk smoothing
- this hardcut does not change logical graph connectivity, route weights, or lane semantics
- this hardcut does not introduce a temporary shader, terrain mask, render-only height patch, or
  flat-junction compatibility mode

#### Terminal Sidewalk End-Band Contract

`Terminal` is not a miniature junction plaza. For grounded `Standard` roads with sidewalk / curb
bands, the visible terminal end is the same side-aware band stack used by the incident span, folded
around the stopped carriageway.

Required terminal inputs:

- the incident span mouth profile at the node handoff
- the solved endpoint profile at the graph endpoint
- the ordered physical bands from the endpoint profile
- the outward dead-end direction opposite the incident edge's inward direction
- the accepted geometry-backend configuration for offset / contour generation, boolean ownership,
  grade evaluation, validation, and CDT triangulation

Deterministic terminal topology:

1. The incident carriageway surface continues from the handoff to the graph endpoint and stops
   there.
2. The left non-road bands continue from the handoff to the graph endpoint using the left endpoint
   band rails.
3. The right non-road bands continue from the handoff to the graph endpoint using the right endpoint
   band rails.
4. For each non-road band kind that exists on both sides of the endpoint profile, emit one end-band
   strip across the exposed road end:
   - the inner rail is the endpoint-side rail nearest the carriageway / previous inner band
   - the outer rail is offset outward by that band's solved physical depth
   - the strip spans laterally from the left band rail to the right band rail
   - the strip reuses the same solved band heights as the side bands
5. Adjacent end-band strips share exact vertices on their common rail. The final sidewalk / curb
   end must therefore be one continuous topological band from left side, around the end, to right
   side.
6. The end-band corner joins are deterministic quadrilateral or triangular closures between the
   side-band rails and the end-band rails. They may be split for CDT constraints, but they must
   reuse the same rail vertices and band heights.
7. Terrain clipping receives the union of the final terminal top-surface polygons only:
   carriageway-to-endpoint, side non-road bands, and end non-road bands.

Forbidden terminal outputs:

- a full-roadbed-depth or half-roadbed-depth sidewalk slab beyond the endpoint
- a generic `footprint - asphalt` polygon as the rendered terminal sidewalk when it erases physical
  curb / sidewalk band boundaries
- render-only wall, skirt, closure, end-cover, or background-colored polygons for grounded `Standard`
  terminal closure
- vertical terminal closure faces inside the ordinary grounded-road surface mesh
- terrain, earthwork, or water polygons drawn under the terminal top surface
- independent terminal end polygons whose vertices do not match the side sidewalk / curb rails

Terminal-cap acceptance:

The legacy terminal end-band helper is deleted. Terminal cap ownership is generated by the
canonical `surface::terminal` rail / contour adapter and must satisfy these rules:

- in top view, the sidewalk / curb forms a U-shaped band with the same width on the left side, road
  end, and right side
- the terminal cap band has no visible seam, gap, or z-fighting where it meets either side sidewalk
- the terminal cap depth equals the sum of solved non-road bands on one side of the road, not
  the roadbed half-width and not a fixed visual padding value
- asphalt never extends beyond the graph endpoint on ordinary dead ends
- no visible terminal triangle is below the terrain surface or hidden by the road-touched terrain
  mesh
- car-only terminals with no sidewalk / curb bands emit no sidewalk cap and still keep a valid
  asphalt terminal surface

#### Bend Corridor And Side-Join Fill

A two-mouth `Bend` is an explicit two-arm ownership problem. It is not a single center sector,
single self-crossing stroke, hull, or bubble. The bend owns deterministic candidates built from the
actual clipped throat boundary segments, plus local side joins around the graph node:

- order the two mouths by the smaller angular turn from start mouth to end mouth
- build one full-roadbed mouth corridor from each throat segment to its node-side segment as the
  non-road ownership carrier
- build one carriageway mouth corridor from each carriageway throat segment to its node-side
  segment as the asphalt ownership carrier
- build non-road height fields from the solved curb / shoulder / sidewalk / footpath bands
- classify each throat segment into left and right endpoints relative to its local travel
  direction; start travel is throat-to-node, end travel is node-to-throat
- build local full-roadbed side joins and local carriageway side joins for ownership
- build local non-road band joins as explicit canonical arrangement edges with owner-preserving
  height fields
- side joins are generated as explicit backend-produced contour segments around the graph node using
  only the real side offset radius and owner-preserving height fields; they must not use the
  throat distance as radius
- final bend asphalt is the `i_overlay` union of carriageway corridors and carriageway side joins
- final bend sidewalk / curb / shoulder shape is `full_roadbed_candidates - carriageway_candidates`
  split by explicit band seam rails; its vertices use only their owning non-road band height field
- no single bend candidate may contain crossing throat caps; if two cap segments would cross, they
  must remain separate overlay candidates
- a bend with sidewalks must not expose terrain or world background between the two incident
  roadbeds
- the bend must not generate any sidewalk triangle on top of asphalt, even when the angle is acute
  or obtuse

This is intentionally narrower than using a hull. The bend does not decide arbitrary `JunctionN`
ownership; it only closes the local two-arm turn with the same material ordering as the rest of the
road surface system.

### 10A. Conflict Regions Replace Fixed Local Sidewalk Strips

The span / node boundary is not allowed to be chosen only from a fixed local radius when incident
road arms geometrically overlap before they reach the graph node.

For every incident mouth:

- derive one canonical side-aware mouth profile from the solved edge section
- compute `roadbed_half_width_m` from the full visible footprint side, including sidewalk or
  shoulder when present
- compute `carriageway_half_width_m` from the asphalt band only
- compute the local minimum handoff:

```text
local_handoff_m = roadbed_half_width_m + visual_handoff_padding_m
```

- compute pairwise conflict distances against every other incident mouth using the centerline angle
  `theta` around the node:

```text
roadbed_vs_roadbed_m = (roadbed_half_width_a + roadbed_half_width_b) / sin(theta)
roadbed_vs_asphalt_m = (roadbed_half_width_a + carriageway_half_width_b) / sin(theta)
asphalt_vs_roadbed_m = (carriageway_half_width_a + roadbed_half_width_b) / sin(theta)
```

- ignore pairwise distances only when `theta` is deterministically classified as pass-through for
  the same continuous edge corridor
- if `sin(theta)` is below the documented angular epsilon and the mouths are not pass-through, the
  independent-road geometry is unsolvable as separate visible corridors; the edit must be rejected,
  merged, or normalized instead of dividing by an unstable value or emitting overlapping sidewalks
- clamp every computed distance by the available edge segment before the next visual owner
- choose the mouth handoff as the maximum of the local minimum and every valid pairwise material
  conflict distance for that mouth

The pairwise formula is not an aesthetic miter. It is a conflict detector: if two shallow-angle arms
would cause a sidewalk or shoulder strip to cross another arm's asphalt before the local node, the
shared node-owned region must start there.

Required outcomes:

- shallow-angle roads create a longer shared ownership region when their roadbeds genuinely
  overlap before the graph node
- that growth follows the overlapping arms and must not become one unjustified central bubble
- if the required conflict region exceeds the available segment length, the edit cannot emit
  independent legal road surfaces; the runtime must reject, merge, or normalize the geometry rather
  than produce sidewalk-over-asphalt
- graph clips may still provide routing / lane metadata, but visual ownership is driven by material
  conflict distances and solved mouth profiles
- the same graph built in a different edit order must produce the same canonical conflict regions

Forbidden outcomes:

- independent span-owned sidewalk strips continuing into a region where they overlap node asphalt
- one fixed node radius that ignores shallow-angle pre-node conflicts
- one central fill polygon that leaves outer road-arm sidewalks unowned
- using a convex hull, concave hull, or rounded outline to decide material ownership
- using CDT as a material ownership solver
- special casing only `1 deg`, `15 deg`, `30 deg`, `60 deg`, `90 deg`, or any other named angle

General connector rule:

- every mouth emits conflict-bounded corridor candidates in node-local space:
  - a full-roadbed corridor from the mouth outer-left / outer-right boundaries through the conflict
    region
  - a carriageway corridor from the mouth asphalt-left / asphalt-right boundaries through the
    conflict region
  - optional profile sub-corridors for later curb / sidewalk material detail
- `JunctionN` sorts mouths by centerline angle around the node with a stable tie-breaker only for
  deterministic ordering, debug output, and side-aware marking anchors
- adjacent-mouth side strips are retired as candidate geometry; they are not allowed to define the
  node footprint or visible sidewalk ownership
- connector sampling must use a fixed deterministic step no larger than `1 m`
- the runtime must not generate the full node from a single generic fill polygon and hope later
  triangulation recovers the intended ownership
- the runtime must not keep overlapping road / sidewalk candidate polygons after the boolean
  ownership stage

Sidewalk ownership remains explicit per side before boolean resolution:

- left and right sidewalk bands are separate authored / derived bands at the mouth
- footpath connections attach to one side or the other, not to an abstract road-center sidewalk
- after boolean resolution, those side labels may map to split or collapsed regions
- crosswalk and frontage-side semantics must stay aligned with [`entrance_and_exit.md`](entrance_and_exit.md)
- pedestrian crosswalk mouths must use the same deterministic conflict-bounded mouth positions as
  the visible road surface; they must not invent a separate shallower node boundary

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

must be derived from the actual triangles or reconstructed cross-section planes so longitudinally
graded geometry shades correctly. Banking / crossfall is not part of the roadbed contract.

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
- grounded-road crossfall, crown, and banking are intentionally not generated; the shared tie-in
  back to terrain is owned by [`earthworks.md`](earthworks.md)

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

### 2A. Render Z-Bias Is Not Road Geometry

Required rule:

- road-owned top surfaces and vertical road faces must be emitted at their solved physical
  coordinates with no render-only Z-bias
- terrain under the footprint is topologically clipped, so adjacent road / sidewalk / terrain edges
  must stay watertight in render space instead of being separated by draw-order offsets
- decals such as lane markings and crosswalk stripes may use `ROAD_DECAL_RENDER_Z_BIAS_M` because
  they intentionally sit on top of already-owned road surfaces
- render Z-bias must not be used to make sidewalks physically higher than asphalt
- physical sidewalk elevation is owned by the compiled road profile:
  - curb step height
  - solved section / node-piece heights
- `Span`, `Bend`, `Terminal`, and `JunctionN` sidewalk triangles must therefore get their real
  height from the same profile-height reconstruction path before rendering
- if a node-piece sidewalk appears lower than a span sidewalk, the fix belongs in node-piece height
  reconstruction after `i_overlay`, not in a larger sidewalk render Z-bias

Forbidden outcome:

- increasing render Z-bias to hide a physical height mismatch between span and node sidewalks

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

## Piece-Owned Chunk Coverage Hardcut

Dirty road rebuilds must be driven by compiled visual pieces, not by graph centerline guesses.

This is the hard-cut path for surface, query, and road-touched terrain invalidation. It replaces
any runtime path that tries to decide dirty render chunks from only an edited edge centerline, an
edited node point, or a scan across all compiled node pieces.

Required ownership indices:

- every compiled `Span` stores its deterministic surface chunk coverage set
- every compiled `Terminal`, `Bend`, and `JunctionN` stores its deterministic surface chunk
  coverage set
- every grounded compiled piece that contributes road-touched terrain stores its deterministic
  terrain patch / chunk coverage set
- every surface chunk stores sorted contributing span IDs and node IDs
- every road-touched terrain chunk stores sorted contributing grounded-road footprint owners
- chunk coverage sets are canonicalized by stable chunk key and contain no duplicates

Coverage calculation rules:

- coverage is computed from compiled piece geometry, not from the logical graph alone
- surface coverage uses the compiled visible carrier bounds for asphalt, curb / shoulder, sidewalk,
  markings, and piece-local top-surface polygons
- terrain coverage uses the road-owned grounded footprint loops that will be sent into the terrain
  CDT path
- coverage is computed after `i_overlay` ownership cleanup and before chunk cache publication
- chunk keys are derived from the same road / terrain chunk grids already used by the renderer
- bounds-to-chunk conversion must include both lower and upper edge contact deterministically, so a
  piece whose boundary lies exactly on a chunk border has one canonical owner set
- coverage must not depend on camera position, render LOD, debug flags, thread scheduling, or
  previous cache contents

Dirty rebuild algorithm:

1. Convert the edit into dirty piece IDs: changed spans and every node piece whose incident span set
   or incident mouth profile can change.
2. Read old surface and terrain coverage for every dirty piece before recompiling it.
3. Remove every dirty piece from the reverse chunk ownership indices for its old coverage.
4. Recompile dirty pieces through the normal `Span`, `Terminal`, `Bend`, or `JunctionN` pipeline.
5. Resolve ownership with `i_overlay` and triangulate accepted regions with Spade CDT where the
   piece type requires CDT.
6. Compute new surface and terrain coverage from the newly compiled piece geometry.
7. Insert the dirty pieces into the reverse chunk ownership indices for the new coverage.
8. Rebuild exactly `old_coverage union new_coverage` for surface chunks, terrain chunks, and
   visible-world query caches.
9. Publish the new compiled pieces and chunk cache entries only after the affected chunk entries
   are internally consistent.

Required complexity bound:

- one local edit may touch only changed spans, incident node pieces, and `old_coverage union
  new_coverage`
- rebuilding a dirty surface chunk must use its sorted contributor lists; it must not scan every
  compiled node piece in the world
- rebuilding road-touched terrain chunks must use their sorted grounded-road footprint contributors;
  it must not rebuild unrelated terrain patches
- the steady-state cost is proportional to changed pieces plus contributors in touched chunks, not
  total road-network size

Determinism requirements:

- contributor IDs inside a chunk are sorted by stable piece kind, stable owner ID, then local piece
  order
- removing a piece that no longer compiles removes its old chunk ownership before any new cache is
  emitted
- changing a node from `Terminal` to `Bend` to `JunctionN`, or back again, is treated as replacing
  one node-owned visual piece with another under the same stable node ID
- failed piece compilation must not leave a chunk cache containing a mix of old and partially
  rebuilt topology
- tests must cover node expansion and shrinkage, including an arbitrary-angle junction changing
  from 3 incident roads to 4 or more incident roads

Not accepted:

- global scans over all compiled node pieces during ordinary dirty chunk rebuilds
- dirtying only the edge centerline AABB for a span whose sidewalks, markings, or terrain footprint
  extend beyond that centerline
- dirtying only the node point for a `JunctionN` whose resolved footprint spans multiple chunks
- retaining an old chunk entry until a later frame happens to notice the compiled piece changed
- falling back to whole-network rebuilds to hide stale cache ownership

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
- the road-owned footprint given to one terrain patch is first unioned across all grounded
  `Standard` span and node pieces that intersect the patch query, so shared throats and
  arbitrary-angle multiway nodes cannot send crossing constraints into Spade
- terrain CDT patch invalidation follows the piece-owned chunk coverage hardcut above; dirty edge
  centerlines and node points are not enough for arbitrary-angle junctions
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

### Terrain CDT Implementation Status

The terrain CDT hardcut is the accepted baseline. Its required properties are:

- Spade is a runtime dependency, not a test-only spike.
- The terrain CDT module has no Godot dependency.
- Road footprint loops are clipped to patch boundaries before CDT input is built.
- Road-touched terrain patches use CDT output instead of seam strips or conservative terrain-cell
  omission.
- CDT diagnostics report constraints, accepted faces, rejected faces, invalid constraints, and
  preserved seam-edge counts through road-geometry debug output.
- Live terrain CDT uses `try_bulk_load_cdt`; malformed constraints are debug-counted and must not
  panic the simulation thread or re-enable an older terrain path.

The node-piece implementation status is summarized in [`Current Status`](#current-status). Future
changes must extend the canonical arrangement contract rather than reintroduce transitional repair
helpers.

Acceptance for changes in this area requires `cargo check`, the relevant road-surface contract
tests, and Godot headless load to pass.

### Library-Backed Node Rework Plan

This is a clean-cut replacement of the current node surface compiler. The implementation must not
ship or merge with a temporary dual path where old node code remains reachable. The hardcut change
must route `Terminal`, `Bend`, and `JunctionN` through the new arrangement builder in the same
patch series that removes the retired helpers from runtime use.

Implementation phases:

1. Dependency and adapter boundary:
   - add direct dependencies only for accepted backends used by the implementation: `glam`,
     `cavalier_contours`, `splines`, `parry2d`, and optional `lyon_geom`
   - keep `i_overlay`, `rstar`, and Spade as existing accepted backends
   - add a small road-geometry adapter layer for converting between Godot vectors, `glam`
     vectors, overlay contours, Spade points, and backend contour types
   - keep Godot `Vector2` / `Vector3` out of new core geometry structs except at bridge and output
     conversion points

2. Canonical identity and data model:
   - introduce `NodeArrangement`, `NodeArrangementVertex`, `NodeArrangementEdge`, and
     `NodeOwnedRegion` records backed by quantized keys / stable IDs
   - represent material owner, band owner, seam source, and `NodeBandHeightFieldId` explicitly on
     every arrangement vertex / edge / face
   - reject float-vector equality as topology identity; `glam` values are working coordinates only

3. Input extraction:
   - keep the existing solved edge section/profile pipeline as the source of mouth data
   - extract mouth rails, endpoint rails, band intervals, conflict handoff distance, and solved
     boundary heights once per incident mouth
   - do not duplicate curb height, sidewalk width, shoulder width, or grade constants inside the
     node builder

4. Rail and contour generation:
   - use `cavalier_contours` for offsets, joins, caps, parallel curves, and contour cleanup
   - generate full-roadbed, carriageway, curb / shoulder, sidewalk, and footprint seam rails from
     backend-produced contours
   - represent every generated rail as an arrangement constraint before boolean ownership or CDT
     input is built

5. Boolean ownership:
   - use `i_overlay` for union / intersection / difference of candidate regions
   - produce `node_footprint`, `node_asphalt`, and `node_non_road`
   - split `node_non_road` into explicit curb / shoulder and sidewalk regions using profile seam
     rails; reject unowned residuals
   - preserve owner metadata through every clipping operation

6. Height fields:
   - build explicit height carriers from authorized source rails, endpoint profiles, generated
     contours, and terminal cap contours
   - consume paired, canonical source-band rail paths as input; upstream rail / topology ownership
     must materialize any required opposite rail before height evaluation
   - assign every accepted owned region, arrangement vertex, arrangement edge, and triangulated
     vertex to exactly one `NodeBandHeightFieldId`
   - evaluate heights only at already-known canonical arrangement vertices
   - treat source-rail and terminal-cap contour split vertices as explicit carrier points when the
     owned boundary was canonicalized to that carrier
   - reject vertices outside their explicit carrier; do not fall back to parametric band sampling
   - reject same-XZ height conflicts instead of welding, averaging, nearest sampling, owner-priority
     selection, or min/max selection

7. Triangulation:
   - feed only final owned regions and explicit seam constraints into Spade
   - require CDT output faces to stay inside one material owner
   - keep CDT out of material ownership decisions

8. Validation and debug:
   - use Parry / backend shape queries for containment, crossing, distance, and overlap validation
     when those checks are available from the backend
   - debug output must name the failing stage and backend: contour generation, boolean ownership,
     height evaluation, validation, or CDT triangulation
   - report rejected residuals, non-explicit boundary vertices, height conflicts, open boundaries,
     duplicate exposed edges, and invalid constraints as structured road-geometry diagnostics

9. Runtime integration:
   - route `build_terminal_visual_node_piece`, `build_bend_visual_node_piece`, and
     `build_junction_visual_node_piece` through the new arrangement builder
   - export render polygons, query triangles, terrain clip loops, earthwork roots, and chunk
     coverage from the same arrangement-owned mesh
   - keep span compilation and dirty-chunk indexing unless the new node output requires a narrow
     adapter

10. Deletion pass:
    - remove or make unreachable post-overlay height repair, boundary snapping, shared seam
      welding, sampled missing-boundary height export, full-roadbed closure fallback, and
      hand-written offset / join helpers
    - remove tests that only preserve the retired implementation shape; keep or rewrite black-box
      contract tests

Acceptance gates:

- `cargo check --manifest-path rust/Cargo.toml`
- focused `simulation::network::surface` tests for flat and elevated `Terminal`, `Bend`,
  T-junction, 4-way junction, and arbitrary `N > 4` junctions
- deterministic rebuild tests that compare canonical arrangement keys and emitted mesh indices
- terrain seam tests proving every exported footprint loop equals the resolved boolean footprint and
  every emitted top-surface centroid lies inside it
- road-geometry debug tests proving failures are reported before visual fallback can hide them
- Godot headless load with road-touched terrain enabled

### Road-Piece CDT Rules

Road-piece CDT is now part of the accepted visual carrier for node pieces:

- `Span`, `Bend`, `Terminal`, and `JunctionN` still own semantic road shape
- `Bend` and `JunctionN` must generate deterministic full-roadbed and carriageway corridor
  candidates from incident mouth profiles, then resolve those candidates with `i_overlay`
- `Terminal` must generate deterministic carriageway, side non-road, and end-band top-surface
  polygons directly from the incident mouth and endpoint band profiles
- asphalt owns carriageway first; for `Bend` and `JunctionN`, non-road bands are derived from
  `node_footprint - node_asphalt`; for `Terminal`, non-road ownership is the explicit side and end
  band topology. In all cases, curb / shoulder and sidewalk geometry must not own asphalt.
- terrain is computed as patch area minus the full road-owned footprint
- CDT only triangulates already-decided polygons and material boundaries
- CDT must not decide lane, sidewalk, crosswalk, mouth, or frontage semantics
- road-piece CDT face filtering must use deterministic overlay area, not empty-shape identity, for
  owner containment. A candidate triangle whose `triangle - owner` residual area is within the
  documented overlay numeric area budget may be accepted; the final union of accepted triangles
  must still pass the whole-region `owner - triangles` / `triangles - owner` coverage check using
  the same budget. The budget is `base + boundary_length * 0.1 mm + vertex_count * 1 mm^2`, capped
  at 10 cm^2 / 0.001 m^2, where `base` is the existing 16 mm^2 overlay floor. Residuals above that
  cap are real geometry errors and must still reject the node.
- road-piece validation must treat an exposed CDT edge as closed when it is either the exact
  explicit boundary constraint, a backend-verified subsegment of that same explicit boundary
  constraint, or a sub-budget clipped-corner edge whose endpoints are both on explicit boundary
  constraints and whose whole owned region still passes overlay coverage. It must still reject
  exposed edges that are not bounded by explicit seam / footprint constraints.
- Parry crossing diagnostics are debug-significant but do not veto a node by themselves when Spade
  accepted the constraints and the overlay coverage check proves the emitted triangles cover the
  owner region within the deterministic numeric budget. Degenerate constraints, open boundaries,
  height conflicts, missing coverage, and non-boundary exposed edges remain blocking errors.
- terrain clip / earthwork root boundary loops are not independent render surfaces. Their loop
  points come from the final boolean footprint and must remain usable even when a separate
  single-polygon CDT fill of that loop would fail; height recovery for later terrain-clip union may
  interpolate deterministically along the exported boundary contour.
- unioned terrain clip cutters are 2D terrain-removal loops, not render polygons. They must not be
  dropped because a secondary visual CDT fill fails. When several candidate top surfaces provide
  different heights at the same unioned clip vertex, the exported cutter uses the highest visible
  top height deterministically so terrain cannot survive above any road surface at that XZ point.
  This exception applies only after final road ownership/source provenance is solved; dust
  connector recovery, output source selection, and node height ownership must reject conflicting
  height/source identities instead of applying the top-envelope policy.
- invalid Spade constraints are counted as geometry bugs; the live node-piece path must not fall
  back to nearest-material classification, centroid voting, render order, or paired strip sectors

### Conflict-First Node Candidate Hardcut

The node-piece hardcut replaces the paired adjacent-mouth strip candidate model. The old strip model
can produce malformed local wedges: one missing asphalt wedge lets sidewalk own the junction
center, and one missing footprint wedge lets terrain or background appear inside the junction. More
cleanup after strip generation is not accepted as the final fix, and neither is a smaller central
core that ignores overlapping road arms outside the core.

Scope:

- first target: `Terminal`, `Bend`, and `JunctionN`
- `Bend` is not a separate geometry family; it is the same node-region builder with two mouths
- `Terminal` is a one-mouth explicit band builder. The legacy sidewalk / curb end-band helper is
  not carried over by this hardcut.
- `JunctionN` is the same node-region builder with `n >= 3` mouths
- completed follow-up: `Span` output adapts to the same resolved-region staging shape after node
  pieces are stable
- out of scope for this hardcut: lane-routing policy, pedestrian legality, building frontage rules,
  retaining-wall variants, and cosmetic sidewalk texture changes

For `Bend` and `JunctionN`, the hardcut has two primary ownership candidate classes:

- full-roadbed corridor candidates define the road-owned node footprint
- carriageway corridor candidates define the asphalt region

Everything else derives from those two classes plus explicit profile seam rails:

```text
node_footprint = union(full_roadbed_corridors)
node_asphalt   = union(carriageway_corridors) intersect node_footprint
node_non_road  = node_footprint - node_asphalt
```

For `Bend` and `JunctionN`, `node_non_road` is an intermediate ownership area only. It must be
split into explicit `CurbOrShoulder` and `Sidewalk` owned regions before any render triangle or
height evaluation is emitted. `Terminal` uses explicit side-band and end-band topology from the
solved endpoint bands, so it does not render a generic `node_non_road` remainder.

Required node-piece input:

- stable piece ID
- node class: `Terminal`, `Bend`, or `JunctionN`
- node world position in XZ
- incident mouths sorted by deterministic class rule:
  - terminal: the only mouth
  - bend: edge ID after geometric normalization, then side
  - junction: counter-clockwise centerline angle, then edge ID, then side
- for each mouth:
  - outward centerline direction from the node
  - inward direction toward the node
  - local handoff distance
  - conflict handoff distance
  - throat section sampled at the final conflict handoff
  - roadbed half-width
  - carriageway half-width
  - full-roadbed left / right outer boundary points
  - carriageway left / right boundary points
  - curb / sidewalk profile boundary points required for band-owned subdivision
  - solved boundary heights from the same section profile used by spans
  - stable band indices for every profile interval
  - road class and profile flags
- project epsilon, minimum-area threshold, and minimum-triangle-altitude threshold

Candidate generation:

1. Convert each mouth into node-local XZ coordinates with stable quantization.
2. For every mouth pair, compute the deterministic material-conflict distance described in
   section 10A. This distance is part of visible ownership, not only routing metadata.
3. For each `Bend` / `JunctionN` mouth, build one full-roadbed corridor polygon from the full mouth
   segment through the final conflict handoff region. This polygon defines where road-owned walking
   / shoulder surface can exist, but it does not decide that the area will remain sidewalk.
4. For each `Bend` / `JunctionN` mouth, build one carriageway corridor polygon from the mouth
   carriageway segment through the same conflict handoff region. This polygon defines asphalt
   ownership before any sidewalk region is accepted.
5. For `Terminal`, route the one-mouth side-band ownership through the same canonical pipeline as
   bends and junctions. The U-shaped terminal cap is generated by `surface::terminal` as canonical
   owned cap carriers with explicit rail and height provenance, not by the retired input-side
   endpoint helper.
6. For `Bend` and `JunctionN`, add a deterministic asphalt kernel at the node center only when
   needed to close sub-centimeter numeric gaps between carriageway corridors. The kernel radius is
   bounded by the smallest incident carriageway half-width and must be clipped to `node_footprint`;
   it must not create a roundabout-sized node surface.
7. For `Bend` and `JunctionN`, curb / sidewalk subdivision is required before rendering. The
   subdivisions are clipped by `node_non_road`, carry their source band owner and height surface,
   and cannot enlarge the footprint or override asphalt ownership.
8. Crosswalk candidates are anchors only. They are generated after asphalt / sidewalk ownership is
   resolved and must not create topology holes.

Forbidden candidate generation:

- paired adjacent-mouth side strips as the source of `node_footprint`
- paired adjacent-mouth side strips as the source of visible sidewalk ownership
- miter caps, miter guards, or miter connector cells as the primary source of material ownership;
  mitered visuals may only be output after canonical ownership has already been solved
- a global annulus between one outer loop and one inner loop
- a global angle-sorted polygon that pretends to know final asphalt / sidewalk ownership before
  boolean cleanup
- a center-core polygon that fixes the node center while leaving pre-node arm overlaps to
  independent span strips
- per-angle branches for `15 deg`, `30 deg`, `60 deg`, `90 deg`, or any other named angle
- discarding acute-angle sidewalk candidates before boolean ownership merely because they look
  awkward
- using CDT to decide material ownership

Boolean ownership order for `Bend` and `JunctionN`:

1. Quantize and canonicalize every corridor candidate ring.
2. `i_overlay` unions all full-roadbed corridors into `node_footprint`.
3. `i_overlay` unions all carriageway corridors plus the optional bounded center kernel into
   `node_asphalt_raw`.
4. `i_overlay` intersects `node_asphalt_raw` with `node_footprint` to produce final
   `node_asphalt`.
5. `i_overlay` subtracts `node_asphalt` from `node_footprint` to produce final `node_non_road`.
6. Degenerate output rings below the documented minimum area threshold are dropped and counted.
7. `node_non_road` is split into final curb / shoulder and sidewalk owned regions using explicit
   profile seam rails. Each accepted non-road owned region must still carry explicit seam-rail
   evidence after clipping, cleanup, and seam materialization. Unowned residual area is rejected or
   debug-counted.
8. Final asphalt, curb / shoulder, and sidewalk owned regions are triangulated with Spade CDT.
9. `Terminal` skips this boolean ownership order for visible non-road geometry and instead exports
   the explicit terminal top-surface footprint from its asphalt, side non-road, and end-band
   polygons.
10. Terrain patch input receives an `i_overlay` union over all grounded `Standard` piece footprints
   intersecting the patch query before terrain CDT constraints are built.

Acute-angle rule:

- asphalt has priority over every curb / shoulder / sidewalk region
- sidewalk / curb regions may shrink, split into multiple islands, or collapse to nothing at acute
  angles
- a collapsed sidewalk is valid only when the remaining boolean area is below the documented
  minimum area threshold
- no final sidewalk / curb triangle may overlap final asphalt
- no final sidewalk / curb triangle may cross the asphalt boundary to reach another sidewalk island
- no final asphalt triangle may be omitted merely to preserve sidewalk continuity
- all acute-angle behavior must be symmetric under reversed road direction and equivalent edit
  order

Crosswalk and marking rule:

- crosswalk stripes remain a decal / marking layer, not topology
- crosswalk anchors are generated from resolved mouth boundaries and resolved asphalt /
  `node_non_road` contact edges
- a crosswalk must never be wider than the resolved asphalt it crosses
- a crosswalk must not be emitted through a missing footprint wedge
- old lane-only crosswalk geometry may provide semantic intent, but final render positions must use
  the resolved node surface boundaries

Output contract:

- the region builder returns disjoint material regions, not render strips:
  - asphalt regions
  - curb / shoulder regions
  - sidewalk / walkable regions
  - optional decal anchor records
  - one or more road-owned outer footprint loops
- every output region includes canonical rings, holes, area, material, band owner, source seam
  rails, height surface, and source piece ID
- Spade CDT emits triangles only from resolved regions
- the renderer consumes Spade triangles grouped by material
- terrain patch generation consumes outer footprint loops extracted from the final band-owned top
  mesh, not a re-extracted render boundary or a candidate footprint loop
- visible-world surface queries consume the same resolved triangles as the renderer

Debug contract:

- `--debug road-geometry` must report, per node piece:
  - node class
  - incident mouth count
  - full-roadbed corridor candidate count
  - carriageway corridor candidate count
  - optional center-kernel radius
  - `i_overlay` footprint / asphalt / non-road output region counts
  - dropped degenerate ring count
  - final Spade constraint count
  - skipped / invalid Spade constraint count
  - final triangle count by material
- debug output must make acute-angle collapses visible as explicit dropped or split non-road
  regions, not as silent missing geometry
- debug output must make any missing footprint wedge measurable as `node_footprint` area, region
  count, or candidate failure before terrain CDT is blamed
- debug output must make any missing mouth seam or outer-footprint top owner measurable as a
  specific missing `NodeOwnedRegion` / seam constraint, not only as a rendered visual gap
- debug output must make any canonical outline failure measurable as an open boundary, duplicate
  exposed edge, missing owned-region edge, missing top-surface coverage, or rejected
  non-explicit boundary vertex instead of allowing earthwork faces to attach to an unrelated
  reconstructed boundary point
- debug output must report which accepted geometry backend produced each major stage: offset /
  contour generation, boolean ownership, grade evaluation, validation, and CDT triangulation

Legacy node removal target:

The node rework must remove the old node-patch behaviors without throwing away the useful solved
roadbed pipeline around them.

Retire these node-construction patterns:

- paired adjacent-mouth connector strips as final carriers or primary candidates
- gap-by-gap node asphalt wedge assembly around the center
- global annulus / halo sidewalk ownership around one node center
- generic junction fill polygons as the source of bend or multi-arm node semantics
- full-roadbed closure carriers as rendered fallbacks for missing curb, shoulder, or sidewalk
  ownership
- second-pass outer-boundary reconstruction from candidate loops, already-emitted render
  triangles, or terrain clip loops
- shared post-overlay grade sampling as a compatibility path for rendered `Bend` or `JunctionN`
  top-surface heights

Forbid these replacement shortcuts:

- feeding node terrain clipping or local earthwork generation from candidate loops, raw render
  triangles, or any post-export flat road-clip polygon pass
- accepting an outer-boundary vertex that is not part of the canonical boolean footprint
- accepting raw rendered triangle edges as terrain clip or skirt boundaries
- choosing material ownership with hint-only classification, nearest material, centroid voting,
  render order, or CDT face classification
- resolving same-XZ height conflicts by welding, averaging, min/max choice, owner priority, or
  nearest-height sampling
- falling back to shader masks, terrain masks, water, zoning overlays, or background-color
  coincidence for missing topology

Keep and extend these target components:

- mouth / endpoint profile extraction from solved sections
- conflict handoff calculation and longitudinal-grade sampling
- explicit `Span`, `Terminal`, `Bend`, and `JunctionN` visual piece boundaries
- chunk invalidation based on compiled piece coverage
- accepted geometry backends for vector math, offset / contour generation, validation, spline
  evaluation, boolean ownership, spatial lookup, and CDT triangulation
- Godot upload as a thin rendering bridge

Acceptance tests for the conflict-first node hardcut:

- terminal with sidewalks emits asphalt to the graph endpoint, a U-shaped non-road end band, and
  one footprint without overlap or slab overrun
- 2-arm bend at 1, 15, 30, 60, 90, 120, and 150 degrees emits no sidewalk-over-asphalt overlap and
  no terrain/background hole inside the node footprint
- triangle network emits three independent bends with closed outer footprints
- T-junction at 1, 15, 30, 60, 90, 120, and 150 degrees emits no sidewalk-over-asphalt overlap
- 4-way junction with exact 90-degree angles emits symmetric asphalt and non-road ownership
- 4-way junction with one acute arm emits split or collapsed non-road only where boolean area
  requires it
- `N > 4` junction emits deterministic ownership for stable mouth ordering
- `N > 4` junction with arbitrary non-cardinal angles grows only where pairwise material-conflict
  distances require ownership, with no unjustified center bubble
- reversed edge direction and equivalent edit order produce the same canonical regions
- terrain seam footprint equals the node-piece resolved outer footprint exactly
- crosswalk anchors stay inside resolved asphalt and never create holes
- `--debug road-geometry` reports the same region counts for repeated builds of the same save

### Later Extensions

Retaining walls, richer structural tie-in materials, and building-pad engineered-ground clients are
later extensions of the shared earthworks system in [`earthworks.md`](earthworks.md). They do not
block the Spade terrain-patch hardcut.

Rounded sidewalks and smoother junction corners are later visual-quality work. They must operate
only on already-resolved legal regions:

- corner fillets, smoothing, or concave-hull simplification may reshape the outer sidewalk boundary
  only after `node_asphalt`, `node_sidewalk`, and `node_footprint` are disjoint and valid
- rounding must preserve every asphalt / sidewalk boundary produced by boolean ownership unless it
  reruns the same ownership solve and proves the rounded result is still legal
- no smoothing pass may reconnect sidewalk islands across asphalt
- no smoothing pass may enlarge `node_sidewalk` into `node_asphalt`
- convex hulls are not accepted as a sidewalk model because they create unjustified plazas
- concave hulls are allowed only as a later boundary-beautification helper, never as the source of
  material ownership

## Legacy Retirement Rules

The hardcut does not retire the whole roadbed runtime. It retires only the terrain-seam,
centerline-lift, and generic node-ownership patterns that conflict with the piece/profile model.
Node-specific removal details are owned by `Legacy node removal target` above.

Still valid and must be extended:

- logical graph ownership in `rust/src/simulation/network/mod.rs` and graph modules
- solved edge grades, section sampling, and lateral band definitions
- explicit `Span`, `Bend`, `Terminal`, and `JunctionN` visual pieces
- chunk-local dirtying and rebuild boundaries
- preview / commit parity
- Godot mesh upload as a thin presentation bridge

Retired patterns that must not return:

- centerline-only road lifting or terrain flattening
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
- acute 2-arm bend with no terrain/background hole inside the resolved node footprint
- triangle network composed of three independent bends
- pass-through split with no center bubble
- width transition on a nearly straight corridor
- T-junction center owned by carriageway
- acute T-junction where sidewalk may split or collapse but must not overlap asphalt
- acute T-junction with no terrain/background hole inside the resolved node footprint
- 4-way junction center owned by carriageway
- 4-way junction with one acute arm and deterministic sidewalk ownership
- `N > 4` multi-arm node center owned by carriageway
- `N > 4` multi-arm node with acute neighboring arms and no sidewalk-over-asphalt overlap
- `N > 4` multi-arm node with arbitrary angles and no unjustified center-bubble growth beyond
  pairwise material-conflict ownership
- `N > 4` multi-arm node with no terrain/background hole inside the resolved footprint
- car-only road with no sidewalk bands
- footpath joining only one sidewalk side
- bridge span above terrain without terrain flatten under the span
- tunnel portal behavior without surface carving along the buried segment
- preview / commit parity for the same input path
- terrain earthwork agreement with the roadbed inside the paved footprint on supportive terrain
  densities
- deterministic cut / fill transition outside the paved footprint inside the earthwork margin
- grounded hillside roads keep a laterally flat carriageway instead of following raw terrain
  cross-slope
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
through the contracts in this document instead of reintroducing retired centerline ownership,
generic node patches, or renderer-owned road topology.

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
