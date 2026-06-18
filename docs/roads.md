# Roads

## Purpose

This document is the live road-surface contract for Metrum Rise. It owns the shipped roadbed
runtime, editor preview / commit geometry parity, road-touched terrain integration, and the
guardrails that keep roads deterministic and performant.

Historical `ROAD-01` / `ROAD-02` hardcut notes are preserved in
[`archive/roads_hardcut_history_2026-05-31.md`](archive/roads_hardcut_history_2026-05-31.md).
That archive is reference-only; this file is authoritative.

This document does not own lane-routing policy, building frontage semantics, terrain storage
internals, or the shared engineered-ground model. Those are owned by
[`entrance_and_exit.md`](entrance_and_exit.md), [`terrain.md`](terrain.md), and
[`earthworks.md`](earthworks.md).

## Current Status

The roadbed rewrite is shipped for the current surface-road scope:

- the logical road graph owns connectivity, IDs, lanes, authored plan curves, and road class
- `RoadSurfaceSystem` owns the visible roadbed, query surface, terrain clip inputs, and chunk
  coverage
- road pieces are explicit `Span`, `Terminal`, `Bend`, and `JunctionN` visual carriers
- asphalt, curb / shoulder, sidewalk, markings, terrain clip loops, earthwork roots, and query
  support all derive from the same compiled piece ownership
- node pieces route through canonical rails, boolean ownership, height carriers, Spade
  triangulation, and validation
- grounded `Standard` road-touched terrain patches are generated in Rust with Spade CDT over
  unioned road-owned footprint loops
- ordinary grounded-road tie-ins use `RoadSurfaceSystem` generated grade-limited guide samples
  around the final unioned road-owned footprint; only a single clean non-hole convex footprint may
  constrain its guide rails, while holed, concave, or multi-loop footprint sets stay sample-only so
  guide rails cannot cross the final roadbed
- ordinary grounded-road tie-ins do not emit retaining-wall mesh as a visual cleanup path
- Godot is a thin input/render bridge: it uploads cached payloads and must not decide road
  topology, heights, terrain holes, or material ownership

The old centerline-lift, generic node-patch, seam-strip, and renderer-owned road-hole paths are
retired.

## Ownership Boundaries

### Logical Graph

The graph is authoritative for:

- node and edge identity
- route connectivity
- lane counts and modal permissions
- authored road class: `Standard`, `Bridge`, or `Tunnel`
- authored plan polyline control points in world XZ

The graph is not the final visible road surface, final terrain clip carrier, final earthwork
boundary, or final node polygon carrier.

### RoadSurfaceSystem

`RoadSurfaceSystem` owns:

- edge longitudinal grade solutions
- ordered edge sections and lateral band profiles
- visual piece classification
- compiled road top-surface meshes
- visible-world road queries and picking support
- terrain clip loops for grounded roads
- terrain-CDT grading-envelope guide samples / constraints derived from final roadbed loops
- road surface and terrain chunk coverage
- road debug geometry and provenance output

### TerrainSystem

`TerrainSystem` owns source terrain, visual terrain storage, chunk residency, and upload
boundaries. It consumes road-derived terrain clip / earthwork inputs; it must not decide road
heights or invent road seams.

### Godot

Godot-side road, terrain, and water scripts upload cached Rust payloads, bind materials, collect
input, and display debug data. They must not:

- resample terrain to repair road heights
- rebuild road topology from graph guesses
- clip road holes with `Geometry2D`
- mask missing topology with shader, water, zoning, material order, or background color
- perform expensive geometry decisions in editor input loops

## Roadbed Model

One authoritative roadbed model drives preview, committed mesh, visible-world picking, lane
marking anchors, terrain clip loops, and earthwork roots.

For every road-owned top surface:

- lateral width is explicit, not inferred from a centerline render offset
- each section stores center position, tangent, lateral axis, solved height, and ordered lateral
  bands
