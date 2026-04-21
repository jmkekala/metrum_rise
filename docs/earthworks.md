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
- `corridor mesh`: a local owner-controlled cut / fill mesh that represents the engineered slope or
  retaining face near the footprint

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

- inside the footprint, the visible world surface must support the client-owned surface
- outside the footprint but inside the earthwork margin, the visible world surface transitions back
  toward source terrain using deterministic cut / fill rules

The system must not treat "sample terrain at the centerline and widen it later" as the shared
earthworks model.

### 6. World-Surface Query Precedence Must Be Explicit

Terrain-only queries remain authoritative against source terrain.

Visible world-surface queries must resolve in this order:

1. client-owned top surface
2. client-owned local earthwork / corridor geometry when it exists
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
  direct edits to a roadbed, pad, or local corridor mesh

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
- treat future client-owned corridor or pad geometry as an additive layer above the current terrain
  ownership model, not as something denser terrain alone can replace

## Geometry Decision

The former whole-map dense terrain / water render boundary is intentionally not listed here as an
open blocker, because [`TERRAIN-01`](roadmap.md) already removed it.

### 1. Roads-First Tie-In Geometry Is A Client-Owned Corridor Mesh

The exact first geometry form is now fixed.

Deterministic choice:

- linear engineered-ground clients such as roads use client-owned corridor geometry on both sides
  of the owned top surface
- area / pad clients such as future flat building foundations use client-owned perimeter ring
  geometry around the owned top surface
- terrain owns only the far-field ground outside the deterministic tie-in boundary
- the local geometry layer is the required near-footprint visible carrier; visual terrain inside
  that local tie-in region becomes compatibility / blend support, not the final owner of the
  visible cut / fill shape

### 2. Road Corridor Geometry Uses Deterministic Section Anchors

The first implementation slice is roads-first and must use one deterministic geometry form.

Required road geometry contract:

- each compiled road section owns one left-side and one right-side local earthwork strip
- each strip begins at the outer edge of the road-owned footprint for that section sample
- each strip ends at the deterministic tie-in boundary already implied by the road earthwork solve
- consecutive section samples stitch those inner and outer anchors into one continuous side mesh
  per side
- edge-local corridor geometry stops at the existing road throat boundaries so road-edge ownership
  still hands off cleanly into node patches and terminal ownership
- terminal road ends must emit deterministic cap geometry from the owned top-surface boundary to
  the tie-in boundary
- future node-patch tie-ins and future building pads follow the same owner model, but use a
  perimeter ring instead of two longitudinal side strips

### 3. Slope Strips Are The First Required Variant

The first required geometry variant is intentionally narrow.

Required first variant:

- the local corridor geometry must render a deterministic slope strip from the owned top surface to
  the tie-in boundary
- cut versus fill is determined from the support-surface anchor heights relative to the tie-in
  boundary, not from ad hoc widening of sampled terrain
- the first shipped geometry variant does not require retaining walls, cliff faces, or other
  special-case vertical structures
- future retaining or wall variants may replace the slope strip when deterministic thresholds say
  the slope solution is no longer acceptable, but that is a later extension of the same ownership
  model rather than a separate geometry system

### 4. Terrain Runtime Rewrites Are Allowed, But The Ownership Contract Is Not Optional

A terrain-runtime rewrite is acceptable if it materially improves the system and still preserves the
shared engineered-ground rules.

Required rule:

- whether the implementation extends the current terrain runtime or rewrites it, the resulting
  runtime must still preserve:
  1. authoritative source terrain
  2. explicit client-owned top surfaces
  3. client-owned local corridor / pad geometry near the footprint
  4. chunk-local invalidation and rebuild boundaries
  5. visible-world query precedence
  6. no regression back to whole-world rebuild or upload behavior

### 5. Fixed-Client Runtime Behavior Is Deterministic After Commit

The fixed-client rule now has one required runtime interpretation.

Required runtime contract:

- placement-time grounding may choose the initial support surface only before the client is
  committed
- committing the client freezes that support surface for later terrain-authoring edits
- terrain-authoring edits write source terrain only
- after the source-terrain edit, the runtime rebuilds only the touched local earthworks and derived
  visual terrain around the already committed support surface
- explicit client-edit operations remain the only path that may move, regrade, or replace the
  committed support surface
- if a later terrain edit would make the surrounding cut / fill extreme, the runtime still keeps
  the committed client fixed; later geometry variants may change how that earthwork is represented,
  but terrain brushes must not silently move the client

### 6. Extra Geometry Uses Chunk-Local Client Caches And One Shared Query Order

The extra geometry layer now has one required cache and query model.

Required cache and query contract:

- client-owned top surfaces remain owned by the client that authored them
- client-owned corridor / pad geometry is stored in chunk-local caches aligned to terrain chunk
  boundaries
- when a client rebuilds, it must partition the produced local geometry into touched terrain chunks
  instead of keeping one whole-world monolithic mesh
- renderer uploads for that local geometry must stay bounded to the touched chunk caches
- visible-world queries must resolve in this order:
  1. client-owned top surface
  2. client-owned local corridor / pad geometry for the touched chunk
  3. visual terrain
  4. source terrain only for terrain-only APIs
- world-surface picking and visible-surface queries must not require a whole-world scan across every
  engineered-ground client
- rebuilding local corridor / pad geometry must not force whole-world terrain restamps or
  whole-world render uploads

## First-Draft Status

The first roads-first earthworks implementation no longer has unresolved architecture blockers in
this document.

For the first draft, the following are now deterministic:

- roads use client-owned corridor geometry near the footprint
- the first geometry variant is the slope strip
- committed clients stay fixed under later terrain-authoring edits
- local earthwork geometry uses chunk-local caches and one explicit visible-world query order

That means the remaining items below are later additions, not blockers for the first road corridor
implementation.

## Later Additions

The following items remain intentionally open as later extensions of the same subsystem.

### 1. When Do Buildings Become Live Engineered-Ground Clients?

Future flat building pads and foundations are already part of the shared target, but they are not
yet implemented as first-class engineered-ground clients.

Open decision:

- how building-pad support surfaces, footprint ownership, tie-in boundaries, and future retaining
  behavior integrate with allocator / zoning placement without inventing a second terrain-override
  model separate from roads

### 2. When Do Later Geometry Variants Replace The First Slope Strip?

The first shipped geometry variant is intentionally only the slope strip.

Open decision:

- which deterministic thresholds or authored classes should replace the first slope-strip geometry
  with retaining walls, cliff faces, or other later variants when the cut / fill case becomes too
  extreme for the simple slope solution

## Shared Target

### 1. Local Corridor / Pad Geometry Becomes Owner-Controlled

The longer-term shared target is for engineered-ground clients to own local tie-in geometry near
their footprints instead of asking the terrain heightfield to represent every cut and fill detail.

That means:

- top support surface remains client-owned
- side slopes, embankments, retaining faces, or local skirts become client-owned geometry
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

- once a road corridor or flat foundation pad is committed, later terrain brushes reshape authored
  ground and derived earthworks around that placed client
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
- add client-owned local corridor / pad geometry caches near engineered-ground footprints
- extend visible-surface queries to consider:
  1. client-owned top surface
  2. client-owned local earthwork geometry
  3. visual terrain
  4. source terrain only for terrain-only APIs

This means the long-term earthworks architecture may extend or rewrite the current terrain runtime,
but it must preserve the same client-ownership rules and must not regress back to a terrain-only
model near engineered-ground footprints.
