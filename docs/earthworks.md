# Earthworks / Engineered Ground

## Purpose

This document owns the shared engineered-ground contract for local terrain overrides such as road
cuts, embankments, flat building pads, and future retaining structures.

Tracked work currently lives under [`ROAD-01`](roadmap.md), because roads are the first live
client. Future building-pad or plot-foundation work should extend this document instead of
duplicating the same terrain-override rules elsewhere.

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

Those remain owned by [`improved_roads.md`](improved_roads.md), [`terrain.md`](terrain.md),
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
- future zoning / building pads, using a flat foundation pad
- future parking platforms, rail beds, retaining structures, or other built ground

The support surface must not be inferred from the terrain heightfield after the fact.

### 3. Heightfields Alone Are Not The Long-Term Visual Carrier

A single-height terrain grid can only store one height per `(x, z)` position.

That is not sufficient to cleanly represent all of these at once near the same footprint edge:

- uphill cut face
- support surface
- downhill fill / support

Current terrain stamping may still be used as a compatibility or far-field step, but the shared
target must allow engineered-ground clients to own local side geometry near their footprints.

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

This rule applies equally to linear clients such as roads and to area clients such as future flat
building foundations.

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
  - either terrain chunks are clipped / triangulated so their inner boundary exactly follows the
    road-owned outer boundary
  - or the client emits local earthwork / tie-in geometry whose inner edge reuses the same
    road-owned outer-boundary vertices and whose outer edge ties back to terrain
- terrain suppression alone is not a seam solution; it may only hide terrain under geometry that
  actually exists
- for grounded clients, the runtime must not render a second visible support mesh under an ordinary
  grounded footprint just to hide terrain overlap
- explicit local earthwork geometry remains valid as an internal ownership / stamping carrier and
  as a visible carrier for structural or explicitly exposed cases, but ordinary grounded roads must
  not depend on a separate visible mesh below asphalt, shoulder, curb, or sidewalk

Deterministic seam contract:

- the seam is the shared boundary between the client-owned footprint and the far-field terrain
- for roads, the footprint includes asphalt, shoulder / curb, and sidewalk bands; the seam starts at
  the exact outer sidewalk edge, or at the exact outer shoulder edge when a profile has no sidewalk
- every seam segment must be backed on both sides by visible carriers before terrain below the
  footprint is suppressed:
  1. the client-owned top surface covers the footprint side of the seam
  2. terrain topology is clipped / triangulated so its inner edge exactly matches the client-owned
     seam vertices
- terrain masking, terrain alpha, terrain discard, or footprint suppression is not a seam carrier;
  those tools are valid only after one of the visible carriers above already covers the boundary
- grounded `Standard` roads must use exact clipped terrain topology as the normal seam carrier; a
  separate visible local tie-in mesh is reserved for structural classes and deterministic retaining
  / wall variants rather than as the ordinary grounded-road join
- the clipped terrain inner edge must reuse the same ordered vertices and heights as the road-owned
  outer sidewalk / shoulder boundary; it must not resample, offset, snap, or simplify that edge
- clipped terrain patches must receive exact road footprint clip polygons from the same `Span`,
  `Terminal`, `Bend`, and `JunctionN` pieces that render asphalt, shoulder / curb, and sidewalk
- clipped terrain topology must insert road-boundary vertices into the terrain mesh rather than
  approximating the seam only from terrain-cell centers or a texture mask
- no `Bend` or `JunctionN` clip boundary may be synthesized from one anonymous annulus, one global
  outer loop, or a terrain-cell mask after the fact
- a valid seam is closed independent of triangle winding or backface-culling behavior; clipped
  terrain may render double-sided, but the renderer must not reveal world background, holes, or
  source terrain through the road / sidewalk / tie-in boundary
- water is not a valid fallback carrier under the road footprint; water render patches that overlap
  grounded road-owned asphalt, shoulder / curb, or sidewalk must receive the same footprint clip
  polygons and omit water topology inside that owned footprint

For roads, that means:

- grounded road footprint support is owned by the committed road top surface
- bridges limit that support to abutment-owned grounded regions
- tunnels limit that support to visible portal-owned grounded regions
- grounded `Standard` roads replace the near-road visible terrain locally:
  - asphalt and sidewalk render as the visible terrain replacement inside the road-owned footprint
  - the terrain renderer emits no terrain under the road-owned footprint
  - the seam to far-field terrain is covered by clipped terrain topology whose inner boundary is
    the road-owned outer sidewalk / shoulder edge
  - terrain suppression is allowed only as a consequence of the clipped terrain topology
  - the runtime must not render a separate visible cut / fill support mesh below ordinary grounded
    asphalt or sidewalk
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