- band heights derive from the solved grade and explicit profile offsets
- preview and committed placement use the same Rust surface solve rules
- render triangles and query triangles use the same compiled ownership

The current required band model supports carriageway, curb / shoulder, sidewalk, and no-sidewalk
profiles. Later medians, parking lanes, cycle tracks, tram reservations, or richer shoulders must
add explicit ordered bands instead of special-case render offsets.

## Visual Pieces

### Span

`Span` owns ordinary edge corridors between node throats:

- span sections are ordered along the edge plan polyline
- every original edge knot and every node throat appears as a section
- long spans are refined by deterministic world-space constants owned in Rust code
- span output emits role-tagged asphalt, curb / shoulder, non-road, query, terrain clip, earthwork,
  and chunk coverage data

### Terminal

`Terminal` owns one-mouth endpoints:

- terminal asphalt follows the carriageway to the graph endpoint
- side non-road bands and end-band closure are explicit solved-band topology
- terminal caps are not generic annulus, disk, or endpoint helper geometry
- terminal output carries the same owner, height-field, provenance, and chunk-coverage contracts as
  other node pieces

### Bend And JunctionN

`Bend` and `JunctionN` share the node-region ownership model:

- `Bend` is a two-mouth node piece
- `JunctionN` is an `n >= 3` node piece
- mouths are sorted deterministically
- full-roadbed corridor candidates define the node footprint
- carriageway corridor candidates define asphalt
- `node_non_road = node_footprint - node_asphalt` is an intermediate domain only
- curb / shoulder and sidewalk are accepted only after explicit profile seam-rail evidence
- sidewalks may shrink, split, or disappear when asphalt legally owns the conflict region
- no non-road triangle may overlap asphalt or reconnect sidewalk islands across asphalt

The final node output is a canonical arrangement:

- every vertex has a canonical key, material owner, height-field owner, and grade authority
- every seam edge has explicit source / owner-pair provenance
- height is evaluated only after ownership and seam vertices are known
- missing, ambiguous, or conflicting owner / height support is a hard diagnostic, not a repair path
- Spade CDT triangulates already-owned material regions; CDT does not decide ownership

Node throat clips use the roadbed half-width plus a small numeric safety margin as their baseline
distance from the graph node, so ordinary orthogonal junction mouths stay close to the junction
centre without producing exact tangencies in node ownership. Acute-angle and near-parallel conflicts
may still expand the clip distance deterministically from incident roadbed widths and angular
separation. Same-width conflicts cap that expansion to a small multiplier of the incident roadbed
half-width so ordinary acute mouths do not become long flat platforms; mixed-width conflicts may
expand farther when needed for canonical side-join ownership. That expansion is geometry ownership,
not visual padding.
Edge span sampling, visible-surface section queries, and grounded `Standard` road earthwork ranges
must consume the same node-mouth ownership policy. Profile blending may use a shorter range than
ownership only for the explicit sparse grounded `Standard` `Bend` hard-pin case described below.

Incident `Bend` / `JunctionN` rails use a shared graph endpoint profile plane before section, span,
and node compilation. Profile distances are measured in horizontal XZ metres because the cap is a
road grade rule, not a 3-D polyline-length rule. The conservative solve may grade-limit only when
limiting stays close to the incident road samples; otherwise the original source-supported plane is
preserved.
For a true two-mouth `Bend`, that endpoint plane is a horizontal node-local anchor at the graph node
height. Sparse, two-control-point grounded `Standard` Bend edges force only the small width-scaled
profile hard pin exactly onto that plane. If the ownership mouth has to sit farther away to prevent
material overlap, the Bend mouth section samples the vertical-curve blend instead of extending the
horizontal platform all the way to that far handoff. Uphill or downhill change therefore starts
after the short hard pin and blends through the owned footprint / adjacent edge profile instead of
tilting the whole bend as one plane. Dense source-sampled, elevated, or structural Bend edges keep
exact mouth-height authority for their existing support-rich vertical-face contracts, and
`JunctionN` keeps the multi-mouth solve and mouth-authority contract described below.

