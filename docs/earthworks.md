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
- buildings, using a fixed flat structural foundation, entrance landings and usable yard interior;
  authored `[[site_surfaces]]` partition that support and the graded lot-edge tie-ins by material
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
  - physical road profiles remain distinct from hard-pinned junction solver controls; clipping,
    splitting and support materialization must not substitute those controls as visible heights
  - node/span footprint ownership does not define an elevation plateau: junction approaches may
    change elevation inside node-owned surfaces, using the final `RoadEditPlan` physical profile
  - bends preserve their fitted grade; crossing cores and material seams use matching heights,
    and straight footprint boundaries retain any non-linear elevation supports
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
  owned by the site / engineered-ground path and must remain sample-only around the hard support
  footprint
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
  - building-site apron guides are always soft guide samples; the site support footprint is the hard
    CDT boundary, and apron guides must not add rail constraints across roads or neighboring sites
  - Spade CDT faces are classified after triangulation against the final road-owned footprint;
    ordinary non-seam faces may use centroid ownership, but faces carrying a road constraint edge
    classify the exact seam side so narrow concave tie-ins are not lost to centroid-only ownership
  - emitted terrain triangles must preserve the CDT constraint edges at the road seam and must not
    cross a road-owned footprint loop
  - CDT triangulation failures are hard errors in debug output and must not fall back to cell
    subtraction, seam carpets, closure strips, water, or shader masks
  - all local CDT windows in one refined render-patch generation are one atomic replacement; if any
    window fails, the entire generation is non-renderable and the renderer keeps the last valid
    clipped patch rather than filling the failed window from raw terrain
  - a building-site loop cannot satisfy road clipping: a patch already owned by grounded roads must
    retain nonempty authoritative road sources and road loops on every refined generation
  - the exporter may omit individual pathological ordinary terrain faces before final validation,
    record the omitted count, and keep the remaining clipped baked patch active only when its final
    status is `ok`. This containment is not a seam-closure fallback and remains visible in logs. If
    the final output is still `pathological`, the complete engineered generation is non-renderable
    and the renderer retains the previous terrain/road pair; raw terrain never replaces it
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
- the planned longitudinal profile preserves a local source-supported grade envelope; endpoint
  slope alone must not erase an interior hill or valley. Junction finalization keeps a supported
  hillside corridor authoritative, and terrain compilation consumes that same final profile
- when multiple committed road-owned top surfaces overlap at the same XZ, terrain support clearance
  uses the lower top-surface envelope so terrain remains below every visible road-owned face; this
  does not change visible-surface picking, which still resolves the topmost rendered surface
- away from junction transitions, committed road sections are laterally flat and sidewalks use
  `road_height + curb_step`; near crossings, lateral offsets follow the shared solved crossing
  plane relative to the physical centerline, without replacing its longitudinal height
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

Road edit plans retain local structural stamp writes, road/site CDT tiles and joined patch buffers.
Preview compilation reads source terrain and existing indexed contributors without modifying the
resident visual grid. A local copy-on-write overlay applies source resets and stamp writes in the
same interleaved sorted-chunk order as commit. Planned bordered textures, CDT boundary/background
samples, road grading and building-site grading all read that final visual result. Explicit chunk
coverage preserves reset-to-default samples; untouched chunks still read the pinned resident grid.
Work/storage is bounded by touched storage chunks and stamp samples, not total map size. Commit
checks exact road/site/source/visual dependencies, coverage, patch snapshots and ownership before
adopting the entire planned mesh batch. Structural stamps separately require matching source/grid
dependencies and ordered support triangles. Complete patch/tile/seam buffers retain Arc identity;
only payload generation metadata changes. A different query margin must select the identical
road/site contributors. Mismatches roll back, rather than replacing ready geometry through fresh
tile or patch assembly. Plan-less subsystem callers retain the cold compiler.
Ordinary grounded roads are CDT-only, not height-stamped. The worker captures local building-site
footprints/support heights and paving regions using the prepared allocator index, then compiles grading off-lock
against planned roads plus unchanged resident owners. Neither capture nor status checking may
rebuild the city index; unprepared inputs remain provisional/pending. Exact local site comparisons
detect added/removed footprints and height/ID/material changes. Planned building grading, coverage and final
quality validation govern readiness, with exact post-topology checks at adoption. Eligible road
previews now stage the complete local patch batch with canonical unlifted road meshes using the
production exporters/builders. Planned ownership can restore regular terrain where a cutout
disappears. Ownership is resolved before CDT assembly using the same cached per-loop grading
calculation as live patches, combining retained owners with planned replacements. Superseded
owners contribute only to the dirty envelope, not final ownership. Padded query hits cannot claim
ordinary patches; missing clipping inputs cannot release genuinely owned patches. Captured sites
use the live footprint-overlap rule and grading margin. Patch clip queries then use the resulting
production margin. Work remains bounded to affected patches and indexed local owners: O(local
query/ordering work + unique local loops × affected patches + uncached grading samples), with
cached grading evaluated once per loop and patch compilation parallelized through Rayon.
Structural ownership/clip discovery now consumes the final ordered resets/writes before CDT
assembly. Resident per-loop caches depend on source and visual-only revisions; changed overlays
use a local cache without replacing live entries. Unchanged overlays retain resident cache reuse.
Site dependencies are captured using each patch's final clip-query margin, not a fixed margin.
Visual-only changes stale the candidate. Incomplete or missing resident patches still defer
the paired display; isolated and connected roads use the same path. Temporary meshes inherit resident
patch visibility and restore the exact original resources on cancellation or invalidation before
patch updates/recycling/LOD/reset. No terrain samples, payload caches or acknowledgments change.
Road splits/attachment repair preserve authored site pose and support height; the captured site
set is checked again after topology adoption. Complete ready plans publish their existing terrain
buffers directly after road, visual-sample, coverage and ownership checks, without a second CDT
assembly. Failure restores exact local visual samples as well as graph/split references. A staged
road/terrain pair may now show readiness; road-only or missing-resource displays remain explicitly
provisional (`terrain preview pending`). See the readiness/adoption contract in `roads.md`.

The local compiler frontier includes every edge dirtied by final profile solving and every affected
node, not only the inserted stroke. This same frontier drives preview products and commit's old/new
earthwork coverage. Grading guides sample a common terrain field; retained road seams own their
contact heights. Shared tile sides use the uncut local grading halo and keep canonical boundary
samples. Out-of-core samples, false beyond-endpoint constraint intersections and independently
heighted guide slivers are rejected or corrected at input construction, never concealed by dropping
terrain faces. A nonzero omitted-face count prevents readiness and publication. Contributor/window
validation authorizes buffer composition only; final acceptance also requires present, valid render
buffers for every clipped patch. Cold compilation, cache insertion and planned adoption share that
gate; an ordinary loop-free patch does not require a clipped mesh.

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

The current runtime ships roads and level building/yard pads with graded edge tie-ins as
engineered-ground clients.

That means:

- `RoadSurfaceSystem` owns the roadbed support surface
- `BuildingAllocator` owns required flat building-site support clients registered at construction
  start
- authored `[[site_surfaces]]` polygons expand the level support hull inside the lot's tie-in strip
  and partition the final support/terrain triangles by material; only outside the support pad do
  they follow the compiled road/terrain tie-in, not an independent flat sheet
- live gameplay must not render authored yard surfaces as loose decal / overlay meshes over terrain;
  their geometry must be the shared support/terrain geometry, without offset or overlapping covers
- grounded `Standard` roads do not stamp their footprint or ordinary outer margin into visual
  terrain; road-touched terrain patches are stitched to the road-owned outer edge
- bridges do not stamp terrain earthworks; raised spans and elevated terminals render structural
  concrete support instead of terrain fill
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
- building-site clients now ship as fixed flat in-lot pad surfaces with deterministic sample-only
  perimeter tie-in guides; richer foundation meshes remain later work

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

