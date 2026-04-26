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

The current road-touched terrain path is still provisional.

Live behavior:

- grounded `Standard` roads send piece-owned footprint clip polygons into road-touched terrain
  patches
- Rust returns baked terrain patch mesh payloads for those road-touched patches
- terrain under the road-owned footprint is no longer intended to be a visible carrier
- Spade is now a runtime dependency and the first isolated terrain CDT kernel lives in Rust under
  `simulation::terrain::cdt`; it is tested for deterministic contained-road-footprint
  triangulation but is not yet wired into the live terrain patch bridge

Not accepted as the final target:

- seam-strip emission from road outer-loop segments to nearby terrain
- conservative terrain-cell omission based on fully-owned cell triangles
- terrain density experiments as the main answer to road / terrain gaps
- any shader mask, water plane, zoning overlay, or background-color coincidence that hides missing
  topology

The next hard-cut target is therefore explicit: road-touched terrain patches must be rebuilt with
Spade-backed constrained triangulation, not with the current seam-strip / cell-triangle hybrid.

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

## Spade CDT Terrain-Patch Hardcut

The next accepted representation is a Spade-backed terrain patch generator. It replaces the current
road-touched terrain mesh builder; it does not sit beside it as a fallback.

### Target Backend

Required rules:

- `spade::ConstrainedDelaunayTriangulation` is the only planned CDT backend for the production
  terrain-road seam
- `bulk_load_cdt` is the only accepted insertion path for the first implementation
- Spade refinement helpers are not used until this project has a pinned deterministic refinement
  contract
- any future replacement of Spade requires a new explicit benchmarked spec change

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
  - hard error details when constraints are invalid

The API must not expose:

- road graph internals
- Godot mesh types
- shader-mask assumptions
- water or zoning overlay state

### CDT Input Rules

The CDT input is canonical and deterministic:

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

After `bulk_load_cdt`, terrain faces are classified in Rust:

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
8. Replace the live road-touched terrain patch builder with the CDT module.
9. Remove the retired seam-strip / cell-triangle live path.
10. Update `--debug road-geometry` output to report CDT constraints, accepted faces, rejected
    faces, invalid constraints, and preserved seam-edge counts.

Acceptance requires `cargo check`, the CDT contract tests, and Godot headless load to pass.

### Later Extensions

Spade may later be reused for road-piece polygon fill triangulation, but that is not part of the
terrain-patch hardcut.

If road pieces use Spade later:

- `Span`, `Bend`, `Terminal`, and `JunctionN` still own semantic road shape
- CDT only triangulates already-decided polygons
- CDT must not decide lane, sidewalk, crosswalk, mouth, or frontage semantics

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