Road-edit rebuilds preserve that conservative solve for the edited edge set, then use the stronger
profile path only when an affected `JunctionN` still solves to an over-limit platform, or when an
adaptable `Bend` leg still deviates from its horizontal platform at the profile control sample.
For any `JunctionN`, the endpoint profile solve first looks for deterministic opposite-mouth
authority corridors. Existing stable corridor mouths score above edited branch mouths, and the
highest-scored corridor owns the node base plane. Secondary opposite branch pairs must not rotate
that base plane when another road is added to the same `JunctionN`. When such a corridor exists,
the authority mouths keep their original edge profile and only non-authority branch mouths are
blended into that plane. When no compatible authority corridor exists, the node falls back to the
all-mouth least-squares plane. Any changed mouth is capped to the mouth-grade limit, uses a small
width-scaled hard mouth pin (`~1..2 m` for current standard roads), and blends back to source grade
over a bounded transition. The 12 m profile sample is only a stable solve/control sample, not a
visible hard platform extent; it may anchor the source profile plane, but physical road geometry
must receive the same vertical-curve blend as other post-mouth support points. Support vertices are
materialized at the solve sample and sparsely through the outer transition so later section sampling
reads a gradual source-grade transition instead of one large planar ramp back to raw terrain.
Visible road-surface sections inside an active profile fade use a denser transition cadence than
ordinary road spans, so the rendered asphalt approximates the vertical curve instead of exposing
long planar facets.
Incremental regrade receives the edit's adaptable edge set; adding one road must not make
already-stable incident roads adaptable simply because they touch the same junction. Section
sampling must also suppress sub-decimetre protected-handoff slivers so a support point cannot
create a visible near-vertical roadbed face immediately outside node ownership.

### `ROAD-04` Node Top-Surface Quality

`ROAD-04` is shipped for the current `Bend` / `JunctionN` road-owned top-surface path. It hardens
the Rust node triangulation and validation stages against visually harsh carriageway triangles
without moving ownership into terrain, Godot, or render-time repair.

The implemented contract:

- numeric-dust split / intersection vertices canonicalize only when the existing and incoming
  support have matching material owner, height-field owner, quantized height, and source provenance
- exact-key height conflicts and nearby conflicting height support remain blocking diagnostics
- sloped `Bend` and `JunctionN` carriageway regions may receive deterministic interior guide
  vertices before Spade CDT input; guides are inserted only inside the final road-owned region,
  require a verified road-owned grade plane from the region boundary, and carry canonical key /
  owner / height-field / grade authority
- road-edit junction profile solving first applies the conservative edited-edge fit; true
  two-mouth `Bend` nodes use a horizontal node-local platform, while affected `JunctionN` mouths
  solve through deterministic authority-corridor selection; stable opposite-mouth corridors are
  scored deterministically, the best corridor keeps the node base grade for the whole `JunctionN`,
  non-authority edited branches adapt into that plane, and no-compatible-corridor cases fall back
  to the all-mouth solve;
  this is the source grade solution, not a render-time repair, it must use horizontal profile
  distances, keep the hard mouth pin small, materialize the solve/control sample plus sparse outer
  vertical-curve support points, and update changed edge cost / length data before lane rebuild
- exposed final-footprint boundary endpoints may resolve a collapsed raised-step corner only when
  exact endpoint source edges prove an adjacent lower / raised material pair at that key; direct
  asphalt / sidewalk height conflicts remain blocking unless explicit step authority exists or both
  exact final-boundary endpoints carry source-intersection provenance for a collapsed generated
  mouth corner
- guide insertion never subdivides or moves final footprint boundaries, so terrain clip loops and
  earthwork provenance continue to consume the same road-owned footprint
