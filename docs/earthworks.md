# Earthworks / Engineered Ground

## Purpose

This document owns the shared engineered-ground contract for local terrain overrides such as road
cuts, embankments, flat building pads, and future retaining structures.

The first live road client was closed under [`ROAD-01`](roadmap.md). Building-site support tie-ins
now share the road-touched terrain CDT carrier; future terrain closure, plot-foundation, or
retaining work should extend this document under its own tracked ID instead of duplicating the same
terrain-override rules elsewhere.

It answers these questions:

- what `source terrain`, `visual terrain`, and `engineered ground` each mean
- which subsystem owns the support surface, cut / fill envelope, and terrain tie-in
- how local ground overrides participate in rendering, picking, and chunk invalidation
- why a single-height terrain grid is not sufficient to represent every engineered-ground case
- what the current runtime does today and what the longer-term shared target must become

It does not own:

- lane routing, junction routing, or roadbed section generation details
- terrain chunk storage internals or water runtime rules
- zoning legality, frontage semantics, or building program rules

Those remain owned by [`roads.md`](roads.md), [`terrain.md`](terrain.md),
[`zoning.md`](zoning.md), and [`building_allocator.md`](building_allocator.md).

## Document Conventions

Interpretation rules:

- `current runtime` means the shipped implementation in the repository today
- `shared target` means the required long-term subsystem contract across multiple clients
- `must` means required for the owning contract
- `should` means intended unless a better measured implementation replaces it
- `may` means optional

Terminology:

- `engineered-ground client`: a subsystem that owns a local support surface and the surrounding cut
  / fill transition back to terrain
- `support surface`: the authoritative local surface that terrain must support, such as a roadbed
  or flat build pad
- `placed client`: a committed engineered-ground client whose support surface must remain fixed
  until the player explicitly edits, moves, or removes that client
- `footprint`: the directly owned top surface area of the client
- `earthwork margin`: the local transition zone outside the footprint where terrain ties back
  toward source terrain
- `tie-in boundary`: the outer edge where engineered ground hands ownership back to visual terrain
- `local earthwork mesh`: a local owner-controlled cut / fill mesh that represents the engineered
  slope, retaining face, closure, or other tie-in geometry near the footprint

## Shared Model

### 1. Source Terrain Remains The Authored Ground

The engineered-ground system must not rewrite authored source terrain.

The terrain runtime continues to own:

- source terrain
- visual terrain storage
- chunk residency and upload boundaries

Engineered-ground clients contribute derived local overrides that affect the visible world surface,
not the authored source terrain.

### 2. Clients Own Local Support Surfaces

Each engineered-ground client owns one authoritative support surface for its footprint.

Current and planned clients include:

- roads, using the compiled roadbed
- building sites, using the placed building's fixed flat lot plane; authored
  `[[site_surfaces]]` polygons are material/layout regions on top of that plane
- future parking platforms, rail beds, retaining structures, or other built ground

The support surface must not be inferred from the terrain heightfield after the fact.

### 3. Heightfields Alone Are Not The Long-Term Visual Carrier

A single-height terrain grid can only store one height per `(x, z)` position.

That is not sufficient to cleanly represent all of these at once near the same footprint edge:

- uphill cut face
- support surface
- downhill fill / support

Structural terrain stamping may still be used for explicit bridge / tunnel / retaining cases, but
ordinary grounded roads must solve the road / terrain boundary through road-owned top surfaces plus
Rust-generated stitched terrain topology.

### 4. Placed Client Surfaces Stay Fixed Under Later Terrain Edits

Placement and later terrain authoring are separate operations.

Required rule:

- placement-time grounding may choose the initial support surface for a client
- once a client is committed, later terrain-authoring edits must not move that client's support
  surface implicitly
- later terrain edits must instead recompute the surrounding earthworks and tie-in back to terrain
  around the already placed client
- moving, regrading, or deleting the client remains an explicit client-edit operation, not a side
  effect of terrain brushes

This rule applies equally to linear clients such as roads and to area clients such as placed flat
building-site support footprints.

### 5. Earthworks Derive From The Client Support Surface

Earthworks must be generated from the client-owned support surface, not from ad hoc terrain
flattening.

Required rule:

- inside the footprint, the visible world surface is the client-owned surface
- inside the footprint, visual terrain is not a visible carrier:
  - for grounded roads, asphalt, shoulder / curb, and sidewalk geometry are the visible ground
  - terrain fragments under those road-owned bands must not be rendered
  - the road-owned top surface follows the client-owned solved profile rather than a separate
    world-horizontal terrain plane
  - source terrain is not blended or drawn there; the client-owned support surface replaces visual
    terrain locally inside the owned footprint
- outside the footprint but inside the earthwork margin, the visible world surface transitions back
  toward source terrain using deterministic cut / fill rules
- the seam between the road-owned top surface and surrounding terrain must be covered by a
  deterministic tie-in carrier:
  - Rust generates the road-touched terrain patch topology as an explicit local mesh
  - that mesh omits terrain below the road-owned footprint and inserts road-boundary vertices at
    the exact road / sidewalk outer edge
  - vertices created on that boundary reuse the road-owned edge heights, not resampled terrain
    heights, so the terrain and road share the same seam coordinates
- terrain suppression alone is not a seam solution; it may only hide terrain under geometry that
  actually exists
- for grounded clients, the runtime must not render a second visible support mesh under an ordinary
  grounded footprint just to hide terrain overlap