The required flat support footprint is the area that terrain may never enter after placement. The
occupied lot rectangle remains the reservation / overlap envelope, while runtime derives the
required support footprint from mesh parts, entrance landings, and authored yard material regions.
Yard support vertices are clamped to the lot rectangle inset by **2 m** on all sides before joining
the existing convex support hull. For very small lots, the inset is capped at half each half-extent.
This makes the usable yard level while reserving road, neighbor and rear-ground transition strips.
Structural mesh support uses the imported LOD0 bounds, transformed with the same position, yaw,
scale and pivot convention as rendering. It is not expanded by a fixed-radius estimate or reduced
by the yard inset. Godot imports these bounds once per distinct mesh path during pack loading;
the asset registry retains them for pure Rust site derivation, with no per-building file I/O.
Buildings with an unimportable mesh are skipped with a pack warning, not assigned invented bounds.
An authoritative pack refresh rebuilds site clients and entrance caches, advances site revisions,
and invalidates the old/new terrain envelopes. Load regenerates sites from the current asset bounds.
Part transforms now share the editor's positive-Y yaw (+X toward -Z), including scale and pivot;
rendering also reuses the allocator's frontage basis. CDT revision **13** invalidates derived
terrain products made with the former opposite-sign part yaw. Authoring files are unchanged.
Driveway, parking and loading anchors
alone do not expand the pad. Road-facing entrance support stays behind the exact road /
sidewalk boundary by a deterministic clearance so the frontage strip is a tie-in/apron region
instead of a second hard terrain-CDT loop sharing the road seam. Terrain, road, and apron tie-ins
start outside the support footprint; they must not cross back through it, and apron guides remain
sample-only so neighboring site or road windows cannot gain extra hard CDT rails. Authored
`[[site_surfaces]]` polygons do not define separate terrain-ownership cuts. They define
material regions on the final support triangles, such as asphalt, concrete, paved yards, or
walkways. Outside the level support pad, those triangles can rise or fall to match the whole frontage.
Partitioning preserves triangle planes and area; road boundary heights remain authoritative.
Authored `y_m` is a flat-editor preview offset, not a gameplay displacement or raised-platform contract.