- triangle numeric-area filtering uses actual triangle area, not double-area, against the shared
  overlay numeric threshold
- the post-CDT validation gate reports node ID, piece kind, material owner, source owner index,
  height field, triangle index, canonical keys, millimetre coordinates, edge lengths, area, aspect
  ratio, slope angle, adjacent normal angle, height delta, and local grade-plane residual
- near-zero, aspect-ratio, and slope failures block only when the triangle is large enough to be a
  visible top-surface problem; subvisual slivers remain bounded by the existing coverage and
  numeric-threshold validation

This remains a road-owned geometry rule. It must not be replaced with render z-bias, terrain
sampling, nearest-owner / nearest-height repair, min/max repair, averaging, old-road-wins priority,
or hidden compatibility fallback.

## Terrain And Earthworks

Grounded `Standard` roads replace visible terrain inside the road-owned footprint. Terrain under
asphalt, curb / shoulder, or sidewalk is not an independent visible carrier.

Road-touched terrain patches obey this contract:

- Rust unions grounded road-owned footprint loops for the patch before CDT input
- terrain patch rectangles, footprint loops, and source-terrain samples are inserted in canonical
  order
- road footprint constraints are clipped and split at patch boundaries before Spade input
- source-terrain samples are inserted only outside road-owned footprints
- Rust inserts deterministic grade-limited tie-in guide samples outside grounded `Standard`
  footprints before ordinary source terrain samples, using the final unioned roadbed loops rather
  than per-piece or Godot-side repair geometry
- constrained guide rails are allowed only for a single clean non-hole convex footprint; holed,
  concave `Bend` / `JunctionN`, multi-loop, or non-road client footprint sets leave those rails
  unconstrained, so guide constraints cannot cross through the road-owned footprint
- accepted terrain faces stay outside the unioned road-owned footprint
- rejected road-footprint faces are not emitted
- emitted terrain seam vertices reuse road-owned outer-edge coordinates and heights
- ordinary `Standard` span and node seam sources stay in the terrain bucket even when the authored
  terrain is steep; retaining-wall output is reserved for explicit structural bridge / tunnel /
  future retaining sources
- no shader mask, water plane, closure carpet, guard strip, or seam strip may replace missing
  topology

`Bridge` and `Tunnel` edges do not flatten or clip ordinary midspan terrain. Their terrain support
is limited to class-owned abutment or portal regions.

Structural earthwork stamping remains for explicit bridge, tunnel, retaining, and future
engineered-ground cases. Its current acceleration is chunk-local: prepared triangles are bucketed
inside dirty chunks, support candidates preserve the closest-distance / lower-height tie-break
rule, and writes are applied in canonical chunk order.

Shared engineered-ground rules live in [`earthworks.md`](earthworks.md).

Future building-site earthworks consume road-owned visible-surface/query heights for driveway or
frontage connections, but roads do not own lot grading, flat-site height selection, or authored
yard material regions.

## Preview, Query, And Editing

Editor preview must use the same road-surface solve rules as committed placement. Preview and
commit may differ in cache lifetime or display detail, but not in geometry ownership.

Visible-world queries use this precedence:

1. road-owned top surface
2. intentionally surfaced road earthwork / structure
3. visual terrain
4. source terrain only for terrain-only APIs

Road placement must not run heavy geometry synchronously from Godot mouse motion. Godot should
enqueue or poll Rust-owned preview results and keep input/render code thin.

Straight road edits should commit endpoint-only plan input. Curved edits may use deterministic
world-space sampling, but must preserve authored endpoints exactly. Oversampled straight Godot
`Curve3D` streams are not allowed to become semantic road input.

## Performance Contract

Correctness without acceptable performance is not done.

Required bounds:

- one local road edit rebuilds only touched spans, incident node pieces, and affected surface /
  terrain chunks