- ordinary grounded roads must not render a visible closure strip, carpet, or second support mesh;
  the terrain patch mesh itself is the only ordinary seam carrier outside the road-owned footprint

Deterministic seam contract:

- the seam is the shared boundary between the client-owned footprint and the far-field terrain
- for roads, the footprint includes asphalt, shoulder / curb, and sidewalk bands; the seam starts at
  the exact outer sidewalk edge, or at the exact outer shoulder edge when a profile has no sidewalk
- no-sidewalk `Standard` road profiles still expose an explicit curb / shoulder band, so terrain
  clipping receives a real road-owned outer seam instead of a zero-width or asphalt-only fallback
- every seam segment must be backed on both sides by visible carriers before terrain below the
  footprint is suppressed:
  1. the client-owned top surface covers the footprint side of the seam
  2. terrain topology is clipped / triangulated so its inner edge exactly matches the client-owned
     seam vertices
- terrain masking, terrain alpha, terrain discard, or footprint suppression is not a seam carrier;
  those tools are valid only after one of the visible carriers above already covers the boundary
- grounded `Standard` roads use one seam carrier: Rust-generated terrain patch topology clipped to
  the road-owned footprint
- the clipped terrain inner edge must reuse the same coordinates and heights as the road-owned
  outer sidewalk / shoulder boundary; it must not resample, offset, snap, simplify, or widen that
  edge into a visible strip
- clipped terrain patches must receive exact road footprint loops from the same `Span`,
  `Terminal`, `Bend`, and `JunctionN` pieces that render asphalt, shoulder / curb, and sidewalk
- Rust must triangulate from those footprint loops and must insert deterministic seam triangles
  that use the loop as a hard inner constraint; terrain clipping must not reuse asphalt / sidewalk
  render triangles, because those triangles are not the ownership contract and can over-cut concave
  road footprints
- the road-touched patch mesh must emit terrain-owned seam faces from each road-owned outer-loop
  segment to source terrain outside the footprint before any terrain below the footprint is omitted
- the seam faces are part of the terrain patch mesh, use terrain material, and are not a second
  road support mesh, visual carpet, or closure strip owned by the road renderer
- grounded `Standard` seam faces use explicit Rust-generated grade-limited guide samples outside
  the final road-owned footprint, keyed before ordinary source terrain samples, so the CDT tie-in
  is an authored topology input rather than a render-side repair
- those guide samples are generated by `RoadSurfaceSystem` from the final unioned roadbed loops,
  not by Godot, terrain rendering, or a per-piece post-process
- guide rail constraints may be emitted only when the final footprint set is a single clean non-hole
  convex loop; concave junction mouths and multi-loop footprints remain sample-only to prevent
  grading rails from crossing the final road-owned footprint
- building-site loops may share the terrain-CDT patch carrier, but they are not roadbed loops and
  must not emit `RoadSurfaceSystem` roadbed grading-envelope samples or constraints; site grading is
  owned by the site / engineered-ground path
- ordinary `Standard` span and node footprint seam sources must not be promoted into retaining-wall
  topology; wall output is reserved for explicit structural bridge / tunnel / future retaining
  sources with preserved provenance
- terrain cell triangles may still provide far-field terrain outside the road footprint, but they
  are not allowed to be the only seam carrier because their grid edges rarely coincide with the
  road-owned outer sidewalk / shoulder edge
- target CDT contract:
  - road-piece polygon ownership is resolved with `i_overlay` before triangulation; Spade receives
    already-owned asphalt, sidewalk, and terrain regions rather than overlapping material hints
  - road-touched terrain patches are generated with Spade's Rust-side
    `ConstrainedDelaunayTriangulation`
  - `i_overlay` is the chosen Rust-side polygon boolean backend for road / terrain ownership
    cleanup; it owns union, intersection, difference, hole handling, and overlap removal before
    CDT input is built
  - Spade is the chosen CDT backend; `ghx_constrained_delaunay` is not part of this spec, is not a
    fallback, and may only be reconsidered through a new explicit benchmarked spec change
  - road node contour construction, grade evaluation, spatial lookup, and validation follow the
    accepted geometry-backend responsibilities in [`roads.md`](roads.md);
    earthworks must not depend on hand-rolled road offset, boundary recovery, or sampled-height
    repair paths to produce a valid seam
  - road / earthwork seam math should use the same internal `glam` vector representation and
    explicit quantized keys as the road arrangement builder; Godot vectors are render / bridge
    payloads, not the authoritative seam identity
  - `robust` is not part of the accepted implementation path for now; exact-predicate needs should
    first be handled by `i_overlay` and Spade, and any standalone predicate dependency requires a
    narrow future spec change
  - the production path uses deterministic `try_bulk_load_cdt` inputs; conflicting constraints are
    counted and skipped rather than allowed to panic the simulation thread, and Spade refinement
    helpers are not used until they have a pinned deterministic contract for this project
  - the terrain patch rectangle is the outer constrained contour
  - every grounded road-owned outer footprint loop inside or crossing the patch is inserted as a
    hard constrained contour
  - road footprint constraints that cross patch edges are pre-split before CDT input; constraint
    segments must only meet at shared endpoints and must not cross through each other
  - road-piece seam vertices and constraint edges must appear exactly in the generated terrain
    mesh; no post-process snapping, widening, shader discard, or Godot-side clipping may create
    the seam
  - source-terrain sample points may be inserted as Steiner / interior points only outside road
    footprints, and their insertion order must be canonical and deterministic
  - roadbed grading-envelope guide samples are inserted before ordinary source-terrain samples and
    share the same deterministic point keying, so near-road tie-ins are bounded without moving the
    road seam
  - guide constraints are a local convex-footprint aid, not a footprint repair path; if the final
    loop set is concave, contains holes, or contains multiple non-hole loops, the grading envelope
    must rely on guide samples without adding rail constraints
  - Spade CDT faces are classified after triangulation against the final road-owned footprint;
    ordinary non-seam faces may use centroid ownership, but faces carrying a road constraint edge
    classify the exact seam side so narrow concave tie-ins are not lost to centroid-only ownership
  - emitted terrain triangles must preserve the CDT constraint edges at the road seam and must not
    cross a road-owned footprint loop
  - CDT triangulation failures are hard errors in debug output and must not fall back to cell
    subtraction, seam carpets, closure strips, water, or shader masks
  - `Terminal`, `Bend`, and `JunctionN` visual node pieces resolve asphalt / sidewalk /
    outer-footprint ownership through `i_overlay` before Spade triangulation, so sharp-angle
    sidewalks shrink or split instead of overlapping asphalt