- only touched terrain chunks are reset and restamped
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

### 1. Roads Are The First Live Client

The current runtime ships roads as the first engineered-ground client.

That means:

- `RoadSurfaceSystem` owns the roadbed support surface
- grounded `Standard` roads stamp a deterministic footprint plus outer earthwork margin
- bridge earthworks remain abutment-only
- tunnel earthworks remain portal-only

Road-specific section and junction rules continue to live in [`improved_roads.md`](improved_roads.md).

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

### 3. Terrain Density Is Still A Limitation

The current runtime still relies on the visual terrain heightfield to carry too much of the local
cut / fill shape near roads.

Phase 11 characterization exists because authored `10 m` terrain can still smear visible terrain
back over a grounded road corridor even when the roadbed and earthwork ownership rules are
otherwise correct.

The current deterministic characterization now proves:

- the `10 m` grid still shows measurable footprint overlap in the shared hillside case
- `5 m` materially improves that same case
- `5 m` does not by itself eliminate the overlap problem
- density therefore helps blend quality, but it is not allowed to become the sole engineered-ground
  fix
- any future baseline move from `10 m` to `5 m` or finer terrain must follow the chunk-local
  terrain / water render-boundary split owned by [`terrain.md`](terrain.md), not the current
  whole-map dense renderer upload path

So terrain density is part of the answer, but not the whole answer.

### 4. Current Runtime Compatibility Gap On Post-Placement Terrain Edits

The current runtime still violates the fixed-client rule for grounded roads.

Current compatibility gap:

- later terrain edits still resynchronize placed `Standard` road geometry against edited source
  terrain
- visual terrain is then restamped from that moved roadbed, so the road can shift instead of the
  terrain alone reshaping around it
- future building pads and foundations are not live yet, so their fixed-surface semantics are
  still a shared-target rule rather than a shipped behavior

### 5. Current Terrain Runtime Is A Compatible Base, Not The Final Visual Carrier

The current terrain runtime is sufficient for first-stage engineered ground:

- it keeps separate authoritative `source terrain` and derived `visual terrain`
- it supports chunk-local reset and restamp of touched visual regions
- it already accepts client-derived footprint and outer-margin earthwork inputs

But it is not sufficient as the final near-footprint visual carrier because the current visual
terrain remains a single-height field.

Current deterministic conclusion:

- a terrain-runtime rewrite is allowed if needed, but it is not by itself an earthworks solution
- do not expect the current visual terrain heightfield, even at a denser cell size, to represent
  all near-footprint cut / fill detail by itself
- the next solution must move near-footprint ownership into client-owned local earthwork geometry
  instead of trying to keep terrain stamping as the primary visible carrier

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
- the replacement target is a closed road-owned earthwork mesh

### 2. Roads-First Rewrite Uses A Separate Piece/Profile Carrier Plus A Closed Road-Owned Earthwork Mesh

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
- edge-side and node-side earthworks must derive from those same piece-owned profile boundaries
  instead of from separate terrain-side widening or generic node-fill polygons
- grounded roads own closed local earthwork geometry adjoining the committed roadbed footprint
- that local geometry must include:
  1. the road-owned top-surface boundary condition
  2. left and right tie-in faces from the footprint to the tie-in boundary
  3. terminal cap geometry at dead ends
  4. closure / underside geometry anywhere the side faces would otherwise expose a visible void
- terrain owns only the far-field ground outside the deterministic tie-in boundary
- flat-ground cases must collapse toward a visually minimal shoulder / verge join instead of
  emitting a wide apron
- sloped cases must emit real cut / fill faces instead of asking the terrain heightfield to fake
  the entire join

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
- node top-surface ownership must compile to explicit road polygons plus explicit sidewalk sectors
  owned by those pieces; it must not rely on one annulus-style ring carrier
- future building pads follow the same owner model, but with a perimeter tie-in ring instead of
  two longitudinal side runs

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
- clipped water topology must leave the same road-shaped hole where visible water overlaps a grounded
  road-owned footprint; road placement over water must not leave a hidden lake plane between the road
  mesh and its owned support surface
