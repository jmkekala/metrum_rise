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

`TerrainSystem` continues to own:

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

### 2. Terrain Density Is Still A Limitation

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

### 3. Current Runtime Compatibility Gap On Post-Placement Terrain Edits

The current runtime still violates the fixed-client rule for grounded roads.

Current compatibility gap:

- later terrain edits still resynchronize placed `Standard` road geometry against edited source
  terrain
- visual terrain is then restamped from that moved roadbed, so the road can shift instead of the
  terrain alone reshaping around it
- future building pads and foundations are not live yet, so their fixed-surface semantics are
  still a shared-target rule rather than a shipped behavior

### 4. Current Terrain Runtime Is A Compatible Base, Not The Final Visual Carrier

The current terrain runtime is sufficient for first-stage engineered ground:

- it keeps separate authoritative `source terrain` and derived `visual terrain`
- it supports chunk-local reset and restamp of touched visual regions
- it already accepts client-derived footprint and outer-margin earthwork inputs

But it is not sufficient as the final near-footprint visual carrier because the current visual
terrain remains a single-height field.

Current deterministic conclusion:

- do not replace `TerrainSystem` just to continue earthworks work
- do not expect the current visual terrain heightfield, even at a denser cell size, to represent
  all near-footprint cut / fill detail by itself
- treat future client-owned corridor or pad geometry as an additive layer above the current terrain
  runtime, not as a reason to rewrite terrain storage first

## Open Questions

The following questions remain intentionally open and still need concrete implementation decisions.

### 1. What Exact Local Geometry Should Own The Tie-In Near The Footprint?

The shared target now requires client-owned local geometry near roads and future pads, but the
exact runtime form is still open.

Open decision:

- whether the near-footprint layer should be implemented as corridor skirts, explicit slope meshes,
  retaining-face meshes, pad perimeter geometry, or some combination of those depending on the
  local cut / fill case

### 2. How Does The Fixed-Client Rule Land In Live Runtime?

The contract is now explicit: once a client is placed, later terrain edits must reshape terrain and
earthworks around it instead of moving the client implicitly.

Open decision:

- how placement-time grounding is separated from later terrain-authoring edits in the runtime so
  committed roads, and later foundations, stay fixed unless the player explicitly edits that client

### 3. When Do Buildings Become Live Engineered-Ground Clients?

Future flat building pads and foundations are already part of the shared target, but they are not
yet implemented as first-class engineered-ground clients.

Open decision:

- how building-pad support surfaces, footprint ownership, tie-in boundaries, and future retaining
  behavior integrate with allocator / zoning placement without inventing a second terrain-override
  model separate from roads

### 4. What Exact Cache And Query Shape Supports The Extra Geometry Layer?

The query precedence is already defined, but the runtime structure that serves it is still open.

Open decision:

- how client-owned local earthwork geometry is chunked, cached, rebuilt, and queried so that:
  - world-surface picking stays deterministic
  - chunk invalidation remains local
  - renderer upload does not regress back to whole-world rebuild behavior

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

### 4. Transition Path Extends `TerrainSystem` Rather Than Replacing It

The shared target is a layered extension of the current terrain runtime.

Required transition path:

- keep `source terrain` as the authoritative authored ground
- keep `visual terrain` as the far-field derived terrain buffer and renderer upload source
- split terrain and water render/upload work into chunk-local windows before treating denser
  terrain as a baseline runtime choice
- add client-owned local corridor / pad geometry caches near engineered-ground footprints
- extend visible-surface queries to consider:
  1. client-owned top surface
  2. client-owned local earthwork geometry
  3. visual terrain
  4. source terrain only for terrain-only APIs

This means the long-term earthworks architecture is a targeted ownership refactor around the
current terrain runtime, not a complete terrain-system replacement.