- clipped terrain topology must insert road-boundary vertices into the terrain mesh in Rust rather
  than approximating the seam from terrain-cell centers, a texture mask, or a Godot-side polygon
  clipping fallback
- no `Bend` or `JunctionN` clip boundary may be synthesized from one anonymous annulus, one global
  outer loop, or a terrain-cell mask after the fact
- a valid seam is closed independent of triangle winding or backface-culling behavior; clipped
  terrain may render double-sided, but the renderer must not reveal world background, holes, or
  source terrain through the road / sidewalk / tie-in boundary
- water is not a valid fallback carrier under the road footprint; water render patches that overlap
  grounded road-owned asphalt, shoulder / curb, or sidewalk must receive the same footprint clip
  polygons and omit any touched water cells instead of triangulating partial transparent fragments

For roads, that means:

- grounded road footprint support is owned by the committed road top surface
- when multiple committed road-owned top surfaces overlap at the same XZ, terrain support clearance
  uses the lower top-surface envelope so terrain remains below every visible road-owned face; this
  does not change visible-surface picking, which still resolves the topmost rendered surface
- the committed road top surface is intentionally laterally flat at each section station:
  carriageway, left edge, and right edge share the same road height, while sidewalks use
  `road_height + curb_step`
- drainage crown, banking, and terrain-derived crossfall are not part of the road support surface
- bridges limit that support to abutment-owned grounded regions
- tunnels limit that support to visible portal-owned grounded regions
- grounded `Standard` roads replace the near-road visible terrain locally:
  - asphalt and sidewalk render as the visible terrain replacement inside the road-owned footprint
  - the Rust terrain patch mesh emits no terrain under the road-owned footprint
  - the seam to far-field terrain is formed by terrain triangles whose inner boundary is the
    road-owned outer sidewalk / shoulder edge
  - grade-limited guide samples around the footprint keep ordinary tie-in triangles local and
    deterministic without wall teeth beside the road
  - terrain suppression is allowed only as a consequence of the clipped terrain topology
  - the runtime must not render a separate visible cut / fill support mesh or ordinary closure strip
    below or beside grounded asphalt or sidewalk
- structural or intentionally exposed cases such as bridge abutments, tunnel portals, or future
  retaining variants may still render explicit earthwork / wall geometry where terrain alone is
  not the intended visible carrier

The system must not treat "sample terrain at the centerline and widen it later" as the shared
earthworks model.

### 6. World-Surface Query Precedence Must Be Explicit

Terrain-only queries remain authoritative against source terrain.

Visible world-surface queries must resolve in this order:

1. client-owned top surface
2. client-owned local earthwork geometry only when that geometry is intentionally surfaced
3. visual terrain
4. source terrain only for terrain-only APIs

This rule must stay shared between roads, future build pads, and any other engineered-ground
clients.

### 7. Invalidation Must Stay Chunk-Local

A local client edit must invalidate only the touched local region.

Required rule:

- only touched derived terrain regions are rebuilt; structural visual-terrain stamps remain
  chunk-local, and ordinary grounded-road seams rebuild only their road-touched terrain patches
- only touched engineered-ground caches are rebuilt
- unchanged world regions keep their existing visual terrain and cached meshes

The shared subsystem must not fall back to whole-map terrain flattening for one local support
surface edit.

### 8. Authoring Query Domains Must Stay Explicit

Editor and gameplay tools must declare whether they are operating on authored ground or on the
visible engineered surface.

Required rule:

- terrain-authoring tools read and write source terrain only
- visible-surface inspection, placement, and selection tools read the combined visible world
  surface in the precedence defined above
- terrain-authoring brushes must not silently move or regrade an already placed client support
  surface
- engineered-ground clients must not silently intercept terrain-authoring brushes and treat them as
  direct edits to a roadbed, pad, or local earthwork mesh

This keeps authored ground editing deterministic even after local engineered-ground geometry becomes
more capable than the terrain heightfield alone.

## Current Runtime

### 1. Roads Are The Live Client

The current runtime ships roads and flat building sites as live engineered-ground clients.

That means:

- `RoadSurfaceSystem` owns the roadbed support surface
- `BuildingAllocator` owns required flat building-site support clients registered at construction
  start
- authored `[[site_surfaces]]` polygons render/query as material regions on top of the flat site
  plane, but do not define separate terrain ownership footprints
- live gameplay must not render authored yard surfaces as loose decal / overlay meshes over terrain;
  they must be emitted from the runtime site client after the terrain footprint is clipped