- bridges remain a separate class: midspan bridge decks do not claim grounded terrain ownership
  except at explicit abutment / portal support regions

### 4. The First Required Variant Is A Closed Slope / Closure Mesh

The first required rewrite variant is still intentionally narrow, but it is no longer a thin
visible strip.

Required first variant:

- the first shipped rewrite variant must clip terrain patches directly to the road-owned footprint
  boundary for grounded `Standard` roads
- road-locked terrain patch selection is bounded to the road-owned footprint, not to the wider
  earthwork envelope; earthwork margins must not force whole-map clipped-terrain rebuilds
- cut versus fill for structural variants is determined from the support-surface anchors relative
  to the sampled tie-in boundary, not from ad hoc widening of stamped terrain
- the first shipped rewrite variant does not require retaining walls, cliff faces, or other
  special-case vertical structures
- the terrain mesh itself must be open below the road footprint and must terminate at the exact
  road-owned seam boundary
- terrain suppression is allowed only as topology omission from the clipped terrain mesh; fragment
  discard, alpha masking, or post-shader hiding is not an accepted grounded-road solution
- future retaining or wall variants may replace the first slope / closure mesh when deterministic
  thresholds say the simple slope solution is no longer acceptable, but that is a later extension
  of the same ownership model rather than a separate geometry system

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
- after the source-terrain edit, the runtime rebuilds only the touched local earthwork meshes and
  derived visual terrain around the already committed support surface
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
- rebuilding local earthwork geometry must not force whole-world terrain restamps or
  whole-world render uploads

## Roads-First Rewrite Status

The roads-first rewrite is partially live. The piece / profile carrier and deterministic road
ownership data now exist in the live road runtime, but the grounded-road seam is not complete until
road-touched terrain patches are clipped / triangulated to the outer sidewalk / shoulder edge.

For the roads-first rewrite, the following are deterministic and implemented:

- the current corridor-sheet prototype is retired as the target solution
- the logical graph and the visible road carrier are split: graph owns connectivity, while the
  visible road system owns deterministic geometry pieces
- the minimum required visual piece set is `Span`, `Bend`, `Terminal`, and `JunctionN`
- roads compile explicit top-surface ownership and earthwork ownership carriers near the footprint
- tie-in boundaries are specified to use dense local sampling at a maximum `2 m` longitudinal
  spacing
- committed clients stay fixed under later terrain-authoring edits
- local road ownership uses chunk-local rebuild boundaries and one explicit visible-world query
  order

The following are still blockers for the live grounded-road result:

- grounded `Standard` roads now send road footprint clip polygons into road-touched terrain patches
  and the terrain shader-mask discard path has been removed
- the terrain renderer now short-circuits clipped patch emission for untouched cells and fully
  road-owned cells, so only seam-crossing cells pay the exact polygon-clipping cost
- visible water patches now use depth-owned local topology instead of full-patch planes; road-touched
  water meshes receive the same road footprint clip polygons after a network edit, so water is no
  longer allowed to render under grounded road-owned asphalt, shoulder / curb, or sidewalk
- the remaining blocker is validation and hardening of the clipped patch topology against flat,
  diagonal, sloped, water-overlap, bend, terminal, and `JunctionN` cases
- terrain suppression / masking is not accepted as the live seam solution; road-shaped terrain holes
  must continue to be produced by terrain mesh topology
- retaining / wall variants are later replacements for cases where deterministic thresholds require
  structural faces, not the ordinary grounded-road seam carrier

That means the remaining items below are later additions only after clipped terrain topology is
implemented; they are not a substitute for closing the road-to-terrain boundary.

## Later Additions

The following items remain intentionally open as later extensions of the same subsystem.

### 1. When Do Buildings Become Live Engineered-Ground Clients?

Future flat building pads and foundations are already part of the shared target, but they are not
yet implemented as first-class engineered-ground clients.

Open decision:

- how building-pad support surfaces, footprint ownership, tie-in boundaries, and future retaining
  behavior integrate with allocator / zoning placement without inventing a second terrain-override
  model separate from roads

### 2. When Do Later Geometry Variants Replace The First Closed Slope / Closure Mesh?

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

### 2. Roads And Future Foundations Use The Same Rules

This shared subsystem must work for both:

- linear corridor clients such as roads
- area / pad clients such as future flat building foundations

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

If denser terrain still leaves unacceptable overlap, the next fix must be a different
representation rather than more density alone.

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