The site CDT cutout uses the exact flat-support boundary and height, also used by the foundation
mesh and ground queries. The former 2 cm inward cutout offset is removed: it let the terrain
transition begin inside the flat pad and disagree with its height query at the edge. Rust/Godot
CDT contract revision **13** includes that boundary fix and the shared part-transform correction.
Cars and walkers use this same surface
ownership during entry and exit; see [`entrance_and_exit.md`](entrance_and_exit.md#render-height-ownership).

Zoning is not an engineered-ground client:

- creating, previewing, dragging, resizing, or rezoning parcels must not alter source terrain,
  visual terrain, road surfaces, or site surfaces
- terrain integration begins only when allocator placement accepts a building on a parcel

### 2. Asset Editor Is WYSIWYG For The Flat Lot

The asset editor previews the asset on a flat local lot with the authored lot dimensions, not
on an abstract infinite grid. The authoring view should show:

- the lot boundary as the reservation / overlap footprint
- the derived required flat support footprint that runtime keeps terrain out of
- the building mesh parts on that flat plane
- `entrance`, `driveway`, `parking`, and `loading_bay` anchors in the same local coordinate space
- authored `[[site_surfaces]]` polygons as material regions on top of the flat lot

This editor view is WYSIWYG for local layout and materials. It is not responsible for choosing the
world height of the site; runtime placement still chooses that height from road / driveway /
neighbor context.

### 3. Placement Chooses One Flat Foundation Height

Every accepted building has one support height shared by its structure and usable yard interior:

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
7. Use that height as the preferred plane, not an immutable height constraint.
8. Intersect driveway grade intervals with the feasible height intervals of the existing
   perimeter/corner grading rays. Choose the feasible height closest to the preference;
   equal-distance ties choose the lower height. The building and usable yard remain level.
9. Validate remaining driveway connections, touching fixed neighbors and all terrain tie-ins
   before accepting the solved plane. Never move the road or neighboring committed pads.

If an asset has no driveway anchors, the fallback connection is the parcel frontage midpoint on the
claimed road side. All currently implemented placement modes are road-bound: an explicit service
or industry asset must not silently substitute source terrain when the road surface is missing.
Placement is rejected with a diagnostic. A future genuinely non-road mode requires an explicit
eligibility contract rather than checking only `placement_mode = "explicit"`.

The `main` entrance does not choose the site height. It sits on the flat site plane chosen from the
road / driveway rule above.

After the support height is selected, placement validates the required flat support footprint:

- the support footprint must be non-degenerate
- deterministic edge and corner samples must find terrain or visible road surface outside the
  support footprint within the building-site apron envelope
- the height delta from the support plane to that tie-in target must fit the shipped terrain
  tie-in slope budget
- driveway-to-pad height deltas must fit the same 50% maximum grade over their available XZ run
- grading probes include the full existing envelope endpoint (20 m on 10 m terrain), not only
  the interior power-of-two rings; validation and emitted grading guides use the same distances
- if no legal tie-in exists, placement is rejected before the building is committed

### 4. Multiple Driveways And Neighbor Sites Are Validation Inputs

Multiple driveways are allowed, but they do not create multiple site heights.

Required rule:

- primary driveway: closest valid driveway to the frontage edge; tie-break by authored anchor order
- secondary driveways: must be compatible with the primary road connection and the solved pad
- v1 rejection threshold: any secondary driveway whose sampled connection height differs by more
  than `0.35 m` from the primary connection rejects placement; all connections must also satisfy
  the driveway-to-pad grade intervals

Neighboring placed sites are fixed clients:

- existing neighboring site heights must not be moved, averaged, or repaired by a new placement
- if a new site touches an existing site and their heights differ by more than `0.10 m`, reject the
  new placement in v1
- if the height difference is `0.10 m` or less, the future implementation may merge/clean the shared
  seam deterministically without changing either committed height

When the solved pad conflicts with a neighboring fixed site, placement is rejected. The runtime
must not average or move existing road and neighbor heights.

### 5. Terrain Integration Uses The Same Topology Ownership As Roads

The placed building site replaces visible terrain inside the required flat support footprint.

Required runtime behavior:

- source terrain remains unchanged
- the support footprint is clipped out of visual terrain topology
- building and usable-yard top surfaces render on the flat support plane; edge/apron paving
  partitions the final terrain mesh
- terrain outside the support footprint stitches to the site boundary through the same Rust-owned
  terrain-patch / CDT ownership model used by grounded roads
- boundary vertices at the site seam reuse the site plane height, not resampled source terrain
  heights
- visible-world height and ray queries see road surfaces, foundation tops, and current compiled
  terrain triangles; the visual heightfield is the fallback outside compiled engineered ground
- no shader mask, z-bias, loose overlay mesh, terrain alpha, or hidden second support plane may hide
  missing topology

Removal rule:

- removing a building removes its site client
- visual terrain rebuilds from source terrain plus remaining roads and building sites
- v1 does not leave persistent foundation, asphalt, or yard remnants after removal

Construction rule:

- the flat site client is registered at construction start, before the rising building animation
  finishes
- construction pads and scaffolding reference the committed site plane; the unfinished private
  building model may rise from below it without changing the authoritative support or terrain

Shared placement lifecycle:

- Zoned, service and explicit industry assets use the same support preparation, site installation,
  revision/invalidation finalizer and pending-site terrain update mechanism. Zoning and economic
  eligibility remain separate from geometry.
- Site creation, redevelopment and removal record their bounds inside allocator mutators, not in
  caller-specific demand reports. Simulation placement/cadence entry points consume the same
  outbox to invalidate local terrain payload generations.
- Road attachment projection is nearest in XZ but stores the 3D physical arc-length fraction used
  by road sampling and save/load. Mixing 2D projection distance with 3D sampling moves sites along
  sloped roads and is forbidden.
- This is one geometry/update contract, not a new atomic building/terrain renderer transaction.
  The reported floating/buried-house screenshot scene still needs exact-save reproduction;
  construction animation and delayed/failed terrain publication must be distinguished from a
  wrong final support plane.

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
- foundation material partitions are cached per derived site; terrain material partitions are built
  off-thread per affected patch using the existing polygon CDT and an R-tree over local material
  triangles. No city-wide building query or new global spatial index is introduced.
- exact paving inputs participate in terrain-buffer reuse and RoadEditPlan site dependencies;
  a material-only edit can reuse geometric CDT windows but must rebuild the final material partition
- point queries allocate nothing and use the existing patch/64 m CDT tile grid, then the road
  compiler's existing triangle lookup grid (4 m initial cells, at most 256 cells per tile).
  Immutable tile-local indices are built off-thread and reused with unchanged CDT windows.
  Ray queries visit intersected patch rows/columns and tiles, never compile terrain in a query.
- Height/picking queries require matching generations and the same complete terrain acceptance
  checks as rendering. Finite vectors alone cannot authorize conflicted or pathological output.
- In-place level changes validate the new asset support at the existing fixed site height, through
  the same local solver; unsupported changes reject before modifying the building or its terrain.
  Paving regions carry material/layout only, not an independent or duplicated elevation field.
- asynchronous site-terrain rebuilds copy only chunk-indexed sites overlapping the affected patch;
  site grading samples the bounded road-surface ownership index and must not scan all buildings or
  all compiled roads
- creating, changing, or removing a site invalidates only intersecting terrain payload revisions;
  it must not invalidate unrelated patches or permanently add a site-only patch to road ownership
- road and building-site ownership remain distinct authoritative sets plus one engineered union;
  raw terrain is forbidden for the union, and every refined Rust payload carries that provenance so
  renderer safety cannot depend on a later Godot membership query

Deterministic ordering rules:

- driveway candidates sort by distance to frontage edge, then authored anchor order
- neighboring sites sort by stable building index / parcel id before validation or merge handling
- dirty terrain/site patch rebuilds run in canonical chunk order
- no unordered hash iteration may decide accepted height, rejected height, or emitted seam topology

### 7. Initial Graded-Yard Regression Coverage (2026-09-10)

Historical verification for the initial gap fix, before the level-yard follow-up below. These
measurements validate their recorded binary, not the later footprint change.

The initial frontage-gap fix kept only mesh support and entrance landings level. Yard materials partition
the already solved foundation/terrain triangles, so they cannot leave a separate elevated or sunken
yard sheet beside the sidewalk. The renderer, height queries, pedestrian ground sampling and picking
consume that geometry. RoadEditPlan retains the same paving inputs and final material buffers through
preview and adoption. A second reproduced defect, an inside-boundary CDT guide within a micrometre
of the tile side at a conflicting height, now joins the exact boundary within the existing 1 mm
canonicalization tolerance; outside halo samples remain outside.

The later shared-side correction (CDT revision 12) applies this welding symmetrically before
containment. Samples within the side tolerance join the exact boundary in both tiles; more distant
halo samples remain excluded. Otherwise the neighboring tile can discard the same grading sample
and bridge its elevation with an unsplit terrain edge, reopening a narrow gap inside a paved apron.

The reference `benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite` is unchanged:
SHA-256 `e1d7e0baf4eefd23293f0a770345f50e455815124c0a0920aefe10cc7a1033c0`.
It still contains 70 roads and no saved buildings or parcels. The companion `building-site.toml`
provides a portable 20 × 20 m residential lot and paved frontage; tests register it in memory and
use production zoning/placement without installed user assets. Its SHA-256 is
`70d99572341f7be954cb565b40467a2c32ee0ef6b2981c3cd7f147081443c300`.

`nodes::sim::core::tests::building_site_terrain` covers:

- Six adjacent yards on each of +15% and −15% roads, on both sides; foundation triangles stay flat.
- Eight recorded placements at the midpoints of saved edges 3, 9, 15 and 18, on both sides.
  Four are accepted; `(9, +1)`, `(15, −1)`, `(15, +1)` and `(18, +1)` deliberately retain
  `SiteSupportTieInInvalid`. The test asserts these outcomes instead of moving rejected lots.
- 160 interior samples per accepted yard compare rendered paving, world height and picking within
  3 mm, plus 19 frontage samples within 5 cm of the road at a 25 mm inward offset.
- A later T-road edit beside the synthetic yards retains exact ready-preview/adopted buffer identity
  and repeats yard coverage checks.
- Asphalt-to-concrete changes reuse unchanged CDT windows but rebuild final material buffers.

These are reconstructed regressions, not a reproduction of the screenshot's exact save and assets.
The tests do not promise arbitrary terrain/lot compatibility: secondary-driveway and adjacent-pad
height conflicts remain placement rejections. No change to those acceptance thresholds was needed.

Verification commands, run from `rust/` unless a repository-root path is shown:

```bash
cargo test --release --lib
cargo build --release
cargo doc --no-deps
RAYON_NUM_THREADS=24 cargo test --release --lib populated_road_plan_scaling -- --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 cargo test --release --lib populated_paved_road_plan_scaling -- --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 cargo test --release --lib graded_yard_height_query_benchmark -- --ignored --nocapture --test-threads=1
# Repository root, after deploying the release library to godot/bin:
godot --headless --path godot --log-file /tmp/metrum-yard-godot-engine-indexed.log --script res://tests/road_junction_preview_test.gd
xvfb-run -a godot --display-driver x11 --path godot --rendering-method gl_compatibility --log-file /tmp/metrum-yard-rendered-engine-indexed.log --script res://tests/road_junction_preview_test.gd
```

Fresh Rust result: **1,640 passed, 5 intentionally ignored measurements, 0 failed**, including
the unchanged Kuopio road replay's 39 strokes and 158 profile checks. The three commands above
explicitly run the relevant ignored measurements. Release build and rustdoc emit no warnings.
Both Godot runs pass (Godot 4.7.2; Xvfb uses llvmpipe OpenGL 4.6/Mesa 26.2.2). Godot's
regression checks the actual packed vertex/color upload, rejects incomplete material tags, and
under Xvfb renders a graded paving triangle through the real terrain shader; headless mode only
checks the payload, not rasterized shader output.
Deployed release-library SHA-256:
`3432fe089e4ff108859a9942c61e6b404f77edc09861e0749aafa5befd644966`.

Performance acceptance uses sequential, unprofiled release runs on the same host, Rust 1.98.1,
24 Rayon workers, 100 measured plans per population level. Baseline is the unmodified source at
`91af8afca4eb2e9a493ab3bc7f2025a2b87195d9`; final is that source plus this graded-yard change.
Final test-binary SHA-256:
`a16c164aea5e752dc89be2ccc2a12a127de9c1ee95e3ac1ed041e20a6a1e8127`.
No profiler, Godot process or concurrent build ran during the timed measurement loops.

| Background buildings | Baseline bare compile p50 (ms) | Final bare compile p50 (ms) | Final paved compile p50 (ms) | Paved snapshot, once per edit (ms) |
|---:|---:|---:|---:|---:|
| 0 | 19.489 | 20.094 | 20.582 | 0.0065 |
| 1,000 | 19.257 | 19.481 | 21.000 | 0.0514 |
| 10,000 | 20.163 | 19.463 | 21.093 | 0.3674 |
| 100,000 | 19.758 | 20.083 | 22.493 | 3.4700 |

Only the bare columns are a matched before/after comparison. The new paved workload changes the
four fixed local sites to 20 × 20 m paved lots; it is not an old-build speedup comparison. Both
workloads keep the edited neighborhood fixed while background totals increase to 100,004 parcels,
600,024 agents and 391 roads. Local products match exactly across population levels. Final worker
p50 ranges are 20.40–21.37 ms bare and 21.77–22.75 ms paved; paved readiness p50 is 0.017–0.019 ms.
Repeated local planning shows no city-proportional growth. The existing one-time snapshot cost is
reported separately, not hidden inside or confused with repeated local compilation.

The complete world-height query over 76 accepted Kuopio apron points measures **953.4 ns p50**
(10 batches of 100,000 queries, placement/compilation untimed). An intermediate tile-wide triangle
scan measured 10,713.9 ns; it was replaced with the existing owner-local triangle index before
acceptance. The index adds no point-query allocation or query-time compilation.

Temporary raw logs: `/tmp/metrum-site-baseline-scaling-20260910.log`,
`/tmp/metrum-yard-full-rust-indexed.log`, `/tmp/metrum-yard-release-indexed.log`,
`/tmp/metrum-yard-scaling-indexed.log`, `/tmp/metrum-yard-paved-scaling-indexed.log`,
`/tmp/metrum-yard-query-indexed.log`, `/tmp/metrum-yard-godot-indexed.log`,
`/tmp/metrum-yard-rendered-indexed.log`, and `/tmp/metrum-yard-rustdoc.log`.
The measurements above remain here after those temporary
artifacts expire. These are local correctness/locality and renderer-contract checks, not an
uncapped gameplay FPS acceptance or a claim that every possible lot is buildable.

### 8. Level-Yard Follow-up (2026-09-10)

Visual feedback showed that terrain-conforming paving made a large industrial yard look like
a paved hillside. The current contract therefore includes authored yard interiors in the existing
level support hull. Only the 2 m lot-edge strips remain terrain-owned tie-ins; structural support
and entrance landings are preserved. Small-lot inset caps and hull ordering are deterministic.
This adds authored yard vertices to the existing O(V log V) per-site derivation, not a per-frame
solver or a city-wide query. The exact support footprint feeds placement checks, rendering, height
queries, terrain clipping and RoadEditPlan through the existing client. This follow-up introduced
CDT contract revision 7; later buildability and exact pad-boundary updates use revisions 8 and 9.

The regression `frontage_paving_has_a_flat_yard_and_separate_boundary_tie_ins` first failed on the
previous implementation because `(0, -6)` was outside the level pad. It now requires yard interior
coverage while excluding the frontage and side tie-in strips. Existing sloped-road and Kuopio
coverage checks additionally assert eight level interior samples per yard. The new 40 × 40 m
paved-lot regression checks an 11 × 11 level interior grid over both a 3 m hill and a 3 m depression,
on both road sides, plus the existing coverage/picking/frontage checks. The same cases at 14 m
relief must reject placement as `SiteSupportTieInInvalid`: the existing 50% tie-in slope budget
over the fixture's 8 m grading margin is not relaxed to force an impossible lot through.

The saved Kuopio map and portable asset fixture remain unchanged. The exact screenshot save was
not available, so the larger-lot cases are reconstructed geometry regressions, not a replay of that
particular factory. New placements still reject unsupported cut/fill; supporting arbitrary steep
sites or adding retaining structures is not implied by making the usable yard level.

Fresh follow-up checks: `cargo test --release --lib` passes **1,641 tests** (5 measurement tests
ignored). The same production Kuopio road replay, site coverage, material-cache reuse and later
RoadEditPlan adoption checks still pass. The following two release measurements were run
sequentially, with no concurrent assistant build/render test, using 24 Rayon workers:

```bash
RAYON_NUM_THREADS=24 cargo test --release --lib populated_paved_road_plan_scaling -- --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 cargo test --release --lib graded_yard_height_query_benchmark -- --ignored --nocapture --test-threads=1
```

The paved locality baseline was freshly rerun before this footprint change. Workload, host,
Rust 1.98.1 release settings and 100 samples per level match. Its binary is the initial version
recorded above (`a16c164a…`); the follow-up test binary SHA-256 is
`38172ab375c1f7fe7bfa8a43371a5a6ccd8d3e3a278259cb9bfb15500d28db1e`.

| Background buildings | Before compile p50 (ms) | Level-yard compile p50 (ms) | Level-yard worker p50 (ms) | Snapshot, once per edit (ms) |
|---:|---:|---:|---:|---:|
| 0 | 20.164 | 21.342 | 22.258 | 0.0107 |
| 1,000 | 19.611 | 22.137 | 22.782 | 0.0619 |
| 10,000 | 20.014 | 22.535 | 23.020 | 0.3508 |
| 100,000 | 20.124 | 21.478 | 22.163 | 3.5252 |

The observed compile median is 1.2–2.5 ms higher for the changed geometry; this is not a speedup
claim. Local products remain identical across population levels, and repeated planning remains
bounded to the fixed neighborhood rather than growing proportionally with background population.
Snapshot/setup cost remains separate. World-height queries over the same 76 frontage-apron points
measure **676.7 ns p50**, with compilation and placement excluded (10 × 100,000 queries).

Temporary follow-up evidence: `/tmp/metrum-flat-yard-before.log` (expected old-code failure),
`/tmp/metrum-flat-yard-full-rust.log`, `/tmp/metrum-flat-yard-baseline-scaling.log`,
`/tmp/metrum-flat-yard-scaling.log`, and `/tmp/metrum-flat-yard-query.log`.

`cargo build --release` and `cargo doc --no-deps` also pass without warnings. The headless and
Xvfb Godot commands from the preceding section pass against the rebuilt revision-7 library;
follow-up logs are `/tmp/metrum-flat-yard-build.log`, `/tmp/metrum-flat-yard-rustdoc.log`,
`/tmp/metrum-flat-yard-godot.log` and `/tmp/metrum-flat-yard-rendered.log` (same Godot/Mesa versions).
The release library was deployed to `godot/bin/libmetrum_rise.so`, SHA-256
`d203e6ef059e905f4fc3e140593862ad1d4862e8e0b6e727388c97a2cedd2936`.
Formatting and `git diff --check` pass. These checks retain the earlier caveat: the factory in
the user's exact saved scene has not been visually replayed.

#### Imported structural bounds verification (2026-09-11)

The small-shop frontage gap came from the former `max(scale * 0.75, 5.25 m)` mesh-support
half-extent, not from a malformed paving polygon. It expanded the level pad to the sidewalk on
a 10×10 m lot, removing the terrain-owned transition strip. Runtime now imports LOD0 bounds
through Godot, including descendant transforms, then applies the authored part transform during
shared site derivation. It preserves required structural support without that arbitrary expansion.
Rust/Godot CDT revision **11** prevents reuse of the older support/terrain products.

`small_commercial_pads_leave_a_watertight_sloping_frontage` covers a 10×10 m commercial lot on
both sides of ±4% roads, using portable model bounds. It checks level support, paved coverage,
height-query/render agreement and frontage heights, including an asset-footprint refresh.
`imported_support_bounds_apply_scale_yaw_and_pivot_without_padding` locks the part-transform
convention. Existing meshless geometry fixtures now state their structural boxes explicitly.

A read-only headless replay of the reported scene keeps the shop's pad at **98.75009 m**, reduces
its support area from **90 to 48 m²**, and reduces the maximum road/yard difference from
**293.8 to 6.5 mm** across 99 frontage probe pairs placed 25 mm either side of the boundary.
All eight requested terrain patches pass and all frontage probes have surface coverage. The
remaining nonzero probe difference measures grade over 50 mm, not coincident-vertex separation.
No user assets, save files or the checked-in terrain reference were modified.

Import is O(imported model data) once per distinct mesh path per catalog refresh; support
derivation remains O(P log P) in local support points, with four points per mesh part. It performs
no file I/O during placement or per-agent queries. A whole-catalog refresh rebuilds potentially
affected sites in parallel and invalidates their terrain; this is not per-cursor/tick work.

Unprofiled release measurements ran sequentially with `RAYON_NUM_THREADS=24`, after compilation.
The existing `graded_yard_height_query_benchmark --ignored --nocapture --test-threads=1` holds the
76 query points and portable geometry fixed, excluding setup/compilation (10×100,000 queries).
Three paired run medians were **576.2/596.6, 594.6/599.0, 569.4/639.4 ns** before/after; this is a
small increase, not a claimed speedup. Pack import alone took **29.45, 28.41, 27.90 ms**, versus a
**0.95 ms** pre-change manifest-only observation; those startup measurements are separate from
query timing. These do not establish populated-city catalog-reload performance.

Deployed release library SHA-256:
`849010f4a5bb4d96c3023a47cc78cd7f473ed49fc47c44259332f231d1cc6574`.
Query measurement binary SHA-256:
`fb911df62022790741a122b4c5b48296c52e2bec8aa6fabf7f08884922267577`;
baseline is the preceding corner-noding build described in `terrain.md`.
Artifacts: `/tmp/metrum-frontage-query-{before,after}-{1,2,3}.log`,
`/tmp/metrum-frontage-import-{1,2,3}.log`, `/tmp/metrum-frontage-{before,after}-coverage.log`.
Correctness logs: `/tmp/metrum-frontage-final-tests.log`, `/tmp/metrum-frontage-bridge.log`,
`/tmp/metrum-frontage-renderer.log`, `/tmp/metrum-frontage-rustdoc.log`.
Fresh final verification: `cargo test --release --lib` passes **1,658 tests** (9 measurements
ignored), including the asset-refresh assertion added after the query measurements. Headless
`road_junction_preview_test.gd` and `network_tool_chunk_renderer_test.gd` both pass; release build,
rustdoc, formatting and `git diff --check` are clean.

### 9. Shared Zoned / Explicit Site Contract (2026-09-10)

The placement-mode refactor removes duplicated service/industry preview and installation bodies,
moves revision/index/site invalidation into the common building installer, and replaces demand's
separate dirty-bounds result with the allocator's existing pending-site outbox. Level changes and
removal record old/new bounds inside their owning mutators. Private construction duration,
zoning occupancy, service eligibility and charging remain independent of support geometry.

Two inconsistencies were removed:

- Explicit roadside placement no longer falls back to source height when its road surface is
  unavailable. Missing frontage surfaces fail without inserting a building or dirtying its site.
- The shared road-frontage projection previously divided accumulated XZ distances by a 3D road
  length, then fed that fraction into 3D sampling. Uphill/downhill roads consequently shifted
  placements along the road. Projection now uses XZ for nearest-point/side selection and 3D arc
  distance for the stored attachment, matching sampling and save/load. This remains O(S) per
  candidate road's S segments with O(1) scratch space and no new allocations or spatial index.

`queued_asymmetric_houses_publish_graded_terrain` exercises six neighboring placements, on both
sides of a 15% road and on each of ±5% cross-slopes. It uses the asymmetric layout of the installed
family house as portable geometry, runs both minute-queued private spawning and explicit service
placement, rebuilds through the renderer's production terrain-input path after each placement,
and acknowledges exact terrain revisions before the next spawn. Both modes must produce identical
support footprints, foundation/material triangles and final terrain buffers. Interior probes also
require terrain to be excluded from the flat pads, rather than merely trusting a height query
that prioritizes the foundation. Existing Kuopio tests use the unchanged saved reference.

`sloped_frontage_projection_round_trips_the_shared_attachment` covers varying grades and bends on
both road sides. `explicit_roadside_site_rejects_missing_surface_without_raw_terrain_fallback`
locks the fail-closed contract. The yard test helper also requires placement itself to publish site
bounds, so a caller cannot mask an absent invalidation by manually marking terrain dirty.

These tests do not establish that the September 10 floating/buried-house screenshots are fully
fixed. The exact scene/save has not been replayed; private construction still intentionally lowers
unfinished models, and building-frame publication is not an atomic terrain/building transaction.
Those distinctions remain part of `EARTH-02` visual hardening, not a claim of completed repair.

Fresh verification for this refactor: `cargo test --release -- --test-threads=12` passes **1,644
tests** (5 measurement tests ignored); release build, rustdoc, formatting and `git diff --check`
pass without compiler warnings. Source identity is the working tree over
`91af8afca4eb2e9a493ab3bc7f2025a2b87195d9` including the preceding yard changes. Final release test
binary SHA-256: `b1d60aaa4ada18fbd28609ab0f473801fb13c88137975fb4a0bbb27ef3ec1e09`.

Fresh unprofiled measurements ran sequentially after compilation, with 24 Rayon workers and no
concurrent assistant build/render test. They use the same existing paved locality fixture and
100 samples per population level; setup and the one-time snapshot remain outside repeated work:

```bash
RAYON_NUM_THREADS=24 rust/target/release/deps/metrum_rise-9cc8d57998aa4451 populated_paved_road_plan_scaling --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 rust/target/release/deps/metrum_rise-9cc8d57998aa4451 graded_yard_height_query_benchmark --ignored --nocapture --test-threads=1
```

| Background buildings | Compile p50 (ms) | Worker p50 (ms) | One-time snapshot (ms) |
|---:|---:|---:|---:|
| 0 | 20.129 | 21.287 | 0.0106 |
| 1,000 | 19.817 | 21.324 | 0.0574 |
| 10,000 | 19.851 | 21.347 | 0.3785 |
| 100,000 | 20.080 | 21.466 | 9.0101 |

Local geometry matches exactly across levels (up to 600,024 agents / 100,004 parcels / 391 remote
roads). Height queries measure **675.1 ns p50**, **686.8 ns p90**, over the same 76 Kuopio apron
points and 10 × 100,000 queries. Earlier sections are historical evidence, not a freshly rerun
before/after baseline for this refactor; these numbers establish current locality, not a speedup.
The 9 ms largest snapshot is reported explicitly and is not part of the ~20 ms repeated compile.

Logs: `/tmp/metrum-shared-site-final-tests.log`, `/tmp/metrum-shared-site-build.log`,
`/tmp/metrum-shared-site-rustdoc.log`, `/tmp/metrum-shared-site-format.log`,
`/tmp/metrum-shared-site-scaling.log`, `/tmp/metrum-shared-site-query.log`.
The deployed release library SHA-256 is
`2bc8e9a5dc681303995f04f0587d44217a367c51d89035f7fae73c200b1b9449`.
The existing Godot bridge/terrain-payload regression also passes against that library:
`godot --headless --path godot --log-file /tmp/metrum-shared-site-godot-engine.log --script res://tests/road_junction_preview_test.gd`,
with output in `/tmp/metrum-shared-site-godot.log`. This is a fresh headless contract check, not
a graphical replay of the reported houses; prior rendered runs in sections 7–8 are historical.

### 10. Residential Buildability (2026-09-10)

The hillside regression covers five residential lots (6, 7, 8, 9, 16). Deterministic selection
previously chose a small house that failed site support; the
family-house alternative already fit four lots. Parcel 7 additionally needed a different level
pad height: its primary road connection fixed the old plane too low for the rear terrain tie-in.
At one failing rear probe the source rose 8.38 m over 16 m, exceeding the existing 8 m grade budget.

The shared solver now intersects permissible height intervals from road-to-pad runs and existing
perimeter rays, selecting the closest feasible height to the preferred connection. It does not
relax the 50% grade, move roads/neighbors, add a retaining-wall workaround, or make the yard follow
the hill. Ray validation and guide generation both include the exact grading-envelope endpoint.
This changed terrain compiler inputs to CDT contract revision **8**; the subsequent exact
pad-boundary update uses revision **9**.

Demand evaluates support before ranking compatible assets. Zoning preview/commit reuse this
non-mutating feasibility path; unsupported new lots are red with a reason and are not accepted.
Existing blocked lots remain authored intent and are re-evaluated after relevant changes. Both
positive and negative results are cached with exact pose/asset keys and local terrain/road/site
dependencies; unchanged or remote-only edits do not repeat local geometry solves. Queued placement
revalidates current state before insertion. See [`zoning.md`](zoning.md) and
[`building_allocator.md`](building_allocator.md) for API and complexity contracts.

`benchmarks/fixtures/kuopio-terrain/residential-buildability.json` is a portable hillside fixture:
a local source grid, three road profiles, parcel attachments and two house manifests.
It has no mesh dependency and does not replace the Kuopio SQLite or `building-site.toml` fixture.
Portable tests evaluate and construct all five empty lots, require the family house on parcel 7,
compile final terrain patches, check terrain-neutral feasibility, remote-cache reuse, local
terrain/road invalidation, asset removal, stale queued-action rejection and unzoning.

Fresh unprofiled locality measurements used Rust 1.98.1 release settings, 24 Rayon workers,
the working tree over `91af8afca4eb2e9a493ab3bc7f2025a2b87195d9`, and the existing four-local-site
populated fixture (up to 600,024 agents, 100,004 parcels and 391 remote roads). The following ran
sequentially after compilation, without concurrent assistant builds or render tests:

```bash
# Repository root; build with cargo test --release --lib --no-run from rust/ first.
RAYON_NUM_THREADS=24 rust/target/release/deps/metrum_rise-9cc8d57998aa4451 populated_zoning_feasibility_scaling --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=24 rust/target/release/deps/metrum_rise-9cc8d57998aa4451 populated_paved_road_plan_scaling --ignored --nocapture --test-threads=1
```

| Background buildings | First/revalidate site query (ms) | Warm site p50 / p95 (µs) | Road plan compile p50 (ms) | Road snapshot, once per edit (ms) |
|---:|---:|---:|---:|---:|
| 0 | 0.0583 | 0.752 / 0.799 | 20.203 | 0.0067 |
| 1,000 | 0.0143 | 0.761 / 0.809 | 20.205 | 0.0586 |
| 10,000 | 0.0151 | 0.751 / 0.877 | 20.194 | 0.3757 |
| 100,000 | 0.0177 | 0.750 / 1.065 | 20.587 | 3.5066 |

There are 300 warm feasibility samples and 100 measured plans per population level. Feasibility
performs exactly one local solve over the entire matrix; subsequent remote changes only check
local dependencies. Road/terrain products match exactly across population levels. Fixture insertion
and normal index preparation are outside measured queries; cold solving/revalidation and one-time
road snapshots are reported separately. This establishes current locality, not an old-build speedup
or end-to-end zoning-tool frame latency. Earlier sections' timings are historical, not fresh baselines.
Measured executable SHA-256: `16fa69e5a6e2bfcf04f4fe1513c2119eba8a3a8ee981ea734fc785355e5ca353`;
subsequent test-only cleanup leaves the measured production implementation unchanged.
Raw measurements: `/tmp/metrum-buildability-zoning-scaling.log` and
`/tmp/metrum-buildability-road-scaling.log`; this table retains results after those files expire.

Final correctness verification: `cargo test --release` passes **1,646 tests**, with six measurements
ignored by the ordinary suite (the two relevant locality measurements are run explicitly above).
Release test executable SHA-256:
`a8dcdd116cbe04cd73fb5abd5f1c5b08b7f7cd4dc02962eb8b2d75ce62941f3c`.
`cargo build --release`, `cargo check --all-targets`, `cargo doc --no-deps`, formatting and diff
whitespace checks pass without Rust warnings. Godot 4.7.2 passes the headless bridge test and the
Xvfb/llvmpipe rendered test below; the latter reports only its unsupported V-Sync warning. The new
bridge assertions verify red rejected lots, their reason strings, terrain-neutral preview/rejection,
per-parcel drag colors, tool mesh/label handling, free-parcel placement and dependency changes.

```bash
# Repository root, against the deployed release library.
godot --headless --path godot --log-file /tmp/metrum-buildability-godot-engine.log --script res://tests/road_junction_preview_test.gd
xvfb-run -a godot --display-driver x11 --path godot --rendering-method gl_compatibility --log-file /tmp/metrum-buildability-rendered-engine.log --script res://tests/road_junction_preview_test.gd
```

Logs: `/tmp/metrum-buildability-final-tests.log`, `/tmp/metrum-buildability-release-build.log`,
`/tmp/metrum-buildability-all-targets.log`, `/tmp/metrum-buildability-rustdoc.log`,
`/tmp/metrum-buildability-godot.log` and `/tmp/metrum-buildability-rendered.log`.
Deployed `godot/bin/libmetrum_rise.so` SHA-256:
`bbcafb1a30b2f4d50066a9d2fbf55e9178ecc35dc01d71e83d70c6bcd9c913f8`.

### 11. Building-Site Spec Audit (2026-09-10)

The full uncommitted source/fixture/documentation audit found and corrected these contract gaps:

- Engineered height/picking accepted finite buffers even when terrain publication rejected omitted
  pathological faces. Both now use the complete shared acceptance checks on current-generation
  patches; queries remain allocation-free and bounded to local windows.
- In-place upgrades/downgrades bypassed support validation. The target asset now passes the same
  connection, footprint, neighbor and terrain checks at the existing fixed height before mutation.
- Replacing an asset retained old zone/density/family memberships. Registration now removes those
  memberships from the affected buckets before indexing the replacement.
- Shared support preparation repeated driveway queries, and paving retained an unused duplicate
  elevation field. Connections are resolved once per attempt; material regions carry no height.
  An obsolete dead-code suppression and unnecessary helper visibility were also removed.
- Allocator documentation still described daily/founding placement and compared secondary
  driveways against the solved pad instead of the primary road connection. It now matches the
  hourly/minute-queued lifecycle and the actual support constraints.

Regressions cover render-rejected ground queries, changed asset classifications, and rejected
level changes after terrain edits with unchanged building identity, height and revision. Existing
paired zoned/service, hillside placement, coverage, cache and RoadEditPlan regressions remain.
The earlier `EARTH-02` transient renderer-publication and exact-scene limitations are not erased
by this audit.

Fresh final verification: `cargo test --release` passes **1,648 tests**, with six measurement tests
ignored in the ordinary suite. `cargo check --all-targets`, `cargo doc --no-deps`, release build,
formatting and both working/index diff-whitespace checks pass without Rust warnings. The headless
and Xvfb rendered `road_junction_preview_test.gd` both pass against the rebuilt/deployed library;
Xvfb reports only the unsupported V-Sync warning. Commands are the section-10 commands with
`/tmp/metrum-site-audit-` log prefixes.

Matched unprofiled comparisons ran each baseline/final pair sequentially after all builds, using
Rust 1.98.1 release settings and 24 Rayon workers, with no concurrent assistant build or renderer.
Both builds are the working tree over `91af8afca4eb2e9a493ab3bc7f2025a2b87195d9`, before/after the
audit fixes above. The pre-audit test executable is preserved at `/tmp/metrum-site-audit-baseline-tests`
(SHA-256 `a8dcdd116cbe04cd73fb5abd5f1c5b08b7f7cd4dc02962eb8b2d75ce62941f3c`); final executable
SHA-256 is `5c1b6d0505eb5987511aee3311de96afdb3a29e04e46f906aa3d92f16fa9cd6b`.

```bash
# Repository root; each measurement is a fresh process, setup excluded from its timed loops.
for audit_measurement in graded_yard_height_query_benchmark populated_zoning_feasibility_scaling populated_paved_road_plan_scaling; do
  RAYON_NUM_THREADS=24 /tmp/metrum-site-audit-baseline-tests "$audit_measurement" --ignored --nocapture --test-threads=1
  RAYON_NUM_THREADS=24 rust/target/release/deps/metrum_rise-9cc8d57998aa4451 "$audit_measurement" --ignored --nocapture --test-threads=1
done
```

| Background buildings | Road compile p50 before / after (ms) | Warm feasibility p50 before / after (µs) | Final snapshot, once per edit (ms) |
|---:|---:|---:|---:|
| 0 | 20.308 / 20.154 | 0.758 / 0.739 | 0.0069 |
| 1,000 | 20.074 / 20.192 | 0.783 / 0.849 | 0.0585 |
| 10,000 | 20.271 / 20.233 | 0.751 / 0.746 | 0.3858 |
| 100,000 | 20.440 / 20.493 | 0.760 / 0.749 | 3.7441 |

The workload retains four local paved sites and grows to 600,024 agents, 100,004 parcels and 391
remote roads. Each level has 100 road plans and 300 warm feasibility samples. Local products match
exactly across levels; feasibility solves once over the matrix. Final first/revalidation queries
take 0.0522/0.0152/0.0149/0.0186 ms, reported separately from warm queries and fixture/index setup.
Final worker medians are 21.54–21.89 ms. No city-proportional repeated planning cost is observed.
These are locality checks, not whole-game frame-latency or arbitrary-lot acceptance claims.

The complete height query over 76 Kuopio apron points (10 × 100,000 queries) changes from
**683.8 ns p50 / 686.2 ns p90** to **766.3 ns p50 / 810.6 ns p90**. The full acceptance guard
therefore costs about 83 ns per median query; this correctness cost is explicit, not a speedup.

Raw performance logs use `/tmp/metrum-site-audit-{baseline,final}-<measurement>.log` with the exact
measurement names above. Correctness/build logs are `/tmp/metrum-site-audit-{tests,check,rustdoc,build}.log`;
Godot logs are `/tmp/metrum-site-audit-{godot,rendered}.log`. The tables retain evidence after these
temporary artifacts expire. Deployed `godot/bin/libmetrum_rise.so` SHA-256:
`d0b260e94c5aef161c438a44cb35bb27b82befa362f4175beba31c3a5e2117e5`.

### 12. Entry/Exit Grounding And Exact Pad Boundaries (2026-09-10)

Off-lane cars sampled source terrain, while walkers selected the maximum of terrain, road and pad
heights. Both now use the shared ownership-ordered world query in either access direction, with
0.02 m clearance; lane-bound heights are unchanged. No vertical profile is cached in agent/access
state. The existing site index is prepared once before snapshot iteration, avoiding per-agent
fallback scans. See [`entrance_and_exit.md`](entrance_and_exit.md#render-height-ownership).

The new Kuopio regression first reproduced a car origin 0.36 m above the required surface plus
clearance. Sampling the pad boundary also exposed an 8 mm mismatch: the old 2 cm inward terrain
cutout allowed grading inside the pad. Exact cutouts now share the foundation/query boundary
(CDT revision 9). The neighbor-height guard includes touching supports, not only positive-area
overlaps. Different-height neighboring pads need grading space; a legacy low-level test that
placed their incompatible boundaries at identical XZ now supplies that space. The rollback test
uses a registered minimal asset and coherent derived transforms, instead of a missing-asset
full-lot fallback against the road seam.

Fresh correctness verification: **1,648 Rust tests pass, 7 ignored**, including 352 car/walker
ingress/egress transform checks against rendered cut/fill yards, aprons, sidewalk and carriageway;
pending-terrain checks also keep agents on fixed pads above or below the visual heightfield.
Exact live/planned site-loop coordinates, neighbor-height rejection, terrain coverage and
RoadEditPlan rollback remain covered. `cargo check --all-targets`, `cargo doc --no-deps`, release
build, formatting and diff checks pass without Rust warnings. The Kuopio SQLite is unchanged.
`road_junction_preview_test.gd` passes both headless and rendered under Xvfb with the deployed
revision-9 library. The rendered run only reports the virtual driver's unsupported VSync mode.

Matched, unprofiled release measurements ran sequentially after all builds, with Rust 1.98.1 and
`RAYON_NUM_THREADS=24`, over working-tree base `91af8afca4eb2e9a493ab3bc7f2025a2b87195d9`.
Baseline is the pre-fix implementation with the new snapshot benchmark instrumentation:
`/tmp/metrum-access-baseline-tests`, SHA-256
`bec46f458fd998a99e80f6fb1dab35ce38da15ac345969d6ad02d1ec5eb44820`.
Final test executable `rust/target/release/deps/metrum_rise-9cc8d57998aa4451`, SHA-256
`4c01acddd54e3ec9f80af733ada0b98788a6c4f4426e55eec94cda64f3e7ab7c`.

```bash
for measurement in graded_yard_access_snapshot_benchmark populated_paved_road_plan_scaling populated_zoning_feasibility_scaling graded_yard_height_query_benchmark; do
  RAYON_NUM_THREADS=24 /tmp/metrum-access-baseline-tests "$measurement" --ignored --nocapture --test-threads=1
  RAYON_NUM_THREADS=24 rust/target/release/deps/metrum_rise-9cc8d57998aa4451 "$measurement" --ignored --nocapture --test-threads=1
done
```

Snapshot workload: 76 fixed Kuopio frontage/pad-boundary positions, equal cars/walkers and
ingress/egress, 10 warmups and 100 recycled snapshots per count. Placement, compilation, agent
creation and buffer warmup are outside timing; no Godot or build runs concurrently.

| Simultaneously visible access agents | Snapshot p50 before → after (ms) | p90 before → after (ms) |
| --- | --- | --- |
| 100 | 0.2027 → 0.2325 | 0.2203 → 0.2388 |
| 1,000 | 0.5852 → 0.9364 | 0.6085 → 0.9470 |
| 10,000 | 4.4585 → 7.9714 | 4.4977 → 8.0166 |

Correctly grounding cars adds about 0.35 ms per 1,000 access agents in this mixed workload.
That bounded, allocation-free query cost is retained explicitly; it is not a speedup or a
whole-game frame-rate guarantee. Network-bound and camera-culled agents do not pay it.

| Remote buildings | Road compile p50 before → after (ms) | Warm feasibility p50 before → after (µs) | Final one-time edit snapshot (ms) |
| --- | --- | --- | --- |
| 0 | 20.358 → 19.957 | 0.766 → 0.758 | 0.0103 |
| 1,000 | 20.145 → 20.092 | 0.757 → 0.773 | 0.0572 |
| 10,000 | 20.268 → 19.855 | 0.777 → 0.802 | 0.3550 |
| 100,000 | 20.321 → 20.176 | 0.792 → 0.822 | 3.2621 |

Each level measures 100 local road plans and 300 warm feasibility queries, increasing background
state to 600,024 agents / 100,004 parcels / 391 roads. Final local products are identical across
population levels; the feasibility matrix still performs one local solve. Final first/revalidation
queries take 0.0559/0.0145/0.0151/0.0174 ms, separately from warm measurements. Worker p50 stays
21.31–21.40 ms; repeated planning remains local, with one-time snapshot cost reported separately.
The existing 76-point height-query benchmark (10 × 100,000 samples) remains stable:
756.7 → 751.2 ns p50 and 760.3 → 751.7 ns p90. Its pair ran after the Godot checks had exited.

Logs: `/tmp/metrum-access-{before,after}-<measurement>.log` for the exact names above;
`/tmp/metrum-access-{tests,check,rustdoc,build}.log` for correctness/build checks.
Godot logs are `/tmp/metrum-access-{godot,rendered}.log`.
The deployed library SHA-256 is
`7230d78be163ab17e94909acc02e672f9ef47e00c00de98ece5d60d3d528debe`.

### Changeset audit (2026-09-11)

The audit of the pending site/terrain/access changes corrected runtime mesh-part yaw: the editor
uses positive-Y rotation (+X toward -Z), while rendering and support derivation had independently
implemented its opposite. `MeshPart::local_transform` now owns yaw, scale and pivot for both;
rendering reuses the allocator's frontage basis too. The regression
`building_part_pose_matches_editor_yaw_scale_and_pivot` compares packed render transforms against
Godot's independent Basis math for 20 signed-yaw/building-heading combinations. The imported-bounds
regression verifies the corresponding structural footprint. CDT revision 13 prevents stale reuse.

Removed duplicate fresh-asset ranking loops, upgrade/downgrade application loops and live/snapshot
site-bounds calculations. Commercial output priorities, deterministic ranking and downgrade-before-
upgrade ordering are unchanged. Removed the superseded optional-normal surface-query pipeline and
the unused standard-building-scale helper/test, uncalled capacity/world-size wrappers, and direct
car/pedestrian exporters that still sampled raw terrain. Coherent snapshots are the only instance
export path; path debug overlays share the snapshot's access-destination resolver. Vehicle pitch/roll
still uses the active footprint solver; authored mesh scale and the missing-asset error mesh remain
supported. These changes add no spatial index, per-agent allocation or city-wide query: part
transforms remain O(1), site bounds
remain O(local vertices), and selection/terrain query complexity is unchanged.

The changed-line Clippy audit has no unused-variable/dead-code diagnostics; its only retained
changed-line suggestion concerns the intentional release-only benchmark assertion. Default Clippy
still fails at the unchanged `economy/agents/tick/slices.rs` unsafe `RawSlice::get_mut(&self)` API
(`clippy::mut_from_ref`). Diagnostic continuation used a command-line lint override, not a source
suppression or a change to the agent aliasing contract. Other pre-existing repository lints are
outside this changeset cleanup. Historical verification sections above describe their original
builds, not this audit's final executable. Broader `EARTH-02` frame-publication limitations remain.
Removing the direct exporters also exposed a missed migration: only the obsolete exporter used
lane-change S-curves. The active snapshot now consumes the existing sampler with matching-owner
checks, as required by [`traffic.md`](traffic.md#render-movement).

Matched measurements caught a cleanup regression before acceptance: copying an indexed triangle
through the height-query helper introduced 72-byte stack copies per candidate. Generated-code
inspection identified the call boundary; both the index loop and helper now borrow the triangle.
The intermediate 736–744 ns/query results are not the final implementation's acceptance timings.

Fresh final verification (not the historical results above):

- `RAYON_NUM_THREADS=24 cargo test --release --lib`: **1,660 passed**, 10 ignored measurement cases.
- `cargo build --release`, `cargo doc --no-deps`, `cargo fmt --all --check`, `bash -n run.sh`,
  staged/unstaged `git diff --check`, changed-source licensing/module headers and documentation
  file links: pass. Release build and rustdoc emit no warnings.
- `cargo clippy --all-targets --message-format=json -- -A clippy::mut_from_ref`: diagnostic
  continuation completes, with the pre-existing repository warnings described above. The override
  does not constitute default-Clippy acceptance.
- Godot `vehicle_ground_support`, `network_tool_chunk_renderer`, `road_benchmark_metrics`,
  `road_junction_preview`, `road_preview_stream` and `camera_save_load` headless test scripts: pass.
  Vehicle support checks 4,840 poses (2,480 tilted), with minimum contact clearance **0.020000 m**.
  `road_junction_preview_test.gd` also passes under Xvfb/OpenGL Compatibility (Mesa llvmpipe).

Commands run from `rust/` for Cargo and the repository root for Godot:
`RAYON_NUM_THREADS=24 godot --headless --path godot --script res://tests/<name>_test.gd`;
rendered check uses `xvfb-run -a godot --display-driver x11 --path godot
--rendering-method gl_compatibility --script res://tests/road_junction_preview_test.gd`.
Logs: `/tmp/metrum-audit-sealed-{tests,build,doc}.log`,
`/tmp/metrum-audit-sealed-clippy.{jsonl,log}`, `/tmp/metrum-audit-final-<name>.log` and
`/tmp/metrum-audit-rendered.log`. Toolchain: Rust 1.98.1; Godot 4.7.2, gdext API 4.5.
The checked-in Kuopio SQLite and user assets remain unchanged.

Final unprofiled measurements use independent release processes on an i9-12900K, with
`RAYON_NUM_THREADS=24`, no concurrent builds/render tests and no profiler. The height-query
microbenchmark alone uses `taskset -c 0` for matched core affinity: three before/after process
pairs, each sampling 76 points in ten 100,000-query batches. Its median process p50 is
**584.1 → 560.4 ns/query**; final process medians span 557.4–561.4 ns. The generated code also
confirms the eliminated copies; no application-wide speedup is inferred.

Access snapshots and vehicle-support batches use three matched process pairs, each with ten
warmups and 100 measurements at 100/1,000/10,000 entities. Fixture setup is excluded and snapshot
buffers are reused. Median process p50 values (ms):

| Workload | Entities | Before | Final |
| --- | ---: | ---: | ---: |
| Access snapshot | 100 | 0.4160 | 0.5037 |
| Access snapshot | 1,000 | 1.0985 | 1.1637 |
| Access snapshot | 10,000 | 8.6098 | 7.5713 |
| Vehicle-support batch | 100 | 0.4415 | 0.3632 |
| Vehicle-support batch | 1,000 | 1.0333 | 1.3082 |
| Vehicle-support batch | 10,000 | 5.1362 | 4.6879 |

Small parallel batches remain process-sensitive; the largest median increase here is 0.275 ms
at 1,000 support queries, while both 10,000-entity workloads improve. The audit accepts these
bounded costs with unchanged query complexity, not a claim that every workload became faster.
All process results, including slower samples, remain in the logs.

The populated paved-site check uses four unchanged local sites and 0/1,000/10,000/100,000 remote
buildings, up to 100,004 parcels, 600,024 agents and 391 background roads. In the final paired
run, worker p50 spans **22.19–23.86 → 22.01–22.79 ms** across population levels, with identical
local products in every compile/worker assertion. Each level has three warmups and 100 measured
samples. Final readiness p50 stays below 0.020 ms. The one-time 100,000-background snapshot costs
**3.330 → 3.413 ms**, separately from repeated planning. Cached zoning checks retain one local
solve across all four levels; final warm p50 is 0.694–0.718 µs (300 queries/level). The local CDT
fixture has unchanged 763 vertices / 1,369 accepted faces and no missing/invalid constraints;
20 batches of 50 compiles after four warmup batches measure **0.4577 → 0.4444 ms/compile**.
These are locality/cost checks, not a guarantee of city-wide frame time.

Reproduction: `RAYON_NUM_THREADS=24 <binary> <measurement> --ignored --nocapture --test-threads=1`.
Use `taskset -c 0 <binary>` for `graded_yard_height_query_benchmark` only. Other exact workload
names are `graded_yard_access_snapshot_benchmark`, `graded_yard_vehicle_support_batch_benchmark`,
`populated_zoning_feasibility_scaling`, `populated_paved_road_plan_scaling` and
`benchmark_road_site_cdt_noding`. Logs are `/tmp/metrum-audit-sealed-query-{1,2,3}-{before,after}.log`
and `/tmp/metrum-audit-sealed-{before,after}-<measurement>.log`.
The extra access/batch process pairs use
`/tmp/metrum-audit-sealed-{2,3}-{before,after}-<measurement>.log`.

Build identities (working changes on `91af8afca4eb2e9a493ab3bc7f2025a2b87195d9`, SHA-256):

- Pre-audit executable `/tmp/metrum-audit-before-tests`:
  `7767a8db42fa886644764739ad7833eced7afca243462e11678c23a6fe2f5e6f`.
- Final executable `rust/target/release/deps/metrum_rise-9cc8d57998aa4451`:
  `a0346d598d43f8386d373bb0958ab83da9863368f082f68386c2576b01b32cc0`.
- Final deployed library `godot/bin/libmetrum_rise.so`:
  `890c719920b8a947e8067040f069515bb7e36b9d3801e2f296f1464a04f8f62e`.

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