- grounded `Standard` roads do not stamp their footprint or ordinary outer margin into visual
  terrain; road-touched terrain patches are stitched to the road-owned outer edge
- bridge earthworks remain abutment-only and use class-owned endpoint ranges rather than ordinary
  road-width handoff ranges, so midspans are not flattened
- tunnel earthworks remain portal-only and use class-owned endpoint ranges so visible portals are
  not trimmed away before portal visibility is evaluated

Road-specific section and junction rules continue to live in [`roads.md`](roads.md).

### 2. Former Whole-Map Render Boundary Is No Longer The Blocker

The earlier terrain-side blocker for earthworks work was the old whole-map dense render boundary.

That blocker is now removed:

- [`TERRAIN-01`](roadmap.md) is done
- terrain and water rendering now use chunk-local patch upload and residency instead of one
  whole-world mesh plus one whole-map dense steady-state upload
- `10 m` versus `5 m` terrain characterization now runs on the split render path instead of on the
  old overlay-era whole-map render boundary

So the current earthworks problem is no longer "the renderer makes any denser or more local
engineered-ground work pay a whole-world cost." The remaining problem is what the current terrain
representation can express near engineered-ground footprints.

### 3. Terrain Density Is A Blend-Quality Input, Not The Seam Carrier

The current runtime no longer relies on the visual terrain heightfield to carry the ordinary
grounded-road seam. Road-touched terrain patches are stitched in Rust, and terrain density now
affects far-field blend quality and patch cost rather than correctness under the road footprint.

The current deterministic characterization now proves:

- the `10 m` grid can still show coarse far-field blend facets around a road
- `5 m` or finer terrain may improve that blend quality
- density is not allowed to become the engineered-ground seam fix
- any future baseline move from `10 m` to `5 m` or finer terrain must follow the chunk-local
  terrain / water render-boundary split owned by [`terrain.md`](terrain.md), not the current
  whole-map dense renderer upload path

So terrain density may improve far-field blend quality, but the accepted seam representation is the
Spade CDT terrain-patch hardcut in [`roads.md`](roads.md).

### 4. Current Runtime Compatibility Gap On Post-Placement Terrain Edits

The current runtime still violates the fixed-client rule for grounded roads.

Current compatibility gap:

- later terrain edits still resynchronize placed `Standard` road geometry against edited source
  terrain
- structural visual terrain is then rebuilt from that moved roadbed, so the road can shift instead
  of the terrain alone reshaping around it
- building-site clients now ship as fixed flat in-lot pad surfaces with deterministic perimeter
  tie-in guides; richer foundation meshes remain later work

### 5. Current Terrain Runtime Is A Compatible Base, Not The Final Visual Carrier

The current terrain runtime is sufficient for the road-first stitched terrain cut:

- it keeps separate authoritative `source terrain` and derived `visual terrain`
- it supports chunk-local reset and restamp of touched visual regions
- it accepts road-touched patch mesh ownership from Rust for ordinary grounded roads and still
  accepts structural earthwork inputs for explicit bridge / tunnel / retaining cases

But it is not sufficient as the final near-footprint visual carrier because the current visual
terrain remains a single-height field.

Current deterministic conclusion:

- a terrain-runtime rewrite is allowed if needed, but it is not by itself an earthworks solution
- do not expect the current visual terrain heightfield, even at a denser cell size, to represent
  all near-footprint cut / fill detail by itself
- ordinary grounded-road near-footprint ownership is now the client-owned top surface plus stitched
  terrain patch topology; structural cases may still use client-owned local earthwork geometry

## Geometry Decision

The former whole-map dense terrain / water render boundary is intentionally not listed here as an
open blocker, because [`TERRAIN-01`](roadmap.md) already removed it.

### 1. The Current Corridor-Sheet Prototype Is Retired

The repository currently contains a roads-first corridor-sheet prototype, but it is not the target
solution anymore.

Deterministic choice:

- stop treating a thin visible corridor sheet plus terrain stamping / terrain suppression as the
  final near-road representation
- do not continue polishing the current corridor-sheet prototype as the long-term fix for flat
  ground or arbitrary road angles
- keep only the parts of that prototype that remain architecturally useful:
  1. fixed-client semantics after commit
  2. chunk-local invalidation and rebuild boundaries
  3. visible-world query precedence
  4. bounded render uploads
  5. the rule that terrain is not the sole near-footprint owner
- the replacement target for ordinary grounded roads is the road-owned top surface plus
  Rust-stitched terrain topology; visible road-owned closure / earthwork meshes are reserved for
  structural classes or later explicit retaining variants

### 2. Roads-First Rewrite Uses A Separate Piece/Profile Carrier Plus Stitched Terrain

The next implementation slice is still roads-first, but the geometry contract changes.

Required road geometry contract:

- the logical road graph remains the simulation and connectivity authority only; it must not remain
  the direct visible-shape carrier
- the visible road system compiles a separate deterministic piece/profile geometry layer
- the minimum required visual piece set is:
  1. `Span`
  2. `Bend`
  3. `Terminal`
  4. `JunctionN`
- each visual piece must own one ordered side-aware road profile rather than one anonymous width:
  1. left outer sidewalk edge
  2. left curb / shoulder edge
  3. left carriageway edge
  4. right carriageway edge
  5. right curb / shoulder edge
  6. right outer sidewalk edge
- terrain seam generation must derive from those same piece-owned profile boundaries instead of
  from separate terrain-side widening or generic node-fill polygons