- dirty rebuilds use compiled piece coverage and `old_coverage union new_coverage`
- steady-state chunk rebuilds use sorted contributor lists; they must not scan every compiled node
  piece in the world
- independent span and chunk work should use Rayon when mutation boundaries allow it
- hot-path loops must avoid avoidable allocation
- road debug output must split terrain, water, zoning, and total patch-debug timings
- road debug output must use cached zoning statistics instead of scanning parcel payloads

The current remaining large editor hitch is synchronous final `JunctionN` compilation on very large
multi-mouth nodes. Future responsiveness work should be tracked as new road-performance work:

- async final compile with versioned jobs
- immutable compile snapshots
- deterministic latest-result publication
- old-mesh / pending-mesh visual state
- incremental `JunctionN` compile or stronger contact/export indexing

Do not reopen shipped `ROAD-01` geometry hardcuts for editor responsiveness unless the fix changes
the roadbed ownership contract itself.

## Debug And Diagnostics

`--debug road` must report enough data to locate ownership failures before rendering hides them:

- dirty edge / node / chunk counts
- node kind and incident mouth count
- rail, ownership, arrangement, triangulation, export, and validation timings
- contact candidate counts and emitted constraint counts
- terrain CDT input/output counters
- retained road seam constraint counters
- provider-specific terrain / water / zoning debug timings
- structured node diagnostics with stage, backend, owner, source band, height field, canonical key,
  point / edge, residual, seam, and constraint metadata

Missing source rails, missing carrier provenance, rejected residuals, open boundaries, duplicate
exposed edges, non-explicit boundary vertices, ambiguous carrier support, and height conflicts are
diagnostics. They must not become silent visual fallbacks.

## Forbidden Regressions

Do not reintroduce:

- centerline-only road lifting or terrain flattening
- terrain fallback sampling for road height
- nearest-height, nearest-owner, min/max, averaging, owner-priority, or old-road-wins repair
- render z-bias as geometry
- shader, water, zoning, material-order, cull-mode, or background masking for missing topology
- seam strips, closure carpets, guard strips, miter caps, or connector patches not derived from
  final canonical ownership
- paired adjacent-mouth strips as final node ownership
- generic node disks, annuli, halos, or global sidewalk rings
- Godot-side road topology, terrain clipping, or road-height decisions
- fallback from Spade CDT failure into old terrain clipping
- full-network road or terrain rebuilds for one local edit
- treating tiny boundary-touching missing material shapes as acceptable unless they are proven
  canonical numeric dust under the documented budget

`FinalTopBoundaryPair` is diagnostic only. It must not become an emitted road-surface vertical-face
source again.

## Test Contract

Maintained coverage must continue to prove:

- flat and cross-slope grounded spans
- bridge spans and tunnel portals
- terminal cap ownership
- bend ownership across acute, right, obtuse, shallow, and arbitrary angles
- T, 4-way, and `N > 4` `JunctionN` ownership
- mixed-width, mixed-profile, no-sidewalk, and one-sided footpath cases
- preview / commit parity
- visible-world query precedence
- terrain CDT preservation of road seam constraints
- terrain-CDT grading envelope behavior for convex and concave roadbed footprints
- rejection of terrain faces inside road-owned footprints
- authored and imported DEM terrain agreement
- deterministic rebuilds and equivalent edit-order identity
- local invalidation without unrelated chunk rebuilds
- rendered mesh upload containing the same canonical raised-step intervals as the compiled surface

Use the focused surface tests for narrow changes and the full `surface` suite when changing shared
ownership, terrain, query, node, or render contracts.

## Archive

The archived hardcut history remains useful when investigating why a repair path is forbidden or
why a particular provenance rule exists:

- [`archive/roads_hardcut_history_2026-05-31.md`](archive/roads_hardcut_history_2026-05-31.md)

Do not update the archive as live planning. Update this file, [`roadmap.md`](roadmap.md), and
[`project.md`](project.md) for current road status.