- ordinary grounded `Standard` roads use the road-owned top surface inside the footprint and
  Rust-stitched terrain topology outside it; they must not render a visible road-owned closure
  strip, carpet, skirt, or second support mesh as the seam fix
- structural road classes or later explicit retaining variants may render local earthwork geometry,
  but that path is not a compatibility fallback for ordinary grounded-road gaps
- terrain owns the far-field ground outside the deterministic road-owned footprint and receives
  exact road-footprint constraints for the hardcut
- flat-ground cases must collapse toward a visually minimal shoulder / verge join instead of
  emitting a wide apron
- sloped ordinary-road cases must keep asphalt, curb / shoulder, and sidewalk as road-owned top
  surfaces while the clipped terrain mesh terminates at their exact outer footprint

### 3. Tie-In Boundaries Follow Piece-Owned Boundaries, Not Coarse Terrain Cells Or Generic Node Loops

The next geometry pass must stop deriving the visible outline mainly from coarse terrain cells or
from the existing compiled road section spacing.

Required first variant:

- each `Span` must derive one left-side and one right-side outer tie-in polyline
- those tie-in polylines must be sampled from terrain at a maximum `2 m` longitudinal spacing in
  world space
- the visible tie-in boundary must not be inferred only from the authored terrain cell grid or
  only from the current compiled road section spacing
- consecutive samples stitch those inner and outer anchors into continuous side geometry per side
- edge-local ownership still stops at deterministic road throats so `Span` ownership hands off
  cleanly into `Bend`, `Terminal`, or `JunctionN` ownership
- `Bend` and `JunctionN` pieces must be built from incident mouth profiles and their ordered band
  boundaries, not from one generic angle-sorted throat-point polygon and not from one global outer
  loop plus one global inner loop
- two-edge non-pass-through nodes and `3+` arm nodes share the same band semantics, but they
  remain different visual piece classes with different builders
- node top-surface ownership must compile to explicit band-owned regions whose seam constraints and
  height owners are preserved through clipping and triangulation; it must not rely on one
  annulus-style ring carrier or a post-overlay nearest-height sampler
- building site surfaces follow the same owner model, but with authored area loops instead of two
  longitudinal side runs; future full building pads extend this with a perimeter tie-in ring

Required seam acceptance checks:

- a flat straight grounded road must show no source-terrain pixels, background pixels, or shadowed
  voids between asphalt / sidewalk and terrain when terrain under the footprint is suppressed
- a diagonal road against the terrain grid must meet the same rule; correctness must not depend on
  alignment to terrain-cell axes
- a sloped road must keep asphalt, shoulder / curb, and sidewalks as the visible top surface while
  clipped terrain topology terminates exactly at the road-owned seam
- a `JunctionN` with any ordered arm count must build seam sectors from adjacent mouth profiles,
  not from a special-case `3`-arm or `4`-arm template
- clipped terrain topology must leave an actual road-shaped hole under the full asphalt, shoulder /
  curb, and sidewalk footprint; there must be no terrain mesh there for z-fighting to occur
- clipped water topology must leave a conservative road-shaped hole where visible water overlaps a
  grounded road-owned footprint; road placement over water must not leave a hidden lake plane or
  transparent fragment wedges between the road mesh and its owned support surface
- bridges remain a separate class: midspan bridge decks do not claim grounded terrain ownership
  except at explicit abutment / portal support regions

### 4. The First Required Variant Is Rust-Stitched Terrain Topology

The first required rewrite variant is intentionally hard-cut: grounded road seams are generated as
terrain topology in Rust, not as a visible seal drawn by the road renderer.

Required first variant:

- the first shipped rewrite variant must clip terrain patches directly to the road-owned footprint
  boundary for grounded `Standard` roads; the clip carrier is the compiled piece-owned
  `outer_boundary_loops`, not the internal asphalt / sidewalk band polygons
- any internal triangles derived from those outer loops are implementation detail only and must be
  rejected if their sampled interior leaves the source loop; terrain must never be cut by a
  triangulation wedge that extends outside the road-owned footprint
- the terrain patch payload must include baked mesh vertices, normals, and UVs when the patch
  intersects a grounded road-owned footprint
- Godot must upload that baked `ArrayMesh` directly and must not run `Geometry2D` terrain clipping,
  shader discard, alpha masking, or an ordinary road-side closure-strip fallback
- boundary vertices generated by the terrain patch mesh must take their height from the intersected
  road / sidewalk edge and all non-boundary terrain vertices must sample visual terrain
- road-locked terrain patch selection is bounded to each road-owned footprint plus that footprint's
  required grade-limited tie-in envelope; one steep road must not force unrelated roads or whole-map
  terrain patches into clipped-terrain rebuilds
- cut versus fill for structural variants is determined from the support-surface anchors relative
  to the sampled tie-in boundary, not from ad hoc widening of stamped terrain
- the first shipped rewrite variant does not require retaining walls, cliff faces, or other
  special-case vertical structures
- the terrain mesh itself must be open below the road footprint and must terminate at the exact
  road-owned seam boundary
- terrain suppression is allowed only as topology omission from the clipped terrain mesh; fragment
  discard, alpha masking, or post-shader hiding is not an accepted grounded-road solution
- future retaining or wall variants may replace structural local earthwork faces when deterministic
  thresholds say terrain topology alone is not the intended visual treatment, but ordinary grounded
  roads still use the stitched terrain topology as their seam carrier

### 5. Terrain Runtime Rewrites Are Allowed, But The Ownership Contract Is Not Optional

A terrain-runtime rewrite is acceptable if it materially improves the system and still preserves the
shared engineered-ground rules.

Required rule:

- whether the implementation extends the current terrain runtime or rewrites it, the resulting
  runtime must still preserve:
  1. authoritative source terrain
  2. explicit client-owned top surfaces
  3. client-owned local earthwork geometry near the footprint
  4. chunk-local invalidation and rebuild boundaries
  5. visible-world query precedence
  6. no regression back to whole-world rebuild or upload behavior

### 6. Fixed-Client Runtime Behavior Is Deterministic After Commit

The fixed-client rule now has one required runtime interpretation.

Required runtime contract:

- placement-time grounding may choose the initial support surface only before the client is
  committed
- committing the client freezes that support surface for later terrain-authoring edits
- terrain-authoring edits write source terrain only
- after the source-terrain edit, the runtime rebuilds only the touched local earthwork meshes,
  stitched terrain patches, and derived visual terrain around the already committed support surface
- explicit client-edit operations remain the only path that may move, regrade, or replace the
  committed support surface
- if a later terrain edit would make the surrounding cut / fill extreme, the runtime still keeps
  the committed client fixed; later geometry variants may change how that earthwork is represented,
  but terrain brushes must not silently move the client

### 7. Extra Geometry Uses Chunk-Local Client Caches And One Shared Query Order

The extra geometry layer now has one required cache and query model.

Required cache and query contract:

- client-owned top surfaces remain owned by the client that authored them
- client-owned local earthwork geometry is stored in chunk-local caches aligned to terrain chunk
  boundaries
- when a client rebuilds, it must partition the produced local geometry into touched terrain chunks
  instead of keeping one whole-world monolithic mesh
- renderer uploads for that local geometry must stay bounded to the touched chunk caches
- visible-world queries must resolve in this order:
  1. client-owned top surface
  2. client-owned local earthwork geometry for the touched chunk
  3. visual terrain
  4. source terrain only for terrain-only APIs
- the terrain renderer must not continue drawing visual terrain in texels already owned by a
  client-owned top surface or client-owned local earthwork geometry
- terrain omission must be expressed in the terrain mesh topology itself for grounded `Standard`
  roads; shader discard or alpha masking must not be used to hide missing clipped topology
- world-surface picking and visible-surface queries must not require a whole-world scan across every
  engineered-ground client
- rebuilding local earthwork geometry or stitched terrain patches must not force whole-world terrain
  restamps or whole-world render uploads

## Roads-First Rewrite Status

The roads-first rewrite is live for ordinary grounded-road terrain ownership. The piece / profile
carrier owns the road footprint, and Rust now generates the road-touched terrain patch topology that
cuts the terrain to the outer sidewalk / shoulder edge.

For the roads-first rewrite, the following are deterministic and implemented:

- the current corridor-sheet prototype is retired as the target solution
- the logical graph and the visible road carrier are split: graph owns connectivity, while the
  visible road system owns deterministic geometry pieces
- the minimum required visual piece set is `Span`, `Bend`, `Terminal`, and `JunctionN`
- roads compile explicit top-surface ownership and exact footprint constraints near the terrain seam
- tie-in boundaries are specified to use dense local sampling at a maximum `2 m` longitudinal
  spacing
- committed clients stay fixed under later terrain-authoring edits
- local road ownership uses chunk-local rebuild boundaries and one explicit visible-world query
  order
- road-touched terrain patches are currently generated in Rust as baked terrain `ArrayMesh`
  payloads whose boundary vertices reuse the road / sidewalk seam height
- ordinary grounded-road tie-ins now include grade-limited guide rings generated from final
  road-owned footprint loops inside `RoadSurfaceSystem`; guide rails are constrained only for a
  single clean convex loop, while concave junction mouths and multi-loop footprints stay sample-only,
  and ordinary `Standard` / node footprint sources never emit retaining-wall mesh
- the Spade CDT terrain-patch hardcut in [`roads.md`](roads.md) is now the live
  road-touched terrain patch path; the provisional seam-strip / cell-triangle hybrid has been
  removed rather than polished further

The following are current hardcut implementation rules:

- grounded `Standard` roads send grouped road footprint loops into road-touched terrain patches
  and those patches return baked mesh vertices / normals / UVs generated by Rust
- the Godot terrain renderer no longer performs terrain-road polygon clipping; it only uploads the
  baked mesh or the normal rectangular terrain mesh
- grounded `Standard` roads do not render an ordinary visible closure strip, seam carpet, or second
  support mesh; the CDT terrain patch mesh is the seam carrier
- `Terminal`, `Bend`, and `JunctionN` node pieces must compile final asphalt, curb / shoulder, and
  sidewalk regions as band-owned surfaces whose mouth seams, material seams, and outer footprint
  edges survive through `i_overlay` ownership cleanup and local Spade CDT triangulation
- bend / junction full-roadbed closure carriers are legacy debt, not the target rendered ownership
  path; explicit curb / shoulder and sidewalk carriers must claim their seams and heights, and
  remaining non-road residuals must be rejected or debug-counted instead of filled by a generic
  closure carrier
- node-piece terrain clips and local earthwork / skirt roots must be extracted from the canonical
  final band-owned top mesh. If the final owned-region outline introduces boundary vertices, those
  vertices must be inserted into the canonical node arrangement and rendered top mesh before CDT
  input is built. A point that is merely covered by an existing top-surface triangle is not enough
  for export; non-explicit boundary vertices and sampled boundary heights are geometry errors. A
  node path must not reconstruct a later outer boundary loop that contains vertices outside the
  canonical rendered node top-surface coverage.
- road / earthwork seam math shares the road-surface `RoadVec2` / `RoadVec3` representation and
  explicit quantized keys through the owning Rust stages; Godot vectors remain bridge, upload, and
  debug payloads rather than authoritative seam identity.
- visible water patches now use depth-owned local topology instead of full-patch planes; road-touched
  water meshes receive the same grouped road footprint loops after a network edit and suppress
  cells that touch the outer-loop-minus-hole road-owned area, so water is no longer allowed to
  render under grounded road-owned asphalt, shoulder / curb, or sidewalk
- clipped patch topology is validated against flat, diagonal, sloped, bend, terminal,
  `JunctionN`, production authored DEM road cases, and a compact baked Kuopio imported-DEM
  fixture, including steep ordinary tie-ins without wall teeth, raised ridge / valley terminals
  and bends, steep multiway junctions, convex constrained grading-envelope rails, concave
  junction-footprint sample-only grading envelopes, edit-order-stable emitted terrain-CDT topology,
  bridge-midspan, and tunnel-portal structural stamping
- imported DEM JunctionN clipping now reports and removes road-owned internal chords from the
  terrain seam constraint set only when both sides of the exact constraint classify as road-owned
  against the final footprint; exposed seam constraints remain hard CDT constraints with source
  provenance
- terrain suppression / masking is not accepted as the live seam solution; road-shaped terrain holes
  must continue to be produced by terrain mesh topology
- no current `ROAD-01` blocker remains for real-world DEM validation; any future terrain closure
  variant beyond the structural retaining-wall path should be tracked as a new explicit earthworks
  item

That means the building-site target below extends the same clipped-topology ownership model to
area clients; it is not a substitute for the road-to-terrain boundary that already ships.

## Building-Site Target (`EARTH-02`)

`EARTH-02` is the live first-pass target for building yards, pads, and authored site surfaces.
Buildings register a required flat support footprint at construction start, and visual terrain is
clipped through the same terrain-patch / CDT ownership model used by grounded roads.

### 1. The Runtime Site Support Footprint Is Protected

The v1 runtime building-site authoring envelope is the occupied lot rectangle:

- for zoned private buildings, the claimed zoning parcel area occupied by the asset's
  `lot_width_cells` and `lot_depth_cells`
- for explicit buildings, the explicit placement footprint defined by the asset and allocator

The required flat support footprint is the area that terrain may never enter after placement. Until
the asset schema exports explicit support extents, runtime derives this footprint conservatively
from the occupied lot rectangle. Meshes, entrance landings, driveway/loading/parking anchors, and
authored hard surfaces must fit on that flat support plane. Terrain, road, and apron tie-ins start
outside the support footprint; they must not cross back through it. Authored `[[site_surfaces]]`
polygons do not define separate terrain-ownership cuts. They define visible/material regions on the
selected site plane, such as asphalt, concrete, paved yards, or walkways.

Zoning is not an engineered-ground client:

- creating, previewing, dragging, resizing, or rezoning parcels must not alter source terrain,
  visual terrain, road surfaces, or site surfaces
- terrain integration begins only when allocator placement accepts a building on a parcel

### 2. Asset Editor Is WYSIWYG For The Flat Lot

The asset editor previews the asset on a flat local lot with the authored lot dimensions, not
on an abstract infinite grid. The authoring view should show:

- the lot boundary as the current conservative runtime support footprint
- the building mesh parts on that flat plane
- `entrance`, `driveway`, `parking`, and `loading_bay` anchors in the same local coordinate space
- authored `[[site_surfaces]]` polygons as material regions on top of the flat lot

This editor view is WYSIWYG for local layout and materials. It is not responsible for choosing the
world height of the site; runtime placement still chooses that height from road / driveway /
neighbor context.

### 3. Placement Chooses One Flat Site Height

Every accepted building site has exactly one support height:

```text
building.support_height_m = site_plane_y
```

For normal roadside buildings, height selection is deterministic:

1. Build candidate connection points from authored `driveway` anchors.
2. Convert anchors to world space using the accepted building transform.
3. Project/query each candidate against the claimed road edge and side.
4. Sort valid candidates by distance to the frontage edge, then by authored anchor order.
5. Select the first valid driveway as the primary driveway.
6. Sample the existing visible road/world surface at that connection.
7. Use that height as the flat site plane.
8. Validate remaining driveway anchors against the chosen site height.
9. Validate touching neighboring building sites against the chosen site height.

If an asset has no driveway anchors, the fallback connection is the parcel frontage midpoint on the
claimed road side. If an explicit non-road service asset has no road connection by design, it may
fall back to source terrain at the placement center. If no valid connection exists for a road-bound
building, placement is rejected with a diagnostic.

The `main` entrance does not choose the site height. It sits on the flat site plane chosen from the
road / driveway / explicit-site rule above.

After the support height is selected, placement validates the required flat support footprint:

- the support footprint must be non-degenerate
- deterministic edge and corner samples must find terrain or visible road surface outside the
  support footprint within the building-site apron envelope
- the height delta from the support plane to that tie-in target must fit the shipped terrain
  tie-in slope budget
- if no legal tie-in exists, placement is rejected before the building is committed

### 4. Multiple Driveways And Neighbor Sites Are Validation Inputs

Multiple driveways are allowed, but they do not create multiple site heights.

Required rule:

- primary driveway: closest valid driveway to the frontage edge; tie-break by authored anchor order
- secondary driveways: must be compatible with the chosen flat site height
- v1 rejection threshold: any secondary driveway whose sampled connection height differs by more
  than `0.35 m` rejects placement

Neighboring placed sites are fixed clients:

- existing neighboring site heights must not be moved, averaged, or repaired by a new placement
- if a new site touches an existing site and their heights differ by more than `0.10 m`, reject the
  new placement in v1
- if the height difference is `0.10 m` or less, the future implementation may merge/clean the shared
  seam deterministically without changing either committed height

When the selected road connection height conflicts with a neighboring fixed site, the placement is
rejected. The runtime must not average road and neighbor heights.

### 5. Terrain Integration Uses The Same Topology Ownership As Roads

The placed building site replaces visible terrain inside the required flat support footprint.

Required runtime behavior:

- source terrain remains unchanged
- the support footprint is clipped out of visual terrain topology
- site top surfaces render on the flat support plane
- terrain outside the support footprint stitches to the site boundary through the same Rust-owned
  terrain-patch / CDT ownership model used by grounded roads
- boundary vertices at the site seam reuse the site plane height, not resampled source terrain
  heights
- visible-world height and ray queries see road surfaces first, building-site top surfaces second,
  and source/visual terrain after those owned surfaces
- no shader mask, z-bias, loose overlay mesh, terrain alpha, or hidden second support plane may hide
  missing topology

Removal rule:

- removing a building removes its site client
- visual terrain rebuilds from source terrain plus remaining roads and building sites
- v1 does not leave persistent foundation, asphalt, or yard remnants after removal

Construction rule:

- the flat site client is registered at construction start, before the rising building animation
  finishes
- later construction visuals are drawn on the already committed site plane

### 6. Determinism And Performance

The building-site implementation must reuse existing ownership and indexing systems:

- `BuildingAllocator` placement lifecycle and parcel claims
- zoning parcel geometry for lot footprints
- `Building.support_height_m` for the chosen flat support plane
- `RoadSurfaceSystem` / visible-world queries for road connection height
- terrain patch / CDT clipping infrastructure already used by grounded roads
- building chunk indices for nearby fixed-site adjacency checks
- asset `[[anchors]]` and `[[site_surfaces]]` schemas for local layout metadata
- building-site render buffers are rebuilt from allocator revisions, not every frame

Deterministic ordering rules:

- driveway candidates sort by distance to frontage edge, then authored anchor order
- neighboring sites sort by stable building index / parcel id before validation or merge handling
- dirty terrain/site patch rebuilds run in canonical chunk order
- no unordered hash iteration may decide accepted height, rejected height, or emitted seam topology

## Later Additions

The following items remain intentionally open as later extensions of the same subsystem.

### 1. When Do Later Geometry Variants Replace The First Closed Slope / Closure Mesh?

The first shipped geometry variant is intentionally only the closed slope / closure mesh.

Open decision:

- which deterministic thresholds or authored classes should replace the first closed slope /
  closure mesh with retaining walls, cliff faces, or other later variants when the cut / fill case
  becomes too extreme for the simple slope solution

## Shared Target

### 1. Local Earthwork Geometry Becomes Owner-Controlled

The longer-term shared target is for engineered-ground clients to own closed local tie-in geometry
near their footprints instead of asking the terrain heightfield to represent every cut and fill
detail.

That means:

- top support surface remains client-owned
- side slopes, embankments, retaining faces, closure faces, or local skirts become client-owned
  geometry
- terrain becomes the far-field ground that the local geometry ties back into

### 2. Roads And Building Support Footprints Use The Same Rules

This shared subsystem must work for both:

- linear corridor clients such as roads
- area clients such as flat building-site support footprints

The client shape differs, but the shared ownership model is the same:

- support surface
- earthwork envelope
- tie-in boundary
- query precedence
- chunk-local invalidation

Placed-client rule:

- once a road or flat foundation pad is committed, later terrain brushes reshape authored ground
  and derived earthworks around that placed client
- the placed client surface moves only when the player explicitly edits that client itself

### 3. Density Changes Must Be Measured, Not Assumed

Changing terrain density is a world-storage and performance decision, not a visual tweak.

Deterministic conclusion:

- denser terrain may improve visual blend quality around engineered ground
- denser terrain is not allowed to become the sole required earthworks fix
- any future move from `10 m` to `5 m` or finer terrain must happen only after the chunk-local
  terrain / water render-boundary split defined in [`terrain.md`](terrain.md) is live
- any future density move must be justified by deterministic characterization tests against the
  same world-space engineered-ground cases on that split render path

The selected non-density seam representation is the Spade CDT terrain-patch hardcut in
[`roads.md`](roads.md); density changes remain a later quality / cost decision.

### 4. Transition Path Preserves Ownership Even If The Terrain Runtime Changes

The shared target is an ownership contract first and an implementation choice second.

Required transition path:

- keep `source terrain` as the authoritative authored ground
- keep a derived far-field terrain surface as the renderer upload source beyond engineered-ground
  tie-in boundaries
- split terrain and water render/upload work into chunk-local windows before treating denser
  terrain as a baseline runtime choice
- add client-owned local earthwork geometry caches near engineered-ground footprints
- extend visible-surface queries to consider:
  1. client-owned top surface
  2. client-owned local earthwork geometry
  3. visual terrain
  4. source terrain only for terrain-only APIs

This means the long-term earthworks architecture may extend or rewrite the current terrain runtime,
but it must preserve the same client-ownership rules and must not regress back to a terrain-only
model near engineered-ground footprints.
